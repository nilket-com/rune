//! Local store construction regressions, not reviewed conformance evidence.
use crate::experimental_core::{
	error::ErrorKind,
	heap::{Limits, Store},
	value::{Type, Value},
};

#[test]
fn grown_string_charge_is_released_once_through_aliases() {
	let mut store = Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap();
	let text = store.string("abc").unwrap();
	store.retain(text).unwrap();
	let alias = text;
	for _ in 0..4 {
		store.append_string(text, text).unwrap();
	}
	assert_eq!(store.text(alias).unwrap().len(), 48);
	assert!(store.payload_bytes() >= 48);
	store.release(text).unwrap();
	assert_eq!(store.live_entries(), 1);
	store.invalidate(alias).unwrap();
	assert_eq!(store.payload_bytes(), 0);
	store.release(alias).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn vector_growth_transfers_existing_edges_and_charges_actual_capacity() {
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let child = store.string("held").unwrap();
	let list = store.list(Type::Vec, &[child]).unwrap();
	store.release(child).unwrap();
	for n in 0..17 {
		store.push(list, Value::I64(n)).unwrap();
	}
	assert_eq!(store.len(list).unwrap(), 18);
	let retained = store.get(list, 0).unwrap();
	assert_eq!(store.text(retained).unwrap(), "held");
	store.release(retained).unwrap();
	assert!(store.payload_bytes() >= 18 * core::mem::size_of::<Value>() + 4);
	store.release(list).unwrap();
	assert_eq!(store.payload_bytes(), 0);
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn refused_string_growth_preserves_content_and_charge() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4,
	})
	.unwrap();
	let text = store.string("abc").unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store.append_string(text, text).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(store.text(text).unwrap(), "abc");
	assert_eq!(store.payload_bytes(), before);
	store.release(text).unwrap();
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn refused_vector_growth_does_not_retain_new_edge_or_publish_it() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: core::mem::size_of::<Value>() + 4,
	})
	.unwrap();
	let child = store.string("held").unwrap();
	let list = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store.push(list, child).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(store.len(list).unwrap(), 1);
	assert_eq!(store.payload_bytes(), before);
	store.release(child).unwrap();
	assert_eq!(store.live_entries(), 1);
	store.release(list).unwrap();
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn vector_growth_retain_failure_leaves_old_buffer_and_charge() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let stale = store.string("gone").unwrap();
	store.release(stale).unwrap();
	let list = store.list(Type::Vec, &[Value::I64(7)]).unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store.push(list, stale).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.payload_bytes(), before);
	assert_eq!(store.len(list).unwrap(), 1);
	assert_eq!(store.get(list, 0).unwrap(), Value::I64(7));
	store.release(list).unwrap();
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn refused_list_admission_undoes_duplicate_retained_prefix_and_metadata_reservation() {
	let mut store = Store::new(Limits {
		entries: 2,
		bytes: 4096,
	})
	.unwrap();
	// A retired handle is the in-core invalid edge; a foreign store's handle is
	// indistinguishable bits inside the core and is refused at the root boundary.
	let invalid = store.string("gone").unwrap();
	store.release(invalid).unwrap();
	let child = store.string("kept").unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store
			.list(Type::Vec, &[child, child, invalid])
			.unwrap_err()
			.kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 1);
	assert_eq!(store.payload_bytes(), before);
	assert_eq!(store.text(child).unwrap(), "kept");
	store.release(child).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
	// The unpublished metadata reservation did not consume a usable slot.
	let a = store.string("a").unwrap();
	let b = store.string("b").unwrap();
	store.release(a).unwrap();
	store.release(b).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn refused_record_admission_undoes_only_its_retained_prefix() {
	let mut store = Store::new(Limits {
		entries: 3,
		bytes: 4096,
	})
	.unwrap();
	let stale = store.string("gone").unwrap();
	store.release(stale).unwrap();
	let child = store.string("kept").unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store
			.record(Type::Object, &[("a", child), ("b", child), ("c", stale)])
			.unwrap_err()
			.kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 1);
	assert_eq!(store.payload_bytes(), before);
	store.release(child).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn entry_capacity_refusals_acquire_neither_children_nor_iterator_lease() {
	let mut store = Store::new(Limits {
		entries: 1,
		bytes: 4096,
	})
	.unwrap();
	let root = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let before = store.payload_bytes();
	assert_eq!(
		store.list(Type::Vec, &[root]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		store.record(Type::Object, &[("a", root)]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		store
			.variant(Type::Option, true, Some(root))
			.unwrap_err()
			.kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		store.iterator(root).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(store.payload_bytes(), before);
	assert_eq!(store.live_entries(), 1);
	store.push(root, Value::I64(2)).unwrap();
	store.release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn byte_capacity_refusals_acquire_no_child_or_iterator_lease() {
	let bytes = core::mem::size_of::<Value>();
	let mut store = Store::new(Limits { entries: 8, bytes }).unwrap();
	let root = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
	assert_eq!(
		store.list(Type::Vec, &[root]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		store
			.variant(Type::Option, true, Some(root))
			.unwrap_err()
			.kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		store.iterator(root).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	store.invalidate(root).unwrap(); // No refused iterator left a shared lease.
	store.release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn admitted_variant_and_iterator_own_exactly_one_root_and_lease() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let root = store.list(Type::Vec, &[Value::I64(7)]).unwrap();
	let iterator = store.iterator(root).unwrap();
	assert_eq!(
		store.push(root, Value::I64(8)).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(store.iterator_next(iterator).unwrap(), Some(Value::I64(7)));
	store.release(iterator).unwrap();
	store.push(root, Value::I64(8)).unwrap();
	let variant = store.variant(Type::Option, true, Some(root)).unwrap();
	store.release(root).unwrap();
	assert_eq!(store.live_entries(), 2);
	store.release(variant).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn release_drains_valid_siblings_after_an_invalid_edge_and_allows_reuse() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let good = store.string("good").unwrap();
	let bad = store.string("bad").unwrap();
	let parent = store.list(Type::Vec, &[good, bad]).unwrap();
	store.release(good).unwrap();
	store.release(bad).unwrap();
	// Deliberately misuse the raw host API to leave a stale edge in parent.
	// It is queued last, hence visited first by the LIFO release work list.
	store.release(bad).unwrap();
	assert_eq!(
		store.release(parent).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
	let fresh = store.string("fresh").unwrap();
	store.release(fresh).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn invalidation_drains_valid_siblings_after_an_invalid_edge() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let good = store.string("good").unwrap();
	let bad = store.string("bad").unwrap();
	let parent = store
		.record(Type::Object, &[("good", good), ("bad", bad)])
		.unwrap();
	store.release(good).unwrap();
	store.release(bad).unwrap();
	store.release(bad).unwrap();
	assert_eq!(
		store.invalidate(parent).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 1); // The invalidated parent root is still owned.
	assert_eq!(store.payload_bytes(), 0);
	store.release(parent).unwrap();
	assert_eq!(store.live_entries(), 0);
	let fresh = store.string("fresh").unwrap();
	store.release(fresh).unwrap();
}

#[test]
fn a_rejected_last_release_does_not_consume_an_active_lease_root() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let root = store.list(Type::Vec, &[Value::I64(7)]).unwrap();
	let iterator = store.iterator(root).unwrap();
	store.release(root).unwrap();
	assert_eq!(
		store.release(root).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(store.iterator_next(iterator).unwrap(), Some(Value::I64(7)));
	store.release(iterator).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn drainage_reports_the_first_error_and_still_releases_other_edges() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let leased = store.list(Type::Vec, &[Value::I64(7)]).unwrap();
	let iterator = store.iterator(leased).unwrap();
	let stale = store.string("bad").unwrap();
	let parent = store.list(Type::Vec, &[leased, stale]).unwrap();
	// Deliberate raw-API misuse: stale is visited first (InvalidHandle), then
	// leased's last release is blocked (AccessConflict). Preserve the former.
	store.release(stale).unwrap();
	store.release(stale).unwrap();
	store.release(leased).unwrap();
	store.release(leased).unwrap();
	assert_eq!(
		store.release(parent).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 2);
	assert_eq!(store.iterator_next(iterator).unwrap(), Some(Value::I64(7)));
	store.release(iterator).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
	let fresh = store.string("fresh").unwrap();
	store.release(fresh).unwrap();
}
