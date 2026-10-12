//! B3: actual child ownership and release capacity across growth and rollback.
use super::*;
use rune_alloc::limit;

fn store() -> Store {
	Store::new(Limits {
		entries: 512,
		bytes: 1 << 20,
	})
	.unwrap()
}
fn check(s: &Store) {
	let actual: usize = s
		.slots
		.iter()
		.filter_map(|s| s.entry.as_ref())
		.filter_map(|e| e.payload.as_ref())
		.map(Store::payload_edges)
		.sum();
	assert_eq!(s.edges, actual);
	s.assert_work();
	assert!(s.work.is_empty());
}
fn heap_state(s: &Store) -> (usize, usize, usize, std::string::String) {
	(s.live, s.bytes, s.edges, std::format!("{:?}", s.slots))
}

#[test]
fn large_configurations_validate_without_proportional_allocations() {
	for limits in [
		Limits {
			entries: 0,
			bytes: 1,
		},
		Limits {
			entries: usize::MAX,
			bytes: 1,
		},
		Limits {
			entries: 1,
			bytes: usize::MAX,
		},
	] {
		assert_eq!(
			Store::new(limits).err().unwrap().kind,
			ErrorKind::StoreCapacity
		);
	}
	#[cfg(target_pointer_width = "64")]
	{
		let s = Store::new(Limits {
			entries: (u32::MAX - 1) as usize,
			bytes: isize::MAX as usize,
		})
		.unwrap();
		assert_eq!(
			(s.slots.capacity(), s.free.capacity(), s.work.capacity()),
			(INITIAL_ENTRIES, INITIAL_ENTRIES, INITIAL_WORK)
		);
		check(&s);
	}
	let s = store();
	assert_eq!(s.bookkeeping_bytes().unwrap(), s.reserved_metadata_bytes());
	std::eprintln!(
		"B3 Slot={} Entry={} Value={} initial_metadata={}",
		size_of::<Slot>(),
		size_of::<Entry>(),
		size_of::<Value>(),
		s.bookkeeping_bytes().unwrap()
	);
}

#[test]
fn table_growth_reserves_free_for_all_slots_and_reuses_without_allocating() {
	let mut s = store();
	let mut values = std::vec::Vec::with_capacity(200);
	for _ in 0..200 {
		values.push(s.string("").unwrap());
		check(&s);
	}
	let high = s.reserved_metadata_bytes();
	assert!(s.slots.capacity() >= 200 && s.free.capacity() >= s.slots.len());
	limit::with(0, || {
		for value in values.drain(..) {
			s.release(value).unwrap();
			check(&s);
		}
	})
	.call();
	assert_eq!(s.live, 0);
	limit::with(0, || {
		for _ in 0..200 {
			values.push(s.string("").unwrap());
		}
		for value in values.drain(..) {
			s.release(value).unwrap();
		}
	})
	.call();
	assert_eq!(s.reserved_metadata_bytes(), high);
	check(&s);
	let mut tiny = Store::new(Limits {
		entries: 1,
		bytes: 8,
	})
	.unwrap();
	let value = tiny.string("").unwrap();
	let retired = tiny.set_generation_for_test(value, u32::MAX);
	tiny.release(retired).unwrap();
	assert_eq!(tiny.slots.len(), 1);
	assert!(tiny.free.is_empty());
	check(&tiny);
	assert_eq!(tiny.string("").unwrap_err().kind, ErrorKind::StoreCapacity);
}

#[test]
fn wide_aliases_and_iterator_owners_release_under_zero_allocation_budget() {
	let mut s = store();
	let child = s.string("owned").unwrap();
	let list = s.list(Type::Vec, &std::vec![child; 200]).unwrap();
	assert_eq!(s.edges, 200);
	check(&s);
	let iter = s.iterator(list).unwrap();
	assert_eq!(s.edges, 201);
	check(&s);
	s.release(child).unwrap();
	s.release(list).unwrap();
	let metadata = s.reserved_metadata_bytes();
	limit::with(0, || s.release(iter)).call().unwrap();
	assert_eq!(s.live, 0);
	assert_eq!(s.bytes, 0);
	check(&s);
	assert_eq!(metadata, s.reserved_metadata_bytes());
}

#[test]
fn replacements_track_actual_edges_including_aliases_and_self_references() {
	let mut s = store();
	let child = s.string("child").unwrap();
	let list = s.list(Type::Vec, &[Value::Unit]).unwrap();
	let object = s.record(Type::Object, &[("x", Value::Unit)]).unwrap();
	for new in [child, child, Value::I64(7), child, Value::Unit] {
		s.set(list, 0, new).unwrap();
		check(&s);
		s.set_field(object, "x", new).unwrap();
		check(&s);
		assert_eq!(s.edges, 2 * Store::heap_edge(new));
	}
	s.set(list, 0, list).unwrap();
	check(&s);
	s.release(list).unwrap(); // Self-edge remains until explicit invalidation.
	limit::with(0, || s.invalidate(list)).call().unwrap();
	check(&s);
	assert_eq!(s.edges, 0);
	s.release(object).unwrap();
	s.release(child).unwrap();
	check(&s);
	assert_eq!((s.live, s.bytes), (0, 0));
}

#[test]
fn work_reservation_and_retain_failures_precede_any_mutation() {
	let mut s = store();
	let child = s.string("child").unwrap();
	let list = s.list(Type::Vec, &[Value::Unit]).unwrap();
	let object = s.record(Type::Object, &[("x", Value::Unit)]).unwrap();
	for which in 0..3 {
		s.work = Vec::try_with_capacity(s.edges + 1).unwrap();
		let before = heap_state(&s);
		let failure = limit::with(0, || match which {
			0 => s.set(list, 0, child),
			1 => s.set_field(object, "x", child),
			_ => s.push(list, child),
		})
		.call()
		.unwrap_err();
		assert_eq!(failure.kind, ErrorKind::Allocation);
		assert_eq!(heap_state(&s), before);
		check(&s);
		assert_eq!(s.test_strong(child), 1);
	}
	s.reserve_work(2, 2).unwrap();
	s.test_set_strong(child, u32::MAX);
	let before = heap_state(&s);
	assert_eq!(s.set(list, 0, child).unwrap_err().kind, ErrorKind::Overflow);
	assert_eq!(heap_state(&s), before);
	check(&s);
	s.test_set_strong(child, 1);
	s.set(list, 0, child).unwrap();
	check(&s);
	// Replacement conservative peak includes both the installed edge and queued old edge.
	s.work = Vec::try_with_capacity(s.edges + 1).unwrap();
	let before = heap_state(&s);
	assert_eq!(
		limit::with(0, || s.set(list, 0, child))
			.call()
			.unwrap_err()
			.kind,
		ErrorKind::Allocation
	);
	assert_eq!(heap_state(&s), before);
	check(&s);
	s.release(list).unwrap();
	s.release(object).unwrap();
	s.release(child).unwrap();
	check(&s);
}

#[test]
fn admission_failure_at_slot_or_free_growth_keeps_roots_and_links() {
	for site in 0..2 {
		let mut s = store();
		let owned = s.string("").unwrap();
		// Force the next new slot through one actual failing reservation at a time.
		if site == 0 {
			s.free = Vec::try_with_capacity(1).unwrap();
		} else {
			let slot = s.slots.pop().unwrap();
			s.slots = Vec::try_with_capacity(1).unwrap();
			s.slots.try_push(slot).unwrap();
		}
		let before = heap_state(&s);
		assert_eq!(
			limit::with(0, || s.string("")).call().unwrap_err().kind,
			ErrorKind::Allocation
		);
		assert_eq!(heap_state(&s), before);
		check(&s);
		assert_eq!(s.test_strong(owned), 1);
		let next = s.string("").unwrap();
		s.release(owned).unwrap();
		s.release(next).unwrap();
		check(&s);
	}
}
