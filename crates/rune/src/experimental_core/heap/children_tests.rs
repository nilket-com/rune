//! Frozen child traversal and ownership contracts, established before optimization.
use super::*;
use rune_alloc::limit;

impl Payload {
	fn children_reference(&self) -> impl Iterator<Item = Value> + '_ {
		let mut index = 0;
		core::iter::from_fn(move || {
			let value = match self {
				Self::Moved(_) => None,
				Self::OtherRange(shape) => {
					[shape.start, shape.end].into_iter().flatten().nth(index)
				}
				Self::OtherRangeIterator(state) if index == 0 => state.source.map(Value::Heap),
				Self::Variant { payload, .. } | Self::OptionIterator { payload } if index == 0 => {
					*payload
				}
				Self::VectorIterator(iter) if index == 0 => Some(Value::Heap(iter.handle)),
				Self::SortGuard(state) if index == 0 => Some(Value::Heap(state.parent)),
				Self::Record { fields, .. } => fields.get(index).map(|(_, value)| *value),
				Self::List { values, .. } => values.get(index).copied(),
				Self::Closure { captures, .. } => captures.get(index).copied(),
				Self::ValueRange { start, .. } if index == 0 => Some(*start),
				Self::ValueRange { end, .. } if index == 1 => Some(*end),
				_ => None,
			};

			index += usize::from(value.is_some());
			value
		})
	}
}

fn values(input: &[Value]) -> Vec<Value> {
	let mut result = Vec::new();
	for value in input {
		result.try_push(*value).unwrap();
	}
	result
}

fn fields(input: &[Value]) -> Vec<(String, Value)> {
	let mut result = Vec::new();
	for (index, value) in input.iter().enumerate() {
		let key = if index == 0 { "first" } else { "next" };
		result
			.try_push((String::try_from(key).unwrap(), *value))
			.unwrap();
	}
	result
}

// Exhaustive on purpose: a new Payload variant must update this oracle matrix.
fn variant(payload: &Payload) -> &'static str {
	match payload {
		Payload::OtherRange(_) => "OtherRange",
		Payload::OtherRangeIterator(_) => "OtherRangeIterator",
		Payload::SortGuard(_) => "SortGuard",
		Payload::Moved(_) => "Moved",
		Payload::String(_) => "String",
		Payload::Variant { .. } => "Variant",
		Payload::Range { .. } => "Range",
		Payload::ValueRange { .. } => "ValueRange",
		Payload::VectorIterator(_) => "VectorIterator",
		Payload::OptionIterator { .. } => "OptionIterator",
		Payload::RangeIterator { .. } => "RangeIterator",
		Payload::ValueRangeIterator { .. } => "ValueRangeIterator",
		Payload::Record { .. } => "Record",
		Payload::List { .. } => "List",
		Payload::Closure { .. } => "Closure",
	}
}

#[test]
fn every_payload_preserves_reference_sequence_edges_and_exhaustion() {
	let h = Handle {
		slot: 17,
		generation: 42,
	};
	let heap = Value::Heap(h);
	let mut cases = std::vec![
		Payload::Moved(Type::Vec),
		Payload::String(String::try_from("leaf").unwrap()),
		Payload::Range { start: -1, end: 3 },
		Payload::RangeIterator { next: 1, end: 2 },
		Payload::VectorIterator(Iter {
			handle: h,
			index: 2
		}),
		// This synthetic variant witnesses that traversal remains empty;
		// actual ValueRangeIterator bounds are scalar snapshots, not edges.
		Payload::ValueRangeIterator {
			next: heap,
			end: heap
		},
	];
	for payload in [None, Some(Value::Unit), Some(heap)] {
		cases.push(Payload::OptionIterator { payload });
		for tag in [Type::Option, Type::Result] {
			cases.push(Payload::Variant {
				tag,
				success: true,
				payload,
			});
		}
	}
	for (start, end) in [
		(heap, heap),
		(heap, Value::I64(2)),
		(Value::I64(2), heap),
		(Value::U64(2), Value::I64(3)),
	] {
		cases.push(Payload::ValueRange { start, end });
	}
	for input in [
		&[][..],
		&[Value::Unit][..],
		&[heap, Value::I64(7), heap][..],
	] {
		for tag in [Type::Vec, Type::Tuple] {
			cases.push(Payload::List {
				tag,
				values: values(input),
			});
		}
		for tag in [
			Type::Object,
			Type::Struct(crate::experimental_core::value::Nominal(0)),
		] {
			cases.push(Payload::Record {
				tag,
				fields: fields(input),
			});
		}
	}
	cases.push(Payload::Closure {
		code: 1,
		function: 2,
		captures: values(&[heap, Value::I64(7), heap]),
	});
	for (tag, start, end) in [
		(Type::RangeFull, None, None),
		(Type::RangeFrom, Some(heap), None),
		(Type::RangeTo, None, Some(heap)),
		(Type::RangeInclusive, Some(heap), Some(heap)),
	] {
		cases.push(Payload::OtherRange(
			Box::try_new(ranges::RangeShape { tag, start, end }).unwrap(),
		));
	}
	cases.push(Payload::OtherRangeIterator(
		Box::try_new(boxed_iterator::State {
			source: None,
			kind: boxed_iterator::Kind::Range(ranges::RangeCursor {
				next: Value::I64(0),
				end: Some(Value::I64(1)),
				done: false,
			}),
		})
		.unwrap(),
	));
	for source in [None, Some(h)] {
		cases.push(Payload::OtherRangeIterator(
			Box::try_new(boxed_iterator::State {
				source,
				kind: boxed_iterator::Kind::Split(
					Box::try_new(text_methods::Split {
						pattern: text_methods::Pattern::Char(','),
						from: 0,
						to: 0,
					})
					.unwrap(),
				),
			})
			.unwrap(),
		));
	}
	let mut seen = std::collections::BTreeSet::new();
	let mut s = Store::new(Limits::default()).unwrap();
	for payload in &cases {
		seen.insert(variant(payload));
		let expected: std::vec::Vec<_> = payload.children_reference().collect();
		let edges = expected
			.iter()
			.filter(|v| matches!(v, Value::Heap(_)))
			.count();
		limit::with(0, || {
			let mut actual = payload.children();
			for value in &expected {
				assert_eq!(actual.next(), Some(*value), "{}", variant(payload));
			}
			for _ in 0..3 {
				assert_eq!(actual.next(), None, "{}", variant(payload));
			}
			assert_eq!(Store::payload_edges(payload), edges, "{}", variant(payload));
		})
		.call();
		// Count-only preflight: no fabricated handle is followed or retained.
		s.edges = edges;
		s.assert_work();
		assert!(s.children_room(payload).is_ok());
		if edges != 0 {
			s.edges = edges - 1;
			assert_eq!(
				s.children_room(payload).unwrap_err().kind,
				ErrorKind::InvalidCode
			);
			s.edges = edges;
			let work = core::mem::replace(&mut s.work, Vec::new());
			assert_eq!(
				s.children_room(payload).unwrap_err().kind,
				ErrorKind::InvalidCode
			);
			s.work = work;
		}
		s.edges = 0;
	}
	assert_eq!(seen.len(), 14);
	assert_eq!(
		(size_of::<Payload>(), size_of::<Entry>(), size_of::<Slot>()),
		(40, 72, 80)
	);
	std::println!(
		"CHILDREN cases={} iterator_bytes={}",
		cases.len(),
		core::mem::size_of_val(&cases[0].children())
	);
}

#[test]
fn captured_closure_cycle_stays_charged_until_store_close() {
	let mut store = Store::new(Limits::default()).unwrap();
	let vector = store.list(Type::Vec, &[]).unwrap();
	let env = store.list(Type::Tuple, &[vector]).unwrap();
	let closure = store.closure(1, 2, env).unwrap();
	assert_eq!(store.tag(closure).unwrap(), Type::Function);
	store.push(vector, closure).unwrap();

	store.release(env).unwrap();
	store.release(closure).unwrap();
	store.release(vector).unwrap();
	assert_eq!(store.live_entries(), 2);
	assert_eq!(store.edges, 2);

	store.close().unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.edges, 0);
}

#[test]
fn nested_duplicate_children_release_and_close_without_allocating() {
	for close in [false, true] {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.string("child").unwrap();
		let inner = s.list(Type::Tuple, &[child, child, Value::Unit]).unwrap();
		let option = s.variant(Type::Option, true, Some(inner)).unwrap();
		let outer = s
			.record(Type::Object, &[("a", option), ("b", inner), ("c", child)])
			.unwrap();
		s.release(child).unwrap();
		s.release(inner).unwrap();
		s.release(option).unwrap();
		assert_eq!((s.live, s.edges), (4, 6));
		s.assert_work();
		limit::with(0, || if close { s.close() } else { s.release(outer) })
			.call()
			.unwrap();
		assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
		s.assert_work();
	}
}
