//! D9 tests-first: private retain's existing behavior, with no new host API.

use super::*;

fn state(store: &Store) -> std::string::String {
	std::format!(
		"{:?}",
		(
			store.closed,
			store.live,
			store.bytes,
			&store.slots,
			&store.free,
			&store.work,
			store.first,
			store.last
		)
	)
}

fn scalar_matrix(closed: bool) {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	if closed {
		store.close().unwrap();
	}

	let before = state(&store);
	let values = [
		Value::Unit,
		Value::Bool(false),
		Value::Bool(true),
		Value::I64(i64::MIN),
		Value::I64(i64::MAX),
		Value::U64(u64::MAX),
		Value::F64(0.0),
		Value::F64(-0.0),
		Value::F64(f64::INFINITY),
		Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
		Value::Char('μ'),
		Value::Function { code: 42, id: 7 },
	];

	for value in values {
		// The success payload is deliberately unused: D11 returns unit.
		store.retain(value).unwrap();
		assert_eq!(state(&store), before);
	}

	let types = [
		Type::Unit,
		Type::Bool,
		Type::I64,
		Type::U64,
		Type::F64,
		Type::Char,
		Type::String,
		Type::Vec,
		Type::Tuple,
		Type::Object,
		Type::Struct(super::super::value::Nominal(42)),
		Type::Option,
		Type::Result,
		Type::Range,
		Type::Iterator,
	];
	for tag in types {
		let value = Value::Type(tag);

		// The success payload is deliberately unused: D11 returns unit.
		store.retain(value).unwrap();
		assert_eq!(state(&store), before);
	}
}

#[test]
fn d9_all_non_heap_variants_retain_bits_on_open_store() {
	scalar_matrix(false);
}

#[test]
fn d9_all_non_heap_variants_retain_bits_on_closed_store() {
	scalar_matrix(true);
}

#[test]
fn d9_live_retain_counts_and_alias_survives_original_release() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let original = store.string("kept").unwrap();
	let Value::Heap(h) = original else {
		panic!("heap expected")
	};

	assert_eq!(store.entry(h).unwrap().strong, 1);
	store.retain(original).unwrap();
	let alias = original;
	assert_eq!(store.entry(h).unwrap().strong, 2);
	assert_eq!((store.live_entries(), store.payload_bytes()), (1, 4));
	store.release(original).unwrap();
	assert_eq!(store.entry(h).unwrap().strong, 1);
	assert_eq!(store.text(alias).unwrap(), "kept");
	store.release(alias).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn d9_invalid_and_wrong_generation_retains_leave_store_unchanged() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let valid = store.string("valid").unwrap();
	let Value::Heap(h) = valid else {
		panic!("heap expected")
	};

	for bad in [
		Handle {
			slot: u32::MAX,
			generation: 1,
		},
		Handle {
			slot: h.slot,
			generation: h.generation + 1,
		},
	] {
		let before = state(&store);
		assert_eq!(
			store.retain(Value::Heap(bad)).unwrap_err().kind,
			ErrorKind::InvalidHandle
		);
		assert_eq!(state(&store), before);
	}

	store.release(valid).unwrap();
}

#[test]
fn d9_stale_retain_refused_before_and_after_slot_reuse() {
	let mut store = Store::new(Limits {
		entries: 1,
		bytes: 4096,
	})
	.unwrap();
	let stale = store.string("old").unwrap();
	store.release(stale).unwrap();
	let before = state(&store);
	assert_eq!(
		store.retain(stale).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(state(&store), before);
	let new = store.string("new").unwrap();
	let (Value::Heap(old_h), Value::Heap(new_h)) = (stale, new) else {
		panic!("heap expected")
	};

	assert_eq!(old_h.slot, new_h.slot);
	assert_ne!(old_h.generation, new_h.generation);
	let before = state(&store);
	assert_eq!(
		store.retain(stale).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(state(&store), before);
	assert_eq!(store.text(new).unwrap(), "new");
	store.release(new).unwrap();
}

#[test]
fn d9_closed_heap_retains_keep_closed_error_precedence_and_state() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let value = store.string("closed").unwrap();
	store.close().unwrap();

	for value in [
		value,
		Value::Heap(Handle {
			slot: u32::MAX,
			generation: 1,
		}),
	] {
		let before = state(&store);
		assert_eq!(
			store.retain(value).unwrap_err().kind,
			ErrorKind::ClosedStore
		);
		assert_eq!(state(&store), before);
	}
}

#[test]
fn d9_checked_strong_overflow_is_nonmutating_and_recovery_counts() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let value = store.string("count").unwrap();
	let Value::Heap(h) = value else {
		panic!("heap expected")
	};

	// Module-local synthetic count: no production hook or visibility change.
	store.entry_mut(h).unwrap().strong = u32::MAX;
	let before = state(&store);
	assert_eq!(store.retain(value).unwrap_err().kind, ErrorKind::Overflow);
	assert_eq!(state(&store), before);
	store.entry_mut(h).unwrap().strong = u32::MAX - 1;
	store.retain(value).unwrap();
	let alias = value;
	assert_eq!(store.entry(h).unwrap().strong, u32::MAX);
	store.release(alias).unwrap();
	assert_eq!(store.entry(h).unwrap().strong, u32::MAX - 1);

	// Restore synthetic outstanding references, then release the real edge.
	store.entry_mut(h).unwrap().strong = 1;
	store.release(value).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

// Fix 5 freezes full errors and the existing permissive Moved/readers behavior.
#[test]
fn fix5_invalid_and_free_retains_preserve_full_errors_and_state() {
	let mut store = Store::new(Limits {
		entries: 4,
		bytes: 4096,
	})
	.unwrap();
	let mut foreign = Store::new(Limits {
		entries: 4,
		bytes: 4096,
	})
	.unwrap();
	let value = store.string("local").unwrap();
	let other = foreign.string("foreign").unwrap();
	let Value::Heap(h) = value else {
		panic!("heap expected")
	};
	// Internal handles have no owner: ownership was checked at Root import.
	// Equal raw slot/generation coordinates refer to the LOCAL entry here.
	assert_eq!(value, other);
	let foreign_before = state(&foreign);
	store.retain(other).unwrap();
	assert_eq!(store.test_strong(value), 2);
	assert_eq!(state(&foreign), foreign_before);
	store.release(other).unwrap();
	assert_eq!(store.test_strong(value), 1);
	let free = store.string("free").unwrap();
	let Value::Heap(free_h) = free else {
		panic!("heap expected")
	};
	store.release(free).unwrap();
	// Both the old generation and the CURRENT generation of an empty slot
	// must be refused, even though the latter passes generation comparison.
	for bad in [
		free,
		Value::Heap(Handle {
			slot: free_h.slot,
			generation: store.slots[free_h.slot as usize].generation,
		}),
		Value::Heap(Handle {
			slot: h.slot,
			generation: h.generation.wrapping_add(1),
		}),
		Value::Heap(Handle {
			slot: u32::MAX,
			generation: 1,
		}),
	] {
		let before = state(&store);
		assert_eq!(store.retain(bad), Err(err(ErrorKind::InvalidHandle)));
		assert_eq!(state(&store), before);
	}
	store.close().unwrap();
	let before = state(&store);
	for bad in [value, other, free] {
		assert_eq!(store.retain(bad), Err(err(ErrorKind::ClosedStore)));
		assert_eq!(state(&store), before);
	}
	foreign.release(other).unwrap();
}

#[test]
fn fix5_strong_boundary_preserves_full_overflow_error_and_count() {
	let mut store = Store::new(Limits {
		entries: 4,
		bytes: 4096,
	})
	.unwrap();
	let value = store.string("count").unwrap();
	store.test_set_strong(value, u32::MAX - 1);
	store.retain(value).unwrap();
	assert_eq!(store.test_strong(value), u32::MAX);
	let before = state(&store);
	assert_eq!(store.retain(value), Err(err(ErrorKind::Overflow)));
	assert_eq!(state(&store), before);
	store.test_set_strong(value, 1);
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn fix5_moved_and_read_borrowed_entries_still_retain() {
	let mut store = Store::new(Limits {
		entries: 4,
		bytes: 4096,
	})
	.unwrap();
	let value = store.string("moved").unwrap();
	let Value::Heap(h) = value else {
		panic!("heap expected")
	};
	store.invalidate(value).unwrap();
	assert!(matches!(
		store.entry(h).unwrap().payload,
		Some(Payload::Moved(Type::String))
	));
	store.retain(value).unwrap();
	assert_eq!(store.test_strong(value), 2);
	store.release(value).unwrap();
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
	let value = store.string("borrowed").unwrap();
	let Value::Heap(h) = value else {
		panic!("heap expected")
	};
	store.entry_mut(h).unwrap().readers = 1;
	store.retain(value).unwrap();
	assert_eq!(store.test_strong(value), 2);
	assert_eq!(store.entry(h).unwrap().readers, 1);
	store.entry_mut(h).unwrap().readers = 0;
	store.release(value).unwrap();
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}
