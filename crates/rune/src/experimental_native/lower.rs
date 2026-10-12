//! Region to Cranelift IR, then to machine code. Safe Rust only.
//!
//! The generated function mirrors the exit simulator node for node:
//!
//! - entry: load each guarded live-in from `buf[8 * i]`, zero the written-only slots, load
//!   the entry fuel from the record, zero the written, bool and (P2b) unit masks, jump to
//!   the header;
//! - charged node: `fuel == 0` branches to its `Fuel` exit, else `fuel -= 1`;
//! - Add/Sub/Mul: `s{add,sub,mul}_overflow`, overflow branches to the `Retry` exit;
//! - Div/Rem by an in-region constant outside {0, -1}: `sdiv`/`srem` by that constant
//!   (the mid-end turns these into multiply/shift sequences; the audit checks it did);
//! - Div/Rem by anything else: explicit zero and `MIN / -1` branches to `Retry`, then
//!   `sdiv`/`srem`, which the audit refuses because the backend still emits a trapping
//!   divide (see the audit findings);
//! - compare: `icmp` zero-extended to 0/1; branch: `brif` on the 0/1 value;
//! - P2b: a `Unit` literal (also `Release`) is the constant 0 tagged Unit; a `Copy` defines
//!   its out variable as the source variable, tagged with the source's static type;
//! - exit block: store every possibly-written slot to `buf`, then written mask, bool mask,
//!   unit mask (only when some tracked slot can hold a Unit; otherwise the record keeps its
//!   initial 0), fuel (0 for `Fuel`, `fuel + 1` for a `Retry` at a charged pc, the
//!   checkpoint for a B1 `Replay`), status, exit id and pc to the record, and return the
//!   status;
//! - B1 (only in regions that have them, so earlier regions lower to the same IR): the
//!   header `IterNext` over `next` (a variable loaded from `buf[n]`, stored back at every
//!   exit) and `end` (an SSA value loaded once from `buf[n + 1]`, never stored):
//!   `next >= end` takes the none edge, otherwise `out := next; next += 1` (no overflow
//!   branch: `next < end`); an inlined callee runs on its own variables (virtual locals
//!   `VIRTUAL_BASE..`), which never touch `buf`, a mask or a caller variable: `Enter` sets
//!   the checkpoint to the fuel and zeroes them, `Arg` copies a caller variable in, `Ret`
//!   defines the caller's `out` once.
//!
//! Loop-carried slots are `cranelift_frontend` variables, so they live in registers across
//! the back edge; memory is touched only at entry and exits.
use super::NativeRefusal;
use crate::experimental_core::arithmetic::Binary;
use crate::experimental_core::region::abi::ExitRecord;
use crate::experimental_core::region::{
	EntryClass, NodeOp, Region, Scalar, Status, Target, MAX_INLINE_SLOTS, MAX_REGION_CELLS, MAX_REGION_NODES,
	MAX_REGION_SLOTS, VIRTUAL_BASE,
};
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{
	self, condcodes::IntCC, types, AbiParam, Block, InstBuilder, MemFlagsData, Signature,
	UserFuncName,
};

use cranelift_codegen::isa::{self, CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use std::format;
use std::prelude::rust_2021::*;

/// The fixed P0 target: baseline x86-64 System V, no host CPU feature detection, so the
/// audited code is the same on every x86-64 host.
pub(crate) const TRIPLE: &str = "x86_64-unknown-linux-gnu";

/// The shared flags of every P0 compile (and of the boundary's JIT module).
pub(crate) fn isa() -> Result<OwnedTargetIsa, NativeRefusal> {
	let mut flags = settings::builder();
	for (name, value) in [
		("opt_level", "speed"),
		("enable_verifier", "true"),
		("is_pic", "false"),
		("enable_probestack", "false"),
		("unwind_info", "false"),
		("preserve_frame_pointers", "false"),
	] {
		flags.set(name, value).map_err(|_| NativeRefusal::Isa)?;
	}
	isa::lookup_by_name(TRIPLE)
		.map_err(|_| NativeRefusal::Isa)?
		.finish(settings::Flags::new(flags))
		.map_err(|_| NativeRefusal::Isa)
}

std::thread_local! {
	/// This thread's ISA, built once at first use and shared (an `Arc` clone) by every
	/// compile and every loaded module on the thread. Immutable after construction.
	static SHARED_ISA: core::cell::OnceCell<Result<OwnedTargetIsa, NativeRefusal>> =
		const { core::cell::OnceCell::new() };
}

/// The thread's shared ISA ([`isa`] built once). During thread teardown, when the
/// thread-local is gone, a fresh [`isa`] is built instead (same flags, same code).
pub(crate) fn shared_isa() -> Result<OwnedTargetIsa, NativeRefusal> {
	SHARED_ISA
		.try_with(|cell| cell.get_or_init(isa).clone())
		.unwrap_or_else(|_| isa())
}

/// `extern "C" fn(buf: *mut i64, record: *mut ExitRecord) -> u32`.
pub(crate) fn signature() -> Signature {
	let mut sig = Signature::new(CallConv::SystemV);
	sig.params.push(AbiParam::new(types::I64));
	sig.params.push(AbiParam::new(types::I64));
	sig.returns.push(AbiParam::new(types::I32));
	sig
}

/// Builds the region's IR. `Shape` if the region exceeds the ABI's slot, cell or node
/// bound (checked before anything is lowered).
pub(crate) fn build(region: &Region, isa: &OwnedTargetIsa) -> Result<ir::Function, NativeRefusal> {
	let n = region.slots.len();
	let cells = region.cells();
	if n > MAX_REGION_SLOTS
		|| cells > MAX_REGION_CELLS
		|| region.nodes.len() > MAX_REGION_NODES
		|| region.entry.len() != n
		|| region.nodes.is_empty()
	{
		return Err(NativeRefusal::Shape);
	}
	let mut func =
		ir::Function::with_name_signature(UserFuncName::user(0, region.header), signature());
	let mut fctx = FunctionBuilderContext::new();
	let mut b = FunctionBuilder::new(&mut func, &mut fctx);
	// Aligned and non-trapping: the audit requires every access to carry no trap code.
	let mem = MemFlagsData::trusted();
	let entry = b.create_block();
	b.append_block_params_for_function_params(entry);
	let nodes: Vec<Block> = region.nodes.iter().map(|_| b.create_block()).collect();
	let exits: Vec<Block> = region.exits.iter().map(|_| b.create_block()).collect();
	let block = |t: Target| match t {
		Target::Node(i) => nodes[i as usize],
		Target::Exit(e) => exits[e as usize],
	};

	b.switch_to_block(entry);
	let buf = b.block_params(entry)[0];
	let rec = b.block_params(entry)[1];
	let vars: Vec<Variable> = (0..n).map(|_| b.declare_var(types::I64)).collect();
	let fuel = b.declare_var(types::I64);
	let written = b.declare_var(types::I64);
	let bools = b.declare_var(types::I64);
	// P2b: the tracked slots some node writes as Unit. Only they can have a unit bit; when
	// there are none the unit mask is the constant 0 and is neither kept nor stored (the
	// regions P2a admitted lower to the same IR as before).
	let unit_tracked = region.nodes.iter().fold(0u64, |m, n| match n.op {
		NodeOp::Literal {
			out,
			ty: Scalar::Unit,
			..
		}
		| NodeOp::Copy {
			out,
			ty: Scalar::Unit,
			..
		}
		| NodeOp::Ret {
			out,
			ty: Scalar::Unit,
			..
		} if out < VIRTUAL_BASE => m | (1u64 << out),
		_ => m,
	}) & region.tracked;
	let units = b.declare_var(types::I64);
	for (l, class) in region.entry.iter().enumerate() {
		let v = match class {
			EntryClass::Guard(_) => b.ins().load(types::I64, mem, buf, slot_offset(l)),
			EntryClass::Scalar => b.ins().iconst(types::I64, 0),
		};
		b.def_var(vars[l], v);
	}
	// B1: the iterator's `next` (a variable) and `end` (one load), when there is one.
	let iter = if region.iter.is_some() {
		let next = b.declare_var(types::I64);
		let v = b.ins().load(types::I64, mem, buf, slot_offset(n));
		b.def_var(next, v);
		let end = b.ins().load(types::I64, mem, buf, slot_offset(n + 1));
		Some((next, end))
	} else {
		None
	};
	let f0 = b.ins().load(types::I64, mem, rec, ExitRecord::FUEL);
	b.def_var(fuel, f0);
	let zero = b.ins().iconst(types::I64, 0);
	b.def_var(written, zero);
	b.def_var(bools, zero);
	if unit_tracked != 0 {
		b.def_var(units, zero);
	}
	// B1: the virtual locals and the checkpoint, when the region inlines.
	let inlines = region.nodes.iter().any(|n| matches!(n.op, NodeOp::Enter { .. }));
	let (cvars, checkpoint) = if inlines {
		let cvars: Vec<Variable> = (0..MAX_INLINE_SLOTS).map(|_| b.declare_var(types::I64)).collect();
		for &c in &cvars {
			b.def_var(c, zero);
		}
		let checkpoint = b.declare_var(types::I64);
		b.def_var(checkpoint, zero);
		(cvars, Some(checkpoint))
	} else {
		(Vec::new(), None)
	};
	// A caller local (`l < n`) or a virtual local of an inlined callee.
	let var = |l: u32| -> Result<Variable, NativeRefusal> {
		if l >= VIRTUAL_BASE {
			cvars.get((l - VIRTUAL_BASE) as usize).copied().ok_or(NativeRefusal::Shape)
		} else {
			vars.get(l as usize).copied().ok_or(NativeRefusal::Shape)
		}
	};
	b.ins().jump(nodes[region.header_node as usize], &[]);

	for (i, node) in region.nodes.iter().enumerate() {
		b.switch_to_block(nodes[i]);
		if let Some(e) = node.charged {
			let f = b.use_var(fuel);
			let empty = b.ins().icmp_imm_s(IntCC::Equal, f, 0);
			let charged = b.create_block();
			b.ins().brif(empty, exits[e as usize], &[], charged, &[]);
			b.switch_to_block(charged);
			let f = b.ins().iadd_imm_s(f, -1);
			b.def_var(fuel, f);
		}
		let define = |b: &mut FunctionBuilder<'_>, out: u32, v: ir::Value, ty: Scalar| -> Result<(), NativeRefusal> {
			// A virtual local defines its variable only: no written bit, no tag, no cell.
			if out >= VIRTUAL_BASE {
				b.def_var(var(out)?, v);
				return Ok(());
			}
			b.def_var(var(out)?, v);
			let w = b.use_var(written);
			let w = b.ins().bor_imm_s(w, 1i64 << out);
			b.def_var(written, w);
			let bit = (1u64 << out) & region.tracked;
			if bit != 0 {
				let t = b.use_var(bools);
				let t = if ty == Scalar::Bool {
					b.ins().bor_imm_s(t, bit as i64)
				} else {
					b.ins().band_imm_s(t, !bit as i64)
				};
				b.def_var(bools, t);
			}
			let bit = bit & unit_tracked;
			if bit != 0 {
				let u = b.use_var(units);
				let u = if ty == Scalar::Unit {
					b.ins().bor_imm_s(u, bit as i64)
				} else {
					b.ins().band_imm_s(u, !bit as i64)
				};
				b.def_var(units, u);
			}
			Ok(())
		};
		match node.op {
			NodeOp::Literal { out, value, ty } => {
				let v = b.ins().iconst(types::I64, value);
				define(&mut b, out, v, ty)?;
			}
			NodeOp::Copy { out, src, ty } => {
				let v = b.use_var(var(src)?);
				define(&mut b, out, v, ty)?;
			}
			NodeOp::Compare { out, op, lhs, rhs } => {
				let (x, y) = (b.use_var(var(lhs)?), b.use_var(var(rhs)?));
				let c = b.ins().icmp(condition(op)?, x, y);
				let v = b.ins().uextend(types::I64, c);
				define(&mut b, out, v, Scalar::Bool)?;
			}
			NodeOp::Arith {
				out,
				op,
				lhs,
				rhs,
				retry,
				divisor,
			} => {
				let (x, y) = (b.use_var(var(lhs)?), b.use_var(var(rhs)?));
				let guard = |b: &mut FunctionBuilder<'_>, fault: ir::Value| {
					let Some(e) = retry else {
						return Err(NativeRefusal::Shape);
					};
					let ok = b.create_block();
					b.ins().brif(fault, exits[e as usize], &[], ok, &[]);
					b.switch_to_block(ok);
					Ok(())
				};
				let v = match (op, divisor) {
					(Binary::Add | Binary::Sub | Binary::Mul, _) => {
						let (v, of) = match op {
							Binary::Add => b.ins().sadd_overflow(x, y),
							Binary::Sub => b.ins().ssub_overflow(x, y),
							_ => b.ins().smul_overflow(x, y),
						};
						guard(&mut b, of)?;
						v
					}
					(Binary::Div | Binary::Rem, Some(c)) => {
						if c == 0 || c == -1 || retry.is_some() {
							return Err(NativeRefusal::Shape);
						}
						let d = b.ins().iconst(types::I64, c);
						if op == Binary::Div {
							b.ins().sdiv(x, d)
						} else {
							b.ins().srem(x, d)
						}
					}
					(Binary::Div | Binary::Rem, None) => {
						let zero = b.ins().icmp_imm_s(IntCC::Equal, y, 0);
						guard(&mut b, zero)?;
						let minus_one = b.ins().icmp_imm_s(IntCC::Equal, y, -1);
						let min = b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
						let overflow = b.ins().band(minus_one, min);
						guard(&mut b, overflow)?;
						if op == Binary::Div {
							b.ins().sdiv(x, y)
						} else {
							b.ins().srem(x, y)
						}
					}
					_ => return Err(NativeRefusal::Shape),
				};
				define(&mut b, out, v, Scalar::I64)?;
			}
			NodeOp::Branch { condition, yes, no } => {
				let c = b.use_var(var(condition)?);
				b.ins().brif(c, block(yes), &[], block(no), &[]);
			}
			NodeOp::Jump { target } => {
				b.ins().jump(block(target), &[]);
			}
			NodeOp::IterNext { out, none } => {
				let Some((next, end)) = iter else {
					return Err(NativeRefusal::Shape);
				};
				let x = b.use_var(next);
				let done = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, x, end);
				let some = b.create_block();
				b.ins().brif(done, block(none), &[], some, &[]);
				b.switch_to_block(some);
				define(&mut b, out, x, Scalar::I64)?;
				let after = b.ins().iadd_imm_s(x, 1);
				b.def_var(next, after);
			}
			NodeOp::Enter { slots, .. } => {
				let Some(checkpoint) = checkpoint else {
					return Err(NativeRefusal::Shape);
				};
				if slots as usize > MAX_INLINE_SLOTS {
					return Err(NativeRefusal::Shape);
				}
				let f = b.use_var(fuel);
				b.def_var(checkpoint, f);
				let zero = b.ins().iconst(types::I64, 0);
				for &c in &cvars[..slots as usize] {
					b.def_var(c, zero);
				}
			}
			NodeOp::Arg { param, src } => {
				if src >= VIRTUAL_BASE {
					return Err(NativeRefusal::Shape);
				}
				let v = b.use_var(var(src)?);
				b.def_var(var(VIRTUAL_BASE + param)?, v);
			}
			NodeOp::Ret { out, src, ty } => {
				if out >= VIRTUAL_BASE || src < VIRTUAL_BASE {
					return Err(NativeRefusal::Shape);
				}
				let v = b.use_var(var(src)?);
				define(&mut b, out, v, ty)?;
			}
			// B1 test protocol: route on the checkpoint to the site's real Replay exits.
			#[cfg(test)]
			NodeOp::ForceReplay { zero, positive } => {
				let Some(checkpoint) = checkpoint else {
					return Err(NativeRefusal::Shape);
				};
				let c = b.use_var(checkpoint);
				let empty = b.ins().icmp_imm_s(IntCC::Equal, c, 0);
				b.ins().brif(empty, block(zero), &[], block(positive), &[]);
			}
		}
		if let Some(next) = node.next {
			b.ins().jump(block(next), &[]);
		}
	}

	for (e, exit) in region.exits.iter().enumerate() {
		b.switch_to_block(exits[e]);
		let possibly = exit.definite | exit.maybe;
		for (l, &var) in vars.iter().enumerate() {
			if possibly & (1 << l) != 0 {
				let v = b.use_var(var);
				b.ins().store(mem, v, buf, slot_offset(l));
			}
		}
		// B1: `next` on every exit; `end` never.
		if let Some((next, _)) = iter {
			let v = b.use_var(next);
			b.ins().store(mem, v, buf, slot_offset(n));
		}
		let w = b.use_var(written);
		b.ins().store(mem, w, rec, ExitRecord::WRITTEN);
		let t = b.use_var(bools);
		b.ins().store(mem, t, rec, ExitRecord::BOOLS);
		if unit_tracked != 0 {
			let u = b.use_var(units);
			b.ins().store(mem, u, rec, ExitRecord::UNITS);
		}
		let f = match exit.status {
			Status::Fuel => b.ins().iconst(types::I64, 0),
			Status::Retry if exit.restore => {
				let f = b.use_var(fuel);
				b.ins().iadd_imm_s(f, 1)
			}
			// B1: the fuel immediately before the Call.
			Status::Replay => {
				let Some(checkpoint) = checkpoint else {
					return Err(NativeRefusal::Shape);
				};
				b.use_var(checkpoint)
			}
			_ => b.use_var(fuel),
		};
		b.ins().store(mem, f, rec, ExitRecord::FUEL);
		let status = b.ins().iconst(types::I32, exit.status as u32 as i64);
		b.ins().store(mem, status, rec, ExitRecord::STATUS);
		let id = b.ins().iconst(types::I32, e as i64);
		b.ins().store(mem, id, rec, ExitRecord::EXIT);
		let pc = b.ins().iconst(types::I32, i64::from(exit.pc));
		b.ins().store(mem, pc, rec, ExitRecord::PC);
		b.ins().return_(&[status]);
	}
	b.seal_all_blocks();
	b.finalize(isa.frontend_config());
	Ok(func)
}

fn slot_offset(l: usize) -> i32 {
	// l < MAX_REGION_CELLS (66), so the offset is below 528.
	(l * 8) as i32
}

fn condition(op: Binary) -> Result<IntCC, NativeRefusal> {
	Ok(match op {
		Binary::Eq => IntCC::Equal,
		Binary::Ne => IntCC::NotEqual,
		Binary::Lt => IntCC::SignedLessThan,
		Binary::Le => IntCC::SignedLessThanOrEqual,
		Binary::Gt => IntCC::SignedGreaterThan,
		Binary::Ge => IntCC::SignedGreaterThanOrEqual,
		_ => return Err(NativeRefusal::Shape),
	})
}

/// The compiled and audited machine code of one region: the only input
/// [`NativeRegion::load`](super::boundary::NativeRegion::load) accepts.
///
/// The fields are private and the only constructor is [`compile`], which always runs the IR
/// gate before and after the mid-end and the machine gate. Code outside this module can
/// inspect a value but cannot build or change one, so a `Compiled` is evidence that every
/// gate passed for exactly these bytes.
///
/// A production `Compiled` holds the bytes and two numbers only: no listing, no text. The
/// [`Listing`] exists in test builds only (receipts and diagnostics).
pub(crate) struct Compiled {
	bytes: Vec<u8>,
	/// Required code alignment (from Cranelift's function alignment).
	alignment: u64,
	/// The `buf` length the code was compiled for (the region's cells).
	cells: usize,
	#[cfg(test)]
	listing: Listing,
}

/// Textual IR before and after the mid-end, the lowered VCode listing and the machine
/// audit report. Diagnostic only (test builds only): it cannot be loaded.
#[cfg(test)]
pub(crate) struct Listing {
	pub(crate) clif: String,
	pub(crate) optimized: String,
	pub(crate) vcode: String,
	pub(crate) report: super::audit::Report,
}

impl Compiled {
	pub(crate) fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	pub(crate) fn alignment(&self) -> u64 {
		self.alignment
	}

	pub(crate) fn cells(&self) -> usize {
		self.cells
	}

	#[cfg(test)]
	pub(crate) fn listing(&self) -> &Listing {
		&self.listing
	}
}

/// Which gates a lowering runs. Only [`Gates::All`] can yield a loadable [`Compiled`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gates {
	All,
	/// Test diagnostics: the machine gate only.
	#[cfg(test)]
	MachineOnly,
}

/// Builds, verifies, compiles and audits `region` with every gate. Any `Err` is an
/// interpreter fallback. `program_bytes` is the code already compiled for the same program
/// (the cache bound).
pub(crate) fn compile(region: &Region, program_bytes: usize) -> Result<Compiled, NativeRefusal> {
	#[cfg_attr(not(test), allow(unused_variables))]
	let (bytes, alignment, listing) = lower(region, program_bytes, Gates::All)?;
	Ok(Compiled {
		bytes,
		alignment,
		cells: region.cells(),
		#[cfg(test)]
		listing,
	})
}

/// What a lowering retains besides the bytes: the [`Listing`] in test builds, nothing in
/// production.
#[cfg(test)]
type Diagnostics = Listing;
#[cfg(not(test))]
type Diagnostics = ();

/// As [`compile`] without the IR gates, so tests can show what the backend itself emits for
/// IR the gate would refuse (the machine audit still runs). It returns only the
/// non-loadable [`Listing`].
#[cfg(test)]
pub(crate) fn diagnose_machine_only(region: &Region) -> Result<Listing, NativeRefusal> {
	Ok(lower(region, 0, Gates::MachineOnly)?.2)
}

/// P3a agreement tests: every gate, with a FRESH ISA (`fresh`) or the thread's shared one.
/// Returns the bytes, alignment and listing; never a loadable [`Compiled`].
#[cfg(test)]
pub(crate) fn diagnose_isa(region: &Region, fresh: bool) -> Result<(Vec<u8>, u64, Listing), NativeRefusal> {
	let isa = if fresh { isa()? } else { shared_isa()? };
	lower_with(region, 0, Gates::All, &isa)
}

fn lower(
	region: &Region,
	program_bytes: usize,
	gates: Gates,
) -> Result<(Vec<u8>, u64, Diagnostics), NativeRefusal> {
	lower_with(region, program_bytes, gates, &shared_isa()?)
}

/// Every gate runs in every build. Compiler and audit transient memory (Cranelift's
/// compilation heap, the VCode text, the audit's temporaries) is NOT charged to the Rune
/// analysis budget and is not bounded by Rune. The VCode text is required by the machine
/// audit, so it is always created; it is taken out of the compiled code (no copy) and, in
/// production, dropped right after the audit. The CLIF texts are created in test builds
/// only.
fn lower_with(
	region: &Region,
	program_bytes: usize,
	gates: Gates,
	isa: &OwnedTargetIsa,
) -> Result<(Vec<u8>, u64, Diagnostics), NativeRefusal> {
	let ir_gate = gates == Gates::All;
	let func = build(region, isa)?;
	cranelift_codegen::verify_function(&func, &**isa)
		.map_err(|e| NativeRefusal::Verifier(e.to_string()))?;

	let map = super::audit::AccessMap::of(region);
	if ir_gate {
		super::audit::ir(&func, &map).map_err(NativeRefusal::Audit)?;
	}

	#[cfg(test)]
	let clif = func.display().to_string();

	let mut ctx = Context::for_function(func);
	ctx.set_disasm(true);
	ctx.compile(&**isa, &mut ControlPlane::default())
		.map_err(|e| NativeRefusal::Codegen(format!("{:?}", e.inner)))?;
	let mut code = ctx
		.take_compiled_code()
		.ok_or_else(|| NativeRefusal::Codegen("no compiled code".into()))?;
	let bytes = code.code_buffer().to_vec();
	let vcode = code.vcode.take().unwrap_or_default();
	let alignment = u64::from(code.buffer.alignment);
	let report = super::audit::machine(&code, &vcode).map_err(NativeRefusal::Audit)?;
	drop(code);

	#[cfg(not(test))]
	drop((vcode, report));

	// The optimized IR is audited too: the mid-end must not have introduced anything the
	// contract forbids.
	if ir_gate {
		super::audit::ir(&ctx.func, &map).map_err(NativeRefusal::Audit)?;
	}

	#[cfg(test)]
	let optimized = ctx.func.display().to_string();

	drop(ctx);

	if bytes.len() > super::MAX_REGION_CODE_BYTES
		|| program_bytes.saturating_add(bytes.len()) > super::MAX_PROGRAM_CODE_BYTES
	{
		return Err(NativeRefusal::Size);
	}

	#[cfg(test)]
	let diagnostics = Listing {
		clif,
		optimized,
		vcode,
		report,
	};

	#[cfg(not(test))]
	let diagnostics = ();

	Ok((bytes, alignment, diagnostics))
}
