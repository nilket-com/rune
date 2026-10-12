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
fn scalar_reference_shapes_and_sorted_object_debug() {
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
		"{\"x\": 0.0, \"y\": 1}"
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
	assert_eq!(store.format_value(nominal, true).unwrap(), "Point {x: 1}");
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

#[test]
fn sorted_objects_match_stock_debug_for_all_permutations_and_unicode_keys() {
	let keys = ["z", "y", "x", "w"];
	let mut checked = 0;
	for a in 0..4 {
		for b in 0..4 {
			for c in 0..4 {
				for d in 0..4 {
					let order = [a, b, c, d];
					if (0..4).any(|i| (i + 1..4).any(|j| order[i] == order[j])) {
						continue;
					}
					let mut store = store();
					let fields: std::vec::Vec<_> = order
						.iter()
						.map(|i| (keys[*i], Value::I64(*i as i64 + 1)))
						.collect();
					let object = store.record(Type::Object, &fields).unwrap();
					let mut stock = crate::runtime::Object::new();
					for i in order {
						stock
							.insert(
								crate::alloc::String::try_from(keys[i]).unwrap(),
								crate::runtime::Value::from(i as i64 + 1),
							)
							.unwrap();
					}
					assert_eq!(
						store.format_value(object, true).unwrap().as_str(),
						std::format!("{stock:?}").as_str()
					);
					checked += 1;
				}
			}
		}
	}
	assert_eq!(checked, 24);
	let mut store = store();
	let object = store
		.record(
			Type::Object,
			&[
				("é", Value::I64(3)),
				("a", Value::I64(1)),
				("B", Value::I64(2)),
			],
		)
		.unwrap();
	assert_eq!(
		store.format_value(object, true).unwrap(),
		"{\"B\": 2, \"a\": 1, \"é\": 3}"
	);
	let inner = store
		.record(Type::Object, &[("y", Value::I64(2)), ("x", Value::I64(1))])
		.unwrap();
	let list = store.list(Type::Vec, &[inner]).unwrap();
	let outer = store
		.record(Type::Object, &[("z", list), ("a", Value::I64(0))])
		.unwrap();
	assert_eq!(
		store.format_value(outer, true).unwrap(),
		"{\"a\": 0, \"z\": [{\"x\": 1, \"y\": 2}]}"
	);
	let empty = store.record(Type::Object, &[]).unwrap();
	assert_eq!(store.format_value(empty, true).unwrap(), "{}");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "4095/4096-field objects exceed Miri's budget; covered by the native suite"
)]
fn object_sorting_respects_the_existing_node_budget_without_changing_store_usage() {
	for count in [4095, 4096] {
		let mut store = store();
		let keys: std::vec::Vec<_> = (0..count).map(|i| std::format!("k{i:04}")).collect();
		let fields: std::vec::Vec<_> = keys.iter().map(|key| (key.as_str(), Value::Unit)).collect();
		let object = store.record(Type::Object, &fields).unwrap();
		let before = (store.live_entries(), store.payload_bytes());
		let result = store.format_value(object, true);
		if count == 4095 {
			assert!(result.is_ok(), "{result:?}");
		} else {
			assert_eq!(result.unwrap_err().kind, ErrorKind::FormatLimit);
		}
		assert_eq!((store.live_entries(), store.payload_bytes()), before);
	}
}
