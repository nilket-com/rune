//! P2b region tests (safe glue; no native code): `Copy`, `Release` and `Literal Unit` in loop
//! regions. The lattice with `Unit` and the `Copy` transfer, the `CopySource` rule from the
//! final fixpoint, the 48-byte exit record, static and dynamic Unit tags, the record
//! validation of the new masks, and every-fuel differentials of the simulator against the
//! plain interpreter with the full observation (value or error, call chain, cleanup error,
//! live entries, idle state, remaining fuel and the store's whole state).
//!
//! Tests that compile Rune source are ignored under Miri; the hand-built ones are not.
use super::abi::ExitRecord;
use super::commit::{self, Refused};
use super::hook::{self, Stats};
use super::simulate::simulate;
use super::tests::{
	bin, build, compile, fuel_to_finish, lit, loc, main_of, regions, store, while_region, window, Arg,
	Outcome, LISTED,
};
use super::{
	transfer, EntryClass, Exit, Konst, NodeOp, Region, RegionRefusal, Scalar, SlotState, Status, Ty,
	Written,
};

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Code, Function, Op},
	execute::Executor,
	value::{Root, Type, Value},
};

use std::string::String;
use std::vec::Vec as StdVec;

// ------------------------------------------------------------------------ helpers

fn branch(f: &mut Function, condition: u32, yes: u32, no: u32, at: u32) {
	f.push(Op::Branch { condition, yes, no }, loc(at)).unwrap();
}

fn jump(f: &mut Function, target: u32, at: u32) {
	f.push(Op::Jump { target }, loc(at)).unwrap();
}

fn copy(f: &mut Function, out: u32, value: u32, at: u32) {
	f.push(Op::Copy { out, value }, loc(at)).unwrap();
}

fn release(f: &mut Function, value: u32, at: u32) {
	f.push(Op::Release { value }, loc(at)).unwrap();
}

fn assign(f: &mut Function, out: u32, op: Binary, lhs: u32, rhs: u32, at: u32) {
	f.push(Op::AssignBinary { out, op, lhs, rhs }, loc(at)).unwrap();
}

fn ret(f: &mut Function, value: u32, at: u32) {
	f.push(Op::Return { value }, loc(at)).unwrap();
}

/// The one region of function 0 at `header`, admitted.
fn admitted(code: &Code, function: u32, header: u32) -> Region {
	let found = regions(code, function);
	let (_, region) = found
		.into_iter()
		.find(|(h, _)| *h == header)
		.expect("candidate header");
	region.expect("admitted")
}

fn refusal(code: &Code, function: u32, header: u32) -> RegionRefusal {
	let found = regions(code, function);
	let (_, region) = found
		.into_iter()
		.find(|(h, _)| *h == header)
		.expect("candidate header");
	match region {
		Ok(_) => panic!("header {header} admitted, expected a refusal"),
		Err(refusal) => refusal,
	}
}

/// The local index of frame slot `slot` in `region`.
fn local(region: &Region, slot: u32) -> u32 {
	region.slots.iter().position(|&s| s == slot).expect("touched slot") as u32
}

/// Everything observable about one invocation, as in the native harness.
#[derive(Debug, PartialEq)]
struct Full {
	outcome: Outcome,
	live_entries: usize,
	idle: bool,
	fuel_left: Option<usize>,
	store_state: String,
}

fn invoke_full(code: &Code, function: u32, args: &[Arg], fuel: usize) -> Full {
	let mut s = store();
	let roots: StdVec<Root> = args
		.iter()
		.map(|a| match *a {
			Arg::I(v) => s.root_i64(v),
			Arg::U(v) => s.root_u64(v),
			Arg::F(v) => s.root_f64(v),
			Arg::B(v) => s.root_bool(v),
		})
		.collect();
	let refs: StdVec<&Root> = roots.iter().collect();
	let mut executor = Executor::new().unwrap();
	hook::take_final_fuel();
	let outcome = match executor.invoke_code(code, &mut s, function, &refs, fuel) {
		Ok(root) => Outcome::Value(s.root_scalar(&root).unwrap()),
		Err(failure) => Outcome::Failure {
			error: failure.error,
			call_chain: failure.call_chain.iter().copied().collect(),
			cleanup_error: failure.cleanup_error,
		},
	};
	let fuel_left = hook::take_final_fuel();
	drop(roots);
	Full {
		outcome,
		live_entries: s.live_entries(),
		idle: executor.is_idle(),
		fuel_left,
		store_state: s.state_for_test(),
	}
}

fn add(total: &mut Stats, s: &Stats) {
	total.regions = s.regions;
	total.entries += s.entries;
	total.continue_exits += s.continue_exits;
	total.fuel_exits += s.fuel_exits;
	total.retry_exits += s.retry_exits;
	total.refused_cancel += s.refused_cancel;
	total.refused_store += s.refused_store;
	total.refused_tag += s.refused_tag;
	total.refused_other += s.refused_other;
	total.unit_static += s.unit_static;
	total.unit_dynamic += s.unit_dynamic;
}

/// Plain interpreter against the simulator install at every fuel value from 0 to two past
/// completion, comparing the full observation. Returns the summed hook statistics.
fn sweep_full(code: &Code, function: u32, args: &[Arg]) -> Stats {
	let done = fuel_to_finish(code, function, args, 20_000);
	let mut total = Stats::default();
	for fuel in 0..=done + 2 {
		hook::uninstall();
		let plain = invoke_full(code, function, args, fuel);
		hook::install(code);
		let simulated = invoke_full(code, function, args, fuel);
		let stats = hook::uninstall().unwrap();
		assert_eq!(plain, simulated, "fuel {fuel}, args {args:?}");
		assert!(plain.idle && plain.live_entries == 0, "fuel {fuel}, args {args:?}");
		add(&mut total, &stats);
	}
	total
}

fn sweep_inputs(code: &Code, function: u32, inputs: &[StdVec<Arg>]) -> Stats {
	let mut total = Stats::default();
	for args in inputs {
		let s = sweep_full(code, function, args);
		add(&mut total, &s);
	}
	total
}

fn ints(rows: &[&[i64]]) -> StdVec<StdVec<Arg>> {
	rows.iter()
		.map(|r| r.iter().map(|&v| Arg::I(v)).collect())
		.collect()
}

/// Inputs at every overflow boundary of a 4-iteration loop (as the P0 tests).
fn overflow_inputs() -> StdVec<StdVec<Arg>> {
	let max = i64::MAX;
	let min = i64::MIN;
	let mut inputs = StdVec::new();
	for k in 0..4 {
		inputs.push(vec![Arg::I(max - k), Arg::I(1)]);
		inputs.push(vec![Arg::I(min + k), Arg::I(-1)]);
		inputs.push(vec![Arg::I(max / 4 - k), Arg::I(2)]);
		inputs.push(vec![Arg::I(min / 4 + k), Arg::I(-2)]);
	}
	inputs.push(vec![Arg::I(7), Arg::I(0)]);
	inputs.push(vec![Arg::I(min), Arg::I(-1)]);
	inputs
}

// ------------------------------------------------------------- hand-built programs

/// `fn(limit) { i = 0; while i < limit { u = (); r = 5; release r; if i == 7 { return
/// <ret> } i += 1 } return <ret> }`. Slots: 0 limit, 1 i, 2 c, 3 k, 4 u, 5 r.
pub(crate) fn unit_and_release(ret_slot: u32) -> Function {
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 12, 2); // 2
	lit(&mut f, 4, Value::Unit, 3); // 3: Literal Unit
	lit(&mut f, 5, Value::I64(5), 4); // 4
	release(&mut f, 5, 5); // 5: Release
	lit(&mut f, 3, Value::I64(7), 6); // 6
	bin(&mut f, 2, Binary::Eq, 1, 3, 7); // 7
	branch(&mut f, 2, 12, 9, 8); // 8: break at i == 7
	lit(&mut f, 3, Value::I64(1), 9); // 9
	assign(&mut f, 1, Binary::Add, 1, 3, 10); // 10
	jump(&mut f, 1, 11); // 11
	ret(&mut f, ret_slot, 12); // 12
	f
}

/// `fn(limit) { i = 0; while i < limit { src = <value>; out = src; i += 1 } return out }`.
/// Slots: 0 limit, 1 i, 2 c, 3 one, 4 src, 5 out.
pub(crate) fn copy_loop(value: Value) -> Function {
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 8, 2); // 2
	lit(&mut f, 4, value, 3); // 3
	copy(&mut f, 5, 4, 4); // 4
	lit(&mut f, 3, Value::I64(1), 5); // 5
	assign(&mut f, 1, Binary::Add, 1, 3, 6); // 6
	jump(&mut f, 1, 7); // 7
	ret(&mut f, 5, 8); // 8
	f
}

/// Copy from a loop variable (`Maybe` at the Copy) or from the never-written parameter.
pub(crate) fn copy_from_live_in(source: u32) -> Function {
	let mut f = Function::new(5, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 7, 2); // 2
	copy(&mut f, 4, source, 3); // 3
	lit(&mut f, 3, Value::I64(1), 4); // 4
	assign(&mut f, 1, Binary::Add, 1, 3, 5); // 5
	jump(&mut f, 1, 6); // 6
	ret(&mut f, 4, 7); // 7
	f
}

/// The source is Bool on one path and I64 on the other when copied.
pub(crate) fn copy_from_top() -> Function {
	let mut f = Function::new(7, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 13, 2); // 2
	lit(&mut f, 6, Value::I64(2), 3); // 3
	bin(&mut f, 2, Binary::Lt, 1, 6, 4); // 4
	branch(&mut f, 2, 6, 8, 5); // 5
	lit(&mut f, 4, Value::Bool(true), 6); // 6
	jump(&mut f, 9, 7); // 7
	lit(&mut f, 4, Value::I64(1), 8); // 8
	copy(&mut f, 5, 4, 9); // 9: source is Top
	lit(&mut f, 3, Value::I64(1), 10); // 10
	assign(&mut f, 1, Binary::Add, 1, 3, 11); // 11
	jump(&mut f, 1, 12); // 12
	ret(&mut f, 5, 13); // 13
	f
}

/// `k = i + 3; k = k` (out == src). Slots: 0 limit, 1 i, 2 c, 3 one, 4 k.
pub(crate) fn self_copy() -> Function {
	let mut f = Function::new(5, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 9, 2); // 2
	lit(&mut f, 3, Value::I64(3), 3); // 3
	bin(&mut f, 4, Binary::Add, 1, 3, 4); // 4
	copy(&mut f, 4, 4, 5); // 5: self copy
	lit(&mut f, 3, Value::I64(1), 6); // 6
	assign(&mut f, 1, Binary::Add, 1, 3, 7); // 7
	jump(&mut f, 1, 8); // 8
	ret(&mut f, 4, 9); // 9
	f
}

/// A released slot read by Add (`by_branch == false`) or used as a branch condition.
pub(crate) fn released_read(by_branch: bool) -> Function {
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 9, 2); // 2
	lit(&mut f, 3, Value::I64(1), 3); // 3
	let value = if by_branch {
		Value::Bool(true)
	} else {
		Value::I64(5)
	};
	lit(&mut f, 4, value, 4); // 4
	release(&mut f, 4, 5); // 5
	if by_branch {
		branch(&mut f, 4, 7, 7, 6); // 6: Branch on a Unit
	} else {
		bin(&mut f, 5, Binary::Add, 4, 3, 6); // 6: Add of a Unit
	}
	assign(&mut f, 1, Binary::Add, 1, 3, 7); // 7
	jump(&mut f, 1, 8); // 8
	ret(&mut f, 1, 9); // 9
	f
}

/// The temporary `t` (slot 5) moves I64 -> Bool -> Unit -> I64 across iterations by
/// `i % 4`, on four paths: literal I64, literal Bool, Release, and a Copy of an I64. On the
/// Release path, `i == 6` breaks out with `t` statically Unit at that exit while `t` stays a
/// tracked (dynamic) slot elsewhere. Slots: 0 limit, 1 i, 2 c, 3 four, 4 k, 5 t, 6 one, 7 m.
pub(crate) fn transitions() -> Function {
	let mut f = Function::new(8, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 27, 2); // 2
	lit(&mut f, 3, Value::I64(4), 3); // 3
	bin(&mut f, 4, Binary::Rem, 1, 3, 4); // 4: k = i % 4 (constant divisor)
	lit(&mut f, 7, Value::I64(0), 5); // 5
	bin(&mut f, 2, Binary::Eq, 4, 7, 6); // 6
	branch(&mut f, 2, 16, 8, 7); // 7
	lit(&mut f, 7, Value::I64(1), 8); // 8
	bin(&mut f, 2, Binary::Eq, 4, 7, 9); // 9
	branch(&mut f, 2, 18, 11, 10); // 10
	lit(&mut f, 7, Value::I64(2), 11); // 11
	bin(&mut f, 2, Binary::Eq, 4, 7, 12); // 12
	branch(&mut f, 2, 20, 14, 13); // 13
	copy(&mut f, 5, 4, 14); // 14: k == 3: t = k (I64 by Copy)
	jump(&mut f, 24, 15); // 15
	lit(&mut f, 5, Value::I64(10), 16); // 16: k == 0: I64
	jump(&mut f, 24, 17); // 17
	lit(&mut f, 5, Value::Bool(true), 18); // 18: k == 1: Bool
	jump(&mut f, 24, 19); // 19
	release(&mut f, 5, 20); // 20: k == 2: Unit
	lit(&mut f, 7, Value::I64(6), 21); // 21
	bin(&mut f, 2, Binary::Eq, 1, 7, 22); // 22
	branch(&mut f, 2, 28, 24, 23); // 23: break at i == 6, t statically Unit
	lit(&mut f, 6, Value::I64(1), 24); // 24
	assign(&mut f, 1, Binary::Add, 1, 6, 25); // 25
	jump(&mut f, 1, 26); // 26
	ret(&mut f, 5, 27); // 27
	ret(&mut f, 5, 28); // 28
	f
}

/// 64 touched slots; local 0 and local 63 are dynamic Unit/I64 temporaries: Release of
/// slot 0 and a Copy into slot 63 on even `i`, an I64 into slot 0 and a Release of slot 63
/// on odd `i`. No parameter; the limit is a literal written in the header.
pub(crate) fn sixty_four(limit: i64, ret_slot: u32) -> Function {
	let mut f = Function::new(64, 0).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 3, Value::I64(limit), 1); // 1: header
	bin(&mut f, 2, Binary::Lt, 1, 3, 2); // 2
	branch(&mut f, 2, 4, 73, 3); // 3
	lit(&mut f, 4, Value::I64(2), 4); // 4
	bin(&mut f, 5, Binary::Rem, 1, 4, 5); // 5
	lit(&mut f, 6, Value::I64(0), 6); // 6
	bin(&mut f, 2, Binary::Eq, 5, 6, 7); // 7
	branch(&mut f, 2, 9, 12, 8); // 8
	release(&mut f, 0, 9); // 9: bit 0 Unit
	copy(&mut f, 63, 5, 10); // 10: bit 63 I64 by Copy
	jump(&mut f, 14, 11); // 11
	lit(&mut f, 0, Value::I64(7), 12); // 12: bit 0 I64
	release(&mut f, 63, 13); // 13: bit 63 Unit
	for (k, slot) in (7..=62u32).enumerate() {
		let pc = 14 + k as u32;
		lit(&mut f, slot, Value::I64(i64::from(slot)), pc); // 14..=69
	}
	lit(&mut f, 4, Value::I64(1), 70); // 70
	assign(&mut f, 1, Binary::Add, 1, 4, 71); // 71
	jump(&mut f, 1, 72); // 72
	ret(&mut f, ret_slot, 73); // 73
	f
}

/// Every overlapping-slot case of the P2b design table in one loop, with a fallible Add
/// before and an AssignBinary `x = x + x` after the Copy into the guarded live-in `x`.
/// Slots: 0 a, 1 b, 2 i, 3 c, 4 x, 5 y, 6 z, 7 one, 8 four.
pub(crate) fn overlapping() -> Function {
	let mut f = Function::new(9, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0); // 0
	copy(&mut f, 4, 0, 1); // 1: x = a (outside the region)
	lit(&mut f, 8, Value::I64(4), 2); // 2: header
	bin(&mut f, 3, Binary::Lt, 2, 8, 3); // 3
	branch(&mut f, 3, 5, 16, 4); // 4
	bin(&mut f, 5, Binary::Add, 4, 1, 5); // 5: y = x + b (Retry)
	copy(&mut f, 6, 5, 6); // 6: z = y
	copy(&mut f, 4, 6, 7); // 7: x = z: Copy into a guarded live-in, z is a Copy out
	release(&mut f, 6, 8); // 8: release z, copied earlier in this arrival
	copy(&mut f, 6, 6, 9); // 9: self copy of a released slot (Unit)
	release(&mut f, 5, 10); // 10: y copied at 6, now released
	copy(&mut f, 6, 5, 11); // 11: Copy -> Release -> Copy of the same source (Unit)
	assign(&mut f, 4, Binary::Add, 4, 4, 12); // 12: x = x + x after the Copy into x (Retry)
	lit(&mut f, 7, Value::I64(1), 13); // 13
	assign(&mut f, 2, Binary::Add, 2, 7, 14); // 14
	jump(&mut f, 2, 15); // 15
	ret(&mut f, 4, 16); // 16
	f
}

/// The Copy at pc 258 is charged by the 256-op gap rule (`code::MAX_GAP`).
/// Slots: 0 limit, 1 i, 2 c, 3 one, 4 x, 5 y.
pub(crate) fn charged_copy() -> Function {
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
	branch(&mut f, 2, 3, 262, 2); // 2
	for pc in 3..=257u32 {
		lit(&mut f, 4, Value::I64(i64::from(pc)), pc); // 3..=257
	}
	copy(&mut f, 5, 4, 258); // 258: charged
	lit(&mut f, 3, Value::I64(1), 259); // 259
	assign(&mut f, 1, Binary::Add, 1, 3, 260); // 260
	jump(&mut f, 1, 261); // 261
	ret(&mut f, 5, 262); // 262
	f
}

/// Slot `target` holds a dead-but-owned string when the loop is first reached; inside the
/// loop it is a Copy out (`kind 0`), a Release target (`kind 1`) or a Copy source that is
/// overwritten before the Copy (`kind 2`). Slots: 0 limit, 1 i, 2 c, 3 one, 4 owned, 5 v.
pub(crate) fn heap_at_entry(kind: u32) -> Function {
	let mut f = Function::new(6, 1).unwrap();
	let text = f.string("owned").unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	let owned = if kind == 2 { 5 } else { 4 };
	f.push(Op::String { out: owned, text }, loc(1)).unwrap(); // 1
	bin(&mut f, 2, Binary::Lt, 1, 0, 2); // 2: header
	branch(&mut f, 2, 4, 10, 3); // 3
	lit(&mut f, 3, Value::I64(1), 4); // 4
	match kind {
		0 => copy(&mut f, 4, 3, 5),    // 5: owned is a Copy out
		1 => release(&mut f, 4, 5),    // 5: owned is a Release target
		_ => lit(&mut f, 5, Value::I64(2), 5), // 5: v is overwritten ...
	}
	if kind == 2 {
		copy(&mut f, 4, 5, 6); // 6: ... before it is the Copy source
	} else {
		lit(&mut f, 5, Value::I64(2), 6); // 6
	}
	assign(&mut f, 1, Binary::Add, 1, 3, 7); // 7
	jump(&mut f, 2, 8); // 8
	ret(&mut f, 1, 9); // 9 (unreached)
	ret(&mut f, 1, 10); // 10
	f
}

/// Slots overwritten by Literal Unit, Release, Copy and a Bool literal hold F64, Type, Char
/// and Function values (scalars whose release is a no-op) when the loop is first reached.
/// Slots: 0 limit, 1 i, 2 c, 3 one, 4 F64, 5 Type, 6 Char, 7 Function. Header pc 5.
pub(crate) fn untouched_scalars_at_entry() -> Function {
	let mut f = Function::new(8, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 4, Value::F64(2.5), 1); // 1
	lit(&mut f, 5, Value::Type(Type::I64), 2); // 2
	lit(&mut f, 6, Value::Char('x'), 3); // 3
	lit(&mut f, 7, Value::Function { code: 0, id: 0 }, 4); // 4 (stamped by Builder::finish)
	bin(&mut f, 2, Binary::Lt, 1, 0, 5); // 5: header
	branch(&mut f, 2, 7, 15, 6); // 6
	lit(&mut f, 4, Value::Unit, 7); // 7: over the F64
	lit(&mut f, 3, Value::I64(1), 8); // 8
	release(&mut f, 5, 9); // 9: over the Type
	copy(&mut f, 6, 3, 10); // 10: over the Char
	lit(&mut f, 7, Value::Bool(true), 11); // 11: over the Function
	assign(&mut f, 1, Binary::Add, 1, 3, 12); // 12
	jump(&mut f, 5, 13); // 13
	ret(&mut f, 4, 14); // 14 (unreached)
	ret(&mut f, 6, 15); // 15
	f
}

pub(crate) const AND_AND: &str = "pub fn main(a, b) { let i = 0; let s = 0; while i < a && i < b { s += i; i += 1; } s }";
pub(crate) const UNIT_BOOL_I64_REUSE: &str = "pub fn main(a) { let i = 0; let s = 0; while i < a { let t = i > 2; if t { s += 1; } let u = i * 3; s += u; i += 1; } s }";
pub(crate) const COPY_OVERFLOW_RETRY: &str = "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x = x + b; i += 1; } x }";
pub(crate) const COPY_NOT_A_DIVISOR: &str = "pub fn main(a) { let n = 0; let t = 0; while n < a { let d = 17; let e = d; t += n % e; n += 1; } t }";
pub(crate) const COPY_OF_LOOP_VARIABLE: &str = "pub fn main(a) { let i = 0; let s = 0; while i < a { let y = i; s += y; i += 1; } s }";

// ------------------------------------------------------------- 1, 2: lattice and ABI

#[test]
fn p2b_lattice_with_unit_is_join_semilattice_with_bottom() {
	let tys = [Ty::Bottom, Ty::I64, Ty::Bool, Ty::Unit, Ty::Top];
	for a in tys {
		assert_eq!(a.join(a), a);
		assert_eq!(a.join(Ty::Bottom), a);
		assert_eq!(a.join(Ty::Top), Ty::Top);
		for b in tys {
			assert_eq!(a.join(b), b.join(a));
			for c in tys {
				assert_eq!(a.join(b).join(c), a.join(b.join(c)));
			}
		}
	}
	// Flat: two distinct written types join to Top, Unit included.
	assert_eq!(Ty::Unit.join(Ty::I64), Ty::Top);
	assert_eq!(Ty::Unit.join(Ty::Bool), Ty::Top);
	assert_eq!(Ty::of(Scalar::Unit), Ty::Unit);

	// Product join with a Unit write: "maybe written, Unit when written", never Top.
	let entry = SlotState::ENTRY;
	let unit = SlotState {
		written: Written::Yes,
		ty: Ty::Unit,
		konst: Konst::Top,
	};
	assert_eq!(
		entry.join(unit),
		SlotState {
			written: Written::Maybe,
			ty: Ty::Unit,
			konst: Konst::Top
		}
	);
	let int = SlotState {
		written: Written::Yes,
		ty: Ty::I64,
		konst: Konst::Const(3),
	};
	assert_eq!(
		unit.join(int),
		SlotState {
			written: Written::Yes,
			ty: Ty::Top,
			konst: Konst::Top
		}
	);
}

/// Every slot state of the region lattice (with Unit).
fn all_states() -> StdVec<SlotState> {
	let mut out = StdVec::new();
	for written in [Written::No, Written::Yes, Written::Maybe] {
		for ty in [Ty::Bottom, Ty::I64, Ty::Bool, Ty::Unit, Ty::Top] {
			for konst in [Konst::Bottom, Konst::Const(0), Konst::Const(17), Konst::Top] {
				out.push(SlotState { written, ty, konst });
			}
		}
	}
	out
}

fn leq(a: SlotState, b: SlotState) -> bool {
	a.join(b) == b
}

#[test]
fn p2b_transfer_is_monotone_with_unit_and_copy() {
	// Two touched slots: 3 (source) and 5 (out). The Copy transfer reads the source state
	// and writes the out state; Literal Unit and Release write a constant state.
	let slots = [3u32, 5];
	let states = all_states();
	let ops = [
		Op::Copy { out: 5, value: 3 },
		Op::Copy { out: 3, value: 3 },
		Op::Release { value: 5 },
		Op::Literal {
			out: 5,
			value: Value::Unit,
		},
	];
	for op in ops {
		for &a0 in &states {
			for &b0 in &states {
				if !leq(a0, b0) {
					continue;
				}
				for &o in &states {
					let mut a = [a0, o];
					let mut b = [b0, o];
					transfer(op, &slots, &mut a).unwrap();
					transfer(op, &slots, &mut b).unwrap();
					assert!(
						leq(a[0], b[0]) && leq(a[1], b[1]),
						"{op:?}: {a0:?} <= {b0:?} but {a:?} !<= {b:?}"
					);
				}
			}
		}
	}

	// The Copy result: a Yes source keeps its type; anything less than Yes gives Top. Never
	// a constant (no divisor through Copy).
	for &src in &states {
		let mut s = [src, SlotState::ENTRY];
		transfer(Op::Copy { out: 5, value: 3 }, &slots, &mut s).unwrap();
		assert_eq!(s[0], src, "the source is unchanged");
		assert_eq!(s[1].written, Written::Yes);
		assert_eq!(s[1].konst, Konst::Top);
		let want = if src.written == Written::Yes {
			src.ty
		} else {
			Ty::Top
		};
		assert_eq!(s[1].ty, want, "{src:?}");
	}
	let mut s = [SlotState::ENTRY, SlotState::ENTRY];
	transfer(Op::Release { value: 5 }, &slots, &mut s).unwrap();
	assert_eq!(
		s[1],
		SlotState {
			written: Written::Yes,
			ty: Ty::Unit,
			konst: Konst::Top
		}
	);
}

#[test]
fn p2b_exit_record_layout_appends_units_at_40() {
	assert_eq!(core::mem::size_of::<ExitRecord>(), 48);
	assert_eq!(ExitRecord::SIZE, 48);
	assert_eq!(core::mem::align_of::<ExitRecord>(), 8);
	assert_eq!(core::mem::offset_of!(ExitRecord, fuel), 0);
	assert_eq!(core::mem::offset_of!(ExitRecord, written), 8);
	assert_eq!(core::mem::offset_of!(ExitRecord, bools), 16);
	assert_eq!(core::mem::offset_of!(ExitRecord, status), 24);
	assert_eq!(core::mem::offset_of!(ExitRecord, exit), 28);
	assert_eq!(core::mem::offset_of!(ExitRecord, pc), 32);
	assert_eq!(core::mem::offset_of!(ExitRecord, reserved), 36);
	assert_eq!(core::mem::offset_of!(ExitRecord, units), 40);
	assert_eq!(ExitRecord::UNITS, 40);
	assert_eq!(ExitRecord::new(9).units, 0);
	assert_eq!(
		[Status::Continue, Status::Fuel, Status::Retry].map(|s| s as u32),
		[1, 2, 3]
	);
	assert_eq!(Status::from_u32(0), None);
	// B1: 4 is Replay; 5 is still unknown.
	assert_eq!(Status::from_u32(4), Some(Status::Replay));
	assert_eq!(Status::from_u32(5), None);
}

// ------------------------------------------------------- 3 to 13: admission and maps

#[test]
fn p2b_literal_unit_admitted_with_static_unit_tag() {
	let code = build(vec![unit_and_release(4)]);
	let region = admitted(&code, 0, 1);
	let (u, r) = (local(&region, 4), local(&region, 5));
	assert_eq!(region.tracked, 0, "Unit is a static tag here");
	// Every exit where `u` may be written tags it statically Unit, never dynamically.
	let mut seen = 0;
	for e in region.exits.iter() {
		if (e.definite | e.maybe) & (1 << u) != 0 {
			assert_ne!(e.units & (1 << u), 0, "{e:?}");
			assert_eq!((e.bools | e.dynamic) & (1 << u), 0, "{e:?}");
			seen += 1;
		}
		assert_eq!(e.units & e.bools, 0);
		assert_eq!((e.units | e.bools) & e.dynamic, 0);
	}
	assert!(seen >= 2);
	assert!(region.nodes.iter().any(|n| matches!(
		n.op,
		NodeOp::Literal {
			out,
			value: 0,
			ty: Scalar::Unit
		} if out == u
	)));

	// Simulator: i = 7 breaks out in the first iteration; u and r hold payload 0.
	let n = region.slots.len();
	let mut buf = vec![0i64; n];
	buf[local(&region, 0) as usize] = 10;
	buf[local(&region, 1) as usize] = 7;
	buf[u as usize] = 99;
	let mut record = ExitRecord::new(5);
	assert_eq!(simulate(&region, &mut buf, &mut record), Status::Continue as u32);
	assert_eq!(record.pc, 12);
	assert_eq!(record.written & (1 << u | 1 << r), 1 << u | 1 << r);
	assert_eq!((buf[u as usize], buf[r as usize]), (0, 0));
	assert_eq!(record.units, 0, "no tracked slot: the dynamic mask stays 0");

	// Commit writes Value::Unit over an untouched-at-entry F64 and Type.
	let mut s = store();
	let mut w = vec![
		Value::I64(10),
		Value::I64(7),
		Value::Unit,
		Value::Unit,
		Value::F64(2.5),
		Value::Type(Type::I64),
	];
	let exited = commit::enter(&region, &mut w, &mut s, false, 5, commit::ROOM_FOR_TEST, simulate)
		.unwrap()
		.unwrap();
	assert_eq!((exited.status, exited.pc), (Status::Continue, 12));
	assert_eq!((w[4], w[5]), (Value::Unit, Value::Unit));
	assert_eq!((w[0], w[1]), (Value::I64(10), Value::I64(7)));

	for ret in [4, 5] {
		let code = build(vec![unit_and_release(ret)]);
		let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[3], &[9]]));
		assert!(stats.entries > 0 && stats.unit_static > 0, "{stats:?}");
	}
}

#[test]
fn p2b_release_writes_unit_and_commit_writes_value_unit() {
	let code = build(vec![unit_and_release(5)]);
	let region = admitted(&code, 0, 1);
	let r = local(&region, 5);
	// The Release lowers to a Unit literal of payload 0 into its own slot.
	assert!(region.nodes.iter().any(|n| n.pc == 5
		&& n.op
			== NodeOp::Literal {
				out: r,
				value: 0,
				ty: Scalar::Unit
			}));
	// At the break exit (after the Release on every path) r is definitely written, Unit.
	let brk = region
		.exits
		.iter()
		.find(|e| e.status == Status::Continue && e.definite & (1 << r) != 0)
		.expect("break exit");
	assert_ne!(brk.units & (1 << r), 0);
	assert_eq!(brk.pc, 12);

	// The window holds Value::Unit after commit, whatever scalar r held at entry.
	for old in [Value::I64(3), Value::Bool(true), Value::Unit, Value::F64(-0.0), Value::Char('q')] {
		let mut s = store();
		let mut w = vec![
			Value::I64(10),
			Value::I64(7),
			Value::Unit,
			Value::Unit,
			Value::Unit,
			old,
		];
		commit::enter(&region, &mut w, &mut s, false, 5, commit::ROOM_FOR_TEST, simulate)
			.unwrap()
			.unwrap();
		assert_eq!(w[5], Value::Unit, "{old:?}");
	}
}

#[test]
fn p2b_copy_from_in_region_scalar_types_out_like_source() {
	for (value, scalar, ty) in [
		(Value::I64(3), Scalar::I64, Ty::I64),
		(Value::Bool(true), Scalar::Bool, Ty::Bool),
		(Value::Unit, Scalar::Unit, Ty::Unit),
	] {
		let code = build(vec![copy_loop(value)]);
		let region = admitted(&code, 0, 1);
		let (src, out) = (local(&region, 4), local(&region, 5));
		assert!(region.nodes.iter().any(|n| n.op
			== NodeOp::Copy {
				out,
				src,
				ty: scalar
			}));
		for e in region.exits.iter() {
			let bit = 1u64 << out;
			assert_ne!((e.definite | e.maybe) & bit, 0, "{e:?}");
			assert_eq!(e.dynamic & bit, 0, "{e:?}");
			assert_eq!(e.bools & bit != 0, ty == Ty::Bool, "{e:?}");
			assert_eq!(e.units & bit != 0, ty == Ty::Unit, "{e:?}");
		}
		let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[4]]));
		assert!(stats.entries > 0, "{value:?}");
	}

	// Listing program 4 (`x = x + x`): Binary, Copy, Release; now admitted.
	let program = compile(LISTED[3]);
	let main = main_of(&program);
	let region = admitted(&program.code, main, 2);
	assert!(region
		.nodes
		.iter()
		.any(|n| matches!(n.op, NodeOp::Copy { ty: Scalar::I64, .. })));
}

#[test]
fn p2b_copy_from_live_in_or_maybe_written_refused_copy_source() {
	// A loop variable is Maybe at the Copy; the parameter is never written (No).
	for source in [1, 0] {
		let code = build(vec![copy_from_live_in(source)]);
		assert_eq!(
			refusal(&code, 0, 1),
			RegionRefusal::CopySource { pc: 3 },
			"source slot {source}"
		);
	}
}

/// `let y = i; s += y` with `i` a loop variable: the Copy's out is Top (its source is
/// Maybe), and the Add reading `y` is refused by the read-type check, which runs before the
/// CopySource check. The mismatch refusal is not weakened to admit a live-in Copy.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn p2b_copy_of_loop_variable_in_rune_refused() {
	let program = compile(COPY_OF_LOOP_VARIABLE);
	let main = main_of(&program);
	let found = regions(&program.code, main);
	assert_eq!(found.len(), 1);
	assert!(
		matches!(found[0].1, Err(RegionRefusal::Type { .. })),
		"{:?}",
		found[0].1.as_ref().err()
	);
}

#[test]
fn p2b_copy_from_top_typed_source_refused_copy_source() {
	let code = build(vec![copy_from_top()]);
	assert_eq!(refusal(&code, 0, 1), RegionRefusal::CopySource { pc: 9 });
}

/// The CopySource rule reads the FINAL fixpoint state before each Copy: a source written
/// on one branch arm only is Maybe at the Copy whichever arm the work list visits first.
#[test]
fn p2b_copy_source_uses_final_fixpoint_not_visit_order() {
	for (yes, no) in [(3, 5), (5, 3)] {
		let mut f = Function::new(6, 1).unwrap();
		lit(&mut f, 1, Value::I64(0), 0); // 0
		bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
		branch(&mut f, 2, yes, no, 2); // 2
		lit(&mut f, 4, Value::I64(1), 3); // 3: writes the source on one arm
		jump(&mut f, 5, 4); // 4
		copy(&mut f, 5, 4, 5); // 5: the Copy (the join point)
		lit(&mut f, 3, Value::I64(1), 6); // 6
		assign(&mut f, 1, Binary::Add, 1, 3, 7); // 7
		bin(&mut f, 2, Binary::Lt, 1, 0, 8); // 8
		branch(&mut f, 2, 1, 10, 9); // 9
		ret(&mut f, 5, 10); // 10
		let code = build(vec![f]);
		assert_eq!(
			refusal(&code, 0, 1),
			RegionRefusal::CopySource { pc: 5 },
			"arms ({yes}, {no})"
		);
	}

	// The source is written before the loop (outside the region: No at entry), Released on
	// the back edge and written as Bool on one arm only: Maybe at the Copy in the final
	// fixpoint, whatever an early visit of the Copy saw.
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 4, Value::I64(1), 1); // 1
	bin(&mut f, 2, Binary::Lt, 1, 0, 2); // 2: header
	branch(&mut f, 2, 4, 11, 3); // 3
	branch(&mut f, 2, 5, 6, 4); // 4
	lit(&mut f, 4, Value::Bool(true), 5); // 5
	copy(&mut f, 5, 4, 6); // 6: I64 or Unit on arrival 1 paths, Bool on others
	release(&mut f, 4, 7); // 7
	lit(&mut f, 3, Value::I64(1), 8); // 8
	assign(&mut f, 1, Binary::Add, 1, 3, 9); // 9
	jump(&mut f, 2, 10); // 10
	ret(&mut f, 5, 11); // 11
	let code = build(vec![f]);
	assert_eq!(refusal(&code, 0, 2), RegionRefusal::CopySource { pc: 6 });
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn p2b_copy_does_not_make_a_constant_divisor() {
	let program = compile(COPY_NOT_A_DIVISOR);
	let main = main_of(&program);
	let found = regions(&program.code, main);
	assert_eq!(found.len(), 1);
	let region = found[0].1.as_ref().expect("admitted");
	let rem = region
		.nodes
		.iter()
		.find_map(|n| match n.op {
			NodeOp::Arith {
				op: Binary::Rem,
				retry,
				divisor,
				..
			} => Some((retry, divisor)),
			_ => None,
		})
		.expect("Rem node");
	assert!(rem.0.is_some(), "a runtime divisor keeps its Retry exit");
	assert_eq!(rem.1, None, "no constant through Copy");
	assert_eq!(super::division_kind(region), Some(super::DivisionKind::Runtime));

	// DESIGN DEVIATION (P2B-8 expected `Audit(Division)`): the region keeps its Retry exit
	// and explicit zero / MIN-by--1 branches, but Cranelift's SSA builder resolves the
	// Copy's out variable to the literal's `iconst 17`, so the IR gate sees a constant
	// divisor outside {0, -1}, the mid-end turns the `srem` into a multiply, and every gate
	// passes. That is exact (`e` is 17 on every path); the Retry exit is unreachable.
	#[cfg(all(feature = "fresh-core-native", target_arch = "x86_64", target_os = "linux"))]
	{
		use crate::experimental_native::lower;
		let compiled = lower::compile(region, 0).expect("every gate passes");
		let listing = compiled.listing();
		assert!(!listing.optimized.contains("srem"), "{}", listing.optimized);
		assert!(!listing.vcode.contains("idiv"), "{}", listing.vcode);
	}
}

#[test]
fn p2b_self_copy_admitted_and_identity() {
	let code = build(vec![self_copy()]);
	let region = admitted(&code, 0, 1);
	let k = local(&region, 4);
	assert!(region.nodes.iter().any(|n| n.op
		== NodeOp::Copy {
			out: k,
			src: k,
			ty: Scalar::I64
		}));
	// One iteration from i = 5, then the fuel exit at the header: k = 8, written.
	let n = region.slots.len();
	let mut buf = vec![0i64; n];
	buf[local(&region, 0) as usize] = 10;
	buf[local(&region, 1) as usize] = 5;
	let mut record = ExitRecord::new(1);
	assert_eq!(simulate(&region, &mut buf, &mut record), Status::Fuel as u32);
	assert_eq!(buf[k as usize], 8);
	assert_ne!(record.written & (1 << k), 0);
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[3]]));
	assert!(stats.entries > 0);
}

#[test]
fn p2b_released_slot_read_by_arithmetic_refused_type() {
	let code = build(vec![released_read(false)]);
	assert_eq!(refusal(&code, 0, 1), RegionRefusal::Type { pc: 6 });
}

#[test]
fn p2b_released_slot_as_branch_condition_refused_type() {
	let code = build(vec![released_read(true)]);
	assert_eq!(refusal(&code, 0, 1), RegionRefusal::Type { pc: 6 });
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn p2b_unit_i64_bool_temp_reuse_uses_dynamic_units_mask() {
	// Listing program 3: `let t = i * 2; if t > 6 { s = s + t; }`; slot 6 is the `s + t`
	// temporary (Copy source, then Released) on one path and the literal 6 on the other.
	let program = compile(LISTED[2]);
	let main = main_of(&program);
	let region = admitted(&program.code, main, 2);
	let t = local(&region, 6);
	assert_ne!(region.tracked & (1 << t), 0, "slot 6 is tracked");
	assert!(region.exits.iter().any(|e| e.dynamic & (1 << t) != 0));
	// One iteration, then the fuel exit at the header. i = 4: t = 8 > 6, the Unit path;
	// i = 0: the I64 path.
	for (i, unit) in [(4i64, true), (0, false)] {
		let n = region.slots.len();
		let mut buf = vec![0i64; n];
		buf[local(&region, 0) as usize] = 10;
		buf[local(&region, 1) as usize] = i;
		buf[local(&region, 2) as usize] = 100;
		let mut record = ExitRecord::new(1);
		assert_eq!(simulate(&region, &mut buf, &mut record), Status::Fuel as u32);
		assert_ne!(record.written & (1 << t), 0);
		assert_eq!(record.units & (1 << t) != 0, unit, "i = {i}");
		assert_eq!(record.bools & (1 << t), 0);
		if unit {
			assert_eq!(buf[t as usize], 0, "Unit payload 0");
		}
	}
}

#[test]
fn p2b_transition_masks_i64_bool_unit_i64_across_paths_and_iterations() {
	let code = build(vec![transitions()]);
	let region = admitted(&code, 0, 1);
	let t = local(&region, 5);
	let bit = 1u64 << t;
	assert_eq!(region.tracked, bit, "only t is tracked");
	let header_exits: StdVec<&Exit> = region
		.exits
		.iter()
		.filter(|e| e.pc == 1 || e.pc == 27)
		.collect();
	assert!(!header_exits.is_empty());
	for e in &header_exits {
		assert_ne!(e.dynamic & bit, 0, "t is dynamic at the header: {e:?}");
		assert_eq!((e.units | e.bools) & bit, 0, "static masks exclude dynamic slots");
	}
	let brk = region.exits.iter().find(|e| e.pc == 28).expect("break exit");
	assert_eq!(brk.dynamic & bit, 0, "statically Unit at the break");
	assert_ne!(brk.units & bit, 0);
	assert_ne!(brk.definite & bit, 0);

	// After f iterations from i = 0, the fuel exit at the header carries t's last tag:
	// I64 (literal), Bool, Unit, I64 (Copy), I64 (literal).
	let n = region.slots.len();
	for (f, bools, units) in [(1u64, 0, 0), (2, bit, 0), (3, 0, bit), (4, 0, 0), (5, 0, 0)] {
		let mut buf = vec![0i64; n];
		buf[local(&region, 0) as usize] = 100;
		let mut record = ExitRecord::new(f);
		assert_eq!(simulate(&region, &mut buf, &mut record), Status::Fuel as u32);
		assert_eq!(record.fuel, 0);
		assert_eq!((record.bools, record.units), (bools, units), "{f} iterations");
		assert_ne!(record.written & bit, 0);
		assert_eq!(record.bools & record.units, 0);
	}
	// The break at i = 6: t is Unit, and the dynamic mask agrees with the static tag.
	let mut buf = vec![0i64; n];
	buf[local(&region, 0) as usize] = 100;
	buf[local(&region, 1) as usize] = 6;
	let mut record = ExitRecord::new(3);
	assert_eq!(simulate(&region, &mut buf, &mut record), Status::Continue as u32);
	assert_eq!(record.pc, 28);
	assert_eq!(record.units, bit);
	assert_eq!(buf[t as usize], 0);

	// Every fuel value, every limit through two full cycles and the break (not under Miri:
	// minutes there; the direct simulator checks above run under Miri).
	if cfg!(miri) {
		return;
	}
	let stats = sweep_inputs(
		&code,
		0,
		&ints(&[&[0], &[1], &[2], &[3], &[4], &[5], &[6], &[7], &[9]]),
	);
	assert!(stats.entries > 0 && stats.unit_static > 0 && stats.unit_dynamic > 0, "{stats:?}");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn p2b_listing_programs_now_admitted() {
	for (index, expected) in [
		(2usize, &[(2u32, true)][..]),
		(3, &[(2, true)][..]),
		(5, &[(2, true), (5, true)][..]),
		(6, &[(3, true), (7, true)][..]),
		// B1: the calls kernel is admitted (native IterNext and an inlined `add`).
		(9, &[(5, true)][..]),
	] {
		let program = compile(LISTED[index]);
		let main = main_of(&program);
		let found: StdVec<(u32, bool)> = regions(&program.code, main)
			.into_iter()
			.map(|(h, r)| (h, r.is_ok()))
			.collect();
		assert_eq!(found.as_slice(), expected, "listing program {}", index + 1);
	}
	// B1: the calls kernel, refused at its IterNext header through P2b, is admitted.
	let program = compile(LISTED[9]);
	let main = main_of(&program);
	let found = regions(&program.code, main);
	assert_eq!(found[0].0, 5);
	assert!(found[0].1.as_ref().is_ok_and(|r| r.iter.is_some() && r.sites.len() == 1));
}

// ----------------------------------------------------- 14: forged records, both masks

/// Forges one record field over an honest simulated run.
type Forge = fn(&Region, &mut [i64], &mut ExitRecord);

/// Runs `commit::enter` with the simulator and then `forge`, and asserts InvalidExit with
/// the window and the whole store state unchanged.
fn assert_forged_rejected(region: &Region, window: &[Value], fuel: usize, name: &str, forge: Forge) {
	let mut s = store();
	let keep = s.string("keep").unwrap();
	let mut w = window.to_vec();
	let before = (w.clone(), s.state_for_test());
	let result = commit::enter(region, &mut w, &mut s, false, fuel, commit::ROOM_FOR_TEST, |r, b, rec| {
		simulate(r, b, rec);
		forge(r, b, rec);
		rec.status
	})
	.unwrap();
	assert_eq!(result, Err(Refused::InvalidExit), "{name}");
	assert_eq!((w, s.state_for_test()), before, "{name}: nothing mutated");
	s.release(keep).unwrap();
}

#[test]
fn p2b_forged_records_with_units_rejected_before_any_write() {
	// The ten P0 forgeries again, now also checking the whole store state.
	let (_code, region) = while_region();
	let cont = region
		.exits
		.iter()
		.position(|e| e.status == Status::Continue)
		.unwrap() as u32;
	let fuel_exit = region
		.exits
		.iter()
		.position(|e| e.status == Status::Fuel)
		.unwrap() as u32;
	let pc = region.exits[cont as usize].pc;
	let honest = move |r: &mut ExitRecord| {
		r.status = 1;
		r.exit = cont;
		r.pc = pc;
		r.written = 0b001000;
		r.fuel = 3;
		r.bools = 0;
		r.units = 0;
	};
	let forged: [(&str, fn(&mut [i64], &mut ExitRecord, u32, u32, u32)); 10] = [
		("status 0", |_, r, _, _, _| r.status = 0),
		("status mismatch", |_, r, _, _, _| r.status = 2),
		("pc mismatch", |_, r, _, p, _| r.pc = p + 1),
		("exit out of range", |_, r, _, _, _| r.exit = 9999),
		("written outside map", |_, r, _, _, _| r.written |= 1 << 40),
		("definite not written", |_, r, _, _, _| r.written = 0),
		("bool not 0/1", |b, _, _, _, _| b[3] = 2),
		("untracked dynamic bool bit", |_, r, _, _, _| r.bools = 1 << 3),
		("fuel grew", |_, r, _, _, _| r.fuel = 51),
		("fuel exit with fuel", |_, r, _, _, f| {
			r.status = 2;
			r.exit = f;
			r.pc = 2;
			r.fuel = 1;
		}),
	];
	for (name, forge) in forged {
		let mut s = store();
		let keep = s.string("keep").unwrap();
		let original = window();
		let mut w = original.clone();
		let state = s.state_for_test();
		let result = commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, |_, b, r| {
			honest(r);
			b[3] = 1;
			forge(b, r, cont, pc, fuel_exit);
			r.status
		})
		.unwrap();
		assert_eq!(result, Err(Refused::InvalidExit), "{name}");
		assert_eq!(w, original, "{name}");
		assert_eq!(s.state_for_test(), state, "{name}");
		s.release(keep).unwrap();
	}
	// A units bit outside `tracked` on the P0 region (tracked = 0).
	let mut s = store();
	let mut w = window();
	let result = commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, |_, b, r| {
		honest(r);
		b[3] = 1;
		r.units = 1 << 3;
		r.status
	})
	.unwrap();
	assert_eq!(result, Err(Refused::InvalidExit), "units outside tracked");
	assert_eq!(w, window());

	// The transitions region: t is tracked (dynamic at the header exits, statically Unit at
	// the break exit).
	let code = build(vec![transitions()]);
	let region = admitted(&code, 0, 1);
	let t = local(&region, 5) as usize;
	let entry = vec![
		Value::I64(100),
		Value::I64(6),
		Value::Unit,
		Value::Unit,
		Value::Unit,
		Value::Unit,
		Value::Unit,
		Value::Unit,
	];
	// Fuel 3 from i = 6: the break exit with t statically Unit. Fuel 1 from i = 5: k = 1, a
	// Bool t, then the header's fuel exit (t dynamic). Fuel 0: the header's fuel exit with
	// nothing written.
	let mut bool_entry = entry.clone();
	bool_entry[1] = Value::I64(5);
	let unit_cases: [(&str, Forge); 4] = [
		("units and bools on the same static Unit slot", |r, _, rec| {
			rec.bools |= 1 << local(r, 5);
		}),
		("units bit outside tracked", |r, _, rec| {
			rec.units |= 1 << local(r, 4);
		}),
		("Unit-tagged written payload 1", |r, b, _| {
			b[local(r, 5) as usize] = 1;
		}),
		("static Unit slot without its dynamic units bit", |r, _, rec| {
			rec.units &= !(1 << local(r, 5));
		}),
	];
	for (name, forge) in unit_cases {
		assert_forged_rejected(&region, &entry, 3, name, forge);
	}
	let bool_cases: [(&str, Forge); 3] = [
		("dynamic slot with both bools and units", |r, _, rec| {
			rec.units |= 1 << local(r, 5);
		}),
		("dynamic Bool payload 2", |r, b, _| {
			b[local(r, 5) as usize] = 2;
		}),
		("bools bit on a slot this path did not write", |r, _, rec| {
			rec.written &= !(1 << local(r, 5));
		}),
	];
	for (name, forge) in bool_cases {
		assert_forged_rejected(&region, &bool_entry, 1, name, forge);
	}
	let unwritten_cases: [(&str, Forge); 2] = [
		("units bit on an unwritten tracked slot", |r, _, rec| {
			rec.units |= 1 << local(r, 5);
		}),
		("bools bit on an unwritten tracked slot", |r, _, rec| {
			rec.bools |= 1 << local(r, 5);
		}),
	];
	for (name, forge) in unwritten_cases {
		assert_forged_rejected(&region, &bool_entry, 0, name, forge);
	}

	// The honest runs commit: Unit at the break, Bool at the header exit.
	let mut s = store();
	let mut w = entry.clone();
	commit::enter(&region, &mut w, &mut s, false, 3, commit::ROOM_FOR_TEST, simulate)
		.unwrap()
		.unwrap();
	assert_eq!(w[t], Value::Unit);
	let mut w = bool_entry.clone();
	commit::enter(&region, &mut w, &mut s, false, 1, commit::ROOM_FOR_TEST, simulate)
		.unwrap()
		.unwrap();
	assert_eq!(w[t], Value::Bool(true));
}

// ------------------------------------------------- 15 to 18: heap, scalars, the store

#[test]
fn p2b_heap_in_copy_target_refuses_entry_and_matches_interpreter() {
	let code = build(vec![heap_at_entry(0)]);
	admitted(&code, 0, 2);
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[5]]));
	assert!(stats.refused_tag > 0, "the first arrival is refused: {stats:?}");
	assert!(stats.entries > 0, "a later arrival enters");
}

#[test]
fn p2b_heap_in_release_target_refuses_entry_and_matches_interpreter() {
	let code = build(vec![heap_at_entry(1)]);
	admitted(&code, 0, 2);
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[5]]));
	assert!(stats.refused_tag > 0, "{stats:?}");
	assert!(stats.entries > 0);
}

#[test]
fn p2b_heap_in_copy_source_slot_at_entry_refuses_entry() {
	let code = build(vec![heap_at_entry(2)]);
	let region = admitted(&code, 0, 2);
	assert_eq!(region.entry[local(&region, 5) as usize], EntryClass::Scalar);
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[5]]));
	assert!(stats.refused_tag > 0, "{stats:?}");
	assert!(stats.entries > 0);
}

#[test]
fn p2b_untouched_scalars_at_entry_are_released_and_never_produced() {
	let code = build(vec![untouched_scalars_at_entry()]);
	let region = admitted(&code, 0, 5);
	for slot in [4, 5, 6, 7] {
		assert_eq!(region.entry[local(&region, slot) as usize], EntryClass::Scalar);
	}
	// F64, Type, Char and Function at entry are accepted old values; the region writes only
	// Unit, Bool or I64 over them.
	let mut s = store();
	let mut w = vec![
		Value::I64(3),
		Value::I64(0),
		Value::Unit,
		Value::Unit,
		Value::F64(2.5),
		Value::Type(Type::I64),
		Value::Char('x'),
		Value::Function { code: code.id(), id: 0 },
	];
	commit::enter(&region, &mut w, &mut s, false, 1, commit::ROOM_FOR_TEST, simulate)
		.unwrap()
		.unwrap();
	assert_eq!(
		(w[4], w[5], w[6], w[7]),
		(Value::Unit, Value::Unit, Value::I64(1), Value::Bool(true))
	);
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[3]]));
	assert!(stats.entries > 0 && stats.refused_tag == 0, "{stats:?}");

	// A non-I64/Bool/Unit literal in the body is never admitted as new computation.
	for value in [
		Value::F64(1.0),
		Value::Type(Type::I64),
		Value::Char('c'),
		Value::U64(1),
		Value::Function { code: 0, id: 0 },
	] {
		let mut f = Function::new(4, 1).unwrap();
		lit(&mut f, 1, Value::I64(0), 0); // 0
		bin(&mut f, 2, Binary::Lt, 1, 0, 1); // 1: header
		branch(&mut f, 2, 3, 7, 2); // 2
		lit(&mut f, 3, value, 3); // 3
		lit(&mut f, 3, Value::I64(1), 4); // 4
		assign(&mut f, 1, Binary::Add, 1, 3, 5); // 5
		jump(&mut f, 1, 6); // 6
		ret(&mut f, 1, 7); // 7
		let code = build(vec![f]);
		assert_eq!(refusal(&code, 0, 1), RegionRefusal::Op { pc: 3 }, "{value:?}");
	}
}

#[test]
fn p2b_copy_and_release_never_touch_the_store() {
	let code = build(vec![transitions()]);
	let region = admitted(&code, 0, 1);
	let mut s = store();
	let keep = s.string("a live heap entry elsewhere").unwrap();
	let mut entries = 0;
	let count = if cfg!(miri) { 100 } else { 1_000 };
	for k in 0..count {
		let mut w = vec![
			Value::I64(100),
			Value::I64(k % 13),
			Value::Unit,
			Value::Unit,
			Value::Unit,
			match k % 4 {
				0 => Value::Unit,
				1 => Value::Bool(k % 3 == 0),
				2 => Value::F64(1.5),
				_ => Value::I64(k),
			},
			Value::Unit,
			Value::Unit,
		];
		let before = s.state_for_test();
		let result = commit::enter(&region, &mut w, &mut s, false, (k % 9) as usize, commit::ROOM_FOR_TEST, simulate).unwrap();
		assert!(result.is_ok(), "{k}: {result:?}");
		assert_eq!(s.state_for_test(), before, "entry {k}");
		entries += 1;
	}
	assert_eq!(entries, count);
	s.release(keep).unwrap();
}

// ------------------------------------------------------- 20 to 22: fuel and overlaps

#[test]
#[cfg_attr(miri, ignore = "Miri: per-pc fuel contract over a 263-instruction loop")]
fn p2b_simulator_fuel_and_retry_contract_with_copy_release() {
	let code = build(vec![charged_copy()]);
	let f = code.function(0).unwrap();
	assert!(f.instructions()[258].is_charged(), "the Copy is charged by the gap rule");
	assert!(!f.instructions()[257].is_charged());
	let region = admitted(&code, 0, 1);
	let copy_exits: StdVec<&Exit> = region.exits.iter().filter(|e| e.pc == 258).collect();
	assert_eq!(copy_exits.len(), 1, "one exit at the Copy");
	assert_eq!(copy_exits[0].status, Status::Fuel, "a Fuel exit, never a Retry");
	// The only Retry exit is the `i += 1` Add's.
	assert!(region
		.exits
		.iter()
		.filter(|e| e.status == Status::Retry)
		.all(|e| e.pc == 260));

	let n = region.slots.len();
	for fuel in 0..6u64 {
		let mut buf = vec![0i64; n];
		buf[local(&region, 0) as usize] = 100;
		let mut record = ExitRecord::new(fuel);
		let status = simulate(&region, &mut buf, &mut record);
		assert_eq!(status, Status::Fuel as u32, "fuel {fuel}");
		assert_eq!(record.fuel, 0);
		// Arrivals alternate header (pc 1) and Copy (pc 258): fuel f stops at the f-th.
		let expect = if fuel % 2 == 0 { 1 } else { 258 };
		assert_eq!(record.pc, expect, "fuel {fuel}");
	}
	let stats = sweep_inputs(&code, 0, &ints(&[&[0], &[1], &[3]]));
	assert!(stats.entries > 0 && stats.fuel_exits > 0);
}

#[test]
fn p2b_overlapping_slots_every_fuel() {
	let code = build(vec![overlapping()]);
	let region = admitted(&code, 0, 2);
	let (y, z) = (local(&region, 5), local(&region, 6));
	// z = z (pc 9) copies a released, Unit slot; z = y (pc 11) copies the released y.
	for pc in [9, 11] {
		let node = region.nodes.iter().find(|n| n.pc == pc).unwrap();
		assert!(matches!(node.op, NodeOp::Copy { ty: Scalar::Unit, .. }), "{node:?}");
	}
	let node = region.nodes.iter().find(|n| n.pc == 6).unwrap();
	assert_eq!(node.op, NodeOp::Copy { out: z, src: y, ty: Scalar::I64 });
	let stats = sweep_inputs(&code, 0, &overflow_inputs());
	assert!(stats.entries > 0 && stats.retry_exits > 0, "{stats:?}");

	// Self copy of an I64 (out == src) and the 64-slot masks are swept too.
	let code = build(vec![self_copy()]);
	sweep_inputs(&code, 0, &ints(&[&[0], &[2]]));
}

#[test]
fn p2b_sixty_four_slots_bit_0_and_bit_63() {
	let code = build(vec![
		sixty_four(5, 0),
		sixty_four(4, 0),
		sixty_four(5, 63),
		sixty_four(4, 63),
	]);
	let region = admitted(&code, 0, 1);
	assert_eq!(region.slots.len(), 64);
	assert_eq!(region.tracked, 1 | 1 << 63, "bit 0 and bit 63 are the dynamic slots");
	// Two iterations then the header's fuel exit: i = 0 (even: slot 0 Unit, slot 63 I64),
	// i = 1 (odd: slot 0 I64, slot 63 Unit).
	for (fuel, units) in [(1u64, 1u64), (2, 1 << 63)] {
		let mut buf = vec![0i64; 64];
		let mut record = ExitRecord::new(fuel);
		assert_eq!(simulate(&region, &mut buf, &mut record), Status::Fuel as u32);
		assert_eq!(record.units, units, "fuel {fuel}");
		assert_eq!(record.bools, 0);
		assert_eq!(record.written & (1 | 1 << 63), 1 | 1 << 63);
		// The commit writes Unit and I64 at bit 0 and bit 63.
		let mut s = store();
		let mut w = vec![Value::Unit; 64];
		w[1] = Value::I64(0);
		commit::enter(&region, &mut w, &mut s, false, fuel as usize, commit::ROOM_FOR_TEST, simulate)
			.unwrap()
			.unwrap();
		let (unit, int) = if units == 1 { (0, 63) } else { (63, 0) };
		assert_eq!(w[unit], Value::Unit, "fuel {fuel}");
		assert!(matches!(w[int], Value::I64(_)), "fuel {fuel}: {:?}", w[int]);
	}
	// Every-fuel sweeps: skipped under Miri (minutes per sweep there); the direct simulator
	// and commit checks above run under Miri.
	if cfg!(miri) {
		return;
	}
	for function in 0..4 {
		let stats = sweep_full(&code, function, &[]);
		assert!(stats.entries > 0 && stats.unit_dynamic > 0, "{function}: {stats:?}");
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn p2b_simulated_every_fuel_differential() {
	let arithmetic = overflow_inputs();
	let cases: [(&str, StdVec<StdVec<Arg>>); 7] = [
		(LISTED[2], ints(&[&[0], &[5], &[9]])),
		(LISTED[3], arithmetic.clone()),
		(LISTED[5], ints(&[&[0], &[3], &[6]])),
		(LISTED[6], ints(&[&[0], &[3], &[6]])),
		(AND_AND, ints(&[&[0, 5], &[5, 3], &[3, 7], &[6, 6]])),
		(UNIT_BOOL_I64_REUSE, ints(&[&[0], &[2], &[6]])),
		(COPY_OVERFLOW_RETRY, arithmetic),
	];
	for (src, inputs) in cases {
		let program = compile(src);
		let main = main_of(&program);
		let stats = sweep_inputs(&program.code, main, &inputs);
		assert!(stats.regions >= 1, "{src}");
		assert!(stats.entries > 0, "{src}: {stats:?}");
	}
}
