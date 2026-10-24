//! Representable defaults retain fallible allocation and publication boundaries.
use super::*;
use rune_alloc::limit;

#[test]
fn defaults_use_representable_maxima_without_preallocation() {
	let limits = Limits::default();
	assert_eq!(
		limits.entries,
		MAX_ENTRIES
			.min(MAX_BYTES / size_of::<Slot>())
			.min(MAX_BYTES / size_of::<u32>())
	);
	assert_eq!(limits.bytes, MAX_BYTES);
	let store = Store::new(limits).unwrap();
	assert_eq!(
		(
			store.slots.capacity(),
			store.free.capacity(),
			store.work.capacity()
		),
		(INITIAL_ENTRIES, INITIAL_ENTRIES, INITIAL_WORK)
	);
	assert_eq!(
		limit::with(0, || Store::new(limits))
			.call()
			.err()
			.unwrap()
			.kind,
		ErrorKind::Allocation
	);
}

#[test]
fn maximum_default_allocation_failures_preserve_owned_values() {
	let mut store = Store::new(Limits::default()).unwrap();
	let owned = store.string("owned").unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	assert_eq!(
		limit::with(0, || store.string("new payload"))
			.call()
			.unwrap_err()
			.kind,
		ErrorKind::Allocation
	);
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	assert_eq!(store.text(owned).unwrap(), "owned");
	store.release(owned).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn maximum_default_slot_growth_fails_before_publication() {
	let mut store = Store::new(Limits::default()).unwrap();
	let mut values = [Value::Unit; INITIAL_ENTRIES];
	for value in &mut values {
		*value = store.string("").unwrap();
	}
	assert_eq!(store.slots.len(), INITIAL_ENTRIES);
	assert_eq!(
		limit::with(0, || store.string("")).call().unwrap_err().kind,
		ErrorKind::Allocation
	);
	assert_eq!(
		(store.live_entries(), store.payload_bytes()),
		(INITIAL_ENTRIES, 0)
	);
	for value in values {
		assert_eq!(store.text(value).unwrap(), "");
		limit::with(0, || store.release(value)).call().unwrap();
	}
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}
