//! N1 numeric live-in copies: admission, full-state fuel/retry differentials and
//! guard/cancel refusal before the runner or any frame/store mutation.
use super::p2b_tests::{admitted, local, sweep_inputs};
use super::tests::{bin, build, compile, lit, loc, main_of, regions, store, Arg};
use super::{
	analyze,
	budget::{Budget, Held},
	commit, EntryClass, FunctionContext, RegionRefusal, Scalar, Stop,
};
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Function, Op},
	value::Value,
};

pub(crate) const FOR: &str =
	"pub fn main(x) { let total=x; for i in 0..4 { total=(total+i%97)%1000000007; } total }";
pub(crate) const WHILE: &str = "pub fn main(x) { let total=x; let i=0; while i<4 { total=(total+i%97)%1000000007; i+=1; } total }";
pub(crate) const OVERFLOW: &str =
	"pub fn main(x,b) { let total=x; for i in 0..4 { total=(total+b)%97; } total }";

pub(crate) fn copied_parameter() -> Function {
	let mut f = Function::new(6, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0);
	bin(&mut f, 3, Binary::Lt, 2, 0, 1);
	for (op, pc) in [
		(
			Op::Branch {
				condition: 3,
				yes: 3,
				no: 7,
			},
			2,
		),
		(Op::Copy { out: 5, value: 1 }, 3),
		(
			Op::Literal {
				out: 4,
				value: Value::I64(1),
			},
			4,
		),
		(
			Op::AssignBinary {
				out: 2,
				op: Binary::Add,
				lhs: 2,
				rhs: 4,
			},
			5,
		),
		(Op::Jump { target: 1 }, 6),
		(Op::Return { value: 5 }, 7),
	] {
		f.push(op, loc(pc)).unwrap();
	}
	f
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and sweeps every fuel")]
fn n1_original_assignments_and_copy_chains_every_fuel() {
	for src in [FOR,WHILE,
		"pub fn main(x) { let total=1; for i in 0..4 { total=(total+i%97)%1000000007; } total }",
		"pub fn main(x) { let total=x; for i in 0..4 { let a=total; let b=a; total=(b+i)%97; } total }",
		"pub fn main(x) { let total=x; for i in 0..4 { total=total; total=(total+i)%97; } total }",
		"fn inner(x) { let total=x; for i in 0..4 { total=(total+i)%97; } total } pub fn main(x) { inner(x) }",
	] {
		let p=compile(src);let main=main_of(&p);
		let admitted_count=(0..p.code.functions()as u32).flat_map(|id|analyze(&p.code,id).unwrap()).filter(|(_,r)|r.is_ok()).count();
		assert!(admitted_count>0,"{src}");
		let stats=sweep_inputs(&p.code,main,&[vec![Arg::I(0)],vec![Arg::I(-5)],vec![Arg::I(17)]]);
		assert!(stats.entries>0,"{src}");
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: compiles Rune source and sweeps errors at every fuel"
)]
fn n1_checked_arithmetic_retry_matches_full_interpreter_observation() {
	let p = compile(OVERFLOW);
	let main = main_of(&p);
	let stats = sweep_inputs(
		&p.code,
		main,
		&[
			vec![Arg::I(i64::MAX), Arg::I(1)],
			vec![Arg::I(i64::MIN), Arg::I(-1)],
			vec![Arg::I(7), Arg::I(-2)],
			vec![Arg::I(i64::MAX - 1), Arg::I(1)],
		],
	);
	assert!(stats.entries > 0 && stats.retry_exits > 0);
	for op in ["/", "%"] {
		for divisor in ["-1", "0", "-3"] {
			let src=std::format!("pub fn main(x) {{ let total=x; for i in 0..3 {{ total=total{op}{divisor}; }} total }}");
			let p = compile(&src);
			let main = main_of(&p);
			assert!(regions(&p.code, main).iter().any(|(_, r)| r.is_ok()));
			let stats = sweep_inputs(
				&p.code,
				main,
				&[vec![Arg::I(i64::MIN)], vec![Arg::I(-7)], vec![Arg::I(7)]],
			);
			assert!(stats.entries > 0);
			if divisor != "-3" {
				assert!(stats.retry_exits > 0);
			}
		}
	}
}

#[test]
fn n1_new_copy_guard_and_cancel_refuse_before_runner_or_mutation() {
	let code = build(vec![copied_parameter()]);
	let r = admitted(&code, 0, 1);
	assert_eq!(
		r.entry[local(&r, 1) as usize],
		EntryClass::Guard(Scalar::I64)
	);
	let mut s = store();
	let heap = s.string("live shared value").unwrap();
	for value in [Value::F64(1.5), Value::Unit, Value::Bool(true), heap] {
		let mut w = vec![
			Value::I64(3),
			value,
			Value::I64(0),
			Value::Unit,
			Value::Unit,
			Value::Unit,
		];
		let before = w.clone();
		let state = s.state_for_test();
		let result = commit::enter(
			&r,
			&mut w,
			&mut s,
			false,
			7,
			commit::ROOM_FOR_TEST,
			|_, _, _| panic!("guard must precede runner"),
		)
		.unwrap();
		assert_eq!(result, Err(commit::Refused::Tag { slot: 1 }));
		assert_eq!(w, before);
		assert_eq!(s.state_for_test(), state);
	}
	let mut w = vec![
		Value::I64(3),
		Value::I64(7),
		Value::I64(0),
		Value::Unit,
		Value::Unit,
		Value::Unit,
	];
	let before = w.clone();
	let state = s.state_for_test();
	assert_eq!(
		commit::enter(
			&r,
			&mut w,
			&mut s,
			true,
			7,
			commit::ROOM_FOR_TEST,
			|_, _, _| panic!("cancel must precede runner")
		)
		.unwrap(),
		Err(commit::Refused::Cancel)
	);
	assert_eq!(w, before);
	assert_eq!(s.state_for_test(), state);
	s.release(heap).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn n1_second_pass_shares_the_transfer_cutoff_and_releases_all_bytes() {
	let p = compile(FOR);
	let main = main_of(&p);
	let header = regions(&p.code, main)[0].0;
	let budget = Budget::unlimited();
	let context = FunctionContext::new(&p.code, main, &budget, super::TRANSFER_FACTOR).unwrap();
	let (region, bytes) = context.analyze_header(header).unwrap().unwrap();
	let held = Held::adopt(&budget, bytes);
	let combined = context.max_transfers();
	let body = region.nodes.len();
	assert!(
		combined > body * 2,
		"two passes and backedge propagation must be counted"
	);
	drop(region);
	drop(held);
	drop(context);
	assert_eq!(budget.used(), 0);
	let mut steps = false;
	for factor in 0..super::TRANSFER_FACTOR {
		let b = Budget::unlimited();
		let c = FunctionContext::new(&p.code, main, &b, factor).unwrap();
		match c.analyze_header(header).unwrap() {
			Ok((region, bytes)) => {
				let held = Held::adopt(&b, bytes);
				drop(region);
				drop(held);
			}
			Err(RegionRefusal::Steps) => steps = true,
			Err(e) => panic!("unexpected refusal {e:?}"),
		}
		drop(c);
		assert_eq!(b.used(), 0);
	}
	assert!(steps);
	for limit in [0, 64, 1024, 4096] {
		let b = Budget::new(limit);
		if let Ok(c) = FunctionContext::new(&p.code, main, &b, super::TRANSFER_FACTOR) {
			match c.analyze_header(header) {
				Ok(Ok((region, bytes))) => {
					let held = Held::adopt(&b, bytes);
					drop(region);
					drop(held);
				}
				Ok(Err(_)) | Err(Stop::Budget) => (),
				Err(e) => panic!("unexpected {e:?}"),
			}
		}
		assert_eq!(b.used(), 0);
		assert!(b.peak() <= limit);
		assert_eq!(b.violations(), 0);
	}
}

#[test]
fn n1_guarded_pass_cannot_restart_the_transfer_allowance() {
	let mut f = Function::new(1, 1).unwrap();
	f.push(Op::Copy { out: 0, value: 0 }, loc(0)).unwrap();
	f.push(Op::Jump { target: 0 }, loc(1)).unwrap();
	let code = build(vec![f]);
	let budget = Budget::unlimited();
	// Each pass uses three transfers. A factor of two permits four TOTAL, so it
	// must stop in the guarded pass; resetting the allowance would incorrectly pass.
	let tight = FunctionContext::new(&code, 0, &budget, 2).unwrap();
	assert!(matches!(
		tight.analyze_header(0),
		Ok(Err(RegionRefusal::Steps))
	));
	assert_eq!(tight.max_transfers(), 4);
	drop(tight);
	assert_eq!(budget.used(), 0);
	let enough = FunctionContext::new(&code, 0, &budget, 3).unwrap();
	let (region, bytes) = enough.analyze_header(0).unwrap().unwrap();
	let held = Held::adopt(&budget, bytes);
	assert_eq!(enough.max_transfers(), 6);
	drop(region);
	drop(held);
	drop(enough);
	assert_eq!(budget.used(), 0);
}
