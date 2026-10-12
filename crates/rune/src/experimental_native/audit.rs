//! The no-trap / no-call / bounded-memory audit, on the IR (before and after the mid-end)
//! and on the lowered machine code (Cranelift's own trap, relocation and call-site tables,
//! plus the lowered VCode listing). Safe Rust only.
//!
//! The machine-code side is what catches backend-inserted instructions: a CLIF `sdiv` with
//! explicit zero and `MIN / -1` branches in front of it still lowers to an x86 `idiv`
//! (which raises #DE on those inputs) and to Cranelift trap sites, so a region with a
//! runtime divisor is refused even though its branches make those paths unreachable.
use crate::experimental_core::region::abi::ExitRecord;
use cranelift_codegen::ir::{self, InstructionData, Opcode};
use cranelift_codegen::CompiledCode;
use std::format;
use std::prelude::rust_2021::*;

/// What the audit found that the P0 contract forbids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
	/// An IR opcode outside the P0 allowlist (calls, traps, global values, atomics, ...).
	Opcode(String),
	/// A load or store whose address is not a function parameter plus an in-bounds
	/// constant offset.
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

/// What a passing audit saw (reported in receipts). Backend-inserted memory operands are
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

/// IR audit: opcodes from a fixed allowlist; every load/store addresses parameter 0
/// (`buf`, `n` 8-byte slots) or parameter 1 (the record) at a constant offset that keeps
/// the whole access in bounds; any `sdiv`/`srem`/`udiv`/`urem` must have a constant
/// divisor outside {0, -1}.
pub(crate) fn ir(func: &ir::Function, slots: usize) -> Result<(), Finding> {
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
					let limit = if base == buf {
						(slots * 8) as i64
					} else if base == rec {
						ExitRecord::SIZE as i64
					} else {
						-1
					};
					if offset < 0 || offset + size > limit {
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
