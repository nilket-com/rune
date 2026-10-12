//! Targeted allocation transitions, independently of the executor budget sweep.

use super::*;
use rune_alloc::limit;

fn store() -> Store {
	Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap()
}

fn state(s: &Store) -> (usize, usize, usize) {
	(
		s.live_entries(),
		s.payload_bytes(),
		s.reserved_metadata_bytes(),
	)
}

#[test]
fn construction_refuses_each_metadata_reservation() {
	let limits = Limits {
		entries: 4,
		bytes: 128,
	};

	let slots = limits.entries * size_of::<Slot>();
	let free = limits.entries * size_of::<u32>();
	let work = INITIAL_WORK * size_of::<Value>();

	for budget in [slots - 1, slots + free - 1, slots + free + work - 1] {
		let kind = limit::with(budget, || Store::new(limits).err().map(|e| e.kind)).call();
		assert_eq!(kind, Some(ErrorKind::Allocation));
	}

	let built = limit::with(slots + free + work, || Store::new(limits))
		.call()
		.unwrap();
	assert_eq!(built.reserved_metadata_bytes(), slots + free + work);
}

#[test]
fn payload_copy_refusals_publish_nothing_and_keep_children_owned() {
	let mut s = store();
	let child = s.string("held").unwrap();
	let baseline = state(&s);
	let kind = limit::with(0, || s.string("new").unwrap_err().kind).call();
	assert_eq!(kind, ErrorKind::Allocation);
	let kind = limit::with(size_of::<Value>() - 1, || {
		s.list(Type::Vec, &[child]).unwrap_err().kind
	})
	.call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(state(&s), baseline);
	assert_eq!(
		s.entry(match child {
			Value::Heap(h) => h,
			_ => unreachable!(),
		})
		.unwrap()
		.strong,
		1
	);

	// Record fields reserve first, then copy keys; one byte short of the key
	// reservation tests destruction of the unpublished field buffer.
	let fields = size_of::<(String, Value)>();
	let kind = limit::with(fields + 3, || {
		s.record(Type::Object, &[("key!", child)]).unwrap_err().kind
	})
	.call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(state(&s), baseline);
	s.release(child).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
	let fresh = s.string("reused").unwrap();
	s.release(fresh).unwrap();
}

#[test]
fn clone_second_copy_refusal_keeps_original_and_children() {
	let mut s = store();
	let child = s.string("held").unwrap();
	let list = s.list(Type::Vec, &[child]).unwrap();
	s.release(child).unwrap();
	let baseline = state(&s);

	// clone_outer allocates a scratch list, then list allocates its own copy.
	let kind = limit::with(size_of::<Value>(), || s.clone_outer(list).unwrap_err().kind).call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(state(&s), baseline);
	let retained = s.get(list, 0).unwrap();
	assert_eq!(s.text(retained).unwrap(), "held");
	s.release(retained).unwrap();
	s.release(list).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn growth_allocation_refusals_preserve_buffer_charge_and_aliases() {
	let mut s = store();
	let text = s.string("abc").unwrap();
	let list = s.list(Type::Vec, &[text]).unwrap();
	let baseline = state(&s);
	let kind = limit::with(0, || s.append_string(text, text).unwrap_err().kind).call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(s.text(text).unwrap(), "abc");
	let kind = limit::with(0, || s.push(list, text).unwrap_err().kind).call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(s.len(list).unwrap(), 1);
	assert_eq!(state(&s), baseline);
	s.push(list, text).unwrap();
	s.release(text).unwrap();
	s.release(list).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn admission_retain_overflow_rolls_back_the_prefix_without_allocation() {
	let mut s = store();
	let child = s.string("held").unwrap();
	let overflowing = s.string("full").unwrap();
	let Value::Heap(h) = overflowing else {
		unreachable!()
	};

	s.entry_mut(h).unwrap().strong = u32::MAX;
	let baseline = state(&s);
	let kind = s
		.list(Type::Vec, &[child, child, overflowing])
		.unwrap_err()
		.kind;
	assert_eq!(kind, ErrorKind::Overflow);
	assert_eq!(state(&s), baseline);
	let Value::Heap(c) = child else {
		unreachable!()
	};

	assert_eq!(s.entry(c).unwrap().strong, 1);
	assert_eq!(s.entry(h).unwrap().strong, u32::MAX);
	s.entry_mut(h).unwrap().strong = 1;
	s.release(child).unwrap();
	s.release(overflowing).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn formatting_refusal_and_owned_admission_keep_original_state() {
	let mut s = store();
	let text = s.string("held").unwrap();
	let baseline = state(&s);
	let kind = limit::with(3, || s.format_value(text, false).unwrap_err().kind).call();
	assert_eq!(kind, ErrorKind::Allocation);
	assert_eq!(state(&s), baseline);
	assert_eq!(s.text(text).unwrap(), "held");

	// The pre-count and render share an immutable store; only the bounded
	// output buffer allocates. Owned admission must not copy it again.
	let rendered = s.format_value(text, false).unwrap();
	let result = limit::with(0, || s.string_owned(rendered)).call().unwrap();
	assert_eq!(s.text(result).unwrap(), "held");
	s.release(result).unwrap();
	s.release(text).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn iterator_admission_and_cleanup_allocate_nothing_after_reservation() {
	let mut s = store();
	let child = s.string("held").unwrap();
	let list = s.list(Type::Vec, &[child]).unwrap();
	s.release(child).unwrap();
	let metadata = s.reserved_metadata_bytes();
	let iterator = limit::with(0, || s.iterator(list)).call().unwrap();
	assert_eq!(
		s.push(list, Value::I64(2)).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	limit::with(0, || {
		s.release(list).unwrap();
		s.release(iterator).unwrap();
	})
	.call();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
	assert_eq!(s.reserved_metadata_bytes(), metadata);
	assert!(s.work.is_empty());
}
