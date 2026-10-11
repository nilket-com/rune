//! D4 candidate: selected_release uses the guarded slot helper.
use super::*;

// Heap work-at-capacity delegates unchanged; this matrix targets the scalar
// shortcut plus representative heap errors, not every release invariant.
fn selected_release(store: &mut Store, value: Value) -> Result<()> {
	// Only this selector changed from baseline; expectations stay frozen.
	store.release_slot(value)
}

fn make(kind: usize, store: &mut Store) -> Value {
	match kind {
		0 => Value::Unit,
		1 => Value::Bool(true),
		2 => Value::I64(-42),
		3 => Value::U64(u64::MAX),
		4 => Value::F64(-0.0),
		5 => Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
		6 => Value::Char('μ'),
		7 => Value::Type(Type::I64),
		8 => Value::Function { code: 1, id: 0 },
		9 => store.string("live").unwrap(),
		10 => {
			let v = store.string("stale").unwrap();
			store.release(v).unwrap();
			v
		}
		11 => {
			let v = store.string("retired").unwrap();
			let v = store.set_generation_for_test(v, u32::MAX);
			store.release(v).unwrap();
			v
		}
		_ => unreachable!(),
	}
}
fn state(store: &Store) -> std::string::String {
	// Excludes only the unique owner id; includes generations, refs, payloads,
	// links, work/free order and all accounting. No raw padding comparisons.
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
			store.last,
		)
	)
}
fn setup(kind: usize, closed: bool, queue: usize) -> (Store, Value) {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap();
	let value = make(kind, &mut store);
	if queue != 0 {
		let valid = store.string("independent queued edge").unwrap();
		// Transfer the one owned edge to work, do not retain a second root.
		store.work.try_push(valid).unwrap();
		if queue == 2 {
			// LIFO: malformed edge reports first, independent edge still drains.
			store
				.work
				.try_push(Value::Heap(Handle {
					slot: u32::MAX,
					generation: 1,
				}))
				.unwrap();
		}
	}
	if closed {
		// Artificial closed-with-work state: preserve closed-check precedence.
		store.closed = true;
	}
	(store, value)
}
#[test]
fn d4_release_equivalence_matrix_covers_scalar_and_heap_states() {
	let mut count = 0;
	for kind in 0..12 {
		for closed in [false, true] {
			for queue in 0..3 {
				let (mut a, av) = setup(kind, closed, queue);
				let (mut b, bv) = setup(kind, closed, queue);
				assert_eq!(state(&a), state(&b));
				let actual = selected_release(&mut a, av).map_err(|e| e.kind);
				let expected = b.release(bv).map_err(|e| e.kind);
				assert_eq!(
					actual, expected,
					"kind={kind} closed={closed} queue={queue}"
				);
				assert_eq!(state(&a), state(&b));
				if closed {
					assert_eq!(actual, Err(ErrorKind::ClosedStore));
				} else {
					assert!(a.work.is_empty());
					if queue == 2 || kind >= 10 {
						assert_eq!(actual, Err(ErrorKind::InvalidHandle));
					} else {
						assert_eq!(actual, Ok(()));
					}
				}
				count += 1;
			}
		}
	}
	assert_eq!(count, 72);
}
#[test]
fn d4_scalar_pending_release_retires_the_independent_edge() {
	for queue in [1, 2] {
		let (mut store, value) = setup(0, false, queue);
		let queued = store.work[0];
		let result = selected_release(&mut store, value).map_err(|e| e.kind);
		assert_eq!(
			result,
			if queue == 1 {
				Ok(())
			} else {
				Err(ErrorKind::InvalidHandle)
			}
		);
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		assert!(store.work.is_empty());
		assert_eq!(
			store.text(queued).unwrap_err().kind,
			ErrorKind::InvalidHandle
		);
		let next = store.string("reuse").unwrap();
		store.release(next).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}
