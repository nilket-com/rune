//! B1 region tests: native IterNext over i64 ranges, inlined leaf scalar callees and the explicit
//! Replay checkpoint, at the analysis, simulator and commit level. Safe glue only: nothing
//! here compiles or runs native code. Tests that compile Rune source or sweep every fuel
//! value are ignored under Miri; the hand-built analysis, entry and commit tests are not.
use super::abi::ExitRecord;
use super::budget::{Budget, Held};
use super::commit::{self, Refused, Room};
use super::hook::{self, Stats};
use super::simulate::simulate;
use super::tests::{build, compile, lit, loc, main_of, store, Arg, Outcome};
use super::{
	analyze, callee_charge_shape, charge_flags_inspected_for_test, check_expansion, scan_headers,
	FunctionContext, InlineReason, IterReason, NodeOp, Region, RegionRefusal, ReplayReason, Scalar,
	Status, Stop, Target, MAX_INLINE_SITES, MAX_INLINE_SLOTS, MAX_REGION_CELLS, MAX_REGION_SLOTS,
	TRANSFER_FACTOR, VIRTUAL_BASE,
};

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Code, Function, Op},
	execute::{region_room, Executor, ExecutorLimits},
	heap::Store,
	value::{Root, Type, Value},
};

use std::boxed::Box;
use std::format;
use std::string::String;
use std::vec::Vec as StdVec;

// ------------------------------------------------------------------------ builders

pub(crate) fn op(f: &mut Function, op: Op, at: u32) -> u32 {
	f.push(op, loc(at)).unwrap()
}

pub(crate) fn binary(f: &mut Function, out: u32, op_: Binary, lhs: u32, rhs: u32, at: u32) {
	op(
		f,
		Op::Binary {
			out,
			op: op_,
			lhs,
			rhs,
		},
		at,
	);
}

pub(crate) fn call(f: &mut Function, out: u32, function: u32, args: &[u32], at: u32) {
	let (args_start, args_len) = f.arguments(args).unwrap();
	op(
		f,
		Op::Call {
			out,
			function,
			args_start,
			args_len,
		},
		at,
	);
}

/// `fn add(a, b) { a + b }`: slots 3, parameters 2; pc 0 `Binary`, pc 1 `Return`.
pub(crate) fn add_fn() -> Function {
	let mut f = Function::new(3, 2).unwrap();
	binary(&mut f, 2, Binary::Add, 0, 1, 100);
	op(&mut f, Op::Return { value: 2 }, 101);
	f
}

/// The calls kernel exactly as the frontend lowers it: main = f0 (6 slots, 1
/// parameter), the callee `callee` = f1.
pub(crate) fn calls_main(callee_args: &[u32]) -> Function {
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	lit(&mut f, 3, Value::I64(0), 1);
	op(
		&mut f,
		Op::Range {
			out: 2,
			start: 3,
			end: 0,
		},
		2,
	);
	op(&mut f, Op::IterStart { out: 3, value: 2 }, 3);
	op(
		&mut f,
		Op::IterNext {
			out: 4,
			iterator: 3,
			none: 9,
		},
		4,
	);
	call(&mut f, 5, 1, callee_args, 5);
	op(&mut f, Op::Copy { out: 1, value: 5 }, 6);
	op(&mut f, Op::Release { value: 5 }, 7);
	op(&mut f, Op::Jump { target: 4 }, 8);
	op(&mut f, Op::Release { value: 4 }, 9);
	op(&mut f, Op::Release { value: 3 }, 10);
	op(&mut f, Op::Release { value: 2 }, 11);
	op(&mut f, Op::Return { value: 1 }, 12);
	f
}

pub(crate) fn calls_kernel() -> Code {
	build(vec![calls_main(&[1, 4]), add_fn()])
}

/// `fn(a, b) { s = 0; for i in a..b { s += i } s }` (frontend-shaped). Slots: 0 a, 1 b, 2 s,
/// 3 range, 4 iterator, 5 i. Header pc 4.
pub(crate) fn range_sum() -> Function {
	let mut f = Function::new(6, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0);
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		1,
	);
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 2);
	op(&mut f, Op::Jump { target: 4 }, 3);
	op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 4,
			none: 7,
		},
		4,
	);
	op(
		&mut f,
		Op::AssignBinary {
			out: 2,
			op: Binary::Add,
			lhs: 2,
			rhs: 5,
		},
		5,
	);
	op(&mut f, Op::Jump { target: 4 }, 6);
	op(&mut f, Op::Release { value: 4 }, 7);
	op(&mut f, Op::Release { value: 3 }, 8);
	op(&mut f, Op::Return { value: 2 }, 9);
	f
}

/// The range loop with `body` (ops at pcs 5.. before the back-edge jump; `none` exits to
/// the tail). Slots: 0 a, 1 b, 2 s, 3 range, 4 iterator, 5 i, 6.. scratch (`slots` total).
pub(crate) fn range_loop(slots: u32, body: impl FnOnce(&mut Function)) -> Function {
	let mut f = Function::new(slots, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0);
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		1,
	);
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 2);
	op(&mut f, Op::Jump { target: 4 }, 3);
	let header = op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 4,
			none: 0,
		},
		4,
	);
	body(&mut f);
	op(&mut f, Op::Jump { target: header }, 50);
	let end = op(&mut f, Op::Release { value: 4 }, 51);
	op(&mut f, Op::Release { value: 3 }, 52);
	op(&mut f, Op::Return { value: 2 }, 53);
	f.patch(
		header,
		Op::IterNext {
			out: 5,
			iterator: 4,
			none: end,
		},
	)
	.unwrap();
	f
}

/// `fn k(a, b) { a + 1 }`: b unused. Slots 4.
pub(crate) fn first_plus_one() -> Function {
	let mut f = Function::new(4, 2).unwrap();
	lit(&mut f, 3, Value::I64(1), 100);
	binary(&mut f, 2, Binary::Add, 0, 3, 101);
	op(&mut f, Op::Return { value: 2 }, 102);
	f
}

/// The one candidate region of function `function` at `header`.
pub(crate) fn outcome(code: &Code, function: u32, header: u32) -> Result<Region, RegionRefusal> {
	analyze(code, function)
		.unwrap()
		.into_iter()
		.find(|(h, _)| *h == header)
		.expect("candidate header")
		.1
}

fn admitted(code: &Code, function: u32, header: u32) -> Region {
	match outcome(code, function, header) {
		Ok(region) => region,
		Err(refusal) => panic!("f{function} h{header} refused: {refusal:?}"),
	}
}

fn refused(code: &Code, function: u32, header: u32) -> RegionRefusal {
	match outcome(code, function, header) {
		Ok(_) => panic!("f{function} h{header} admitted, expected a refusal"),
		Err(refusal) => refusal,
	}
}

fn local(region: &Region, slot: u32) -> usize {
	region
		.slots
		.iter()
		.position(|&s| s == slot)
		.expect("touched slot")
}

/// The header pc of every candidate of `function`.
fn headers(code: &Code, function: u32) -> StdVec<u32> {
	analyze(code, function)
		.unwrap()
		.into_iter()
		.map(|(h, _)| h)
		.collect()
}

pub(crate) const V2_CALLS: &str = "fn add(a, b) { a + b }\npub fn main(x) {\n\tlet total = 0;\n\tfor i in 0..x { total = add(total, i); }\n\ttotal\n}\n";
pub(crate) const STAGE_CALLS: &str = "fn add(a, b) { a + b }\npub fn main(_a) {\n\tlet total = 0;\n\tfor i in 0..10000 { total = add(total, i); }\n\tprintln!(\"{}\", total);\n}\n";
pub(crate) const V2_FIB: &str =
	"fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\npub fn main(x) { fib(x) }\n";

// ------------------------------------------------------------- full observation

#[derive(Debug, PartialEq)]
pub(super) struct Full {
	pub(super) outcome: Outcome,
	pub(super) live_entries: usize,
	pub(super) idle: bool,
	pub(super) fuel_left: Option<usize>,
	pub(super) store_state: String,
}

pub(super) fn invoke_full(code: &Code, function: u32, args: &[Arg], fuel: usize) -> Full {
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

fn sum(total: &mut Stats, s: &Stats) {
	total.entries += s.entries;
	total.continue_exits += s.continue_exits;
	total.fuel_exits += s.fuel_exits;
	total.retry_exits += s.retry_exits;
	total.replay_exits += s.replay_exits;
	total.replay_checked += s.replay_checked;
	total.refused_tag += s.refused_tag;
	total.refused_iter += s.refused_iter;
	total.refused_room += s.refused_room;
	total.refused_store += s.refused_store;
	total.refused_cancel += s.refused_cancel;
}

/// Plain interpreter against the simulator install (with the traced checkpoint check on
/// every Replay) at every fuel value from 0 to two past completion.
fn sweep(code: &Code, function: u32, args: &[Arg]) -> Stats {
	hook::uninstall();
	let reference = invoke_full(code, function, args, 1_000_000);
	let done = 1_000_000 - reference.fuel_left.expect("fuel");
	assert!(done < 20_000, "reduce the case");
	let mut total = Stats::default();
	for fuel in 0..=done + 2 {
		hook::uninstall();
		let plain = invoke_full(code, function, args, fuel);
		hook::install(code);
		let simulated = invoke_full(code, function, args, fuel);
		let stats = hook::uninstall().unwrap();
		if let Some(m) = hook::take_mismatch() {
			panic!("fuel {fuel} args {args:?}: {m}");
		}
		assert_eq!(plain, simulated, "fuel {fuel}, args {args:?}");
		assert!(plain.idle && plain.live_entries == 0, "fuel {fuel}");
		sum(&mut total, &stats);
	}
	total
}

// ------------------------------------------------------------------ B1-1 listing

fn listing(code: &Code, function: u32) -> StdVec<String> {
	let f = code.function(function).unwrap();
	(0..f.len())
		.map(|pc| {
			let charged = if f.cached_ref(pc).unwrap().charged {
				"C"
			} else {
				" "
			};
			format!("{pc} {charged} {:?}", f.operation(pc).unwrap())
		})
		.collect()
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_listing_golden_calls_and_fib() {
	let program = compile(V2_CALLS);
	let main = main_of(&program);
	assert_eq!(main, 0);
	let built = calls_kernel();
	assert_eq!(
		listing(&program.code, 0),
		listing(&built, 0),
		"main = f0, the calls kernel listing"
	);
	assert_eq!(listing(&program.code, 1), listing(&built, 1), "add = f1");
	let (m, a) = (
		program.code.function(0).unwrap(),
		program.code.function(1).unwrap(),
	);
	assert_eq!((m.slots, m.parameters, a.slots, a.parameters), (6, 1, 3, 2));
	assert_eq!(&m.args[..], &[1, 4]);

	// The charged pcs, from the flag the interpreter consults.
	let charged = |f: &Function| -> StdVec<usize> {
		(0..f.len())
			.filter(|&p| f.cached_ref(p).unwrap().charged)
			.collect()
	};
	assert_eq!(charged(m), vec![0, 4]);
	assert_eq!(charged(a), vec![0]);

	// fib: recursive, no loop, so no candidate header in either function.
	let fib = compile(V2_FIB);
	for id in 0..fib.code.functions() as u32 {
		std::println!("B1LISTING\tfib\tf{id}\t{:?}", listing(&fib.code, id));
		assert!(headers(&fib.code, id).is_empty());
	}
	// fib calls itself (not a leaf): main = f0, fib = f1.
	let f = fib.code.function(1).unwrap();
	assert!((0..f.len()).any(|p| matches!(f.operation(p).unwrap(), Op::Call { function: 1, .. })));
}

// ------------------------------------------------------- B1-2..5: the range header

#[test]
fn b1_range_header_admitted() {
	let code = build(vec![range_sum()]);
	let region = admitted(&code, 0, 4);
	let spec = region.iter.expect("iterator");
	assert_eq!(spec.slot, 4);
	assert!(!region.slots.contains(&4));
	assert_eq!(region.cells(), region.slots.len() + 2);
	assert_eq!(region.slots.as_slice(), &[2, 5]);

	let header = region.nodes[region.header_node as usize];
	assert!(matches!(header.op, NodeOp::IterNext { .. }));
	assert!(header.charged.is_some());
	let shapes: StdVec<(Status, u32, bool)> = region
		.exits
		.iter()
		.map(|e| (e.status, e.pc, e.iter_done))
		.collect();
	assert_eq!(
		shapes,
		vec![
			(Status::Fuel, 4, false),
			(Status::Continue, 7, true),
			(Status::Retry, 5, false)
		]
	);
	// No Retry exit at the header pc.
	assert!(!region
		.exits
		.iter()
		.any(|e| e.status == Status::Retry && e.pc == 4));
	assert!(region.exits.iter().all(|e| e.replay.is_none()));
}

#[test]
fn b1_iternext_none_edge_does_not_write_out() {
	let code = build(vec![range_sum()]);
	let region = admitted(&code, 0, 4);
	let i = local(&region, 5);
	let none = region.exits.iter().find(|e| e.iter_done).unwrap();
	assert_eq!(
		none.definite & (1 << i),
		0,
		"the none edge does not write `i`"
	);
	assert_ne!(
		none.maybe & (1 << i),
		0,
		"written on earlier iterations only"
	);
	let fuel = region
		.exits
		.iter()
		.find(|e| e.status == Status::Fuel)
		.unwrap();
	assert_ne!(fuel.maybe & (1 << i), 0);

	// Hand-built `none == pc + 1`: both states join there.
	// Slots: 0 a, 1 flag, 2 range, 3 iterator, 4 i, 5 k.
	let mut f = Function::new(6, 2).unwrap();
	op(
		&mut f,
		Op::Range {
			out: 2,
			start: 0,
			end: 0,
		},
		0,
	);
	op(&mut f, Op::IterStart { out: 3, value: 2 }, 1);
	op(
		&mut f,
		Op::IterNext {
			out: 4,
			iterator: 3,
			none: 3,
		},
		2,
	);
	op(
		&mut f,
		Op::Branch {
			condition: 1,
			yes: 4,
			no: 6,
		},
		3,
	);
	lit(&mut f, 5, Value::I64(1), 4);
	op(&mut f, Op::Jump { target: 2 }, 5);
	op(&mut f, Op::Return { value: 5 }, 6);
	let code = build(vec![f]);
	let region = admitted(&code, 0, 2);
	let i = local(&region, 4);
	let out = region
		.exits
		.iter()
		.find(|e| e.status == Status::Continue && e.pc == 6)
		.unwrap();
	assert_eq!(out.definite & (1 << i), 0);
	assert_ne!(out.maybe & (1 << i), 0, "joined Maybe at pc + 1");
	assert!(!out.iter_done, "the none edge stays inside the body");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_iternext_not_header_or_second_refused() {
	// Nested for: the outer region meets the inner loop's Range first (pc order).
	let program =
		compile("pub fn main(a) { let s = 0; for i in 0..a { for j in 0..i { s += j; } } s }");
	let code = &program.code;
	let found = analyze(code, 0).unwrap();
	assert_eq!(
		found.len(),
		2,
		"{:?}",
		found
			.iter()
			.map(|(h, r)| (*h, r.as_ref().err().copied()))
			.collect::<StdVec<_>>()
	);
	let f = code.function(0).unwrap();
	let (outer, inner) = (found[0].0, found[1].0);
	let first_inner_range = (outer as usize..f.len())
		.find(|&p| matches!(f.operation(p).unwrap(), Op::Range { .. }))
		.unwrap() as u32;
	assert_eq!(
		found[0].1.as_ref().err(),
		Some(&RegionRefusal::Op {
			pc: first_inner_range
		})
	);
	assert!(found[1].1.is_ok(), "inner {inner} admitted");

	// A while loop whose body holds an IterNext: NotHeader.
	// Slots: 0 a, 1 range, 2 it, 3 x, 4 c, 5 i.
	let mut f = Function::new(6, 1).unwrap();
	op(
		&mut f,
		Op::Range {
			out: 1,
			start: 0,
			end: 0,
		},
		0,
	);
	op(&mut f, Op::IterStart { out: 2, value: 1 }, 1);
	lit(&mut f, 3, Value::I64(0), 2);
	binary(&mut f, 4, Binary::Lt, 3, 0, 3); // 3: header
	op(
		&mut f,
		Op::Branch {
			condition: 4,
			yes: 5,
			no: 8,
		},
		4,
	);
	op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 2,
			none: 6,
		},
		5,
	);
	binary(&mut f, 3, Binary::Add, 3, 0, 6);
	op(&mut f, Op::Jump { target: 3 }, 7);
	op(&mut f, Op::Return { value: 3 }, 8);
	let code = build(vec![f]);
	assert_eq!(
		refused(&code, 0, 3),
		RegionRefusal::Iter {
			pc: 5,
			reason: IterReason::NotHeader
		}
	);

	// Two IterNexts: the header and a second one in its body.
	// Slots: 0 a, 1 range, 2 it1, 3 it2, 4 i, 5 j.
	let mut f = Function::new(6, 1).unwrap();
	op(
		&mut f,
		Op::Range {
			out: 1,
			start: 0,
			end: 0,
		},
		0,
	);
	op(&mut f, Op::IterStart { out: 2, value: 1 }, 1);
	op(&mut f, Op::IterStart { out: 3, value: 1 }, 2);
	op(
		&mut f,
		Op::IterNext {
			out: 4,
			iterator: 2,
			none: 6,
		},
		3,
	);
	op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 3,
			none: 5,
		},
		4,
	);
	op(&mut f, Op::Jump { target: 3 }, 5);
	op(&mut f, Op::Return { value: 0 }, 6);
	let code = build(vec![f]);
	assert_eq!(
		refused(&code, 0, 3),
		RegionRefusal::Iter {
			pc: 4,
			reason: IterReason::Second
		}
	);
}

#[test]
fn b1_iter_slot_touched_refused() {
	// Each variant names the iterator slot 4 at body pc 5.
	let variants: StdVec<(&str, Box<dyn Fn(&mut Function)>)> = vec![
		(
			"copy_from_iterator",
			Box::new(|f: &mut Function| {
				op(f, Op::Copy { out: 6, value: 4 }, 5);
			}),
		),
		(
			"copy_into_iterator",
			Box::new(|f: &mut Function| {
				op(f, Op::Copy { out: 4, value: 5 }, 5);
			}),
		),
		(
			"release_iterator",
			Box::new(|f: &mut Function| {
				op(f, Op::Release { value: 4 }, 5);
			}),
		),
		(
			"binary_reads_iterator",
			Box::new(|f: &mut Function| {
				binary(f, 6, Binary::Add, 4, 5, 5);
			}),
		),
		(
			"branch_on_iterator",
			Box::new(|f: &mut Function| {
				op(
					f,
					Op::Branch {
						condition: 4,
						yes: 6,
						no: 6,
					},
					5,
				);
			}),
		),
		(
			"used_call_argument",
			Box::new(|f: &mut Function| {
				call(f, 6, 1, &[4, 5], 5);
			}),
		),
		(
			"unused_call_argument",
			Box::new(|f: &mut Function| {
				call(f, 6, 1, &[5, 4], 5);
			}),
		),
		(
			"call_out",
			Box::new(|f: &mut Function| {
				call(f, 4, 1, &[5, 5], 5);
			}),
		),
	];
	for (name, body) in variants {
		let f = range_loop(8, |f| body(f));
		let code = build(vec![f, first_plus_one()]);
		assert_eq!(
			refused(&code, 0, 4),
			RegionRefusal::IterSlot { pc: 5 },
			"{name}"
		);
	}

	// `out == iterator` on the header itself.
	let mut f = Function::new(6, 2).unwrap();
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		0,
	);
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 1);
	op(
		&mut f,
		Op::IterNext {
			out: 4,
			iterator: 4,
			none: 4,
		},
		2,
	);
	op(&mut f, Op::Jump { target: 2 }, 3);
	op(&mut f, Op::Return { value: 0 }, 4);
	let code = build(vec![f]);
	assert_eq!(refused(&code, 0, 2), RegionRefusal::IterSlot { pc: 2 });
}

// ------------------------------------------------------------ B1-6..13: inlining

/// The site's expansion nodes, in node order (Enter excluded).
fn expansion(region: &Region, site: usize) -> &[super::Node] {
	let s = region.sites[site];
	&region.nodes[s.first as usize..s.end as usize]
}

fn assert_calls_kernel_region(region: &Region, name: &str) {
	assert_eq!(region.header, 4, "{name}");
	assert_eq!(region.slots.as_slice(), &[1, 4, 5], "{name}");
	assert_eq!(region.iter.map(|s| s.slot), Some(3), "{name}");
	assert_eq!(region.cells(), 5, "{name}");
	assert_eq!(region.callee_slots, 3, "{name}");
	assert_eq!(
		region.entry.as_slice(),
		&[
			super::EntryClass::Guard(Scalar::I64),
			super::EntryClass::Scalar,
			super::EntryClass::Scalar
		],
		"{name}"
	);

	// The calls kernel's exit table.
	let maybe_header = 0b001 | 0b010 | 0b100;
	let e = |status, pc, definite, maybe, replay, iter_done| super::Exit {
		status,
		pc,
		definite,
		maybe,
		bools: 0,
		dynamic: 0,
		restore: false,
		units: 0b100,
		replay,
		iter_done,
	};
	assert_eq!(
		region.exits.as_slice(),
		&[
			e(Status::Fuel, 4, 0, maybe_header, None, false),
			e(Status::Continue, 9, 0, maybe_header, None, true),
			e(
				Status::Replay,
				5,
				0b010,
				0b101,
				Some(ReplayReason::CalleeFuel),
				false
			),
			e(
				Status::Replay,
				5,
				0b010,
				0b101,
				Some(ReplayReason::CalleeFault),
				false
			),
		],
		"{name}"
	);

	assert_eq!(region.sites.len(), 1, "{name}");
	let site = region.sites[0];
	assert_eq!(
		(site.pc, site.callee, site.slots, site.parameters, site.ret),
		(5, 1, 3, 2, Scalar::I64)
	);
	assert_eq!((site.fuel_exit, site.fault_exit), (2, Some(3)));
	let enter = region.nodes[site.enter as usize];
	assert_eq!(enter.charged, None, "{name}: the Call is uncharged");
	assert_eq!(enter.op, NodeOp::Enter { site: 0, slots: 3 });
	assert_eq!(enter.next, Some(Target::Node(site.first)));

	let ops: StdVec<NodeOp> = expansion(region, 0).iter().map(|n| n.op).collect();
	let v = VIRTUAL_BASE;
	assert_eq!(
		ops,
		vec![
			NodeOp::Arg { param: 0, src: 0 },
			NodeOp::Arg { param: 1, src: 1 },
			NodeOp::Arith {
				out: v + 2,
				op: Binary::Add,
				lhs: v,
				rhs: v + 1,
				retry: Some(3),
				divisor: None,
			},
			NodeOp::Ret {
				out: 2,
				src: v + 2,
				ty: Scalar::I64,
			},
		],
		"{name}"
	);
	let charged: StdVec<Option<u32>> = expansion(region, 0).iter().map(|n| n.charged).collect();
	assert_eq!(charged, vec![None, None, Some(2), None], "{name}");
}

#[test]
fn b1_inline_calls_kernel_admitted_handbuilt() {
	let code = calls_kernel();
	let region = admitted(&code, 0, 4);
	assert_calls_kernel_region(&region, "hand-built");
	check_expansion(&region).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_inline_calls_kernel_admitted() {
	// v2: exactly the hand-built calls kernel's region.
	let program = compile(V2_CALLS);
	let main = main_of(&program);
	let region = admitted(&program.code, main, 4);
	assert_calls_kernel_region(&region, "v2");
	check_expansion(&region).unwrap();

	// stage: literal bounds (one more slot and pc), the same shape.
	let program = compile(STAGE_CALLS);
	let main = main_of(&program);
	let h = headers(&program.code, main)[0];
	let region = admitted(&program.code, main, h);
	let shape: StdVec<(Status, Option<ReplayReason>, bool)> = region
		.exits
		.iter()
		.map(|e| (e.status, e.replay, e.iter_done))
		.collect();
	assert_eq!(
		shape,
		vec![
			(Status::Fuel, None, false),
			(Status::Continue, None, true),
			(Status::Replay, Some(ReplayReason::CalleeFuel), false),
			(Status::Replay, Some(ReplayReason::CalleeFault), false),
		]
	);
	assert_eq!(
		(region.callee_slots, region.sites.len(), region.cells()),
		(3, 1, region.slots.len() + 2)
	);
	assert_eq!(region.nodes[region.sites[0].enter as usize].charged, None);
	check_expansion(&region).unwrap();
}

/// A one-function loop body calling `callee` (f1) once per iteration as `t = callee(args)`.
fn calling(callee: Function, args: &[u32]) -> Code {
	let args = args.to_vec();
	let f = range_loop(8, move |f| {
		call(f, 6, 1, &args, 5);
		op(
			f,
			Op::AssignBinary {
				out: 2,
				op: Binary::Add,
				lhs: 2,
				rhs: 6,
			},
			6,
		);
	});
	build(vec![f, callee])
}

fn inline_refusal(code: &Code) -> InlineReason {
	match refused(code, 0, 4) {
		RegionRefusal::Inline { pc: 5, reason } => reason,
		other => panic!("expected an Inline refusal at pc 5, got {other:?}"),
	}
}

#[test]
fn b1_inline_refusals() {
	// Nested call: the callee calls another function.
	let mut nested = Function::new(3, 2).unwrap();
	call(&mut nested, 2, 2, &[0, 1], 100);
	op(&mut nested, Op::Return { value: 2 }, 101);
	let f = range_loop(8, |f| {
		call(f, 6, 1, &[5, 5], 5);
	});
	let code = build(vec![f, nested, add_fn()]);
	assert_eq!(inline_refusal(&code), InlineReason::NotLeaf);

	// CallValue in the caller: an Op refusal, not an inline one.
	let f = range_loop(8, |f| {
		let (args_start, args_len) = f.arguments(&[5]).unwrap();
		op(
			f,
			Op::CallValue {
				out: 6,
				value: 7,
				args_start,
				args_len,
			},
			5,
		);
	});
	let code = build(vec![f]);
	assert_eq!(refused(&code, 0, 4), RegionRefusal::Op { pc: 5 });

	// 17 instructions.
	let mut big = Function::new(3, 2).unwrap();
	for k in 0..16 {
		lit(&mut big, 2, Value::I64(k), 100 + k as u32);
	}
	op(&mut big, Op::Return { value: 2 }, 120);
	assert_eq!(big.len(), 17);
	assert_eq!(inline_refusal(&calling(big, &[5, 5])), InlineReason::Size);

	// 17 slots.
	let mut wide = Function::new(17, 2).unwrap();
	lit(&mut wide, 16, Value::I64(1), 100);
	op(&mut wide, Op::Return { value: 16 }, 101);
	assert_eq!(inline_refusal(&calling(wide, &[5, 5])), InlineReason::Slots);

	// A String op in the callee.
	let mut text = Function::new(3, 2).unwrap();
	let id = text.string("x").unwrap();
	op(&mut text, Op::String { out: 2, text: id }, 100);
	op(&mut text, Op::Return { value: 2 }, 101);
	assert_eq!(
		inline_refusal(&calling(text, &[5, 5])),
		InlineReason::Op { callee_pc: 0 }
	);

	// A callee loop: refused at its charged back-edge target by the charge shape (check 6),
	// before the forward-DAG check (7). Slots: 0 a, 1 b, 2 acc, 3 c.
	let mut looping = Function::new(4, 2).unwrap();
	lit(&mut looping, 2, Value::I64(0), 100); // 0
	binary(&mut looping, 3, Binary::Lt, 2, 0, 101); // 1: back-edge target
	op(
		&mut looping,
		Op::Branch {
			condition: 3,
			yes: 3,
			no: 5,
		},
		102,
	); // 2
	binary(&mut looping, 2, Binary::Add, 2, 1, 103); // 3
	op(&mut looping, Op::Jump { target: 1 }, 104); // 4
	op(&mut looping, Op::Return { value: 2 }, 105); // 5
	assert_eq!(
		inline_refusal(&calling(looping, &[5, 5])),
		InlineReason::Charged { callee_pc: 1 }
	);

	// I64 and Bool returns. Slots: 0 a, 1 b, 2 c, 3 r.
	let mut mixed = Function::new(4, 2).unwrap();
	binary(&mut mixed, 2, Binary::Lt, 0, 1, 100); // 0
	op(
		&mut mixed,
		Op::Branch {
			condition: 2,
			yes: 2,
			no: 4,
		},
		101,
	); // 1
	lit(&mut mixed, 3, Value::I64(1), 102); // 2
	op(&mut mixed, Op::Return { value: 3 }, 103); // 3
	op(&mut mixed, Op::Return { value: 2 }, 104); // 4: Bool
	assert_eq!(
		inline_refusal(&calling(mixed, &[5, 5])),
		InlineReason::ReturnType
	);

	// `fn id(a) { a }`: the Return reads a parameter.
	let mut id = Function::new(1, 1).unwrap();
	op(&mut id, Op::Return { value: 0 }, 100);
	assert_eq!(
		inline_refusal(&calling(id, &[5])),
		InlineReason::ReturnSource { callee_pc: 0 }
	);

	// A Copy of a parameter.
	let mut copied = Function::new(3, 2).unwrap();
	op(&mut copied, Op::Copy { out: 2, value: 0 }, 100);
	op(&mut copied, Op::Return { value: 2 }, 101);
	assert_eq!(
		inline_refusal(&calling(copied, &[5, 5])),
		InlineReason::CopySource { callee_pc: 0 }
	);

	// A parameter demanded both I64 and Bool. Slots: 0 a, 1 b, 2 r.
	let mut conflict = Function::new(3, 2).unwrap();
	op(
		&mut conflict,
		Op::Branch {
			condition: 0,
			yes: 1,
			no: 1,
		},
		100,
	); // 0
	binary(&mut conflict, 2, Binary::Add, 0, 1, 101); // 1
	op(&mut conflict, Op::Return { value: 2 }, 102); // 2
	assert_eq!(
		inline_refusal(&calling(conflict, &[5, 5])),
		InlineReason::ParamConflict
	);

	// 17 sites in one region.
	let f = range_loop(8, |f| {
		for k in 0..=MAX_INLINE_SITES as u32 {
			call(f, 6, 1, &[5, 5], 5 + k);
		}
	});
	let code = build(vec![f, add_fn()]);
	assert_eq!(
		refused(&code, 0, 4),
		RegionRefusal::Inline {
			pc: 5 + MAX_INLINE_SITES as u32,
			reason: InlineReason::Sites
		}
	);
	// 16 sites are admitted.
	let f = range_loop(8, |f| {
		for k in 0..MAX_INLINE_SITES as u32 {
			call(f, 6, 1, &[5, 5], 5 + k);
		}
	});
	let code = build(vec![f, add_fn()]);
	assert_eq!(admitted(&code, 0, 4).sites.len(), MAX_INLINE_SITES);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_inline_refusals_from_source() {
	let program = compile(
		"fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } } pub fn main(a) { let s = 0; for i in 0..a { s = fib(i); } s }",
	);
	let main = main_of(&program);
	let h = headers(&program.code, main)[0];
	assert_eq!(
		refused(&program.code, main, h),
		RegionRefusal::Inline {
			pc: h + 1,
			reason: InlineReason::NotLeaf
		}
	);
}

#[test]
fn b1_charged_call_refused() {
	// A Call at caller pc 0 as the header (pc 0 is always charged).
	let mut f = Function::new(3, 1).unwrap();
	call(&mut f, 1, 1, &[0, 0], 0); // 0: header
	op(&mut f, Op::Jump { target: 0 }, 1);
	let code = build(vec![f, add_fn()]);
	assert_eq!(
		refused(&code, 0, 0),
		RegionRefusal::Inline {
			pc: 0,
			reason: InlineReason::ChargedCall
		}
	);

	// At a non-zero header.
	let mut f = Function::new(3, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	call(&mut f, 1, 1, &[0, 0], 1); // 1: header
	op(&mut f, Op::Jump { target: 1 }, 2);
	let code = build(vec![f, add_fn()]);
	assert_eq!(
		refused(&code, 0, 1),
		RegionRefusal::Inline {
			pc: 1,
			reason: InlineReason::ChargedCall
		}
	);

	// At a gap-charged pc: 256 uncharged literals after the header, then the Call.
	let mut f = Function::new(3, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	lit(&mut f, 2, Value::I64(0), 1); // 1: header (charged)
	for k in 0..256 {
		lit(&mut f, 2, Value::I64(k), 2 + k as u32);
	}
	let at = call_at(&mut f);
	op(&mut f, Op::Jump { target: 1 }, 400);
	let code = build(vec![f, add_fn()]);
	assert!(
		code.function(0)
			.unwrap()
			.cached_ref(at as usize)
			.unwrap()
			.charged,
		"gap charge"
	);
	assert_eq!(
		refused(&code, 0, 1),
		RegionRefusal::Inline {
			pc: at,
			reason: InlineReason::ChargedCall
		}
	);

	fn call_at(f: &mut Function) -> u32 {
		let (args_start, args_len) = f.arguments(&[0, 0]).unwrap();
		op(
			f,
			Op::Call {
				out: 1,
				function: 1,
				args_start,
				args_len,
			},
			399,
		)
	}
}

#[test]
fn b1_callee_charge_shape_inspects_every_flag() {
	// (a) add: Ok.
	let code = build(vec![add_fn()]);
	assert_eq!(
		callee_charge_shape(code.function(0).unwrap()).unwrap(),
		Ok(())
	);

	// (b) 300 straight-line instructions: the gap charge at 257.
	let mut f = Function::new(1, 0).unwrap();
	for k in 0..299 {
		lit(&mut f, 0, Value::I64(k), k as u32);
	}
	op(&mut f, Op::Return { value: 0 }, 299);
	let code = build(vec![f]);
	assert_eq!(
		callee_charge_shape(code.function(0).unwrap()).unwrap(),
		Err(InlineReason::Charged { callee_pc: 257 })
	);

	// (c) A loop with a back edge to pc 3.
	let mut f = Function::new(2, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 1, Value::I64(1), 1); // 1
	lit(&mut f, 1, Value::I64(2), 2); // 2
	binary(&mut f, 1, Binary::Add, 1, 1, 3); // 3: back-edge target
	op(
		&mut f,
		Op::Branch {
			condition: 0,
			yes: 3,
			no: 5,
		},
		4,
	); // 4
	op(&mut f, Op::Return { value: 1 }, 5);
	let code = build(vec![f]);
	assert_eq!(
		callee_charge_shape(code.function(0).unwrap()).unwrap(),
		Err(InlineReason::Charged { callee_pc: 3 })
	);

	// (d) A loop back to pc 0: the flags are exactly {0}, yet pc 0 runs more than once.
	let mut f = Function::new(2, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	op(
		&mut f,
		Op::Branch {
			condition: 0,
			yes: 0,
			no: 2,
		},
		1,
	); // 1
	op(&mut f, Op::Return { value: 1 }, 2);
	let code = build(vec![f]);
	let callee = code.function(0).unwrap();
	let charged: StdVec<bool> = (0..callee.len())
		.map(|p| callee.cached_ref(p).unwrap().charged)
		.collect();
	assert_eq!(charged, vec![true, false, false], "flags exactly {{0}}");
	assert_eq!(
		callee_charge_shape(callee).unwrap(),
		Err(InlineReason::Pc0Reentry { callee_pc: 1 })
	);

	// Over admitted sites, every callee instruction's flag was inspected, and the cached
	// and encoded flags agree at every inspected pc.
	let corpus = [calls_kernel(), calling(first_plus_one(), &[5, 5])];
	for code in &corpus {
		let before = charge_flags_inspected_for_test();
		let region = admitted(code, 0, 4);
		let inspected = charge_flags_inspected_for_test() - before;
		let expected: usize = region
			.sites
			.iter()
			.map(|s| code.function(s.callee).unwrap().len())
			.sum();
		assert_eq!(inspected, expected);
		for s in region.sites.iter() {
			let callee = code.function(s.callee).unwrap();
			for p in 0..callee.len() {
				callee
					.audit_cached(p, *callee.cached_ref(p).unwrap())
					.unwrap();
			}
		}
	}
}

#[test]
fn b1_expansion_has_exactly_one_charged_node_per_site() {
	let corpus = [
		calls_kernel(),
		calling(first_plus_one(), &[5, 5]),
		calling(two_arm(), &[5, 2]),
		calling(early_return(), &[5]),
		calling(three_returns(), &[5]),
	];
	for code in &corpus {
		let region = admitted(code, 0, 4);
		for (k, site) in region.sites.iter().enumerate() {
			let nodes = expansion(&region, k);
			let charged: StdVec<usize> = (0..nodes.len())
				.filter(|&i| nodes[i].charged.is_some())
				.collect();
			assert_eq!(charged.len(), 1, "site {k}");
			let node = nodes[charged[0]];
			assert_eq!(node.charged, Some(site.fuel_exit));
			// It is the callee pc 0 node: the first node after the Args.
			assert_eq!(charged[0], site.parameters as usize);
			assert_eq!(
				region
					.nodes
					.iter()
					.filter(|n| n.charged == Some(site.fuel_exit))
					.count(),
				1,
				"no other node charges the site's CalleeFuel exit"
			);
		}
		check_expansion(&region).unwrap();
	}

	// A hand-corrupted region: a second charged callee node is an analysis error.
	let mut region = admitted(&calls_kernel(), 0, 4);
	let site = region.sites[0];
	let ret = site.end as usize - 1;
	region.nodes[ret].charged = Some(site.fuel_exit);
	assert!(check_expansion(&region).is_err());
}

#[test]
fn b1_call_args_demands_and_touch() {
	use super::EntryClass;
	// `k(total, i)`: total (slot 2) demanded I64 and live-in; b unused but touched.
	// Slot 7 is passed as the unused argument and never written: Scalar entry class.
	let f = range_loop(8, |f| {
		call(f, 6, 1, &[2, 7], 5);
		op(f, Op::Copy { out: 2, value: 6 }, 6);
	});
	let code = build(vec![f, first_plus_one()]);
	let region = admitted(&code, 0, 4);
	assert_eq!(
		region.entry[local(&region, 2)],
		EntryClass::Guard(Scalar::I64)
	);
	assert_eq!(region.entry[local(&region, 7)], EntryClass::Scalar);
	assert!(region.slots.contains(&7), "an unused argument is touched");

	// Duplicated arguments with conflicting demands: `p(i, i)` with p(a: I64, b: Bool).
	let mut p = Function::new(3, 2).unwrap();
	op(
		&mut p,
		Op::Branch {
			condition: 1,
			yes: 1,
			no: 1,
		},
		100,
	);
	lit(&mut p, 2, Value::I64(1), 101);
	binary(&mut p, 2, Binary::Add, 0, 2, 102);
	op(&mut p, Op::Return { value: 2 }, 103);
	let code = calling(p, &[5, 5]);
	assert_eq!(refused(&code, 0, 4), RegionRefusal::Type { pc: 5 });
}

/// The interpreter footprint, encoded independently of the analysis.
fn table_footprint(f: &Function, op: Op) -> StdVec<u32> {
	match op {
		Op::Literal { out, .. } => vec![out],
		Op::Binary { out, lhs, rhs, .. } | Op::AssignBinary { out, lhs, rhs, .. } => {
			vec![lhs, rhs, out]
		}
		Op::Branch { condition, .. } => vec![condition],
		Op::Jump { .. } => vec![],
		Op::Copy { out, value } => vec![value, out],
		Op::Release { value } => vec![value],
		Op::IterNext { out, iterator, .. } => vec![iterator, out],
		Op::Call {
			out,
			args_start,
			args_len,
			..
		} => {
			let mut v = f.args[args_start as usize..(args_start + args_len) as usize].to_vec();
			v.push(out);
			v
		}
		other => panic!("not an admitted op: {other:?}"),
	}
}

fn assert_semantic_touched(code: &Code, function: u32, region: &Region, name: &str) {
	let f = code.function(function).unwrap();
	let mut pcs: StdVec<u32> = region.nodes.iter().map(|n| n.pc).collect();
	pcs.sort_unstable();
	pcs.dedup();
	let mut union: StdVec<u32> = pcs
		.iter()
		.flat_map(|&pc| table_footprint(f, f.operation(pc as usize).unwrap()))
		.collect();
	union.sort_unstable();
	union.dedup();
	if let Some(spec) = region.iter {
		union.retain(|&s| s != spec.slot);
		assert!(!region.slots.contains(&spec.slot), "{name}");
	}
	assert_eq!(region.slots.as_slice(), union.as_slice(), "{name}");
}

#[test]
fn b1_semantic_touched_set_covers_interpreter_footprint() {
	let cases: StdVec<(&str, Code)> = vec![
		("calls_kernel", calls_kernel()),
		("range_sum", build(vec![range_sum()])),
		("unused_argument", calling(first_plus_one(), &[5, 7])),
		("duplicated_argument", calling(add_fn(), &[5, 5])),
		(
			"copy_source_never_read",
			build(vec![range_loop(8, |f| {
				lit(f, 7, Value::I64(3), 5);
				op(f, Op::Copy { out: 6, value: 7 }, 6);
			})]),
		),
		(
			"release_never_read",
			build(vec![range_loop(8, |f| {
				op(f, Op::Release { value: 7 }, 5);
			})]),
		),
		(
			"call_out_is_arg0",
			build(vec![
				range_loop(8, |f| {
					call(f, 2, 1, &[2, 5], 5);
				}),
				add_fn(),
			]),
		),
	];
	for (name, code) in &cases {
		let region = admitted(code, 0, 4);
		assert_semantic_touched(code, 0, &region, name);
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_semantic_touched_set_over_source_corpus() {
	for text in [
		V2_CALLS,
		STAGE_CALLS,
		"pub fn main(a, b) { let s = 0; for i in a..b { s += i; } s }",
	] {
		let program = compile(text);
		let main = main_of(&program);
		for (_, region) in analyze(&program.code, main).unwrap() {
			if let Ok(region) = region {
				assert_semantic_touched(&program.code, main, &region, text);
			}
		}
	}
}

/// `fn f(a, b) { if a < b { a * b } else { a - b } }`. Slots: 0 a, 1 b, 2 c, 3 r.
pub(crate) fn two_arm() -> Function {
	let mut f = Function::new(4, 2).unwrap();
	binary(&mut f, 2, Binary::Lt, 0, 1, 100); // 0
	op(
		&mut f,
		Op::Branch {
			condition: 2,
			yes: 2,
			no: 4,
		},
		101,
	); // 1
	binary(&mut f, 3, Binary::Mul, 0, 1, 102); // 2
	op(&mut f, Op::Return { value: 3 }, 103); // 3
	binary(&mut f, 3, Binary::Sub, 0, 1, 104); // 4
	op(&mut f, Op::Return { value: 3 }, 105); // 5
	f
}

/// `fn g(a) { if a > 3 { return a * a; } a + 1 }`. Slots: 0 a, 1 k, 2 c, 3 r.
pub(crate) fn early_return() -> Function {
	let mut f = Function::new(4, 1).unwrap();
	lit(&mut f, 1, Value::I64(3), 100); // 0
	binary(&mut f, 2, Binary::Gt, 0, 1, 101); // 1
	op(
		&mut f,
		Op::Branch {
			condition: 2,
			yes: 3,
			no: 5,
		},
		102,
	); // 2
	binary(&mut f, 3, Binary::Mul, 0, 0, 103); // 3
	op(&mut f, Op::Return { value: 3 }, 104); // 4
	lit(&mut f, 1, Value::I64(1), 105); // 5
	binary(&mut f, 3, Binary::Add, 0, 1, 106); // 6
	op(&mut f, Op::Return { value: 3 }, 107); // 7
	f
}

/// Three Returns: `a < 0 -> a * 2`, `a == 0 -> 7`, else `a - 1`. Slots: 0 a, 1 k, 2 c, 3 r.
pub(crate) fn three_returns() -> Function {
	let mut f = Function::new(4, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 100); // 0
	binary(&mut f, 2, Binary::Lt, 0, 1, 101); // 1
	op(
		&mut f,
		Op::Branch {
			condition: 2,
			yes: 3,
			no: 6,
		},
		102,
	); // 2
	lit(&mut f, 1, Value::I64(2), 103); // 3
	binary(&mut f, 3, Binary::Mul, 0, 1, 104); // 4
	op(&mut f, Op::Return { value: 3 }, 105); // 5
	binary(&mut f, 2, Binary::Eq, 0, 1, 106); // 6
	op(
		&mut f,
		Op::Branch {
			condition: 2,
			yes: 8,
			no: 10,
		},
		107,
	); // 7
	lit(&mut f, 3, Value::I64(7), 108); // 8
	op(&mut f, Op::Return { value: 3 }, 109); // 9
	lit(&mut f, 1, Value::I64(1), 110); // 10
	binary(&mut f, 3, Binary::Sub, 0, 1, 111); // 11
	op(&mut f, Op::Return { value: 3 }, 112); // 12
	f
}

/// What a node reads and writes, by local index.
fn node_rw(op: NodeOp) -> (StdVec<u32>, Option<u32>) {
	match op {
		NodeOp::Literal { out, .. } => (vec![], Some(out)),
		NodeOp::Copy { out, src, .. } => (vec![src], Some(out)),
		NodeOp::Arith { out, lhs, rhs, .. } | NodeOp::Compare { out, lhs, rhs, .. } => {
			(vec![lhs, rhs], Some(out))
		}
		NodeOp::Branch { condition, .. } => (vec![condition], None),
		NodeOp::Jump { .. } => (vec![], None),
		NodeOp::Arg { param, src } => (vec![src], Some(VIRTUAL_BASE + param)),
		NodeOp::Ret { out, src, .. } => (vec![src], Some(out)),
		other => panic!("not an expansion node: {other:?}"),
	}
}

#[test]
fn b1_virtual_confinement_and_single_ret() {
	let corpus = [
		("calls_kernel", calls_kernel(), 1usize),
		("two_arm", calling(two_arm(), &[5, 2]), 2),
		("early_return", calling(early_return(), &[5]), 2),
		("three_returns", calling(three_returns(), &[5]), 3),
	];
	for (name, code, returns) in &corpus {
		let region = admitted(code, 0, 4);
		let n = region.slots.len() as u32;
		check_expansion(&region).unwrap();
		for (k, site) in region.sites.iter().enumerate() {
			let range = site.first..site.end;
			let mut rets = 0;
			for node in expansion(&region, k) {
				let (reads, write) = node_rw(node.op);
				match node.op {
					NodeOp::Arg { .. } => {
						assert!(
							reads.iter().all(|&r| r < n),
							"{name}: Arg reads a caller local"
						);
						assert!(write.unwrap() >= VIRTUAL_BASE);
					}
					NodeOp::Ret { .. } => {
						rets += 1;
						assert!(reads.iter().all(|&r| r >= VIRTUAL_BASE), "{name}");
						assert!(write.unwrap() < n, "{name}: Ret writes one caller local");
						assert!(
							!matches!(node.next, Some(Target::Node(t)) if range.contains(&t)),
							"{name}: Ret leaves the expansion"
						);
					}
					_ => {
						assert!(
							reads.iter().all(|&r| r >= VIRTUAL_BASE),
							"{name}: callee reads virtual only"
						);
						assert!(
							write.is_none_or(|w| w >= VIRTUAL_BASE),
							"{name}: callee writes virtual only"
						);
					}
				}
			}
			assert_eq!(rets, *returns, "{name}: one Ret per reachable Return");

			// Every path from the first expansion node reaches exactly one Ret, or a site
			// Replay exit before any Ret.
			let site_exits = [Some(site.fuel_exit), site.fault_exit];
			let mut stack = vec![(site.first, 0usize)];
			let mut paths = 0;
			while let Some((i, seen)) = stack.pop() {
				let node = region.nodes[i as usize];
				let seen = seen + usize::from(matches!(node.op, NodeOp::Ret { .. }));
				let mut targets: StdVec<Target> = node.next.into_iter().collect();
				if let NodeOp::Branch { yes, no, .. } = node.op {
					targets.extend([yes, no]);
				}
				if let NodeOp::Jump { target } = node.op {
					targets.push(target);
				}
				for e in [node.charged, retry_of(node.op)].into_iter().flatten() {
					assert!(site_exits.contains(&Some(e)), "{name}: only site exits");
					assert_eq!(seen, 0, "{name}: a Replay exit before any Ret");
					paths += 1;
				}
				for t in targets {
					match t {
						Target::Node(j) if range.contains(&j) => {
							assert_eq!(seen, 0, "{name}: nothing after a Ret inside the expansion");
							stack.push((j, seen));
						}
						_ => {
							assert_eq!(seen, 1, "{name}: exactly one Ret before leaving");
							paths += 1;
						}
					}
				}
			}
			assert!(paths > 0);
		}
	}

	fn retry_of(op: NodeOp) -> Option<u32> {
		match op {
			NodeOp::Arith { retry, .. } => retry,
			_ => None,
		}
	}
}

// -------------------------------------------------------- B1-21, 22: entry checks

fn never(_: &Region, _: &mut [i64], _: &mut ExitRecord) -> u32 {
	panic!("not entered")
}

const ROOM: Room = Room {
	frames: 1024,
	slots: 65_536,
};

/// The calls kernel's main frame at its header: x, total, range, iterator, i, temporary.
fn calls_window(s: &mut Store, total: i64, start: i64, end: i64) -> StdVec<Value> {
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	vec![
		Value::I64(end),
		Value::I64(total),
		range,
		it,
		Value::Unit,
		Value::Unit,
	]
}

#[test]
fn b1_entry_refusals_change_nothing() {
	let region = admitted(&calls_kernel(), 0, 4);
	let mut s = store();
	let original = calls_window(&mut s, 0, 0, 10);
	let list = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let vector = s.iterator(list).unwrap();
	let other_range = s.range(0, 3).unwrap();

	let mut cases: StdVec<(&str, Value)> = vec![
		("i64", Value::I64(3)),
		("unit", Value::Unit),
		("range", other_range),
		("vector_iterator", vector),
	];
	// A stale handle, a taken payload and readers are built on copies below.
	for (name, value) in cases.drain(..) {
		let mut w = original.clone();
		w[3] = value;
		let before = (w.clone(), s.state_for_test());
		assert_eq!(
			commit::enter(&region, &mut w, &mut s, false, 50, ROOM, never).unwrap(),
			Err(Refused::Iter { slot: 3 }),
			"{name}"
		);
		assert_eq!((w, s.state_for_test()), before, "{name}");
	}

	for name in ["stale", "taken", "readers"] {
		let mut s = store();
		let mut w = calls_window(&mut s, 0, 0, 10);
		match name {
			"stale" => {
				let Value::Heap(h) = w[3] else { unreachable!() };
				s.set_generation_for_test(w[3], h.generation + 1);
			}
			"taken" => s.invalidate(w[3]).unwrap(),
			_ => s.set_readers_for_test(w[3], 1),
		}
		let before = (w.clone(), s.state_for_test());
		assert_eq!(
			commit::enter(&region, &mut w, &mut s, false, 50, ROOM, never).unwrap(),
			Err(Refused::Iter { slot: 3 }),
			"{name}"
		);
		assert_eq!((w, s.state_for_test()), before, "{name}");
	}

	// Queued work and a closed store: Store.
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	let queued = s.string("queued").unwrap();
	s.push_work_for_test(queued);
	let before = (w.clone(), s.state_for_test());
	assert_eq!(
		commit::enter(&region, &mut w, &mut s, false, 50, ROOM, never).unwrap(),
		Err(Refused::Store)
	);
	assert_eq!((w, s.state_for_test()), before);

	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	s.close().unwrap();
	let before = (w.clone(), s.state_for_test());
	assert_eq!(
		commit::enter(&region, &mut w, &mut s, false, 50, ROOM, never).unwrap(),
		Err(Refused::Store)
	);
	assert_eq!((w, s.state_for_test()), before);
}

#[test]
fn b1_room_refusal_changes_nothing() {
	let region = admitted(&calls_kernel(), 0, 4);
	assert_eq!(region.callee_slots, 3);
	for room in [
		Room {
			frames: 0,
			slots: 65_536,
		},
		Room {
			frames: 1024,
			slots: 2,
		},
		Room {
			frames: 0,
			slots: 0,
		},
	] {
		let mut s = store();
		let mut w = calls_window(&mut s, 0, 0, 10);
		let before = (w.clone(), s.state_for_test());
		assert_eq!(
			commit::enter(&region, &mut w, &mut s, false, 50, room, never).unwrap(),
			Err(Refused::Room),
			"{room:?}"
		);
		assert_eq!((w, s.state_for_test()), before);
	}
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	let exited = commit::enter(
		&region,
		&mut w,
		&mut s,
		false,
		50,
		Room {
			frames: 1,
			slots: 3,
		},
		simulate,
	)
	.unwrap()
	.expect("enters with exactly the room");
	assert_eq!(exited.status, Status::Continue);
	assert_eq!(w[1], Value::I64(45));

	// A region without a call never consults the room.
	let range = admitted(&build(vec![range_sum()]), 0, 4);
	assert_eq!(range.callee_slots, 0);
	let mut s = store();
	let r = s.range(0, 4).unwrap();
	let it = s.iterator(r).unwrap();
	let mut w = vec![
		Value::I64(0),
		Value::I64(4),
		Value::I64(0),
		r,
		it,
		Value::Unit,
	];
	assert!(commit::enter(
		&range,
		&mut w,
		&mut s,
		false,
		50,
		Room {
			frames: 0,
			slots: 0
		},
		simulate
	)
	.unwrap()
	.is_ok());
}

/// The room the executor reports includes the caller frame geometry: a window
/// that is not the top frame's whole slot range gets no room at all.
#[test]
fn b1_room_from_executor_geometry() {
	assert_eq!(
		region_room(1, 6, 6, ExecutorLimits::default(), (1024, 65_536, 1024)),
		Room {
			frames: 1023,
			slots: 65_530
		}
	);
	assert_eq!(
		region_room(1024, 6, 6, ExecutorLimits::default(), (1024, 65_536, 1024)),
		Room {
			frames: 0,
			slots: 65_530
		}
	);
	assert_eq!(
		region_room(
			3,
			65_536,
			65_536,
			ExecutorLimits::default(),
			(1024, 65_536, 1024)
		),
		Room {
			frames: 1021,
			slots: 0
		}
	);
	// The window ends below the slot stack's top: not the top frame.
	assert_eq!(
		region_room(2, 20, 12, ExecutorLimits::default(), (1024, 65_536, 1024)),
		Room {
			frames: 0,
			slots: 0
		}
	);
	assert_eq!(
		region_room(2, 20, 21, ExecutorLimits::default(), (1024, 65_536, 1024)),
		Room {
			frames: 0,
			slots: 0
		}
	);
}

// -------------------------------------------------------- B1-23: inline temporaries

#[test]
fn b1_inline_temporaries_charged_before_allocation() {
	let f = range_loop(8, |f| {
		for k in 0..MAX_INLINE_SITES as u32 {
			call(f, 6, 1, &[5, 2], 5 + k);
		}
	});
	let code = build(vec![f, overwrites_second_param()]);
	let expected = admitted(&code, 0, 4);
	assert_eq!(expected.sites.len(), MAX_INLINE_SITES);

	let budget = Budget::unlimited();
	let headers = scan_headers(code.function(0).unwrap(), &budget).unwrap();
	let context = FunctionContext::new(&code, 0, &budget, TRANSFER_FACTOR).unwrap();
	let before = budget.used();
	super::reset_inline_charged_for_test();
	let (region, bytes) = context.analyze_header(headers[0]).unwrap().unwrap();
	let inline_peak = super::inline_charged_peak_for_test();
	assert_eq!(region.nodes.as_slice(), expected.nodes.as_slice());
	let held = Held::adopt(&budget, bytes);
	drop((held, region));
	assert_eq!(budget.used(), before, "the header's charges are released");
	drop((headers, context));
	assert_eq!(budget.used(), 0);
	let peak = budget.peak();

	// The analysis memory formula, with R = 2 * body + 16 * 16.
	let body = code.function(0).unwrap().len();
	let formula = core::mem::size_of::<super::Site>() * 16
		+ core::mem::size_of::<SlotStateAlias>() * 256
		+ 16 * (core::mem::size_of::<bool>() + core::mem::size_of::<Option<Scalar>>())
		+ core::mem::size_of::<super::Node>() * 16 * 32
		+ core::mem::size_of::<(u32, u32, Option<Scalar>)>() * (2 * body + 16 * 16);
	assert!(
		inline_peak > 0 && inline_peak <= formula,
		"inline peak {inline_peak} vs {formula}"
	);

	// One byte below the peak: Transient, nothing left charged, no accounting violation.
	let budget = Budget::new(peak - 1);
	let outcome = (|| -> Result<(), Stop> {
		let headers = scan_headers(code.function(0).unwrap(), &budget)?;
		let context = FunctionContext::new(&code, 0, &budget, TRANSFER_FACTOR)?;
		let (region, bytes) = context.analyze_header(headers[0])?.unwrap();
		let _held = Held::adopt(&budget, bytes);
		drop(region);
		Ok(())
	})();
	assert!(matches!(outcome, Err(Stop::Budget)), "peak - 1 refuses");
	assert_eq!(budget.used(), 0);
	assert_eq!(budget.violations(), 0);

	type SlotStateAlias = super::SlotState;

	/// `fn(a, b)` with two Returns, overwriting `b` before reading it.
	fn overwrites_second_param() -> Function {
		let mut f = Function::new(4, 2).unwrap();
		lit(&mut f, 1, Value::I64(0), 100);
		binary(&mut f, 2, Binary::Lt, 0, 1, 101);
		op(
			&mut f,
			Op::Branch {
				condition: 2,
				yes: 3,
				no: 5,
			},
			102,
		);
		binary(&mut f, 3, Binary::Mul, 0, 0, 103);
		op(&mut f, Op::Return { value: 3 }, 104);
		binary(&mut f, 3, Binary::Sub, 0, 1, 105);
		op(&mut f, Op::Return { value: 3 }, 106);
		f
	}
}

// ------------------------------------------------- B1-44: no store access in-region

#[test]
fn b1_no_store_access_inside_region() {
	let region = admitted(&calls_kernel(), 0, 4);
	let entries = if cfg!(miri) { 20 } else { 1000 };
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, i64::MAX);
	let mut reference = store();
	let ref_window = calls_window(&mut reference, 0, 0, i64::MAX);
	let mut steps = 0u64;
	for k in 0..entries {
		let before = s.state_for_test();
		let mut ran = false;
		let exited = commit::enter(
			&region,
			&mut w,
			&mut s,
			false,
			3 + (k % 7),
			ROOM,
			|r, b, rec| {
				ran = true;
				simulate(r, b, rec)
			},
		)
		.unwrap()
		.unwrap();
		assert!(ran);
		assert_ne!(exited.status, Status::Retry);
		// After the commit the store differs from the entry state only in `next`: equal to a
		// store advanced by the interpreter's own `iterator_next` as many times.
		let (next, _) = s.range_iterator_for_native(w[3]).unwrap();
		while (steps as i64) < next {
			reference.iterator_next(ref_window[3]).unwrap();
			steps += 1;
		}
		assert_eq!(s.state_for_test(), reference.state_for_test(), "entry {k}");
		if next == 0 {
			assert_eq!(s.state_for_test(), before);
		}
		// Re-arm the header state: the temporary is Unit again, the binding an I64.
		w[5] = Value::Unit;
	}
	assert!(steps > 0);
}

// ------------------------------------------------------------- masks, bounds, cfg

/// A range loop touching exactly 64 caller slots with an inlined call: the callee's
/// virtual locals (64..) never set a bit of the caller's 64-bit masks.
fn sixty_four_with_call() -> Code {
	// Slots 0 a, 1 b, 2 s, 3 range, 4 iterator, 5 i, 6 t, 7..=67 scratch.
	let f = range_loop(70, |f| {
		call(f, 6, 1, &[2, 5], 5);
		op(f, Op::Copy { out: 2, value: 6 }, 6);
		for k in 0..61u32 {
			lit(f, 7 + k, Value::I64(i64::from(k)), 7 + k);
		}
	});
	build(vec![f, add_fn()])
}

#[test]
fn b1_virtual_locals_never_touch_caller_masks_at_64_slots() {
	let code = sixty_four_with_call();
	let region = admitted(&code, 0, 4);
	assert_eq!(region.slots.len(), MAX_REGION_SLOTS);
	assert_eq!(region.cells(), MAX_REGION_CELLS);
	check_expansion(&region).unwrap();
	let mut s = store();
	let mut w: StdVec<Value> = (0..70).map(|_| Value::Unit).collect();
	let range = s.range(0, 5).unwrap();
	w[0] = Value::I64(0);
	w[1] = Value::I64(5);
	w[2] = Value::I64(0);
	w[3] = range;
	w[4] = s.iterator(range).unwrap();
	for fuel in [0usize, 1, 2, 3, 5, 200] {
		let mut ww = w.clone();
		let mut s2 = store();
		let r2 = s2.range(0, 5).unwrap();
		ww[3] = r2;
		ww[4] = s2.iterator(r2).unwrap();
		let exited = commit::enter(&region, &mut ww, &mut s2, false, fuel, ROOM, simulate)
			.unwrap()
			.unwrap();
		let _ = exited;
	}
	let mut buf = [0i64; MAX_REGION_CELLS];
	buf[region.slots.len()] = 0;
	buf[region.slots.len() + 1] = 5;
	let mut record = ExitRecord::new(100);
	let status = simulate(&region, &mut buf, &mut record);
	assert_eq!(status, Status::Continue as u32);
	assert_eq!((record.bools | record.units) & !region.tracked, 0);
	assert_eq!(buf[local(&region, 2)], 10, "s = 0 + 1 + 2 + 3 + 4");
	drop(s);
}

#[test]
fn b1_cells_bound_checked_before_allocation() {
	// 64 slots plus the two iterator cells fit exactly.
	let region = admitted(&sixty_four_with_call(), 0, 4);
	assert_eq!(region.cells(), MAX_REGION_CELLS);
	// A forged region with one slot too many is refused before any buffer is filled, and
	// the simulator refuses a buffer of the wrong length.
	let mut forged = admitted(&sixty_four_with_call(), 0, 4);
	forged.slots.try_push(69).unwrap();
	forged.entry.try_push(super::EntryClass::Scalar).unwrap();
	assert_eq!(forged.cells(), MAX_REGION_CELLS + 1);
	let mut s = store();
	let mut w: StdVec<Value> = (0..70).map(|_| Value::Unit).collect();
	let r = s.range(0, 5).unwrap();
	w[3] = r;
	w[4] = s.iterator(r).unwrap();
	let before = (w.clone(), s.state_for_test());
	assert_eq!(
		commit::enter(&forged, &mut w, &mut s, false, 50, ROOM, never).unwrap(),
		Err(Refused::Window)
	);
	assert_eq!((w, s.state_for_test()), before);
	let mut buf = [0i64; MAX_REGION_CELLS];
	let mut record = ExitRecord::new(10);
	assert_eq!(simulate(&forged, &mut buf, &mut record), 0);
	// The inline limits are independent of the cell bound.
	assert_eq!(MAX_INLINE_SLOTS, 16);
	assert_eq!(VIRTUAL_BASE as usize, MAX_REGION_SLOTS);
}

/// The forced-Replay test node exists only in test builds. Every mention of it
/// in the production modules is directly under a `#[cfg(test)]` attribute.
#[test]
fn b1_force_replay_absent_outside_test_cfg() {
	for (name, text) in [
		("region/mod.rs", include_str!("mod.rs")),
		("region/simulate.rs", include_str!("simulate.rs")),
		("region/hook.rs", include_str!("hook.rs")),
		(
			"experimental_native/lower.rs",
			include_str!("../../experimental_native/lower.rs"),
		),
		(
			"experimental_native/audit.rs",
			include_str!("../../experimental_native/audit.rs"),
		),
		("region/commit.rs", include_str!("commit.rs")),
		("region/cache.rs", include_str!("cache.rs")),
	] {
		let lines: StdVec<&str> = text.lines().collect();
		for (i, line) in lines.iter().enumerate() {
			if !line.contains("ForceReplay") || line.trim_start().starts_with("//") {
				continue;
			}
			// The hook module is test-only as a whole.
			if name == "region/hook.rs" {
				continue;
			}
			let above = lines[..i]
				.iter()
				.rev()
				.find(|l| !l.trim().is_empty())
				.unwrap_or(&"");
			assert!(
				above.contains("#[cfg(test)]"),
				"{name}:{}: `{}` is not directly under #[cfg(test)]",
				i + 1,
				line.trim()
			);
		}
	}
	// No ReplayReason is test-specific: production validation is the same in every build.
	let reasons = [ReplayReason::CalleeFuel, ReplayReason::CalleeFault];
	assert_eq!(reasons.len(), 2);
}

// ------------------------------------------- simulator-mode every-fuel differentials

#[test]
#[cfg_attr(miri, ignore = "Miri: every-fuel sweep")]
fn b1_simulator_every_fuel_handbuilt() {
	let mut covered = Stats::default();
	let cases: StdVec<(&str, Code, StdVec<StdVec<Arg>>)> = vec![
		(
			"calls_kernel",
			calls_kernel(),
			vec![
				vec![Arg::I(0)],
				vec![Arg::I(1)],
				vec![Arg::I(5)],
				vec![Arg::I(12)],
			],
		),
		(
			"range_sum",
			build(vec![range_sum()]),
			vec![
				vec![Arg::I(5), Arg::I(2)],
				vec![Arg::I(0), Arg::I(0)],
				vec![Arg::I(0), Arg::I(1)],
				vec![Arg::I(-5), Arg::I(5)],
				vec![Arg::I(i64::MAX - 3), Arg::I(i64::MAX)],
				vec![Arg::I(i64::MIN), Arg::I(i64::MIN + 3)],
			],
		),
		(
			"two_arm",
			calling(two_arm(), &[5, 2]),
			vec![vec![Arg::I(0), Arg::I(6)], vec![Arg::I(-3), Arg::I(3)]],
		),
		(
			"early_return",
			calling(early_return(), &[5]),
			vec![vec![Arg::I(0), Arg::I(7)]],
		),
		(
			"three_returns",
			calling(three_returns(), &[5]),
			vec![vec![Arg::I(-2), Arg::I(3)]],
		),
		(
			"unused_argument",
			calling(first_plus_one(), &[5, 7]),
			vec![vec![Arg::I(0), Arg::I(4)]],
		),
	];
	for (name, code, inputs) in &cases {
		for args in inputs {
			let s = sweep(code, 0, args);
			std::println!(
				"B1SIM\t{name}\t{args:?}\tentries={}\tcontinue={}\tfuel={}\tretry={}\treplay={}\treplay_checked={}",
				s.entries,
				s.continue_exits,
				s.fuel_exits,
				s.retry_exits,
				s.replay_exits,
				s.replay_checked
			);
			sum(&mut covered, &s);
		}
	}
	assert!(covered.entries > 0 && covered.replay_exits > 0);
	assert_eq!(
		covered.replay_exits, covered.replay_checked,
		"every Replay traced"
	);
}
