//! Local renderer construction checks, not the complete reference addendum gate.

use crate::experimental_core::{
	error::ErrorKind,
	heap::{Limits, Store},
	value::{Type, Value},
};

fn store() -> Store {
	Store::new(Limits {
		entries: 8192,
		bytes: 1 << 20,
	})
	.unwrap()
}

#[test]
fn scalar_reference_shapes_and_container_insertion_order() {
	let mut store = store();
	for (n, text) in [
		(0.0, "0.0"),
		(-0.0, "-0.0"),
		(1.0, "1.0"),
		(1.5, "1.5"),
		(-0.5, "-0.5"),
		(f64::from_bits(1), "5e-324"),
		(f64::MAX, "1.7976931348623157e308"),
		(f64::INFINITY, "inf"),
		(f64::NEG_INFINITY, "-inf"),
		(f64::NAN, "NaN"),
		(9223372036854775808.0, "9.223372036854776e18"),
		(18446744073709551616.0, "1.8446744073709552e19"),
	] {
		for debug in [false, true] {
			assert_eq!(store.format_value(Value::F64(n), debug).unwrap(), text);
		}
	}

	assert_eq!(
		store.format_value(Value::Char('\0'), true).unwrap(),
		"'\\0'"
	);
	assert_eq!(store.format_value(Value::Char('\0'), false).unwrap(), "\0");
	let string = store.string("é\n").unwrap();
	assert_eq!(store.format_value(string, true).unwrap(), "\"é\\n\"");
	let tuple = store.list(Type::Tuple, &[Value::F64(0.0)]).unwrap();
	assert_eq!(store.format_value(tuple, true).unwrap(), "(0.0)");
	let object = store
		.record(
			Type::Object,
			&[("y", Value::I64(1)), ("x", Value::F64(0.0))],
		)
		.unwrap();
	assert_eq!(
		store.format_value(object, true).unwrap(),
		"{\"y\": 1, \"x\": 0.0}"
	);
	let some = store.variant(Type::Option, true, Some(string)).unwrap();
	assert_eq!(store.format_value(some, true).unwrap(), "Some(\"é\\n\")");
	assert_eq!(store.format_value(Value::Unit, true).unwrap(), "()");
	let vector = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
	let none = store.variant(Type::Option, false, None).unwrap();
	let result = store
		.variant(Type::Result, false, Some(Value::I64(1)))
		.unwrap();
	for v in [Value::Unit, vector, tuple, object, some, none, result] {
		assert_eq!(
			store.format_value(v, false).unwrap_err().kind,
			ErrorKind::Type
		);
	}

	let tag = Type::Struct(crate::experimental_core::nominal::intern(42, "Point").unwrap());
	let nominal = store.record(tag, &[("x", Value::I64(1))]).unwrap();
	assert_eq!(
		store.format_value(nominal, true).unwrap_err().kind,
		ErrorKind::Unsupported
	);
}

#[test]
fn caps_cycles_invalid_handles_and_closed_store_refuse() {
	let mut store = store();
	let text = store.string(&"a".repeat(65536)).unwrap();
	assert_eq!(store.format_value(text, false).unwrap().len(), 65536);
	assert_eq!(
		store.format_value(text, true).unwrap_err().kind,
		ErrorKind::FormatLimit
	);
	let wide = store.list(Type::Vec, &vec![Value::Unit; 4096]).unwrap();
	assert_eq!(
		store.format_value(wide, true).unwrap_err().kind,
		ErrorKind::FormatLimit
	);
	let cycle = store.list(Type::Vec, &[Value::Unit]).unwrap();
	store.set(cycle, 0, cycle).unwrap();
	assert_eq!(
		store.format_value(cycle, true).unwrap_err().kind,
		ErrorKind::FormatLimit
	);
	store.retain(text).unwrap();
	let alias = text;
	store.invalidate(text).unwrap();
	assert!(store.format_value(alias, false).is_err());
	store.close().unwrap();
	assert_eq!(
		store.format_value(Value::I64(1), true).unwrap_err().kind,
		ErrorKind::ClosedStore
	);
}
