use super::super::error::Detail;
use super::*;
fn parent(s: &mut Store) -> Value {
	s.list(Type::Vec, &[Value::I64(3), Value::I64(1), Value::I64(2)])
		.unwrap()
}
fn values(s: &Store, v: Value) -> Vec<Value> {
	let Value::Heap(h) = v else { panic!() };
	let Payload::List { values, .. } = s.payload(h).unwrap() else {
		panic!()
	};
	{
		let mut copy = Vec::new();
		for &value in values {
			copy.try_push(value).unwrap();
		}
		copy
	}
}
#[test]
fn exclusive_blocks_access_but_inert_aliases_and_children_survive() {
	let mut s = Store::new(Limits::default()).unwrap();
	let child = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let v = s.list(Type::Vec, &[child, child]).unwrap();
	let g = s.sort_begin(v).unwrap();
	s.retain(v).unwrap();
	s.release(v).unwrap();
	assert_eq!(s.len(v).unwrap_err().detail, Detail::ReadExclusive);
	assert_eq!(s.get(v, 0).unwrap_err().detail, Detail::ReadExclusive);
	assert_eq!(s.clone_value(v).unwrap_err().detail, Detail::ReadExclusive);
	assert_eq!(
		s.format_value(v, true).unwrap_err().detail,
		Detail::ReadExclusive
	);
	assert_eq!(s.iterator(v).unwrap_err().detail, Detail::ReadExclusive);
	assert_eq!(
		s.set(v, 0, Value::Unit).unwrap_err().detail,
		Detail::WriteExclusive
	);
	assert_eq!(
		s.push(v, Value::Unit).unwrap_err().detail,
		Detail::WriteExclusive
	);
	assert_eq!(s.sort_begin(v).unwrap_err().detail, Detail::WriteExclusive);
	assert_eq!(s.invalidate(v).unwrap_err().detail, Detail::TakeExclusive);
	s.push(child, Value::I64(2)).unwrap();
	let aliased = s.sort_value(g, 0).unwrap();
	assert_eq!(aliased, child);
	s.release(aliased).unwrap();
	assert_eq!(s.close().unwrap_err().kind, ErrorKind::AccessConflict);
	s.release(g).unwrap();
	assert_eq!(s.len(child).unwrap(), 2);
	s.release(v).unwrap();
	s.release(child).unwrap();
	assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
	// Abort must restore the child edges before releasing the last parent root.
	let child = s.string("held").unwrap();
	let v = s.list(Type::Vec, &[child, child]).unwrap();
	let g = s.sort_begin(v).unwrap();
	let actual: usize = s
		.slots
		.iter()
		.filter_map(|slot| slot.entry.as_ref())
		.filter_map(|entry| entry.payload.as_ref())
		.map(Store::payload_edges)
		.sum();
	assert_eq!(s.edges, actual);
	s.release(child).unwrap();
	s.release(v).unwrap();
	s.release(g).unwrap();
	assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
}
#[test]
fn commit_and_abort_have_exact_owner_balance() {
	let mut s = Store::new(Limits::default()).unwrap();
	let v = parent(&mut s);
	let before = (s.live, s.bytes, s.edges, s.test_strong(v));
	let g = s.sort_begin(v).unwrap();
	s.sort_store(g, 1, 0, 1).unwrap();
	s.sort_store(g, 1, 1, 2).unwrap();
	s.sort_store(g, 1, 2, 0).unwrap();
	s.sort_commit(g, 1).unwrap();
	s.release(g).unwrap();
	assert_eq!(
		values(&s, v).as_slice(),
		&[Value::I64(1), Value::I64(2), Value::I64(3)]
	);
	assert_eq!((s.live, s.bytes, s.edges, s.test_strong(v)), before);
	let g = s.sort_begin(v).unwrap();
	s.sort_store(g, 1, 0, 2).unwrap();
	s.release(g).unwrap();
	assert_eq!(
		values(&s, v).as_slice(),
		&[Value::I64(1), Value::I64(2), Value::I64(3)]
	);
	assert_eq!((s.live, s.bytes, s.edges, s.test_strong(v)), before);
	s.release(v).unwrap();
	assert_eq!(s.live, 0);
}
#[test]
fn malformed_permutation_never_partly_commits() {
	let mut s = Store::new(Limits::default()).unwrap();
	let v = parent(&mut s);
	let g = s.sort_begin(v).unwrap();
	s.sort_store(g, 1, 0, 1).unwrap();
	assert_eq!(
		s.sort_commit(g, 1).unwrap_err().kind,
		ErrorKind::InvalidCode
	);
	assert_eq!(s.len(v).unwrap_err().detail, Detail::ReadExclusive);
	s.release(g).unwrap();
	assert_eq!(
		values(&s, v).as_slice(),
		&[Value::I64(3), Value::I64(1), Value::I64(2)]
	);
	s.release(v).unwrap();
	assert_eq!(s.live, 0);
}
#[test]
fn readers_saturation_and_budgets_never_publish_a_lease() {
	let mut s = Store::new(Limits::default()).unwrap();
	let v = parent(&mut s);
	s.set_readers_for_test(v, EXCLUSIVE_FOR_TEST - 1);
	assert_eq!(s.begin_iter(v).unwrap_err().kind, ErrorKind::Overflow);
	s.set_readers_for_test(v, 1);
	assert_eq!(s.sort_begin(v).unwrap_err().detail, Detail::WriteBorrowed);
	s.set_readers_for_test(v, 0);
	let bytes = s.bytes;
	s.limits.bytes = bytes;
	assert_eq!(s.sort_begin(v).unwrap_err().kind, ErrorKind::StoreCapacity);
	assert_eq!(s.len(v).unwrap(), 3);
	s.limits.bytes = DEFAULT_BYTES;
	let error = rune_alloc::limit::with(0, || s.sort_begin(v))
		.call()
		.unwrap_err();
	assert_eq!(error.kind, ErrorKind::Allocation);
	assert_eq!((s.live, s.bytes, s.edges), (1, bytes, 0));
	s.release(v).unwrap();
	assert_eq!(s.live, 0);
}
const EXCLUSIVE_FOR_TEST: u32 = u32::MAX;

#[test]
fn moved_struct_identity_is_unchanged_and_exclusive_is_narrow() {
	let mut s = Store::new(Limits::default()).unwrap();
	let v = s
		.record(
			Type::Struct(super::super::value::Nominal(0)),
			&[("x", Value::I64(1))],
		)
		.unwrap();
	let Value::Heap(h) = v else { panic!() };
	// Structs cannot be sort parents; their pre-C11 identity path stays exact.
	s.invalidate(v).unwrap();
	let before = format!("{:?}", s.entry(h).unwrap());
	let baseline = (s.live, s.bytes, s.edges, s.work.len());
	// The pre-C11 Struct identity shortcut does exactly retain(value); Ok(value).
	s.retain(v).unwrap();
	let old = v;
	let old_header = format!("{:?}", s.entry(h).unwrap());
	s.release(old).unwrap();
	let new = s.clone_value(v).unwrap();
	assert_eq!(new, old);
	assert_eq!(
		format!("{:?}", s.entry(h).unwrap()).as_bytes(),
		old_header.as_bytes()
	);
	s.release(new).unwrap();
	assert_eq!(
		format!("{:?}", s.entry(h).unwrap()).as_bytes(),
		before.as_bytes()
	);
	assert_eq!((s.live, s.bytes, s.edges, s.work.len()), baseline);
	let new = s.clone_value(v).unwrap();
	assert_eq!(new, v);
	s.release(new).unwrap();
	s.release(v).unwrap();
	assert_eq!((s.live, s.bytes, s.edges, s.work.len()), (0, 0, 0, 0));
}
