//! Range advance contracts, established before the single-lookup optimization.
use super::*;
use crate::experimental_core::error::{Detail, MovedAccess};
use rune_alloc::limit;

fn store_iter(start: i64, end: i64) -> (Store, Value) {
	let mut store = Store::new(Limits::default()).unwrap();
	let range = store.range(start, end).unwrap();
	let iter = store.iterator(range).unwrap();
	store.release(range).unwrap();
	(store, iter)
}

fn handle(value: Value) -> Handle {
	let Value::Heap(handle) = value else {
		panic!("expected heap iterator");
	};
	handle
}

fn accounting(s: &Store) -> (usize, usize, usize, usize, usize) {
	s.assert_work();
	(s.live, s.bytes, s.edges, s.work.len(), s.work.capacity())
}

fn failure(s: &mut Store, value: Value, kind: ErrorKind, detail: Detail) {
	let before = accounting(s);
	let error = limit::with(0, || s.iterator_next(value))
		.call()
		.unwrap_err();
	assert_eq!(error.kind, kind);
	assert_eq!(error.detail, detail);
	assert_eq!(accounting(s), before);
}

#[test]
fn stale_generation_is_refused_before_advance() {
	let (mut s, iter) = store_iter(1, 3);
	s.release(iter).unwrap();
	let replacement = s.admit(Payload::RangeIterator { next: 8, end: 9 }).unwrap();
	assert_eq!(handle(iter).slot, handle(replacement).slot);
	assert_ne!(handle(iter).generation, handle(replacement).generation);
	failure(&mut s, iter, ErrorKind::InvalidHandle, Detail::None);
	assert_eq!(s.iterator_next(replacement).unwrap(), Some(Value::I64(8)));
	s.release(replacement).unwrap();
	assert_eq!(s.live, 0);
}

#[test]
fn foreign_root_is_refused_at_owner_boundary_despite_matching_handle() {
	// Internal Values carry no owner; Root import is the sealed owner check.
	let (mut owner, foreign) = store_iter(1, 3);
	let (mut receiver, local) = store_iter(7, 9);
	assert_eq!(foreign, local);
	let root = owner.export(foreign);
	let before = (accounting(&owner), accounting(&receiver));
	let error = receiver.import(&root).unwrap_err();
	assert_eq!(error.kind, ErrorKind::InvalidHandle);
	assert_eq!(error.detail, Detail::None);
	assert_eq!((accounting(&owner), accounting(&receiver)), before);
	assert_eq!(owner.iterator_next(foreign).unwrap(), Some(Value::I64(1)));
	assert_eq!(receiver.iterator_next(local).unwrap(), Some(Value::I64(7)));
	owner.release(foreign).unwrap();
	receiver.release(local).unwrap();
}

#[test]
fn closed_store_precedes_stale_handle() {
	let (mut s, iter) = store_iter(1, 3);
	s.close().unwrap();
	failure(&mut s, iter, ErrorKind::ClosedStore, Detail::None);
}

#[test]
fn reader_conflict_preserves_position_and_precedes_moved() {
	let (mut s, iter) = store_iter(1, 3);
	let h = handle(iter);
	s.entry_mut(h).unwrap().readers = 1;
	failure(
		&mut s,
		iter,
		ErrorKind::AccessConflict,
		Detail::WriteBorrowed,
	);
	s.entry_mut(h).unwrap().readers = 0;
	assert_eq!(s.iterator_next(iter).unwrap(), Some(Value::I64(1)));
	s.invalidate(iter).unwrap();
	s.entry_mut(h).unwrap().readers = 1;
	failure(
		&mut s,
		iter,
		ErrorKind::AccessConflict,
		Detail::WriteBorrowed,
	);
	s.entry_mut(h).unwrap().readers = 0;
	s.release(iter).unwrap();
}

#[test]
fn moved_iterator_reports_write_access_without_consuming_tombstone() {
	let (mut s, iter) = store_iter(1, 3);
	s.invalidate(iter).unwrap();
	failure(
		&mut s,
		iter,
		ErrorKind::AccessConflict,
		Detail::Moved {
			access: MovedAccess::Write,
		},
	);
	s.release(iter).unwrap();
	assert_eq!(s.live, 0);
}

#[test]
fn sequences_empty_negative_and_reversed_are_allocation_free_and_balanced() {
	for (start, end) in [(0, 3), (-3, 1), (2, 2), (3, 2)] {
		let (mut s, iter) = store_iter(start, end);
		let before = accounting(&s);
		for expected in start..end {
			assert_eq!(
				limit::with(0, || s.iterator_next(iter)).call().unwrap(),
				Some(Value::I64(expected))
			);
			assert_eq!(accounting(&s), before);
		}
		for _ in 0..3 {
			assert_eq!(
				limit::with(0, || s.iterator_next(iter)).call().unwrap(),
				None
			);
			assert_eq!(accounting(&s), before);
		}
		let Payload::RangeIterator { next, .. } = s.payload(handle(iter)).unwrap() else {
			panic!("range iterator");
		};
		assert_eq!(*next, start.max(end));
		s.release(iter).unwrap();
		assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
		s.assert_work();
	}
}

#[test]
fn maximum_endpoint_exhausts_without_overflow_or_position_change() {
	// next < end <= MAX makes checked_add overflow unreachable, including
	// unit-constructed payloads; retain the defensive guard in production.
	let (mut s, iter) = store_iter(i64::MAX - 1, i64::MAX);
	assert_eq!(
		s.iterator_next(iter).unwrap(),
		Some(Value::I64(i64::MAX - 1))
	);
	for _ in 0..3 {
		assert_eq!(s.iterator_next(iter).unwrap(), None);
	}
	let Payload::RangeIterator { next, end } = s.payload(handle(iter)).unwrap() else {
		panic!("range iterator");
	};
	assert_eq!((*next, *end), (i64::MAX, i64::MAX));
	s.release(iter).unwrap();
}

#[test]
fn releasing_partial_iterator_preserves_other_live_children_and_work_reserve() {
	let (mut s, iter) = store_iter(-4, 4);
	let child = s.string("live child").unwrap();
	let parent = s.list(Type::Vec, &[child, child]).unwrap();
	s.release(child).unwrap();
	let before = accounting(&s);
	assert_eq!(s.iterator_next(iter).unwrap(), Some(Value::I64(-4)));
	assert_eq!(accounting(&s), before);
	s.release(iter).unwrap();
	assert_eq!(s.live, before.0 - 1);
	assert_eq!(s.edges, before.2);
	s.release(parent).unwrap();
	assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
	s.assert_work();
}
