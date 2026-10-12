//! The no-trap / no-call / bounded-memory audit, on the IR (before and after the mid-end)
//! and on the lowered machine code (Cranelift's own trap, relocation and call-site tables,
//! plus the lowered VCode listing). Safe Rust only.
//!
//! The machine-code side is what catches backend-inserted instructions: a CLIF `sdiv` with
//! explicit zero and `MIN / -1` branches in front of it still lowers to an x86 `idiv`
//! (which raises #DE on those inputs) and to Cranelift trap sites, so a region with a
//! runtime divisor is refused even though its branches make those paths unreachable.
use crate::experimental_core::region::abi::ExitRecord;
use crate::experimental_core::region::{EntryClass, Region, MAX_REGION_CELLS};
use cranelift_codegen::ir::{self, types, InstructionData, Opcode};
use cranelift_codegen::CompiledCode;
use std::format;
use std::prelude::rust_2021::*;

/// What the audit found that the P0 contract forbids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
	/// An IR opcode outside the P0 allowlist (calls, traps, global values, atomics, ...).
	Opcode(String),
	/// A load or store whose address is not a function parameter plus a constant offset
	/// naming exactly one permitted `buf` cell or record field (B1: per-cell and per-field
	/// permissions).
	Memory(String),
	/// An integer division or remainder survived to the IR given to the backend with a
	/// divisor that is not a constant outside {0, -1}.
	Division(String),
	/// The backend recorded trap sites (for example on `idiv`, or explicit `ud2`s).
	Traps(Vec<String>),
	/// The backend recorded relocations.
	Relocations(usize),
	/// The backend recorded call sites.
	Calls(usize),
	/// The lowered listing contains a forbidden mnemonic.
	Mnemonic(String),
}

/// What a passing audit saw (reported in diagnostics). Backend-inserted memory operands are
/// counted, not hidden: `rip_relative` are reads of the function's own read-only constant
/// pool (for example the magic multiplier of a division by a constant), `stack` are the
/// callee-saved register spills and reloads of the function's own frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
	pub(crate) code_bytes: usize,
	pub(crate) vcode_lines: usize,
	pub(crate) rip_relative: usize,
	pub(crate) stack: usize,
}

/// A load and a store permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Perm {
	pub(crate) load: bool,
	pub(crate) store: bool,
}

const NONE: Perm = Perm {
	load: false,
	store: false,
};

/// B1: the per-cell permissions of `buf` (cell `i` is bytes `[8i, 8i + 8)`), derived from
/// the region alone, the same region `lower::build` lowers. A slot cell is loadable iff it
/// is a guarded live-in (the entry block loads only those), storable iff some exit may
/// have written it (the exit blocks store only those); a guarded live-in that is also
/// written has both (the union). `next` is load and store; `end` is load only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AccessMap {
	buf: [Perm; MAX_REGION_CELLS],
	cells: usize,
}

impl AccessMap {
	pub(crate) fn of(region: &Region) -> Self {
		let n = region.slots.len().min(MAX_REGION_CELLS);
		let stored = region
			.exits
			.iter()
			.fold(0u64, |m, e| m | e.definite | e.maybe);
		let mut buf = [NONE; MAX_REGION_CELLS];
		for (l, cell) in buf.iter_mut().enumerate().take(n) {
			*cell = Perm {
				load: matches!(region.entry.get(l), Some(EntryClass::Guard(_))),
				store: l < 64 && stored & (1u64 << l) != 0,
			};
		}
		let mut cells = n;
		if region.iter.is_some() && n + 2 <= MAX_REGION_CELLS {
			buf[n] = Perm {
				load: true,
				store: true,
			};
			buf[n + 1] = Perm {
				load: true,
				store: false,
			};
			cells = n + 2;
		}
		Self { buf, cells }
	}

	/// The permission of cell `i` (none past the cells).
	pub(crate) fn cell(&self, i: usize) -> Perm {
		if i < self.cells {
			self.buf[i]
		} else {
			NONE
		}
	}

	/// Tests: `cells` cells with load and store each.
	#[cfg(test)]
	pub(crate) fn uniform_for_test(cells: usize) -> Self {
		let mut buf = [NONE; MAX_REGION_CELLS];
		for cell in buf.iter_mut().take(cells) {
			*cell = Perm {
				load: true,
				store: true,
			};
		}
		Self { buf, cells }
	}
}

/// The record fields: (offset, bytes, load, store). Native code loads only `fuel` and
/// stores every out field; `reserved` (36..40) is neither, and nothing else exists.
const RECORD: [(i64, i64, bool, bool); 7] = [
	(ExitRecord::FUEL as i64, 8, true, true),
	(ExitRecord::WRITTEN as i64, 8, false, true),
	(ExitRecord::BOOLS as i64, 8, false, true),
	(ExitRecord::STATUS as i64, 4, false, true),
	(ExitRecord::EXIT as i64, 4, false, true),
	(ExitRecord::PC as i64, 4, false, true),
	(ExitRecord::UNITS as i64, 8, false, true),
];

/// IR audit: opcodes from a fixed allowlist; every load/store addresses parameter 0
/// (`buf`) or parameter 1 (the record, [`ExitRecord::SIZE`] = 48 bytes since P2b) at a
/// constant offset. B1: a `buf` access is exactly one whole, aligned, 8-byte `I64` cell
/// with the opcode's permission in `map`; a record access is exactly one whole field (its
/// own width: 8 bytes for fuel and the masks, 4 for status, exit and pc) with the opcode's
/// permission; anything else (partial, unaligned, cross-cell, reserved or padding bytes,
/// past the end) is refused. Any `sdiv`/`srem`/`udiv`/`urem` must have a constant divisor
/// outside {0, -1}.
pub(crate) fn ir(func: &ir::Function, map: &AccessMap) -> Result<(), Finding> {
	let entry = func
		.layout
		.entry_block()
		.ok_or_else(|| Finding::Opcode("no entry block".into()))?;
	let params = func.dfg.block_params(entry);
	let (buf, rec) = (params[0], params[1]);
	let constant = |v: ir::Value| -> Option<i64> {
		let v = func.dfg.resolve_aliases(v);
		let inst = func.dfg.value_def(v).inst()?;
		match func.dfg.insts[inst] {
			InstructionData::UnaryImm {
				opcode: Opcode::Iconst,
				imm,
			} => Some(imm.bits()),
			_ => None,
		}
	};
	for block in func.layout.blocks() {
		for inst in func.layout.block_insts(block) {
			let data = &func.dfg.insts[inst];
			let opcode = data.opcode();
			match opcode {
				Opcode::Iconst
				| Opcode::Jump
				| Opcode::Brif
				| Opcode::Return
				| Opcode::Nop
				| Opcode::Icmp
				| Opcode::Iadd
				| Opcode::Isub
				| Opcode::Imul
				| Opcode::Ineg
				| Opcode::Iabs
				| Opcode::Smulhi
				| Opcode::Umulhi
				| Opcode::SaddOverflow
				| Opcode::SsubOverflow
				| Opcode::SmulOverflow
				| Opcode::Band
				| Opcode::Bor
				| Opcode::Bxor
				| Opcode::Bnot
				| Opcode::Ishl
				| Opcode::Ushr
				| Opcode::Sshr
				| Opcode::Uextend
				| Opcode::Sextend
				| Opcode::Ireduce
				| Opcode::Select
				| Opcode::Smin
				| Opcode::Smax
				| Opcode::Umin
				| Opcode::Umax => {}
				Opcode::Sdiv | Opcode::Srem | Opcode::Udiv | Opcode::Urem => {
					let args = func.dfg.inst_args(inst);
					let divisor = args.get(1).copied().and_then(constant);
					if !matches!(divisor, Some(c) if c != 0 && c != -1) {
						return Err(Finding::Division(format!(
							"{}",
							func.dfg.display_inst(inst)
						)));
					}
				}
				Opcode::Load | Opcode::Store => {
					let (base, offset, ty, flags) = match *data {
						InstructionData::Load {
							arg, offset, flags, ..
						} => {
							let ty = func.dfg.value_type(func.dfg.first_result(inst));
							(arg, i64::from(offset), ty, flags)
						}
						InstructionData::Store {
							args,
							offset,
							flags,
							..
						} => (
							args[1],
							i64::from(offset),
							func.dfg.value_type(args[0]),
							flags,
						),
						_ => {
							return Err(Finding::Memory(format!("{}", func.dfg.display_inst(inst))))
						}
					};
					// A trapping access would carry trap metadata into the machine code.
					if func.dfg.mem_flags[flags].trap_code().is_some() {
						return Err(Finding::Memory(format!("{}", func.dfg.display_inst(inst))));
					}
					let base = func.dfg.resolve_aliases(base);
					let size = i64::from(ty.bytes());
					let is_store = opcode == Opcode::Store;
					let allowed = if base == buf {
						// Exactly one whole, aligned 8-byte cell, with the opcode's permission.
						offset >= 0
							&& offset % 8 == 0 && size == 8
							&& ty == types::I64 && usize::try_from(offset / 8).is_ok_and(|i| {
							let p = map.cell(i);
							if is_store {
								p.store
							} else {
								p.load
							}
						})
					} else if base == rec {
						// Exactly one whole field, with the opcode's permission.
						RECORD.iter().any(|&(o, bytes, load, store)| {
							offset == o && size == bytes && if is_store { store } else { load }
						})
					} else {
						false
					};
					if !allowed {
						return Err(Finding::Memory(format!("{}", func.dfg.display_inst(inst))));
					}
				}
				other => return Err(Finding::Opcode(format!("{other}"))),
			}
		}
	}
	Ok(())
}

/// Machine audit from the backend's own tables plus the lowered listing.
pub(crate) fn machine(code: &CompiledCode, vcode: &str) -> Result<Report, Finding> {
	let traps = code.buffer.traps();
	if !traps.is_empty() {
		return Err(Finding::Traps(
			traps
				.iter()
				.map(|t| format!("{:#x} {}", t.offset, t.code))
				.collect(),
		));
	}
	if !code.buffer.relocs().is_empty() {
		return Err(Finding::Relocations(code.buffer.relocs().len()));
	}
	let calls = code.buffer.call_sites().count();
	if calls != 0 {
		return Err(Finding::Calls(calls));
	}
	if vcode.is_empty() {
		return Err(Finding::Mnemonic("no lowered listing".into()));
	}
	let mut rip_relative = 0;
	let mut stack = 0;
	for line in vcode.lines() {
		if line.contains("(%rip)") {
			rip_relative += 1;
			// Only a read of the constant pool is expected: never a store through %rip.
			if line.trim_start().starts_with("mov") && line.trim_end().ends_with("(%rip)") {
				return Err(Finding::Mnemonic(line.trim().into()));
			}
		}
		if line.contains("(%rsp)") || line.contains("(%rbp)") {
			stack += 1;
		}
		let lower = line.trim().to_ascii_lowercase();
		let mnemonic = lower.split_whitespace().next().unwrap_or("");
		let forbidden = mnemonic.starts_with("ud2")
			|| mnemonic.contains("trap")
			|| mnemonic.starts_with("call")
			|| mnemonic.starts_with("idiv")
			|| mnemonic.starts_with("div")
			|| mnemonic.contains("probestack")
			|| mnemonic.starts_with("jmp_unknown")
			|| mnemonic.starts_with("return_call");
		if forbidden {
			return Err(Finding::Mnemonic(line.trim().into()));
		}
	}
	Ok(Report {
		code_bytes: code.code_buffer().len(),
		vcode_lines: vcode.lines().count(),
		rip_relative,
		stack,
	})
}
