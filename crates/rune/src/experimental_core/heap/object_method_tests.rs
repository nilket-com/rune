//! C17b Object methods: ownership, order, moves and failure paths, independent of
//! the stock oracle.
use super::*;
use crate::experimental_core::error::Detail;
use crate::function::object::Method;

fn object(s: &mut Store, pairs: &[(&str, Value)]) -> Value {
	s.record(Type::Object, pairs).unwrap()
}

fn keys(s: &Store, o: Value) -> std::vec::Vec<std::string::String> {
	s.object_fields(o)
		.unwrap()
		.iter()
		.map(|(k, _)| std::string::String::from(k.as_str()))
		.collect()
}

fn some_payload(s: &Store, v: Value) -> Option<Value> {
	let Value::Heap(h) = v else {
		panic!("expected an Option");
	};
	match s.payload(h).unwrap() {
		Payload::Variant {
			tag: Type::Option,
			payload,
			..
		} => *payload,
		_ => panic!("expected an Option"),
	}
}

#[test]
fn get_and_remove_return_the_stored_handle() {
	let mut s = Store::new(Limits::default()).unwrap();
	let stored = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let o = object(&mut s, &[("a", stored), ("b", Value::I64(2))]);
	s.release(stored).unwrap();
	let key = s.string("a").unwrap();
	let got = s.object_method(Method::Get, &[o, key]).unwrap();
	assert_eq!(some_payload(&s, got), Some(stored));
	let removed = s.object_method(Method::Remove, &[o, key]).unwrap();
	assert_eq!(some_payload(&s, removed), Some(stored));
	assert_eq!(keys(&s, o), ["b"]);
	let missing = s.object_method(Method::Remove, &[o, key]).unwrap();
	assert_eq!(some_payload(&s, missing), None);
	for v in [got, removed, missing, key, o] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn remove_shift_removes_and_insert_appends() {
	let mut s = Store::new(Limits::default()).unwrap();
	let o = object(
		&mut s,
		&[
			("a", Value::I64(1)),
			("b", Value::I64(2)),
			("c", Value::I64(3)),
		],
	);
	let b = s.string("b").unwrap();
	let r = s.object_method(Method::Remove, &[o, b]).unwrap();
	assert_eq!(keys(&s, o), ["a", "c"]);
	let a = s.string("a").unwrap();
	let replaced = s
		.object_method(Method::Insert, &[o, a, Value::I64(9)])
		.unwrap();
	assert_eq!(some_payload(&s, replaced), Some(Value::I64(1)));
	let b2 = s.string("b").unwrap();
	let new = s
		.object_method(Method::Insert, &[o, b2, Value::I64(4)])
		.unwrap();
	assert_eq!(some_payload(&s, new), None);
	assert_eq!(keys(&s, o), ["a", "c", "b"]);
	for v in [r, replaced, new, a, b, b2, o] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn insert_copies_the_key_and_keeps_its_handle() {
	let mut s = Store::new(Limits::default()).unwrap();
	let o = object(&mut s, &[]);
	let key = s.string("k").unwrap();
	let r = s.object_method(Method::Insert, &[o, key, key]).unwrap();
	// The stored value is the same handle, and the key handle stays readable.
	assert_eq!(s.object_fields(o).unwrap()[0].1, key);
	let got = s.object_method(Method::Get, &[o, key]).unwrap();
	assert_eq!(some_payload(&s, got), Some(key));
	let again = s
		.object_method(Method::Insert, &[o, key, Value::Unit])
		.unwrap();
	assert_eq!(some_payload(&s, again), Some(key));
	assert_eq!(keys(&s, o), ["k"]);
	for v in [r, got, again, key, o] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn argument_order_types_and_leases() {
	let mut s = Store::new(Limits::default()).unwrap();
	let o = object(&mut s, &[("a", Value::I64(1))]);
	let e = s
		.object_method(Method::Get, &[o, Value::I64(1)])
		.unwrap_err();
	assert!(matches!(
		e.detail,
		Detail::NumericArgumentType { arg: 1, .. }
	));
	let key = s.string("a").unwrap();
	let iter = s.iterator(o).unwrap();
	// Reads are allowed during iteration; writes report the receiver lease.
	let got = s.object_method(Method::ContainsKey, &[o, key]).unwrap();
	assert_eq!(got, Value::Bool(true));
	let e = s
		.object_method(Method::Insert, &[o, Value::I64(1), Value::Unit])
		.unwrap_err();
	assert_eq!(e.detail, Detail::SharedArgument { arg: 0, readers: 1 });
	let e = s.object_method(Method::Remove, &[o, key]).unwrap_err();
	assert_eq!(e.detail, Detail::SharedArgument { arg: 0, readers: 1 });
	s.release(iter).unwrap();
	// A key under a read lease is only read, so it can be inserted.
	let text = s.string("x y").unwrap();
	let space = s.string(" ").unwrap();
	let split = s
		.text_method(crate::function::text::Method::Split, &[text, space])
		.unwrap();
	let inserted = s
		.object_method(Method::Insert, &[o, text, Value::Unit])
		.unwrap();
	assert_eq!(keys(&s, o), ["a", "x y"]);
	for v in [inserted, split, text, space, key, o] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn is_empty_receivers_and_missing_methods() {
	let mut s = Store::new(Limits::default()).unwrap();
	let empty = object(&mut s, &[]);
	let text = s.string("t").unwrap();
	let vec = s.list(Type::Vec, &[]).unwrap();
	let tuple = s.list(Type::Tuple, &[Value::Unit]).unwrap();
	for (receiver, expected) in [
		(empty, true),
		(text, false),
		(vec, true),
		(tuple, false),
		(Value::Unit, true),
	] {
		assert_eq!(
			s.object_method(Method::IsEmpty, &[receiver]).unwrap(),
			Value::Bool(expected)
		);
	}
	for (m, args) in [
		(Method::IsEmpty, std::vec![Value::I64(1)]),
		(Method::ContainsKey, std::vec![vec, text]),
		(Method::ContainsKey, std::vec![Value::Unit, text]),
	] {
		let e = s.object_method(m, &args).unwrap_err();
		assert_eq!(e.kind, ErrorKind::Type);
		let Detail::MissingInstanceFunction { hash, instance, .. } = e.detail else {
			panic!("expected a missing method");
		};
		assert_eq!(hash, crate::Hash::associated_function(instance, m.hash()));
	}
	for v in [empty, text, vec, tuple] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn self_insert_keeps_the_cycle_charged() {
	let mut s = Store::new(Limits::default()).unwrap();
	let o = object(&mut s, &[]);
	let key = s.string("me").unwrap();
	let r = s.object_method(Method::Insert, &[o, key, o]).unwrap();
	assert_eq!(s.len(o).unwrap(), 1);
	for v in [r, key, o] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 1);
}
