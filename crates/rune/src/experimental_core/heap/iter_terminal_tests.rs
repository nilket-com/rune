//! C17d terminal steps: conversions, folds and collections, independent of
//! the stock oracle.
use super::*;
use crate::experimental_core::error::{Detail, HostCause, IntTarget, IntValue, TypeName};
use crate::function::iter_terminal::Number;

#[test]
fn adapter_counts_convert_as_a_native_usize() {
	let mut s = Store::new(Limits::default()).unwrap();
	assert_eq!(s.adapter_count(Value::I64(3)).unwrap(), 3);
	assert_eq!(s.adapter_count(Value::U64(u64::MAX)).unwrap(), usize::MAX);
	let e = s.adapter_count(Value::I64(-1)).unwrap_err();
	assert_eq!(e.kind, ErrorKind::IntegerCoercion);
	assert_eq!(
		e.detail,
		Detail::NumericArgumentCoerce {
			cause: HostCause::IntegerCoercion,
			arg: 1,
			value: IntValue::Signed(-1),
			to: IntTarget::Usize,
		}
	);
	let text = s.string("a").unwrap();
	let e = s.adapter_count(text).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::NumericArgumentNumber {
			cause: HostCause::ExpectedNumber,
			arg: 1,
			found: TypeName::String,
		}
	);
	s.release(text).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn range_bounds_convert_in_order() {
	let mut s = Store::new(Limits::default()).unwrap();
	let it = s.range_iterator(Value::I64(1), Value::U64(3)).unwrap();
	assert_eq!(s.iterator_next(it).unwrap(), Some(Value::I64(1)));
	assert_eq!(s.iterator_next(it).unwrap(), Some(Value::I64(2)));
	assert_eq!(s.iterator_next(it).unwrap(), None);
	s.release(it).unwrap();
	let e = s
		.range_iterator(Value::F64(0.0), Value::U64(u64::MAX))
		.unwrap_err();
	assert!(matches!(
		e.detail,
		Detail::NumericArgumentNumber { arg: 0, .. }
	));
	let e = s
		.range_iterator(Value::I64(0), Value::U64(u64::MAX))
		.unwrap_err();
	assert_eq!(
		e.detail,
		Detail::NumericArgumentCoerce {
			cause: HostCause::IntegerCoercion,
			arg: 1,
			value: IntValue::Unsigned(u64::MAX),
			to: IntTarget::I64,
		}
	);
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn folds_keep_the_seed_type_and_copy_stock_failures() {
	let s = Store::new(Limits::default()).unwrap();
	assert_eq!(
		s.number_element(Value::U64(7), Number::I64).unwrap(),
		Value::I64(7)
	);
	assert_eq!(
		s.number_element(Value::I64(7), Number::U64).unwrap(),
		Value::U64(7)
	);
	let e = s.number_element(Value::I64(-1), Number::U64).unwrap_err();
	assert_eq!(e.kind, ErrorKind::IntegerCoercion);
	assert_eq!(
		e.detail,
		Detail::Coerce {
			value: IntValue::Signed(-1),
			to: IntTarget::U64,
		}
	);
	let e = s.number_element(Value::I64(1), Number::F64).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::ExpectedType {
			expected: TypeName::F64,
			found: TypeName::I64,
		}
	);
	let e = s.number_element(Value::F64(1.0), Number::I64).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::ExpectedNumber {
			found: TypeName::F64,
		}
	);
	assert_eq!(
		s.number_fold(Value::I64(2), Value::U64(3), true).unwrap(),
		Value::I64(6)
	);
	let e = s
		.number_fold(Value::I64(i64::MAX), Value::I64(1), false)
		.unwrap_err();
	assert_eq!((e.kind, e.detail), (ErrorKind::Overflow, Detail::None));
	let e = s
		.number_fold(Value::U64(u64::MAX), Value::U64(2), true)
		.unwrap_err();
	assert_eq!(e.kind, ErrorKind::Overflow);
	let Value::F64(sum) = s
		.number_fold(Value::F64(-0.0), Value::F64(-0.0), false)
		.unwrap()
	else {
		panic!("f64 sum");
	};
	assert_eq!(sum.to_bits(), (-0.0f64).to_bits());
	assert_eq!(
		s.number_fold(Value::Bool(true), Value::I64(1), false)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
}

#[test]
fn object_collection_copies_keys_and_checks_pairs() {
	let mut s = Store::new(Limits::default()).unwrap();
	let object = s.record(Type::Object, &[]).unwrap();
	let key = s.string("k").unwrap();
	let value = s.list(Type::Vec, &[]).unwrap();
	let pair = s.list(Type::Tuple, &[key, value]).unwrap();
	s.collect_insert(object, pair).unwrap();
	// The key handle stays readable and the value is the same handle.
	assert_eq!(s.text(key).unwrap(), "k");
	assert_eq!(s.host_observe_fields(object).unwrap()[0].1, value);
	let triple = s.list(Type::Tuple, &[key, value, value]).unwrap();
	let e = s.collect_insert(object, triple).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::TupleLength {
			actual: 3,
			expected: 2,
		}
	);
	let e = s.collect_insert(object, Value::I64(1)).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::ExpectedType {
			expected: TypeName::Tuple,
			found: TypeName::I64,
		}
	);
	let bad = s.list(Type::Tuple, &[Value::I64(1), value]).unwrap();
	let e = s.collect_insert(object, bad).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::ExpectedType {
			expected: TypeName::String,
			found: TypeName::I64,
		}
	);
	for v in [bad, triple, pair, value, key, object] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn tuple_and_string_collections() {
	let mut s = Store::new(Limits::default()).unwrap();
	let element = s.list(Type::Vec, &[]).unwrap();
	let vec = s.list(Type::Vec, &[Value::I64(1), element]).unwrap();
	let tuple = s.collect_tuple(vec).unwrap();
	assert_eq!(
		s.host_observe_values(tuple).unwrap(),
		&[Value::I64(1), element]
	);
	let text = s.string("a").unwrap();
	let out = s.string("").unwrap();
	s.collect_append(out, text).unwrap();
	s.collect_append(out, Value::Char('b')).unwrap();
	assert_eq!(s.text(out).unwrap(), "ab");
	let e = s.collect_append(out, Value::I64(1)).unwrap_err();
	assert_eq!(
		e.detail,
		Detail::ExpectedType {
			expected: TypeName::String,
			found: TypeName::I64,
		}
	);
	assert_eq!(s.text(out).unwrap(), "ab");
	for v in [out, text, tuple, vec, element] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}
