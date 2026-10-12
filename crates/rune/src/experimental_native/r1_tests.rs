//! R1 (rnx 0184 calls study): the header IterNext's `out` is recomputed at every exit as
//! `select(written & bit, next - 1, 0)` instead of being carried around the loop.
//!
//! Tests-first: `r1_iter_out_exit_stores_are_recomputed` fails at 0765cce6 (every exit
//! stores the variable, a header block parameter at the header exits) and passes with R1.
//! The compile-only controls (`r1_ineligible_regions_keep_the_variable_store`), the
//! lowerer's entry value (`r1_out_entry_value_is_the_iconst_zero`) and the executing
//! whole-buffer differential (`r1_full_buffer_byte_equal_every_fuel`, child process) pass
//! before and after: R1 changes no stored byte.
//!
//! Canonical entry: the native and simulated runs here start from buffers that REPRODUCE
//! the commit module's entry fill (`unbox`: a guarded live-in's payload, 0 in every `Scalar`
//! cell, then `next` and `end`). That the real fill writes 0 into the `out` cell is proven
//! by `experimental_core::region::commit::r1_tests`; the simulator does not initialise
//! `Scalar` cells (it consumes the buffer it is given).
//!
//! Whole-buffer equality here is scratch equality of native code against the simulator
//! (every cell, written or not, plus the whole record and the status). Production state
//! equivalence (the interpreter's observable state after commit) is the existing
//! three-mode differential (`b1_tests`), which runs unchanged.
use super::audit::Finding;
use super::boundary::NativeRegion;
use super::p2_tests::V2_CALLS;
use super::w13_tests::{corpus_regions, kernel};
use super::{lower, NativeRefusal};
use crate::experimental_core::region::{
	abi::ExitRecord, simulate::simulate, EntryClass, NodeOp, Region, Scalar, Status, VIRTUAL_BASE,
};
use cranelift_codegen::ir::{self, InstructionData, Opcode};
use std::collections::BTreeMap;
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_R1_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(1200);

fn run_child(inner: &str) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::r1_tests::{inner}");
	let started = Instant::now();
	let mut child = Command::new(exe)
		.args([
			"--exact",
			name.as_str(),
			"--ignored",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD_ENV, "1")
		.env_remove("PERF_BUILDID_DIR")
		.stdin(Stdio::null())
		.spawn()
		.expect("spawn child");

	let status = loop {
		if let Some(status) = child.try_wait().expect("wait child") {
			break status;
		}
		if started.elapsed() > DEADLINE {
			let _ = child.kill();
			let status = child.wait().expect("reap child");
			std::panic!(
				"{inner}: TIMEOUT after {:?}, killed (status {status:?}, signal {:?})",
				DEADLINE,
				status.signal()
			);
		}
		std::thread::sleep(Duration::from_millis(20));
	};

	println!(
		"CHILD\t{inner}\tstatus={status:?}\tsignal={:?}\twall_ms={}",
		status.signal(),
		started.elapsed().as_millis()
	);
	assert_eq!(status.signal(), None, "{inner}: child killed by a signal");
	assert!(status.success(), "{inner}: child failed ({status:?})");
}

fn require_child() {
	assert!(
		std::env::var_os(CHILD_ENV).is_some(),
		"R1 native worker tests run only in the child process started by their parent test"
	);
}

/// Whether a node OTHER than the header disqualifies `out`, by the test's own exhaustive
/// classification of every `NodeOp` (no wildcard, independent of `lower`): any further
/// IterNext (whatever its `out`: it would redefine `next`), or any node whose lowering
/// defines `out`'s variable (a W3 Branch edge redefines its condition's variable; `Enter`
/// and `Arg` define virtual locals only).
fn disqualifies(op: &NodeOp, out: u32) -> bool {
	match *op {
		NodeOp::IterNext { .. } => true,
		NodeOp::Enter { slots, .. } => out >= VIRTUAL_BASE && out - VIRTUAL_BASE < slots,
		NodeOp::Arg { param, .. } => VIRTUAL_BASE.checked_add(param) == Some(out),
		NodeOp::Ret { out: o, .. } => o == out,
		NodeOp::ForceReplay { .. } => false,
		NodeOp::Literal { out: o, .. } => o == out,
		NodeOp::Copy { out: o, .. } => o == out,
		NodeOp::Arith { out: o, .. } => o == out,
		NodeOp::Compare { out: o, .. } => o == out,
		NodeOp::Branch { condition, .. } => condition == out,
		NodeOp::Jump { .. } => false,
	}
}

/// The test's own statement of R1's precondition: an iterator; the header is
/// `IterNext { out }`; `out` is a caller local below 64 with entry class `Scalar`; and no
/// other node disqualifies it.
fn eligible(region: &Region) -> Option<u32> {
	region.iter?;
	let header = region.header_node as usize;
	let NodeOp::IterNext { out, .. } = region.nodes.get(header)?.op else {
		return None;
	};
	if out >= VIRTUAL_BASE || out >= 64 || region.entry.get(out as usize) != Some(&EntryClass::Scalar) {
		return None;
	}
	let other = region
		.nodes
		.iter()
		.enumerate()
		.any(|(i, n)| i != header && disqualifies(&n.op, out));
	(!other).then_some(out)
}

/// The defining instruction's data of `v`, if `v` is an instruction result.
fn def(func: &ir::Function, v: ir::Value) -> Option<InstructionData> {
	func.dfg.value_def(func.dfg.resolve_aliases(v)).inst().map(|i| func.dfg.insts[i])
}

/// The constant `v` is, when an `iconst` in `block` defines it.
fn iconst_in(func: &ir::Function, v: ir::Value, block: ir::Block) -> Option<i64> {
	let v = func.dfg.resolve_aliases(v);
	let inst = func.dfg.value_def(v).inst()?;
	if func.layout.inst_block(inst) != Some(block) {
		return None;
	}
	match func.dfg.insts[inst] {
		InstructionData::UnaryImm {
			opcode: Opcode::Iconst,
			imm,
		} => Some(imm.bits()),
		_ => None,
	}
}

/// One store of `out`'s `buf` cell.
struct OutStore {
	value: ir::Value,
	/// The header block parameter index, when the stored value is one.
	header_param: Option<usize>,
	opcode: Option<Opcode>,
}

/// Every store to local `out`'s `buf` cell in `func`.
fn out_stores(func: &ir::Function, out: u32) -> Vec<OutStore> {
	let entry = func.layout.entry_block().unwrap();
	let buf = func.dfg.block_params(entry)[0];
	let header = func.block_successors(entry).next().unwrap();
	let params = func.dfg.block_params(header);
	let mut found = Vec::new();
	for block in func.layout.blocks() {
		for inst in func.layout.block_insts(block) {
			let InstructionData::Store {
				opcode: Opcode::Store,
				args,
				offset,
				..
			} = func.dfg.insts[inst]
			else {
				continue;
			};
			if func.dfg.resolve_aliases(args[1]) != buf || i64::from(offset) != 8 * i64::from(out) {
				continue;
			}
			let value = func.dfg.resolve_aliases(args[0]);
			found.push(OutStore {
				value,
				header_param: params.iter().position(|&p| p == value),
				opcode: def(func, value).map(|d| d.opcode()),
			});
		}
	}
	found
}

/// The entry block's jump argument for header parameter `i`.
fn entry_arg(func: &ir::Function, i: usize) -> ir::Value {
	let entry = func.layout.entry_block().unwrap();
	let last = func.layout.last_inst(entry).unwrap();
	let InstructionData::Jump { destination, .. } = func.dfg.insts[last] else {
		std::panic!("the entry block does not end in a jump");
	};
	let arg = destination.args(&func.dfg.value_lists).nth(i).unwrap();
	arg.as_value().expect("a value argument")
}

/// What the entry block passes a header parameter that an exit stores into `out`'s cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryArg {
	/// An `iconst 0` of the entry block: `out`'s own variable, carried from its `Scalar`
	/// init.
	Zero,
	/// The load of `buf[out]`: `out`'s own variable, carried from a `Guard` live-in.
	OutLoad,
	/// The load of `buf[n]`: `next`'s variable (IterNext's `x`, stored as `out` at exits
	/// after the IterNext and before the back edge, e.g. the Replay exits).
	NextLoad,
	Other,
}

fn entry_arg_kind(func: &ir::Function, i: usize, out: u32, n: usize) -> EntryArg {
	let entry = func.layout.entry_block().unwrap();
	let buf = func.dfg.block_params(entry)[0];
	let arg = entry_arg(func, i);
	if iconst_in(func, arg, entry) == Some(0) {
		return EntryArg::Zero;
	}
	match def(func, arg) {
		Some(InstructionData::Load { arg: base, offset, .. }) if func.dfg.resolve_aliases(base) == buf => {
			let offset = i64::from(offset);
			if offset == 8 * i64::from(out) {
				EntryArg::OutLoad
			} else if offset == 8 * n as i64 {
				EntryArg::NextLoad
			} else {
				EntryArg::Other
			}
		}
		_ => EntryArg::Other,
	}
}

/// R1's exit value, structurally: `select(band(w, bit), iadd(x, iconst -1), zero)` where
/// `zero` is an `iconst 0` of the entry block. The decrement is a plain (wrapping,
/// non-trapping) `iadd` of -1: no checked subtraction, no trap. Returns the `bit`.
fn select_shape(func: &ir::Function, v: ir::Value) -> Result<i64, String> {
	let entry = func.layout.entry_block().unwrap();
	let Some(InstructionData::Ternary {
		opcode: Opcode::Select,
		args,
	}) = def(func, v)
	else {
		return Err(format!("{v} is not a select"));
	};
	let Some(InstructionData::Binary {
		opcode: Opcode::Band,
		args: band,
	}) = def(func, args[0])
	else {
		return Err(format!("{v}: condition is not a band"));
	};
	let any_block = |x: ir::Value| {
		let x = func.dfg.resolve_aliases(x);
		func.dfg.value_def(x).inst().and_then(|i| match func.dfg.insts[i] {
			InstructionData::UnaryImm {
				opcode: Opcode::Iconst,
				imm,
			} => Some(imm.bits()),
			_ => None,
		})
	};
	let Some(bit) = any_block(band[1]) else {
		return Err(format!("{v}: band mask is not a constant"));
	};
	let Some(InstructionData::Binary {
		opcode: Opcode::Iadd,
		args: add,
	}) = def(func, args[1])
	else {
		return Err(format!("{v}: true arm is not an iadd"));
	};
	if any_block(add[1]) != Some(-1) {
		return Err(format!("{v}: true arm does not add -1"));
	}
	if iconst_in(func, args[2], entry) != Some(0) {
		return Err(format!("{v}: false arm is not the entry block's iconst 0"));
	}
	Ok(bit)
}

/// Every eligible region: the calls kernel plus the eligible corpus regions.
fn eligible_regions() -> Vec<(String, Region, u32)> {
	let calls = kernel(V2_CALLS);
	assert!(eligible(&calls).is_some(), "the calls kernel is eligible");
	let mut regions = vec![(String::from("calls kernel"), calls)];
	regions.extend(corpus_regions());
	regions
		.into_iter()
		.filter_map(|(name, region)| eligible(&region).map(|out| (name, region, out)))
		.collect()
}

/// R1 applies to the calls kernel and to every eligible corpus region: every exit store of
/// `out` is R1's `select` (structurally, `select_shape`, with the bit `1 << out`), never a
/// header block parameter. FAILS at 0765cce6.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn r1_iter_out_exit_stores_are_recomputed() {
	let isa = lower::shared_isa().unwrap();
	let (mut checked, mut stores) = (0, 0);
	for (name, region, out) in eligible_regions() {
		let func = lower::build(&region, &isa).unwrap();
		let found = out_stores(&func, out);
		assert!(!found.is_empty(), "{name}: no exit stores `out`\n{}", func.display());
		for s in found {
			assert_eq!(s.header_param, None, "{name}: an exit stores header parameter {} as `out`\n{}", s.value, func.display());
			assert_eq!(s.opcode, Some(Opcode::Select), "{name}: `out` stored from {}\n{}", s.value, func.display());
			let bit = select_shape(&func, s.value).unwrap_or_else(|e| std::panic!("{name}: {e}\n{}", func.display()));
			assert_eq!(bit, 1i64 << out, "{name}");
			stores += 1;
		}
		checked += 1;
	}
	println!("R1\texit_stores\tregions={checked}\tstores={stores}");
	assert!(checked > 0);
}

/// Ineligible mutations of the calls kernel. Compile-only: they are fed to `lower::build`
/// and never loaded or executed (analysis would not produce them).
fn controls() -> Vec<(&'static str, Region, u32)> {
	let base = kernel(V2_CALLS);
	let out = eligible(&base).unwrap();
	let header = base.nodes[base.header_node as usize];
	let NodeOp::IterNext { none, .. } = header.op else {
		unreachable!()
	};

	// `out` re-classed as a live-in.
	let mut guarded = kernel(V2_CALLS);
	guarded.entry[out as usize] = EntryClass::Guard(Scalar::I64);

	// The kernel's Literal retargeted to `out`: a second writer.
	let mut rewritten = kernel(V2_CALLS);
	let literal = rewritten
		.nodes
		.iter_mut()
		.find(|n| matches!(n.op, NodeOp::Literal { .. }))
		.expect("the calls kernel has a Literal node");
	let other = match &mut literal.op {
		NodeOp::Literal { out: o, .. } => {
			let other = *o;
			*o = out;
			other
		}
		_ => unreachable!(),
	};
	assert_ne!(other, out);

	// The same Literal replaced by a second IterNext whose `out` DIFFERS from the header's
	// (it writes the Literal's local, not `out`): it would redefine `next`.
	let mut second = kernel(V2_CALLS);
	let at = second
		.nodes
		.iter()
		.position(|n| matches!(n.op, NodeOp::Literal { .. }))
		.unwrap();
	second.nodes[at].op = NodeOp::IterNext { out: other, none };

	vec![("guard", guarded, out), ("second_writer", rewritten, out), ("second_iter_next", second, out)]
}

/// Negative controls (compile-only): a Guard-class `out`, a second writer of `out` and a
/// second IterNext with a DIFFERENT output each keep the variable store (some exit stores
/// `out`'s own header block parameter, none a `select`).
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn r1_ineligible_regions_keep_the_variable_store() {
	let isa = lower::shared_isa().unwrap();
	for (what, region, out) in controls() {
		assert_eq!(eligible(&region), None, "{what}");
		let func = lower::build(&region, &isa).unwrap();
		let found = out_stores(&func, out);
		let n = region.slots.len();
		let own: Vec<EntryArg> = found
			.iter()
			.filter_map(|s| s.header_param.map(|i| entry_arg_kind(&func, i, out, n)))
			.filter(|k| matches!(k, EntryArg::Zero | EntryArg::OutLoad))
			.collect();
		assert!(!own.is_empty(), "{what}: no exit stores `out`'s own header parameter\n{}", func.display());
		assert!(found.iter().all(|s| s.opcode != Some(Opcode::Select)), "{what}\n{}", func.display());
		println!("R1\tcontrol\t{what}\tvariable_store_kept\tstores={}\town_param_stores={}", found.len(), own.len());
	}
}

/// The lowerer's entry value of `out` is the entry block's `iconst 0` (a `Scalar` cell is
/// never loaded). Wherever an exit stores a header parameter into the `out` cell, the entry
/// block passes that parameter an `iconst 0` (`out`'s own variable) or the `buf[n]` load
/// (`next`'s, IterNext's `x`); nothing else. Wherever it stores R1's `select`, the false
/// arm is the entry block's `iconst 0` (`select_shape`). No eligible region loads its `out`
/// cell at entry. Holds before and after R1: before, the eligible regions take the first
/// path; after, the second, and the `second_writer` control still takes the first.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn r1_out_entry_value_is_the_iconst_zero() {
	let isa = lower::shared_isa().unwrap();
	let mut regions: Vec<(String, Region, u32)> = eligible_regions();
	let second_writer = controls().into_iter().find(|c| c.0 == "second_writer").unwrap();
	regions.push((String::from("control second_writer"), second_writer.1, second_writer.2));
	let (mut params, mut next_params, mut selects) = (0, 0, 0);
	for (name, region, out) in &regions {
		let func = lower::build(region, &isa).unwrap();
		let entry = func.layout.entry_block().unwrap();
		let buf = func.dfg.block_params(entry)[0];
		for inst in func.layout.block_insts(entry) {
			if let InstructionData::Load { arg, offset, .. } = func.dfg.insts[inst] {
				let cell = func.dfg.resolve_aliases(arg) == buf && i64::from(offset) == 8 * i64::from(*out);
				assert!(!cell, "{name}: the entry loads the out cell\n{}", func.display());
			}
		}
		let n = region.slots.len();
		for s in out_stores(&func, *out) {
			if let Some(i) = s.header_param {
				match entry_arg_kind(&func, i, *out, n) {
					EntryArg::Zero => params += 1,
					EntryArg::NextLoad => next_params += 1,
					other => std::panic!("{name}: header parameter {} stored as `out` enters as {other:?}\n{}", s.value, func.display()),
				}
			} else if s.opcode == Some(Opcode::Select) {
				select_shape(&func, s.value).unwrap_or_else(|e| std::panic!("{name}: {e}\n{}", func.display()));
				selects += 1;
			}
		}
	}
	println!(
		"R1\tentry_value\tregions={}\tzero_param_stores={params}\tnext_param_stores={next_params}\tselect_stores={selects}",
		regions.len()
	);
	assert!(params > 0, "the control keeps a header-parameter store");
}

/// `(next, end)` pairs, all bounded: empty (at 0, at MIN with the wrapping `next - 1`, at
/// MAX), reversed (small, and MAX..MIN), one step, short and longer ranges, a negative
/// start, and both ends of the i64 range (`end = MAX`, `next = MIN`).
pub(super) const RANGES: [(i64, i64); 11] = [
	(0, 0),
	(i64::MIN, i64::MIN),
	(i64::MAX, i64::MAX),
	(3, 2),
	(i64::MAX, i64::MIN),
	(0, 1),
	(0, 5),
	(-4, 3),
	(0, 1000),
	(i64::MAX - 3, i64::MAX),
	(i64::MIN, i64::MIN + 3),
];

/// Guarded `I64` live-in payloads, including values one small add away from overflow (the
/// callee fault Replay exit) in both directions.
pub(super) const PAYLOADS: [i64; 5] = [0, 7, -9, i64::MAX - 5, i64::MIN + 5];

/// A canonical entry buffer, reproducing the commit module's fill (`unbox`): a guarded
/// live-in's payload (`Bool` as 0/1), 0 in every `Scalar` cell, then `next` and `end`.
pub(super) fn canonical_entry(region: &Region, payload: i64, range: Option<(i64, i64)>) -> Vec<i64> {
	let mut entry = vec![0i64; region.cells()];
	for (l, class) in region.entry.iter().enumerate() {
		entry[l] = match class {
			EntryClass::Guard(Scalar::I64) => payload,
			EntryClass::Guard(Scalar::Bool) => payload & 1,
			EntryClass::Guard(Scalar::Unit) => 0,
			EntryClass::Scalar => 0,
		};
	}
	if let Some((next, end)) = range {
		let n = region.slots.len();
		entry[n] = next;
		entry[n + 1] = end;
	}
	entry
}

/// Coverage of the whole-buffer runs.
#[derive(Default)]
struct FullCov {
	points: usize,
	by_status: BTreeMap<u32, usize>,
	/// Eligible regions' exits with `out`'s written bit set / clear.
	out_bit_set: usize,
	out_bit_clear: usize,
	/// Eligible exits with the bit clear and `next == MIN` (the wrapped `next - 1`
	/// discarded by the select).
	wrap_discarded: usize,
	/// Fuel-0 points.
	fuel0: usize,
	/// Eligible exits where `next` moved by at least two from its entry value (several
	/// IterNexts, the bit set by an earlier iteration's IterNext).
	repeated_progress: usize,
	/// Chained re-entries from a previous exit's state.
	chained: usize,
}

/// One native and one simulated run from the same entry; the WHOLE buffer, the whole
/// record and the status must be equal.
fn compare(
	native: &NativeRegion,
	region: &Region,
	entry: &[i64],
	fuel: u64,
	at: &str,
	out: Option<u32>,
	cov: &mut FullCov,
) -> (u32, Vec<i64>, ExitRecord) {
	let (mut buf, mut sim) = (entry.to_vec(), entry.to_vec());
	let (mut record, mut sim_record) = (ExitRecord::new(fuel), ExitRecord::new(fuel));
	let status = native.call(&mut buf, &mut record).expect("runs");
	let sim_status = simulate(region, &mut sim, &mut sim_record);
	assert_eq!(status, sim_status, "{at}");
	assert_eq!(record, sim_record, "{at}");
	assert_eq!(buf, sim, "{at}");
	cov.points += 1;
	*cov.by_status.entry(status).or_default() += 1;
	if fuel == 0 {
		cov.fuel0 += 1;
	}
	if let Some(out) = out {
		let n = region.slots.len();
		if record.written & (1u64 << out) != 0 {
			cov.out_bit_set += 1;
			assert_eq!(buf[out as usize], buf[n].wrapping_sub(1), "{at}: out != next - 1 with the bit set");
		} else {
			cov.out_bit_clear += 1;
			assert_eq!(buf[out as usize], 0, "{at}: out != 0 with the bit clear");
			if buf[n] == i64::MIN {
				cov.wrap_discarded += 1;
			}
		}
		if (i128::from(buf[n]) - i128::from(entry[n])) >= 2 {
			cov.repeated_progress += 1;
		}
	}
	(status, buf, record)
}

/// Native against the simulator on the WHOLE `buf` (every cell, written or not, `next` and
/// `end`), the whole record and the status, from canonical entry buffers, for every corpus
/// region (eligible, ineligible with an iterator, and without one) and the calls kernel,
/// every bounded range, payload and fuel 0..=64 plus two large fuels; then chains of
/// re-entries from each Fuel exit's state (canonically refilled) at small fuels. Passes
/// before and after R1.
#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn r1_full_buffer_byte_equal_every_fuel() {
	run_child("child_full_buffer");
}

#[test]
#[ignore = "R1 native worker: run by its parent test in a child process"]
fn child_full_buffer() {
	require_child();
	let mut regions = vec![(String::from("calls kernel"), kernel(V2_CALLS))];
	regions.extend(corpus_regions());
	let fuels: Vec<u64> = (0..=64).chain([4096, 1 << 20]).collect();
	let mut cov: BTreeMap<&str, FullCov> = BTreeMap::new();
	let mut regions_by: BTreeMap<&str, usize> = BTreeMap::new();
	let mut division = 0;
	for (name, region) in &regions {
		let out = eligible(region);
		let kind = match (out, region.iter.is_some()) {
			(Some(_), _) => "eligible",
			(None, true) => "ineligible_iter",
			(None, false) => "no_iter",
		};
		let compiled = match lower::compile(region, 0) {
			Ok(c) => c,
			Err(NativeRefusal::Audit(Finding::Division(_))) => {
				division += 1;
				continue;
			}
			Err(other) => std::panic!("{name}: {other:?}"),
		};
		let native = NativeRegion::load(&compiled).unwrap();
		*regions_by.entry(kind).or_default() += 1;
		let c = cov.entry(kind).or_default();
		let ranges: Vec<Option<(i64, i64)>> = if region.iter.is_some() {
			RANGES.iter().map(|&r| Some(r)).collect()
		} else {
			vec![None]
		};
		for &range in &ranges {
			for &payload in &PAYLOADS {
				let entry = canonical_entry(region, payload, range);
				for &fuel in &fuels {
					let at = format!("{name} range={range:?} payload={payload} fuel={fuel}");
					compare(&native, region, &entry, fuel, &at, out, c);
				}

				// Repeated progress across entries: re-enter from each Fuel exit's state,
				// canonically refilled (a guarded cell keeps its written value or its entry
				// value, `Scalar` cells 0, `next`/`end` as the exit left them).
				if range.is_none() {
					continue;
				}
				for fuel in 1..=6u64 {
					let mut entry = entry.clone();
					for step in 0..64 {
						let at = format!("{name} range={range:?} payload={payload} fuel={fuel} chain={step}");
						let (status, buf, record) = compare(&native, region, &entry, fuel, &at, out, c);
						if step > 0 {
							c.chained += 1;
						}
						if status != Status::Fuel as u32 {
							break;
						}
						for (l, class) in region.entry.iter().enumerate() {
							if let EntryClass::Guard(_) = class {
								if record.written & (1u64 << l) != 0 {
									entry[l] = buf[l];
								}
							} else {
								entry[l] = 0;
							}
						}
						let n = region.slots.len();
						entry[n] = buf[n];
						entry[n + 1] = buf[n + 1];
					}
				}
			}
		}
	}
	for (kind, c) in &cov {
		println!(
			"R1\tfull_buffer\t{kind}\tregions={}\tpoints={}\tby_status={:?}\tout_bit_set={}\tout_bit_clear={}\twrap_discarded={}\tfuel0={}\trepeated_progress={}\tchained={}",
			regions_by[kind],
			c.points,
			c.by_status,
			c.out_bit_set,
			c.out_bit_clear,
			c.wrap_discarded,
			c.fuel0,
			c.repeated_progress,
			c.chained
		);
	}
	let total: usize = cov.values().map(|c| c.points).sum();
	println!("R1\tfull_buffer\tTOTAL\tregions={}\tpoints={total}\tdivision_refused={division}", regions_by.values().sum::<usize>());
	let e = &cov["eligible"];
	assert!(e.out_bit_set > 0 && e.out_bit_clear > 0 && e.wrap_discarded > 0 && e.repeated_progress > 0 && e.chained > 0);
}

/// The selection golden captured once from the verified R1 implementation
/// (`RUNE_P3A_GOLDEN_OUT`); `p3a_tests::child_selection_golden` pins this run to it.
pub(super) const R1_GOLDEN: &str = include_str!("fixtures/r1_selection_golden.txt");

/// The R1 fixture against the W13 fixture: exactly the restricted size-only delta
/// (`w13_tests::check_w13_delta`: only COMPILE `ok bytes`, LOADED `code_bytes` and SUMMARY
/// `code_bytes` may change; `mapped_bytes`, metadata, admissions and every other counter
/// fixed; every LOADED region's size equal to its own COMPILE size). The changed regions are
/// exactly the calls-shaped ones R1 applies to in the golden corpus.
#[test]
fn r1_selection_golden_delta() {
	let (identical, changed) = super::w13_tests::compare_w13_delta(super::w13_tests::W13_GOLDEN, R1_GOLDEN);
	println!("R1\tgolden_fixtures\tidentical={identical}\tchanged={changed}");
	assert_eq!(identical + changed, R1_GOLDEN.lines().count());
	assert_eq!(changed, 6, "COMPILE, LOADED and SUMMARY of v2_calls f0 h4 and listed_9 f0 h5");
}
