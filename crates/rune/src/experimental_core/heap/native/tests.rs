//! B1 Store interface tests (rnx 0184; STORE-SETTER.md §9): the RangeIterator read at entry
//! and validation, and the native tier's one Store mutation, the commit of `next`. Safe
//! Rust, no native code, Miri-able (safe glue only).
use super::{range_next_reachable, RangeCommitRefused};
use crate::experimental_core::heap::{Limits, Store};
use crate::experimental_core::value::{Type, Value};
use std::vec::Vec as StdVec;

fn store() -> Store {
	Store::new(Limits {
		entries: 64,
		bytes: 65_536,
	})
	.unwrap()
}

/// A fresh RangeIterator over `start..end` (the Range itself stays alive, as in a script).
fn range_iterator(s: &mut Store, start: i64, end: i64) -> (Value, Value) {
	let range = s.range(start, end).unwrap();
	let iterator = s.iterator(range).unwrap();
	(range, iterator)
}

/// Every way `range_iterator_for_native` must refuse, each built on its own store. The
/// closure returns the store and the value to read; `state_for_test()` is compared before
/// and after the read (and the setter) by the callers.
fn refusal_cases() -> StdVec<(&'static str, Store, Value)> {
	let mut out = StdVec::new();

	for (name, value) in [
		("i64", Value::I64(3)),
		("unit", Value::Unit),
		("bool", Value::Bool(true)),
	] {
		out.push((name, store(), value));
	}

	let mut s = store();
	let range = s.range(0, 10).unwrap();
	out.push(("range_handle", s, range));

	let mut s = store();
	let list = s.list(Type::Vec, &[Value::I64(1), Value::I64(2)]).unwrap();
	let vector = s.iterator(list).unwrap();
	out.push(("vector_iterator", s, vector));

	let mut s = store();
	let (_, it) = range_iterator(&mut s, 0, 10);
	let Value::Heap(h) = it else { unreachable!() };
	s.set_generation_for_test(it, h.generation + 1);
	out.push(("stale_handle", s, it));

	// Retired then reused: the iterator's last reference is released, and a new entry
	// reuses its slot with the next generation.
	let mut s = store();
	let (range, it) = range_iterator(&mut s, 0, 10);
	s.release(it).unwrap();
	s.release(range).unwrap();
	let _reuse = s.string("reuse").unwrap();
	let _reuse2 = s.string("reuse2").unwrap();
	out.push(("retired_reused", s, it));

	let mut s = store();
	let (_, it) = range_iterator(&mut s, 0, 10);
	s.invalidate(it).unwrap();
	out.push(("taken_payload", s, it));

	let mut s = store();
	let (_, it) = range_iterator(&mut s, 0, 10);
	s.set_readers_for_test(it, 1);
	out.push(("readers", s, it));

	let mut s = store();
	let (_, it) = range_iterator(&mut s, 0, 10);
	s.close().unwrap();
	out.push(("closed_store", s, it));

	// Exhausted (next >= end) with readers: legal for the interpreter, refused here.
	let mut s = store();
	let (_, it) = range_iterator(&mut s, 5, 5);
	s.set_readers_for_test(it, 2);
	out.push(("exhausted_with_readers", s, it));

	out
}

// ------------------------------------------------------------------- B1-18

#[test]
fn b1_store_range_read_entry_checks() {
	let mut s = store();
	let (_, it) = range_iterator(&mut s, -3, 7);
	let before = s.state_for_test();
	assert_eq!(s.range_iterator_for_native(it), Some((-3, 7)));
	assert_eq!(s.state_for_test(), before, "the read mutates nothing");

	// Advanced once by the interpreter: the read follows.
	s.iterator_next(it).unwrap();
	assert_eq!(s.range_iterator_for_native(it), Some((-2, 7)));

	for (name, s, value) in refusal_cases() {
		let before = s.state_for_test();
		assert_eq!(s.range_iterator_for_native(value), None, "{name}");
		assert_eq!(s.state_for_test(), before, "{name}: the refused read mutates nothing");
	}
}

// ------------------------------------------------------------------- B1-19

#[test]
fn b1_store_range_setter_refusals_mutate_nothing() {
	// Every B1-18 refusal, with an `expect` the payload would match if it were readable.
	for (name, mut s, value) in refusal_cases() {
		let before = s.state_for_test();
		for expect in [(0, 10), (5, 5), (-3, 7)] {
			assert_eq!(
				s.commit_range_next_for_native(value, expect, expect.0),
				Err(RangeCommitRefused),
				"{name}"
			);
		}
		assert_eq!(s.state_for_test(), before, "{name}");
	}

	// `expect` differs from the payload in `next` only, then in `end` only.
	let mut s = store();
	let (_, it) = range_iterator(&mut s, 0, 10);
	let before = s.state_for_test();
	assert_eq!(s.commit_range_next_for_native(it, (1, 10), 2), Err(RangeCommitRefused));
	assert_eq!(s.commit_range_next_for_native(it, (0, 11), 2), Err(RangeCommitRefused));
	assert_eq!(s.state_for_test(), before);

	// Unreachable `new` at the boundaries (checked on fresh stores; no panic with overflow
	// checks on).
	let unreachable_cases: [((i64, i64), i64); 6] = [
		((0, 10), -1),
		((0, 10), 11),
		((5, 2), 6),
		((5, 2), 4),
		((0, 0), 1),
		((i64::MAX - 1, i64::MAX), i64::MIN),
	];
	for ((start, end), new) in unreachable_cases {
		let mut s = store();
		let (_, it) = range_iterator(&mut s, start, end);
		let before = s.state_for_test();
		assert!(!range_next_reachable((start, end), new), "{start}..{end} -> {new}");
		assert_eq!(
			s.commit_range_next_for_native(it, (start, end), new),
			Err(RangeCommitRefused),
			"{start}..{end} -> {new}"
		);
		assert_eq!(s.state_for_test(), before);
	}

	// Over (MIN, MAX) every value is reachable: nothing lies outside it.
	for new in [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX] {
		assert!(range_next_reachable((i64::MIN, i64::MAX), new), "{new}");
	}

	// Empty and reversed ranges admit only the unchanged `next`.
	assert!(range_next_reachable((0, 0), 0));
	assert!(range_next_reachable((5, 2), 5));
	assert!(!range_next_reachable((5, 2), 2));
	assert!(range_next_reachable((i64::MAX, i64::MAX), i64::MAX));
	assert!(range_next_reachable((i64::MIN, i64::MIN), i64::MIN));
}

// ------------------------------------------------------------------- B1-20

#[test]
fn b1_store_range_setter_equals_interpreter_advance() {
	let ranges: [(i64, i64); 6] = [
		(0, 0),
		(5, 2),
		(0, 1),
		(0, 30),
		(i64::MAX - 3, i64::MAX),
		(i64::MIN, i64::MIN + 3),
	];
	for (start, end) in ranges {
		let steps = if start >= end {
			0
		} else {
			// Small by construction (at most 30).
			(end as i128 - start as i128) as u32
		};
		for k in 0..=steps {
			// Store A: the interpreter's advance, k times (plus one exhausted call when k
			// reaches the end, which mutates nothing).
			let mut a = store();
			let (_, ia) = range_iterator(&mut a, start, end);
			a.retain(ia).unwrap();
			let payload = a.payload_bytes();
			let live = a.live_entries();
			for _ in 0..k {
				assert!(a.iterator_next(ia).unwrap().is_some());
			}
			if k == steps {
				assert!(a.iterator_next(ia).unwrap().is_none());
			}

			// Store B: the same construction, then the setter to the same `next`.
			let mut b = store();
			let (_, ib) = range_iterator(&mut b, start, end);
			b.retain(ib).unwrap();
			let new = if start >= end { start } else { start + i64::from(k) };
			assert_eq!(b.commit_range_next_for_native(ib, (start, end), new), Ok(()));

			assert_eq!(a.state_for_test(), b.state_for_test(), "{start}..{end} k={k}");
			assert_eq!((a.payload_bytes(), a.live_entries()), (payload, live));
			assert_eq!((b.payload_bytes(), b.live_entries()), (payload, live));
			// The alias (strong 2) reads the same `next` in both.
			assert_eq!(a.range_iterator_for_native(ia), Some((new, end)));
			assert_eq!(b.range_iterator_for_native(ib), Some((new, end)));
		}
	}
}
