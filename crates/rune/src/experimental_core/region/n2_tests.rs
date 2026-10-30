//! N2 admission and mutation-free snapshot transaction witnesses.
use super::tests::{compile, main_of};
use super::{analyze, budget::Budget, commit, RegionRefusal, Scalar};
use crate::experimental_core::{
	heap::{Limits, Store},
	value::{Type, Value},
};

pub(crate) const INTEGER: &str = "pub fn main(x) { let v=[x,2,3]; let alias=v; for i in 0..3 { v[i]=v[i]+1; } alias[0]+v[1]+v[2] }";
pub(crate) const BOOLEAN: &str = "pub fn main(x) { let v=[true,false,true]; let count=0; for i in 0..3 { if v[i] { count=count+1; } v[i]=false; } if v[0] { 9 } else { count } }";
pub(crate) const SIEVE: &str = include_str!("fixtures/n2_sieve.rn");

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn n2_sieve_outer_region_is_the_single_collection_region() {
	let p = compile(SIEVE);
	let r = analyze(&p.code, 1).unwrap();
	assert_eq!(r.len(), 2);
	assert_eq!(r[0].0, 7);
	let outer = r[0].1.as_ref().unwrap();
	assert_eq!(outer.collection.unwrap().ty, Scalar::Bool);
	assert!(outer.nodes.iter().any(|n| n.pc == 17));
	assert!(matches!(r[1].1, Err(RegionRefusal::CollectionInner)));
}

#[test]
#[cfg_attr(miri, ignore = "Miri: source compilation and every-fuel sweep")]
fn n2_scalar_collection_full_state_every_fuel() {
	use super::p2b_tests::sweep_inputs;
	use super::tests::Arg;
	for text in [
		INTEGER,
		BOOLEAN,
		"pub fn main(x) { let v=[x,1,2]; for i in 0..4 { v[i]=v[i]+1; } 0 }",
		"pub fn main(x) { let v=[1,2]; for i in -1..2 { v[i]=9; } 0 }",
		"pub fn main(x) { let v=[x,1]; for i in 0..2 { v[i]=v[i]+1; } v[0] }",
	] {
		let p = compile(text);
		assert!(
			analyze(&p.code, main_of(&p))
				.unwrap()
				.iter()
				.any(|(_, r)| r.is_ok()),
			"{text}"
		);
		let stats = sweep_inputs(
			&p.code,
			main_of(&p),
			&[
				vec![Arg::I(0)],
				vec![Arg::I(i64::MAX)],
				vec![Arg::I(i64::MIN)],
			],
		);
		assert!(stats.entries > 0, "{text}");
	}
	for text in [
		"pub fn main() { let v=[1,2]; for i in 0..2 { let a=v; a[i]=3; } v[0] }",
		"pub fn main() { let v=[1,2]; let w=[3,4]; for i in 0..2 { v[i]=w[i]; } v[0] }",
	] {
		let p = compile(text);
		let regions = analyze(&p.code, main_of(&p)).unwrap();
		assert!(regions
			.iter()
			.any(|(_, r)| matches!(r, Err(RegionRefusal::CollectionSlot { .. }))));
		assert!(regions.iter().all(|(_, r)| r.is_err()));
	}
}

#[test]
fn n2_snapshot_budget_tag_lease_and_atomic_commit() {
	let mut s = Store::new(Limits {
		entries: 256,
		bytes: 1 << 28,
	})
	.unwrap();
	let v = s
		.list(Type::Vec, &[Value::Bool(true), Value::Bool(false)])
		.unwrap();
	let before = s.state_for_test();
	let budget = Budget::new(15);
	assert!(s.snapshot_for_native(v, Scalar::Bool, &budget).is_none());
	assert_eq!(budget.used(), 0);
	assert_eq!(before, s.state_for_test());
	assert!(s
		.snapshot_for_native(v, Scalar::I64, &Budget::unlimited())
		.is_none());
	let allocation_budget = Budget::unlimited();
	let refused = rune_alloc::limit::with(0, || {
		s.snapshot_for_native(v, Scalar::Bool, &allocation_budget)
	})
	.call();
	assert!(refused.is_none());
	assert_eq!(allocation_budget.used(), 0);
	assert_eq!(before, s.state_for_test());
	s.set_readers_for_test(v, 1);
	assert!(s
		.snapshot_for_native(v, Scalar::Bool, &Budget::unlimited())
		.is_none());
	s.set_readers_for_test(v, 0);
	assert!(!s.commit_snapshot_for_native(v, Scalar::Bool, &[0, 2], None));
	assert_eq!(before, s.state_for_test());
	assert!(s.commit_snapshot_for_native(v, Scalar::Bool, &[0, 1], None));
	assert_eq!(s.get(v, 0).unwrap(), Value::Bool(false));
	s.release(v).unwrap();
	assert!(s
		.snapshot_for_native(v, Scalar::Bool, &Budget::unlimited())
		.is_none());
	let replacement = s.list(Type::Vec, &[Value::Bool(true)]).unwrap();
	assert_ne!(v, replacement);
	assert!(s
		.snapshot_for_native(v, Scalar::Bool, &Budget::unlimited())
		.is_none());
	s.close().unwrap();
	assert!(s
		.snapshot_for_native(replacement, Scalar::Bool, &Budget::unlimited())
		.is_none());
}

#[test]
#[cfg_attr(miri, ignore = "Miri: million-element allocation/scan witness")]
fn n2_one_million_items_are_budgeted_without_an_item_ceiling() {
	let mut s = Store::new(Limits::default()).unwrap();
	let items = vec![Value::I64(i64::MIN); 1_000_000];
	let v = s.list(Type::Vec, &items).unwrap();
	let budget = Budget::new(8_000_000);
	let (mut snapshot, len) = s.snapshot_for_native(v, Scalar::I64, &budget).unwrap();
	assert_eq!((len, budget.used()), (1_000_000, 8_000_000));
	snapshot[len - 1] = i64::MAX;
	assert!(s.commit_snapshot_for_native(v, Scalar::I64, &snapshot[..len], None));
	assert_eq!(s.get(v, len - 1).unwrap(), Value::I64(i64::MAX));
	drop(snapshot);
	assert_eq!(budget.used(), 0);
	assert!(s
		.snapshot_for_native(v, Scalar::I64, &Budget::new(7_999_999))
		.is_none());
	s.release(v).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn n2_pending_cancel_refuses_before_snapshot_or_runner() {
	let p = compile(INTEGER);
	let region = analyze(&p.code, main_of(&p))
		.unwrap()
		.into_iter()
		.find_map(|(_, r)| r.ok())
		.unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut window = vec![Value::Unit; 32];
	assert_eq!(
		commit::enter_collection(
			&region,
			&mut window,
			&mut s,
			true,
			99,
			commit::ROOM_FOR_TEST,
			|_, _, _, _, _| panic!("cancel must refuse before runner")
		)
		.unwrap(),
		Err(commit::Refused::Cancel)
	);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn n2_invalid_exit_discards_all_speculative_writes() {
	let p = compile("pub fn main(v,n,i) { while i<n { v[i]=v[i]+1; i+=1; } i }");
	let id = main_of(&p);
	let region = analyze(&p.code, id)
		.unwrap()
		.into_iter()
		.find_map(|(_, r)| r.ok())
		.unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let value = s
		.list(Type::Vec, &[Value::I64(10), Value::I64(20)])
		.unwrap();
	let mut window = vec![Value::Unit; p.code.function(id).unwrap().slots as usize];
	window[0] = value;
	window[1] = Value::I64(2);
	window[2] = Value::I64(0);
	let initial = window.clone();
	let before = s.state_for_test();
	let result = commit::enter_collection(
		&region,
		&mut window,
		&mut s,
		false,
		100,
		commit::ROOM_FOR_TEST,
		|region, buf, record, snapshot, len| {
			let status = super::simulate::simulate_collection(region, buf, record, snapshot, len);
			record.reserved = 1;
			status
		},
	)
	.unwrap();
	assert_eq!(result, Err(commit::Refused::InvalidExit));
	assert_eq!(window, initial);
	assert_eq!(s.state_for_test(), before);
	s.release(value).unwrap();
}

#[test]
fn n2_moved_payload_and_invalid_iterator_preserve_collection() {
	let mut s = Store::new(Limits::default()).unwrap();
	let value = s.list(Type::Vec, &[Value::I64(10)]).unwrap();
	let before = s.state_for_test();
	assert!(!s.commit_snapshot_for_native(value, Scalar::I64, &[99], Some((value, (0, 1), 1))));
	assert_eq!(s.state_for_test(), before);
	let moved = s.take_capture(value).unwrap();
	assert!(s
		.snapshot_for_native(value, Scalar::I64, &Budget::unlimited())
		.is_none());
	s.release(value).unwrap();
	s.release(moved).unwrap();
}
