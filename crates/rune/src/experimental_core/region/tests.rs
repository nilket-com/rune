//! P0 region tests: lattice laws, eligibility and refusals, the exit contract, and a
//! differential of the interpreter with and without the region simulator over every fuel
//! value and every error boundary of each program (including nested callers).
use super::abi::ExitRecord;
use super::commit::{self, Refused};
use super::hook::{self, Stats};
use super::simulate::simulate;
use super::{
	analyze, function_types, EntryClass, FTy, Konst, NodeOp, Region, RegionRefusal, Scalar,
	SlotState, Status, Ty, Written,
};

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Code, Function, Op},
	error::{Error, Location},
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::{Root, Scalar as HostScalar, Value},
};

use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::vec::Vec as StdVec;

pub(super) fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

pub(super) fn store() -> Store {
	Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap()
}

pub(super) fn compile(text: &str) -> frontend::Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap()
}

pub(super) fn build(functions: StdVec<Function>) -> Code {
	let mut b = Builder::default();
	for f in functions {
		b.push(f).unwrap();
	}
	b.finish().unwrap()
}

/// Everything observable about one invocation.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
	Value(Option<HostScalar>),
	Failure {
		error: Error,
		call_chain: StdVec<Location>,
		cleanup_error: Option<Error>,
	},
}

#[derive(Debug, PartialEq)]
struct Observed {
	outcome: Outcome,
	live_entries: usize,
	idle: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Arg {
	I(i64),
	U(u64),
	F(f64),
	B(bool),
}

fn invoke(code: &Code, function: u32, args: &[Arg], fuel: usize) -> Observed {
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
	let outcome = match executor.invoke_code(code, &mut s, function, &refs, fuel) {
		Ok(root) => Outcome::Value(s.root_scalar(&root).unwrap()),
		Err(failure) => Outcome::Failure {
			error: failure.error,
			call_chain: failure.call_chain.iter().copied().collect(),
			cleanup_error: failure.cleanup_error,
		},
	};
	Observed {
		outcome,
		live_entries: s.live_entries(),
		idle: executor.is_idle(),
	}
}

/// Runs `function` with and without installed regions for every fuel in `fuels`, asserting
/// identical observations. Returns the accumulated region statistics.
fn differential(
	code: &Code,
	function: u32,
	args: &[Arg],
	fuels: impl IntoIterator<Item = usize>,
) -> Stats {
	let mut total = Stats::default();
	for fuel in fuels {
		hook::uninstall();
		let plain = invoke(code, function, args, fuel);
		hook::install(code);
		let regions = invoke(code, function, args, fuel);
		let stats = hook::uninstall().unwrap();
		assert_eq!(plain, regions, "fuel {fuel}, args {args:?}");
		assert!(plain.idle && plain.live_entries == 0);
		total.regions = stats.regions;
		total.entries += stats.entries;
		total.continue_exits += stats.continue_exits;
		total.fuel_exits += stats.fuel_exits;
		total.retry_exits += stats.retry_exits;
		total.refused_cancel += stats.refused_cancel;
		total.refused_store += stats.refused_store;
		total.refused_tag += stats.refused_tag;
		total.refused_other += stats.refused_other;
	}
	total
}

/// The smallest fuel with which the plain interpreter finishes (with any outcome other than
/// fuel exhaustion), searched up to `max`.
pub(super) fn fuel_to_finish(code: &Code, function: u32, args: &[Arg], max: usize) -> usize {
	hook::uninstall();
	(0..=max)
		.find(|&fuel| match invoke(code, function, args, fuel).outcome {
			Outcome::Failure { error, .. } => {
				error.kind != crate::experimental_core::error::ErrorKind::FuelExhausted
			}
			Outcome::Value(_) => true,
		})
		.expect("finishes within max fuel")
}

/// Every fuel value from 0 to two past completion.
fn sweep(code: &Code, function: u32, args: &[Arg]) -> Stats {
	let done = fuel_to_finish(code, function, args, 20_000);
	differential(code, function, args, 0..=done + 2)
}

pub(super) fn main_of(program: &frontend::Program) -> u32 {
	program.function(Hash::type_hash(["main"])).unwrap()
}

// ---------------------------------------------------------------- lattice laws

#[test]
fn lattices_are_join_semilattices_with_bottom() {
	let tys = [Ty::Bottom, Ty::I64, Ty::Bool, Ty::Top];
	let ws = [Written::No, Written::Yes, Written::Maybe];
	let ks = [Konst::Bottom, Konst::Const(0), Konst::Const(17), Konst::Top];
	let fs = [FTy::Bottom, FTy::Unit, FTy::Bool, FTy::I64, FTy::Top];
	macro_rules! laws {
		($xs:expr, $bottom:expr, $top:expr) => {
			for a in $xs {
				assert_eq!(a.join(a), a);
				if let Some(b) = $bottom {
					assert_eq!(a.join(b), a);
				}
				assert_eq!(a.join($top), $top);
				for b in $xs {
					assert_eq!(a.join(b), b.join(a));
					for c in $xs {
						assert_eq!(a.join(b).join(c), a.join(b.join(c)));
					}
				}
			}
		};
	}
	laws!(tys, Some(Ty::Bottom), Ty::Top);
	laws!(ws, None::<Written>, Written::Maybe);
	laws!(ks, Some(Konst::Bottom), Konst::Top);
	laws!(fs, Some(FTy::Bottom), FTy::Top);
	// Product join is componentwise: an unwritten entry joined with an I64 write is
	// "maybe written, I64 when written", never Top.
	let entry = SlotState {
		written: Written::No,
		ty: Ty::Bottom,
		konst: Konst::Bottom,
	};
	let wrote = SlotState {
		written: Written::Yes,
		ty: Ty::I64,
		konst: Konst::Const(1),
	};
	assert_eq!(
		entry.join(wrote),
		SlotState {
			written: Written::Maybe,
			ty: Ty::I64,
			konst: Konst::Const(1)
		}
	);
}

// ------------------------------------------------- hand-built eligibility checks

pub(super) fn lit(f: &mut Function, out: u32, value: Value, at: u32) {
	f.push(Op::Literal { out, value }, loc(at)).unwrap();
}

pub(super) fn bin(f: &mut Function, out: u32, op: Binary, lhs: u32, rhs: u32, at: u32) {
	f.push(Op::Binary { out, op, lhs, rhs }, loc(at)).unwrap();
}

/// `fn(limit) { i = 0; s = 0; while i < limit { s = s <op> d; i = i + 1 } return s }`
/// with `d` from `divisor` (a literal written inside the loop) or the parameter slot 0.
pub(super) fn counted(op: Binary, divisor: Option<Value>) -> Code {
	// slots: 0 limit/param, 1 i, 2 s, 3 cond, 4 one, 5 d
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	lit(&mut f, 2, Value::I64(100), 1);
	bin(&mut f, 3, Binary::Lt, 1, 0, 2); // pc 2: header
	f.push(
		Op::Branch {
			condition: 3,
			yes: 4,
			no: 9,
		},
		loc(3),
	)
	.unwrap();
	match divisor {
		Some(v) => lit(&mut f, 5, v, 4),
		None => f
			.push(Op::Copy { out: 5, value: 0 }, loc(4))
			.map(|_| ())
			.unwrap(),
	}
	bin(&mut f, 2, op, 2, 5, 5);
	lit(&mut f, 4, Value::I64(1), 6);
	bin(&mut f, 1, Binary::Add, 1, 4, 7);
	f.push(Op::Jump { target: 2 }, loc(8)).unwrap();
	f.push(Op::Return { value: 2 }, loc(9)).unwrap();
	build(vec![f])
}

pub(super) fn regions(code: &Code, function: u32) -> StdVec<(u32, Result<Region, RegionRefusal>)> {
	analyze(&code, function)
		.unwrap()
		.into_iter()
		.collect()
}

#[test]
fn counted_loop_region_shape_and_constant_divisors() {
	for (divisor, constant) in [
		(Value::I64(17), Some(17)),
		(Value::I64(-17), Some(-17)),
		(Value::I64(0), None),
		(Value::I64(-1), None),
	] {
		for op in [Binary::Rem, Binary::Div] {
			let code = counted(op, Some(divisor));
			let found = regions(&code, 0);
			assert_eq!(found.len(), 1);
			let (header, region) = &found[0];
			assert_eq!(*header, 2);
			let region = region.as_ref().unwrap();
			// limit and s are guarded live-ins; i is proven by the function-level pass;
			// the others are written before read.
			assert_eq!(region.slots.as_slice(), &[0, 1, 2, 3, 4, 5]);
			assert_eq!(
				region.entry.as_slice(),
				&[
					EntryClass::Guard(Scalar::I64),
					EntryClass::Guard(Scalar::I64),
					EntryClass::Guard(Scalar::I64),
					EntryClass::Scalar,
					EntryClass::Scalar,
					EntryClass::Scalar,
				]
			);
			assert_eq!(region.proven, 0b110, "i and s proven, limit is a parameter");
			let arith = region
				.nodes
				.iter()
				.find_map(|n| match n.op {
					NodeOp::Arith {
						op: o,
						retry,
						divisor,
						..
					} if o == op => Some((retry, divisor)),
					_ => None,
				})
				.unwrap();
			assert_eq!(arith.1, constant);
			assert_eq!(arith.0.is_none(), constant.is_some());
			// One fuel exit (the header is the only charged pc), one continue exit.
			let count = |s| region.exits.iter().filter(|e| e.status == s).count();
			assert_eq!(count(Status::Fuel), 1);
			assert_eq!(count(Status::Continue), 1);
			let cont = region
				.exits
				.iter()
				.find(|e| e.status == Status::Continue)
				.unwrap();
			assert_eq!(cont.pc, 9);
			// At the loop exit, cond (Bool) is definitely written; i, s, one, d maybe.
			assert_eq!(cont.definite, 0b001000);
			assert_eq!(cont.maybe, 0b110110);
			assert_eq!(cont.bools, 0b001000);
		}
	}
	// A divisor that is the parameter is never a constant, even if every call passes 17.
	let code = counted(Binary::Rem, None);
	assert_eq!(
		regions(&code, 0)[0].1.as_ref().err(),
		Some(&RegionRefusal::Type { pc: 5 }),
		"P2b: the Copy (pc 4) of the never-written parameter is admitted as an op, but its out \
		 is Top, so the Rem reading it is refused by the read-type check (which runs before \
		 the CopySource check)"
	);
}

#[test]
fn refusals_side_entry_type_conflict_live_in_conflict_and_slots() {
	// Side entry: pc 0 jumps into the middle of the loop 2..=4 headed at 2.
	let mut f = Function::new(3, 0).unwrap();
	lit(&mut f, 0, Value::I64(0), 0); // 0
	f.push(Op::Jump { target: 3 }, loc(1)).unwrap(); // 1
	bin(&mut f, 1, Binary::Lt, 0, 0, 2); // 2 header
	lit(&mut f, 2, Value::I64(1), 3); // 3 (side entry)
	f.push(Op::Jump { target: 2 }, loc(4)).unwrap(); // 4
	let code = build(vec![f]);
	assert_eq!(
		regions(&code, 0)[0].1.as_ref().err(),
		Some(&RegionRefusal::SideEntry { pc: 3 })
	);

	// Type conflict: slot 1 is I64 on one path and Bool on the other, then read by Add.
	let mut f = Function::new(4, 1).unwrap();
	lit(&mut f, 2, Value::I64(1), 0); // 0
	lit(&mut f, 3, Value::Bool(true), 1); // 1
	f.push(
		Op::Branch {
			condition: 3,
			yes: 3,
			no: 5,
		},
		loc(2),
	)
	.unwrap(); // 2 header (back-edge target)
	lit(&mut f, 1, Value::I64(5), 3); // 3
	f.push(Op::Jump { target: 6 }, loc(4)).unwrap(); // 4
	lit(&mut f, 1, Value::Bool(false), 5); // 5
	bin(&mut f, 0, Binary::Add, 1, 2, 6); // 6
	f.push(Op::Jump { target: 2 }, loc(7)).unwrap(); // 7
	let code = build(vec![f]);
	assert_eq!(
		regions(&code, 0)[0].1.as_ref().err(),
		Some(&RegionRefusal::Type { pc: 6 })
	);

	// Path-dependent tag: slot 1 is I64 on one path and Bool on the other, never read. The
	// region is admitted; slot 1's exit tag comes from the dynamic bool mask.
	let mut f = Function::new(4, 0).unwrap();
	lit(&mut f, 3, Value::Bool(true), 0); // 0
	f.push(
		Op::Branch {
			condition: 3,
			yes: 2,
			no: 7,
		},
		loc(1),
	)
	.unwrap(); // 1 header
	f.push(
		Op::Branch {
			condition: 2,
			yes: 3,
			no: 5,
		},
		loc(2),
	)
	.unwrap(); // 2
	lit(&mut f, 1, Value::I64(5), 3); // 3
	f.push(Op::Jump { target: 6 }, loc(4)).unwrap(); // 4
	lit(&mut f, 1, Value::Bool(false), 5); // 5
	f.push(Op::Jump { target: 1 }, loc(6)).unwrap(); // 6
	f.push(Op::Return { value: 3 }, loc(7)).unwrap(); // 7
	let code = build(vec![f]);
	let found = regions(&code, 0);
	let region = found[0].1.as_ref().unwrap();
	assert_eq!(region.slots.as_slice(), &[1, 2, 3]);
	assert_eq!(region.tracked, 0b001);
	assert!(region
		.exits
		.iter()
		.all(|e| e.dynamic == 0b001 && e.maybe == 0b001));
	// Either path, then a fuel exit: the record carries the tag of the last write.
	for (inner, last_bool) in [(1, false), (0, true)] {
		let mut buf = [0, inner, 1];
		let mut record = ExitRecord::new(2);
		simulate(region, &mut buf, &mut record);
		assert_eq!(record.status, Status::Fuel as u32);
		assert_eq!(record.written, 0b001);
		assert_eq!(record.bools, u64::from(last_bool));
	}

	// Live-in conflict: slot 0 is read as a Bool condition and as an I64 operand before
	// the region writes it.
	let mut f = Function::new(3, 1).unwrap();
	f.push(
		Op::Branch {
			condition: 0,
			yes: 1,
			no: 3,
		},
		loc(0),
	)
	.unwrap(); // 0 header: also the function entry
	bin(&mut f, 1, Binary::Add, 0, 0, 1); // 1
	f.push(Op::Jump { target: 0 }, loc(2)).unwrap(); // 2
	f.push(Op::Return { value: 1 }, loc(3)).unwrap(); // 3
	let code = build(vec![f]);
	assert_eq!(
		regions(&code, 0)[0].1.as_ref().err(),
		Some(&RegionRefusal::LiveIn { pc: 1, slot: 0 })
	);

	// 65 touched slots.
	let mut f = Function::new(70, 0).unwrap();
	lit(&mut f, 69, Value::Bool(false), 0); // 0
	for s in 0..65 {
		lit(&mut f, s, Value::I64(1), 1 + s); // 1..=65 (65 is... header at 1)
	}
	f.push(
		Op::Branch {
			condition: 69,
			yes: 1,
			no: 67,
		},
		loc(66),
	)
	.unwrap(); // 66
	f.push(Op::Return { value: 0 }, loc(67)).unwrap(); // 67
	let code = build(vec![f]);
	assert_eq!(
		regions(&code, 0)[0].1.as_ref().err(),
		Some(&RegionRefusal::Slots)
	);
}

#[test]
fn function_level_inference_reaches_a_fixed_point_over_loops() {
	let code = counted(Binary::Rem, Some(Value::I64(17)));
	let f = code.function(0).unwrap();
	let types = function_types(f).unwrap().unwrap();
	let at = |pc: usize| &types[pc * 6..pc * 6 + 6];
	// At entry: the parameter is Top, everything else Unit.
	assert_eq!(
		at(0),
		&[
			FTy::Top,
			FTy::Unit,
			FTy::Unit,
			FTy::Unit,
			FTy::Unit,
			FTy::Unit
		]
	);
	// At the header, the back edge joined: cond/one/d are Unit or Bool/I64 → Top.
	assert_eq!(
		at(2),
		&[FTy::Top, FTy::I64, FTy::I64, FTy::Top, FTy::Top, FTy::Top]
	);
	// The parameter stays Top everywhere: never a constant, never proven.
	assert!((0..f.len()).all(|pc| at(pc)[0] == FTy::Top || at(pc)[0] == FTy::Bottom));
}

// ----------------------------------------------------------------- commit checks

pub(super) fn while_region() -> (Code, Region) {
	let code = counted(Binary::Rem, Some(Value::I64(17)));
	let region = analyze(&code, 0)
		.unwrap()
		.into_iter()
		.next()
		.unwrap()
		.1
		.unwrap();
	(code, region)
}

pub(super) fn window() -> StdVec<Value> {
	vec![
		Value::I64(10),
		Value::I64(0),
		Value::I64(100),
		Value::Unit,
		Value::Unit,
		Value::Unit,
	]
}

#[test]
fn entry_refusals_change_nothing() {
	let (_code, region) = while_region();
	let mut s = store();
	let original = window();
	let never = |_: &Region, _: &mut [i64], _: &mut ExitRecord| -> u32 { panic!("not entered") };
	let mut w = original.clone();
	assert_eq!(
		commit::enter(&region, &mut w, &mut s, true, 50, commit::ROOM_FOR_TEST, never).unwrap(),
		Err(Refused::Cancel)
	);
	assert_eq!(w, original);
	// A wrong live-in tag (U64 limit, F64 counter) refuses.
	for (slot, value) in [
		(0, Value::U64(10)),
		(1, Value::F64(0.0)),
		(2, Value::Bool(true)),
	] {
		let mut w = original.clone();
		w[slot] = value;
		let before = w.clone();
		assert_eq!(
			commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, never).unwrap(),
			Err(Refused::Tag { slot: slot as u32 })
		);
		assert_eq!(w, before);
	}
	// A written-only slot holding a live heap handle (dead but owned) refuses: its release
	// must stay at the interpreter's instruction, in the interpreter's order.
	let mut w = original.clone();
	w[3] = s.string("owned").unwrap();
	let before = w.clone();
	assert_eq!(
		commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, never).unwrap(),
		Err(Refused::Tag { slot: 3 })
	);
	assert_eq!(w, before);
	assert_eq!(s.live_entries(), 1);
	s.release(w[3]).unwrap();
	// A closed store refuses.
	s.close().unwrap();
	let mut w = original.clone();
	assert_eq!(
		commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, never).unwrap(),
		Err(Refused::Store)
	);
	assert_eq!(w, original);
}

#[test]
fn forged_exit_records_are_rejected_before_any_write() {
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
	let e = region.exits[cont as usize];
	type Forge = fn(&mut [i64], &mut ExitRecord, u32, u32, u32);
	fn honest(_: &mut [i64], r: &mut ExitRecord, exit: u32, pc: u32, _: u32) {
		r.status = 1;
		r.exit = exit;
		r.pc = pc;
		r.written = 0b001000;
		r.fuel = 3;
	}
	let forged: [(&str, Forge); 10] = [
		("status 0", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.status = 0;
		}),
		("status mismatch", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.status = 2;
		}),
		("pc mismatch", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.pc = p + 1;
		}),
		("exit out of range", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.exit = 9999;
		}),
		("written outside map", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.written |= 1 << 40;
		}),
		("definite not written", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.written = 0;
		}),
		("bool not 0/1", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			b[3] = 2;
		}),
		("untracked dynamic bool bit", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.bools = 1 << 3;
		}),
		("fuel grew", |b, r, x, p, f| {
			honest(b, r, x, p, f);
			r.fuel = 51;
		}),
		("fuel exit with fuel", |_, r, _, _, f| {
			r.status = 2;
			r.exit = f;
			r.pc = 2;
			r.fuel = 1;
		}),
	];
	for (name, forge) in forged {
		let mut s = store();
		let original = window();
		let mut w = original.clone();
		let result = commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, |_, b, r| {
			forge(b, r, cont, e.pc, fuel_exit);
			r.status
		})
		.unwrap();
		assert_eq!(result, Err(Refused::InvalidExit), "{name}");
		assert_eq!(w, original, "{name}");
	}
	// The honest record commits exactly the written slot.
	let mut s = store();
	let mut w = window();
	let result = commit::enter(&region, &mut w, &mut s, false, 50, commit::ROOM_FOR_TEST, |_, b, r| {
		honest(b, r, cont, e.pc, fuel_exit);
		b[3] = 1;
		r.status
	})
	.unwrap()
	.unwrap();
	assert_eq!((result.pc, result.fuel), (9, 3));
	assert_eq!(w[3], Value::Bool(true));
	assert_eq!(&w[..3], &window()[..3]);
	assert_eq!(&w[4..], &window()[4..]);
}

#[test]
fn simulator_fuel_and_retry_contract_per_charged_pc() {
	// s = s * 2 overflows; fuel is checked at the header only.
	let code = counted(Binary::Mul, Some(Value::I64(2)));
	let region = analyze(&code, 0)
		.unwrap()
		.into_iter()
		.next()
		.unwrap()
		.1
		.unwrap();
	// limit 100, i 0, s = 2^61: iteration 1 makes 2^62, iteration 2 overflows.
	for fuel in 0..6u64 {
		let mut buf = [100, 0, 1 << 61, 0, 0, 0];
		let mut record = ExitRecord::new(fuel);
		let status = simulate(&region, &mut buf, &mut record);
		let exit = region.exits[record.exit as usize];
		match fuel {
			0 | 1 => {
				// Fuel exit at the header, before charging, fuel 0.
				assert_eq!(status, Status::Fuel as u32);
				assert_eq!((record.pc, record.fuel), (2, 0));
			}
			_ => {
				// Retry at the Mul (pc 5, uncharged): fuel is what remained, the
				// destination still holds 2^62, and pc 5 has not run.
				assert_eq!(status, Status::Retry as u32);
				assert_eq!((record.pc, record.fuel), (5, fuel - 2));
				assert_eq!(buf[2], 1 << 62);
				assert!(!exit.restore);
			}
		}
		if fuel == 1 {
			// One completed iteration: s was written.
			assert_eq!(buf[2], 1 << 62);
			assert_ne!(record.written & 0b100, 0);
		}
	}
}

// ------------------------------------------------ differential over Rune programs

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn kernels_while_and_numeric_every_fuel() {
	for (src, args, expect) in [
        (
            "pub fn main(a) { let n = 0; while n < a { n += 1; } n }",
            [Arg::I(25)],
            25,
        ),
        (
            "pub fn main(a) { let n = 0; let total = 0; while n < a { total += n % 17; n += 1; } total }",
            [Arg::I(40)],
            (0..40).map(|n| n % 17).sum::<i64>(),
        ),
    ] {
        let program = compile(src);
        let main = main_of(&program);
        assert_eq!(
            invoke(&program.code, main, &args, 1_000_000).outcome,
            Outcome::Value(Some(HostScalar::I64(expect)))
        );
        let stats = sweep(&program.code, main, &args);
        assert_eq!(stats.regions, 1, "{src}");
        assert!(stats.entries > 0 && stats.fuel_exits > 0 && stats.continue_exits > 0);
        assert_eq!(stats.refused_tag, 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn every_arithmetic_error_boundary_and_fuel() {
	let programs = [
		// Add/Sub/Mul overflow at every iteration of a 4-iteration loop.
		"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += b; i += 1; } x }",
		"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x -= b; i += 1; } x }",
		"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x *= b; i += 1; } x }",
		// out == lhs == rhs (AssignBinary); `x = x + x` would lower to Binary + Copy +
		// Release, which P0 does not admit.
		"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += x; i += b; } x }",
		// Division and remainder by a runtime divisor: zero and MIN / -1.
		"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a / b; i += 1; } t }",
		"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a % b; i += 1; } t }",
		// The faulting op's destination is a live-in read later in the same iteration.
		"pub fn main(a, b) { let i = 0; while i < 4 { a += b; b += 1; i += 1; } a + b }",
	];
	let max = i64::MAX;
	let min = i64::MIN;
	let mut inputs = StdVec::new();
	for k in 0..6 {
		inputs.push([Arg::I(max - k), Arg::I(1)]);
		inputs.push([Arg::I(min + k), Arg::I(1)]);
		inputs.push([Arg::I(max / 8 - k), Arg::I(2)]);
		inputs.push([Arg::I(min / 8 + k), Arg::I(2)]);
	}
	inputs.push([Arg::I(1 << 60), Arg::I(1)]);
	inputs.push([Arg::I(7), Arg::I(0)]);
	inputs.push([Arg::I(min), Arg::I(-1)]);
	inputs.push([Arg::I(min), Arg::I(1)]);
	inputs.push([Arg::I(max), Arg::I(-1)]);
	for src in programs {
		let program = compile(src);
		let main = main_of(&program);
		let mut stats = Stats::default();
		for args in &inputs {
			let s = sweep(&program.code, main, args);
			stats.entries += s.entries;
			stats.retry_exits += s.retry_exits;
		}
		assert!(stats.entries > 0, "{src}");
		assert!(
			stats.retry_exits > 0,
			"{src}: some input faults inside the region"
		);
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn nested_callers_keep_error_locations_and_call_chains() {
	let src = "
        fn inner(a, b) { let x = a; let i = 0; while i < 5 { x += b; i += 1; } x }
        fn mid(a, b) { let y = inner(a, b); y - 1 }
        pub fn main(a, b) { let z = mid(a, b); z + 1 }
    ";
	let program = compile(src);
	let main = main_of(&program);
	for args in [
		[Arg::I(1), Arg::I(1)],
		[Arg::I(i64::MAX - 2), Arg::I(1)],
		[Arg::I(i64::MAX), Arg::I(1)],
		[Arg::I(i64::MIN + 3), Arg::I(-1)],
	] {
		let stats = sweep(&program.code, main, &args);
		assert!(stats.entries > 0);
	}
	// The failure inside the region carries the two callsites, innermost caller first.
	hook::install(&program.code);
	let observed = invoke(
		&program.code,
		main,
		&[Arg::I(i64::MAX - 2), Arg::I(1)],
		10_000,
	);
	hook::uninstall();
	match observed.outcome {
		Outcome::Failure { call_chain, .. } => assert_eq!(call_chain.len(), 2),
		other => panic!("expected a failure, got {other:?}"),
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn branches_path_dependent_writes_and_bool_live_ins() {
	for (src, args) in [
        (
            "pub fn main(a) { let i = 0; let s = 0; while i < a { if i % 3 == 0 { s += i; } else { s -= 1; } i += 1; } s }",
            vec![Arg::I(20)],
        ),
        (
            "pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } else { s += 1; } i += 1; } s }",
            vec![Arg::I(9), Arg::B(true)],
        ),
        (
            "pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } else { s += 1; } i += 1; } s }",
            vec![Arg::I(9), Arg::B(false)],
        ),
        (
            "pub fn main(a) { let i = 0; let s = 0; while i < a { if i * 2 > 6 { s += i * 2; } i += 1; } s }",
            vec![Arg::I(12)],
        ),
        (
            "pub fn main(a) { let i = 0; let s = 0; while i < a { if i > 3 { if i < 9 { s += 1; } } i += 1; } s }",
            vec![Arg::I(12)],
        ),
    ] {
        let program = compile(src);
        let main = main_of(&program);
        let stats = sweep(&program.code, main, &args);
        assert!(stats.entries > 0, "{src}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn wrong_live_in_tags_fall_back_to_the_interpreter() {
	let src = "pub fn main(a) { let n = 0; while n < a { n += 1; } n }";
	let program = compile(src);
	let main = main_of(&program);
	// U64 limit: the interpreter coerces and finishes; F64: a Type error. Never entered.
	for args in [[Arg::U(7)], [Arg::F(7.0)]] {
		let stats = sweep(&program.code, main, &args);
		assert_eq!(stats.entries, 0);
		assert!(stats.refused_tag > 0);
	}
	// A Bool condition passed as an I64: refused at entry, the interpreter's Type error.
	let program = compile(
        "pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } i += 1; } s }",
    );
	let stats = sweep(&program.code, main_of(&program), &[Arg::I(5), Arg::I(1)]);
	assert_eq!(stats.entries, 0);
	assert!(stats.refused_tag > 0);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn regions_re_entered_inside_a_refused_outer_loop_and_nested_admitted_loops() {
	for (src, outer_admitted) in [
        (
            "fn id(x) { x } pub fn main(a) { let i = 0; let s = 0; while i < a { let j = 0; while j < 3 { s += j; j += 1; } s = id(s); i += 1; } s }",
            false,
        ),
        (
            "pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
            true,
        ),
    ] {
        let program = compile(src);
        let main = main_of(&program);
        let stats = sweep(&program.code, main, &[Arg::I(6)]);
        assert_eq!(stats.regions, if outer_admitted { 2 } else { 1 }, "{src}");
        assert!(stats.entries > 1, "{src}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "Miri: differential sweeps over compiled Rune source")]
fn armed_cancellation_hook_and_reused_executor() {
	let program = compile("pub fn main(a) { let n = 0; while n < a { n += 1; } n }");
	let main = main_of(&program);
	// Cancel at every charged instruction index: the region stays out while the test hook
	// is armed, and the outcome equals the interpreter's.
	for k in 0..30 {
		hook::uninstall();
		crate::experimental_core::execute::cancel_at_charge_for_test(Some(k));
		let plain = invoke(&program.code, main, &[Arg::I(20)], 10_000);
		hook::install(&program.code);
		crate::experimental_core::execute::cancel_at_charge_for_test(Some(k));
		let regions = invoke(&program.code, main, &[Arg::I(20)], 10_000);
		crate::experimental_core::execute::cancel_at_charge_for_test(None);
		hook::uninstall();
		assert_eq!(plain, regions, "cancel at charge {k}");
	}
	// One executor, many invocations: idle after success, fuel halt and error alike.
	hook::install(&program.code);
	let mut executor = Executor::new().unwrap();
	let mut s = store();
	for (arg, fuel) in [(5, 100), (5, 2), (i64::MIN, 100), (30, 100), (30, 7)] {
		let root = s.root_i64(arg);
		let _ = executor.invoke_code(&program.code, &mut s, main, &[&root], fuel);
		assert!(executor.is_idle());
		assert_eq!(s.live_entries(), 0);
	}
	let stats = hook::uninstall().unwrap();
	assert!(stats.entries >= 3);
}

#[test]
fn dead_but_owned_heap_slot_refuses_entry_and_matches_the_interpreter() {
	// slot 2 holds a live string when the loop is first reached; the loop overwrites it
	// with an I64. The interpreter releases the string at that write; the region must not
	// enter while the slot holds it, and enters on later arrivals once it is scalar.
	let mut f = Function::new(5, 1).unwrap();
	let text = f.string("owned").unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	f.push(Op::String { out: 2, text }, loc(1)).unwrap(); // 1
	bin(&mut f, 3, Binary::Lt, 1, 0, 2); // 2 header
	f.push(
		Op::Branch {
			condition: 3,
			yes: 4,
			no: 8,
		},
		loc(3),
	)
	.unwrap(); // 3
	lit(&mut f, 2, Value::I64(1), 4); // 4: overwrites the string
	bin(&mut f, 1, Binary::Add, 1, 2, 5); // 5
	f.push(Op::Jump { target: 2 }, loc(6)).unwrap(); // 6
	f.push(Op::Return { value: 1 }, loc(7)).unwrap(); // 7 (unreached)
	f.push(Op::Return { value: 1 }, loc(8)).unwrap(); // 8
	let code = build(vec![f]);
	let stats = differential(&code, 0, &[Arg::I(5)], 0..30);
	assert!(
		stats.refused_tag > 0,
		"first arrival refused: slot 2 owns a string"
	);
	assert!(stats.entries > 0, "a later arrival enters");
}

/// Listing for receipts (run with `--nocapture`): the decoded bytecode of each kernel-like
/// program and the analysis outcome of every candidate header.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn listing_of_programs_and_region_outcomes() {
	for src in LISTED {
		let program = compile(src);
		std::println!("=== {src}");
		for id in 0..program.code.functions() as u32 {
			let f = program.code.function(id).unwrap();
			std::println!(
				"function {id}: slots={} parameters={}",
				f.slots,
				f.parameters
			);
			for pc in 0..f.len() {
				let charged = if f.instructions()[pc].is_charged() {
					"C"
				} else {
					" "
				};
				std::println!("  {pc:4} {charged} {:?}", f.operation(pc).unwrap());
			}
			for (header, region) in analyze(&program.code, id).unwrap() {
				match region {
					Ok(r) => {
						std::println!(
							"  region @{header}: slots {:?} entry {:?} proven {:#b}",
							r.slots.as_slice(),
							r.entry.as_slice(),
							r.proven
						);
						for n in r.nodes.iter() {
							std::println!("    node {n:?}");
						}
						for (i, e) in r.exits.iter().enumerate() {
							std::println!("    exit {i}: {e:?}");
						}
					}
					Err(refusal) => std::println!("  region @{header}: refused {refusal:?}"),
				}
			}
		}
	}
}

pub(super) const LISTED: &[&str] = &[
    "pub fn main(_a) { let n = 0; while n < 10000 { n += 1; } n }",
    "pub fn main(_a) { let n = 0; let total = 0; while n < 10000 { total += n % 17; n += 1; } total }",
    "pub fn main(a) { let i = 0; let s = 0; while i < a { let t = i * 2; if t > 6 { s = s + t; } i += 1; } s }",
    "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x = x + x; i += b; } x }",
    "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += x; i += b; } x }",
    "pub fn main(a) { let i = 0; let s = 0; while i < a { let j = 0; while j < i { s += j; j += 1; } i += 1; } s }",
    "pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j = 0; while j < i { s += j; j += 1; } i += 1; } s }",
    "pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
    "pub fn main(a) { let i = 0; let s = 0; while i < a { if i > 3 { if i < 9 { s += 1; } } i += 1; } s }",
    "fn add(a, b) { a + b } pub fn main(_a) { let total = 0; for i in 0..10000 { total = add(total, i); } total }",
];

// ------------------------------------------ P1 safe glue (no native code; Miri-able)

#[test]
fn p1_hook_modes_suspend_and_division_classification() {
	// Division kinds of the counted loop: constant 0 and -1 keep their Retry exits and are
	// classified; a constant outside {0, -1} has no fallible division.
	for (divisor, kind) in [
		(Value::I64(0), Some(hook::DivisionKind::Const0)),
		(Value::I64(-1), Some(hook::DivisionKind::ConstNeg1)),
		(Value::I64(17), None),
	] {
		for op in [Binary::Div, Binary::Rem] {
			let code = counted(op, Some(divisor));
			let region = regions(&code, 0).remove(0).1.unwrap();
			assert_eq!(hook::division_kind(&region), kind, "{op:?} by {divisor:?}");
		}
	}

	// A divisor written by two different literals is not a constant: runtime.
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 2, Value::I64(100), 1); // 1
	bin(&mut f, 3, Binary::Lt, 1, 0, 2); // 2: header
	f.push(
		Op::Branch {
			condition: 3,
			yes: 4,
			no: 12,
		},
		loc(3),
	)
	.unwrap(); // 3
	lit(&mut f, 5, Value::I64(0), 4); // 4
	bin(&mut f, 3, Binary::Lt, 2, 1, 5); // 5
	f.push(
		Op::Branch {
			condition: 3,
			yes: 7,
			no: 8,
		},
		loc(6),
	)
	.unwrap(); // 6
	lit(&mut f, 5, Value::I64(3), 7); // 7
	bin(&mut f, 2, Binary::Rem, 2, 5, 8); // 8
	lit(&mut f, 4, Value::I64(1), 9); // 9
	bin(&mut f, 1, Binary::Add, 1, 4, 10); // 10
	f.push(Op::Jump { target: 2 }, loc(11)).unwrap(); // 11 (back edge)
	f.push(Op::Return { value: 2 }, loc(12)).unwrap(); // 12
	let code = build(vec![f]);
	let region = regions(&code, 0).remove(0).1.unwrap();
	assert_eq!(hook::division_kind(&region), Some(hook::DivisionKind::Runtime));

	// Mode bookkeeping and suspend/resume over the counted loop.
	let code = counted(Binary::Rem, Some(Value::I64(17)));
	hook::uninstall();
	let plain = invoke(&code, 0, &[Arg::I(6)], 1_000);
	let plain_fuel = hook::take_final_fuel();
	assert!(plain_fuel.is_some());

	assert_eq!(hook::install(&code), 1);
	hook::set_mode(hook::Mode::Off);
	assert_eq!(invoke(&code, 0, &[Arg::I(6)], 1_000), plain);
	assert_eq!(hook::take_final_fuel(), plain_fuel);
	assert_eq!(hook::take_stats().unwrap().entries, 0);

	hook::set_mode(hook::Mode::Simulate);
	assert_eq!(invoke(&code, 0, &[Arg::I(6)], 1_000), plain);
	assert_eq!(hook::take_final_fuel(), plain_fuel);
	let stats = hook::take_stats().unwrap();
	assert!(stats.entries > 0 && stats.native_entries == 0);
	assert_eq!(hook::take_stats().unwrap().entries, 0, "take_stats resets");

	hook::suspend();
	assert!(hook::take_stats().is_none(), "suspended: no install visible");
	assert_eq!(invoke(&code, 0, &[Arg::I(6)], 1_000), plain);
	hook::resume();
	assert_eq!(hook::take_stats().unwrap().entries, 0);
	assert!(hook::take_mismatch().is_none());
	hook::uninstall();
}

// ---------------------------------- P3a safe glue: budgeted analysis (Miri-able)

/// `count` sequential counted loops (`many_headers` of the P2 tests), hand-built.
fn sequential_loops(count: u32) -> Function {
	// slots: 0 a, 1 n, 2 c, 3 one
	let mut f = Function::new(4, 1).unwrap();
	for k in 0..count {
		let h = 5 * k + 1;
		lit(&mut f, 1, Value::I64(0), h - 1);
		bin(&mut f, 2, Binary::Lt, 1, 0, h);
		f.push(
			Op::Branch {
				condition: 2,
				yes: h + 2,
				no: h + 4,
			},
			loc(h + 1),
		)
		.unwrap();
		bin(&mut f, 1, Binary::Add, 1, 3, h + 2);
		f.push(Op::Jump { target: h }, loc(h + 3)).unwrap();
	}
	f.push(Op::Return { value: 1 }, loc(5 * count)).unwrap();
	f
}

#[test]
fn p3a_scan_headers_matches_back_edge_targets() {
	for count in [0u32, 1, 3, 17] {
		let code = build(vec![sequential_loops(count)]);
		let f = code.function(0).unwrap();
		let budget = super::budget::Budget::unlimited();
		let headers = super::scan_headers(f, &budget).unwrap();
		let expected: StdVec<u32> = (0..count).map(|k| 5 * k + 1).collect();
		assert_eq!(&headers[..], &expected[..]);
		// The bitmap (one byte per instruction) and the list (exact) were both charged.
		assert_eq!(budget.peak(), f.len() + 4 * count as usize);
		drop(headers);
		assert_eq!(budget.used(), 0);
	}
}

#[test]
fn p3a_budgeted_analysis_equals_unbudgeted_and_refuses_cleanly() {
	use super::budget::{Budget, Held};
	use super::{FunctionContext, Stop, TRANSFER_FACTOR};

	let code = counted(Binary::Add, Some(Value::I64(3)));
	let f = code.function(0).unwrap();
	let reference = regions(&code, 0);
	let Ok(expected) = &reference[0].1 else {
		panic!("admitted")
	};

	// Unlimited: the peak is the most this function's analysis holds at once.
	let budget = Budget::unlimited();
	let headers = super::scan_headers(f, &budget).unwrap();
	let context = FunctionContext::new(&code, 0, &budget, TRANSFER_FACTOR).unwrap();
	let (region, bytes) = context.analyze_header(headers[0]).unwrap().unwrap();
	assert_eq!((region.nodes.as_slice(), region.exits.as_slice()), (expected.nodes.as_slice(), expected.exits.as_slice()));
	let held = Held::adopt(&budget, bytes);
	drop((headers, context, held, region));
	assert_eq!(budget.used(), 0, "every charge released");
	let peak = budget.peak();

	// k - 1 refuses (somewhere), k and k + 1 admit the identical region.
	for (limit, admitted) in [(peak - 1, false), (peak, true), (peak + 1, true)] {
		let budget = Budget::new(limit);
		let outcome = (|| -> Result<bool, Stop> {
			let headers = super::scan_headers(f, &budget)?;
			let context = FunctionContext::new(&code, 0, &budget, TRANSFER_FACTOR)?;
			let found = context.analyze_header(headers[0])?;
			let (region, bytes) = found.unwrap();
			let _held = Held::adopt(&budget, bytes);
			Ok(region.nodes.as_slice() == expected.nodes.as_slice())
		})();
		match outcome {
			Ok(same) => assert!(admitted && same, "limit {limit}"),
			Err(Stop::Budget) => assert!(!admitted, "limit {limit}"),
			Err(Stop::Error(e)) => panic!("{e:?}"),
		}
		assert_eq!(budget.used(), 0, "limit {limit}: nothing stays charged");
		assert!(budget.peak() <= limit);
	}
}

#[test]
fn p3a_transfer_cutoff_zero_slots_and_overflow() {
	use super::{cutoff, FunctionContext, TRANSFER_FACTOR};

	assert_eq!(cutoff(8, 10, 0), 80, "zero slots count as one");
	assert_eq!(cutoff(8, 10, 3), 240);
	assert_eq!(cutoff(0, 10, 3), 0);
	assert_eq!(cutoff(8, usize::MAX, 2), usize::MAX, "overflow: no cutoff");

	// A region that touches no slot: `x = 0; loop {}` (pc 1 jumps to itself).
	let mut f = Function::new(1, 1).unwrap();
	lit(&mut f, 0, Value::I64(0), 0);
	f.push(Op::Jump { target: 1 }, loc(1)).unwrap();
	let code = build(vec![f]);
	let found = regions(&code, 0);
	let Ok(region) = &found[0].1 else {
		panic!("zero-slot region admitted")
	};
	assert_eq!((region.slots.len(), region.nodes.len()), (0, 1));

	// Factor 0 refuses it by the cutoff; the production factor admits it.
	let budget = super::budget::Budget::unlimited();
	let refused = FunctionContext::new(&code, 0, &budget, 0).unwrap().analyze_header(1).unwrap();
	assert!(matches!(refused, Err(RegionRefusal::Steps)));
	let context = FunctionContext::new(&code, 0, &budget, TRANSFER_FACTOR).unwrap();
	assert!(context.analyze_header(1).unwrap().is_ok());
	assert_eq!(context.max_transfers(), 1);
}
