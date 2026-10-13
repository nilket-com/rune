//! Ownership and failure-path checks independent of the stock oracle.
use super::*;
use rune_alloc::limit;

#[test]
fn static_insert_allocation_rollback_and_existing_update() {
	let mut failures = 0;
	let mut successes = 0;
	for budget in 0..160 {
		let mut s = Store::new(Limits::default()).unwrap();
		let object = s.record(Type::Object, &[]).unwrap();
		let child = s.string("child").unwrap();
		let before = (s.live, s.bytes, s.edges, std::format!("{:?}", s.slots));
		match limit::with(budget, || s.set_field(object, "new", child)).call() {
			Ok(()) => {
				successes += 1;
				let found = s.field(object, "new").unwrap();
				assert_eq!(found, child);
				s.release(found).unwrap();
			}
			Err(e) => {
				failures += 1;
				assert_eq!(e.kind, ErrorKind::Allocation);
				assert_eq!(
					(s.live, s.bytes, s.edges, std::format!("{:?}", s.slots)),
					before
				);
			}
		}
		s.release(object).unwrap();
		s.release(child).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
	assert!(failures > 0 && successes > 0);
	let mut s = Store::new(Limits::default()).unwrap();
	let object = s.record(Type::Object, &[("x", Value::I64(1))]).unwrap();
	limit::with(0, || s.set_field(object, "x", Value::I64(2)))
		.call()
		.unwrap();
	assert_eq!(s.field(object, "x").unwrap(), Value::I64(2));
	s.release(object).unwrap();
}

#[test]
fn object_iterator_allocation_failures_release_partial_pairs_and_lease() {
	let mut failed = 0;
	let mut passed = 0;
	for budget in 0..256 {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.string("child").unwrap();
		let object = s.record(Type::Object, &[("key", child)]).unwrap();
		let baseline = s.live_entries();
		let iter = s.iterator(object).unwrap();
		match limit::with(budget, || s.iterator_next(iter)).call() {
			Ok(Some(pair)) => {
				passed += 1;
				assert_eq!(s.tag(pair).unwrap(), Type::Tuple);
				s.release(pair).unwrap();
			}
			Err(e) => {
				failed += 1;
				assert_eq!(e.kind, ErrorKind::Allocation);
			}
			other => panic!("unexpected {other:?}"),
		}
		s.release(iter).unwrap();
		assert_eq!(s.live_entries(), baseline);
		s.set_field(object, "next", Value::I64(3)).unwrap();
		s.release(object).unwrap();
		s.release(child).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
	assert!(failed > 0 && passed > 0);
}

#[test]
fn object_and_tuple_leases_cover_nested_readers_and_overflow() {
	let mut s = Store::new(Limits::default()).unwrap();
	for object in [
		s.record(Type::Object, &[("x", Value::I64(1))]).unwrap(),
		s.list(Type::Tuple, &[Value::I64(1)]).unwrap(),
	] {
		let Value::Heap(h) = object else {
			unreachable!()
		};
		s.entry_mut(h).unwrap().readers = u32::MAX;
		assert_eq!(s.iterator(object).unwrap_err().kind, ErrorKind::Overflow);
		s.entry_mut(h).unwrap().readers = 0;
		let first = s.iterator(object).unwrap();
		let second = s.iterator(object).unwrap();
		assert_eq!(s.mutable(h).unwrap_err().kind, ErrorKind::AccessConflict);
		s.release(first).unwrap();
		assert_eq!(s.mutable(h).unwrap_err().kind, ErrorKind::AccessConflict);
		s.release(second).unwrap();
		s.mutable(h).unwrap();
		s.release(object).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn option_snapshot_transfers_payload_and_never_leases_wrapper() {
	let mut s = Store::new(Limits::default()).unwrap();
	let payload = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let option = s.variant(Type::Option, true, Some(payload)).unwrap();
	for consume in [false, true] {
		let iter = s.iterator(option).unwrap();
		let Value::Heap(h) = option else {
			unreachable!()
		};
		assert_eq!(s.entry(h).unwrap().readers, 0);
		s.tuple_set_mut(option, 0, Value::I64(9)).unwrap();
		if consume {
			let value = s.iterator_next(iter).unwrap().unwrap();
			assert_eq!(value, payload);
			s.release(value).unwrap();
			assert_eq!(s.iterator_next(iter).unwrap(), None);
		}
		s.release(iter).unwrap();
		s.tuple_set_mut(option, 0, payload).unwrap();
	}
	s.release(option).unwrap();
	s.release(payload).unwrap();
	assert_eq!(s.live_entries(), 0);
}
