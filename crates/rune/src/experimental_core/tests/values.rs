//! Candidate-local development checks, not batch A–E conformance evidence.

use crate::experimental_core::{
	arithmetic::{binary, cast, unary, Binary, Unary},
	error::ErrorKind,
	heap::{Limits, Store},
	value::{Type, Value},
};

fn store() -> Store {
	Store::new(Limits {
		entries: 128,
		bytes: 32_768,
	})
	.unwrap()
}

#[test]
fn arithmetic_checked_edges() {
	assert_eq!(
		binary(Binary::Add, Value::I64(2), Value::I64(3)).unwrap(),
		Value::I64(5)
	);
	assert_eq!(
		binary(Binary::Add, Value::I64(i64::MAX), Value::I64(1))
			.unwrap_err()
			.kind,
		ErrorKind::Overflow
	);
	assert_eq!(
		binary(Binary::Mul, Value::U64(u64::MAX), Value::U64(2))
			.unwrap_err()
			.kind,
		ErrorKind::Overflow
	);
	assert_eq!(
		unary(Unary::Neg, Value::I64(i64::MIN)).unwrap_err().kind,
		ErrorKind::Overflow
	);
	assert_eq!(
		binary(Binary::Div, Value::I64(7), Value::I64(0))
			.unwrap_err()
			.kind,
		ErrorKind::DivisionByZero
	);
	assert_eq!(
		binary(Binary::Rem, Value::I64(i64::MIN), Value::I64(-1))
			.unwrap_err()
			.kind,
		ErrorKind::Overflow
	);
	assert_eq!(
		binary(Binary::Shl, Value::I64(1), Value::I64(-1))
			.unwrap_err()
			.kind,
		ErrorKind::InvalidShift
	);
	assert_eq!(
		binary(Binary::Shr, Value::U64(1), Value::U64(64))
			.unwrap_err()
			.kind,
		ErrorKind::InvalidShift
	);
	assert_eq!(
		binary(Binary::Eq, Value::I64(1), Value::U64(1)).unwrap(),
		Value::Bool(true)
	);
}

#[test]
fn floating_and_cast_boundaries() {
	assert_eq!(
		binary(Binary::Lt, Value::F64(f64::NAN), Value::F64(1.)).unwrap(),
		Value::Bool(false)
	);
	assert_eq!(
		binary(Binary::Ne, Value::F64(f64::NAN), Value::F64(f64::NAN)).unwrap(),
		Value::Bool(true)
	);
	assert_eq!(
		cast(Value::F64(9223372036854775808.), Type::I64).unwrap(),
		Value::I64(i64::MAX)
	);
	assert_eq!(
		cast(Value::F64(f64::INFINITY), Type::I64).unwrap(),
		Value::I64(i64::MAX)
	);
	assert_eq!(
		cast(Value::I64(-1), Type::U64).unwrap(),
		Value::U64(u64::MAX)
	);
	assert_eq!(
		cast(Value::U64(0xD800), Type::Char).unwrap_err().kind,
		ErrorKind::InvalidCast
	);
	assert_eq!(cast(Value::F64(3.75), Type::I64).unwrap(), Value::I64(3));
}

#[test]
fn alias_writes_and_nested_outer_copy() {
	let mut s = store();
	let row = s.list(Type::Vec, &[Value::I64(2)]).unwrap();
	let outer = s.list(Type::Vec, &[row]).unwrap();
	let copy = s.list(Type::Vec, &[row]).unwrap();
	s.set(row, 0, Value::I64(9)).unwrap();
	let read = s.get(copy, 0).unwrap();
	assert_eq!(s.get(read, 0).unwrap(), Value::I64(9));
	s.release(read).unwrap();
	s.release(row).unwrap();
	s.release(outer).unwrap();
	s.release(copy).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn invalidation_affects_all_aliases() {
	let mut s = store();
	let row = s.list(Type::Vec, &[Value::I64(2)]).unwrap();
	s.retain(row).unwrap();
	let alias = row;
	s.invalidate(row).unwrap();
	assert_eq!(s.len(alias).unwrap_err().kind, ErrorKind::AccessConflict);
	s.release(row).unwrap();
	s.release(alias).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn iterator_blocks_mutation_and_close() {
	let mut s = store();
	let row = s.list(Type::Vec, &[Value::I64(2)]).unwrap();
	let mut iter = s.begin_iter(row).unwrap();
	assert_eq!(
		s.push(row, Value::I64(3)).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(
		s.invalidate(row).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(s.close().unwrap_err().kind, ErrorKind::AccessConflict);
	assert_eq!(s.next(&mut iter).unwrap(), Some(Value::I64(2)));
	assert_eq!(s.next(&mut iter).unwrap(), None);
	s.end_iter(iter).unwrap();
	s.push(row, Value::I64(3)).unwrap();
	s.release(row).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn cycles_are_charged_and_close_invalidates_roots() {
	let mut s = Store::new(Limits {
		entries: 4,
		bytes: 4096,
	})
	.unwrap();
	for _ in 0..4 {
		let v = s.list(Type::Vec, &[]).unwrap();
		s.push(v, v).unwrap();
		s.release(v).unwrap();
	}

	assert_eq!(s.live_entries(), 4);
	assert_eq!(
		s.list(Type::Vec, &[]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	s.close().unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
	assert_eq!(s.string("x").unwrap_err().kind, ErrorKind::ClosedStore);
}

#[test]
fn generation_and_store_identity_are_checked() {
	let mut s = store();
	let v = s.string("first").unwrap();
	s.release(v).unwrap();
	let second = s.string("second").unwrap();
	assert_eq!(s.text(v).unwrap_err().kind, ErrorKind::InvalidHandle);
	assert_eq!(
		store().len(second).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(s.text(second).unwrap(), "second");
	s.close().unwrap();
	assert_eq!(s.text(second).unwrap_err().kind, ErrorKind::ClosedStore);
}

#[test]
fn deep_release_and_byte_cap_preserve_existing_value() {
	let mut s = Store::new(Limits {
		entries: 5000,
		bytes: 1 << 20,
	})
	.unwrap();
	let mut root = s.string("leaf").unwrap();

	for _ in 0..4096 {
		let next = s.list(Type::Vec, &[root]).unwrap();
		s.release(root).unwrap();
		root = next;
	}

	s.release(root).unwrap();
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
	let mut s = Store::new(Limits {
		entries: 4,
		bytes: 4,
	})
	.unwrap();
	let v = s.string("four").unwrap();
	assert_eq!(s.string("x").unwrap_err().kind, ErrorKind::StoreCapacity);
	assert_eq!(s.text(v).unwrap(), "four");
	s.release(v).unwrap();
}

#[test]
fn shared_string_append_self_and_outer_clone() {
	let mut s = store();
	let a = s.string("abc").unwrap();
	s.retain(a).unwrap();
	let alias = a;
	s.append_string(a, a).unwrap();
	assert_eq!(s.text(alias).unwrap(), "abcabc");
	let cloned = s.clone_value(alias).unwrap();
	let suffix = s.string("!").unwrap();
	s.append_string(a, suffix).unwrap();
	assert_eq!(s.text(a).unwrap(), "abcabc!");
	assert_eq!(s.text(cloned).unwrap(), "abcabc");

	for v in [a, alias, cloned, suffix] {
		s.release(v).unwrap();
	}

	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
}

#[test]
fn keyed_record_missing_fields_caps_and_nested_cleanup() {
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 512,
	})
	.unwrap();
	let child = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let record = store
		.record(Type::Object, &[("nested", child), ("count", Value::I64(2))])
		.unwrap();
	store.release(child).unwrap();
	assert_eq!(
		store.field(record, "missing").unwrap_err().kind,
		ErrorKind::IndexOutOfBounds
	);
	// S-field's frozen stock contract inserts missing Object fields.
	store.set_field(record, "missing", Value::I64(3)).unwrap();
	assert_eq!(store.field(record, "missing").unwrap(), Value::I64(3));
	assert_eq!(
		store
			.record(Type::Object, &[("x", Value::Unit), ("x", Value::Unit)])
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
	let before = store.payload_bytes();
	assert_eq!(
		store
			.record(Type::Object, &[("x".repeat(512).as_str(), child)])
			.unwrap_err()
			.kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(store.payload_bytes(), before);
	store.release(record).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn amended_mixed_integer_and_numeric_cast_rules() {
	assert_eq!(
		binary(Binary::Add, Value::I64(2), Value::U64(3)).unwrap(),
		Value::I64(5)
	);
	assert_eq!(
		binary(Binary::Add, Value::U64(2), Value::I64(3)).unwrap(),
		Value::U64(5)
	);

	for (a, b) in [
		(Value::I64(1), Value::U64(u64::MAX)),
		(Value::U64(1), Value::I64(-1)),
	] {
		for op in [Binary::Add, Binary::BitAnd, Binary::Eq, Binary::Lt] {
			assert_eq!(
				binary(op, a, b).unwrap_err().kind,
				ErrorKind::IntegerCoercion
			);
		}
	}

	assert_eq!(
		binary(Binary::Shl, Value::I64(1), Value::U64(63)).unwrap(),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		binary(Binary::Shr, Value::U64(8), Value::I64(2)).unwrap(),
		Value::U64(2)
	);
	assert_eq!(
		binary(Binary::Shl, Value::U64(1), Value::I64(-1))
			.unwrap_err()
			.kind,
		ErrorKind::InvalidShift
	);
	assert_eq!(
		binary(Binary::Lt, Value::Bool(false), Value::Bool(true)).unwrap(),
		Value::Bool(true)
	);
	assert_eq!(
		binary(Binary::Le, Value::Unit, Value::Unit).unwrap(),
		Value::Bool(true)
	);
	assert_eq!(
		cast(Value::F64(f64::NAN), Type::U64).unwrap(),
		Value::U64(0)
	);
	assert_eq!(
		cast(Value::F64(f64::NEG_INFINITY), Type::I64).unwrap(),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		cast(Value::U64(u64::MAX), Type::I64).unwrap(),
		Value::I64(-1)
	);

	for (v, t) in [
		(Value::Bool(true), Type::Bool),
		(Value::Char('a'), Type::Char),
		(Value::Unit, Type::Unit),
	] {
		assert_eq!(cast(v, t).unwrap_err().kind, ErrorKind::InvalidCast);
	}
}
