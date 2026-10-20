//! C17e adapter steps: `rev` offsets, `filter_map` results and
//! `protocol_into_iter`, independent of the stock oracle.
use super::*;
use crate::experimental_core::error::{Detail, TypeName};

/// The elements `rev` reads, back to front.
fn reversed(s: &mut Store, source: Value) -> Vec<Value> {
	let mut out = Vec::new();
	if s.rev_empty(source).unwrap() {
		assert_eq!(s.rev_last(source).unwrap(), 0);
		return out;
	}
	let mut at = s.rev_last(source).unwrap();
	loop {
		out.try_push(s.rev_at(source, Value::U64(at)).unwrap())
			.unwrap();
		if at == 0 {
			return out;
		}
		at -= 1;
	}
}

#[test]
fn rev_reads_each_source_from_the_back_without_advancing_it() {
	let mut s = Store::new(Limits::default()).unwrap();
	let list = s
		.list(Type::Vec, &[Value::I64(1), Value::I64(2), Value::I64(3)])
		.unwrap();
	let it = s.iterator(list).unwrap();
	assert_eq!(s.iterator_next(it).unwrap(), Some(Value::I64(1)));
	assert_eq!(reversed(&mut s, it), [Value::I64(3), Value::I64(2)]);
	assert_eq!(s.iterator_next(it).unwrap(), Some(Value::I64(2)));
	s.release(it).unwrap();
	s.release(list).unwrap();

	let it = s.range_iterator(Value::I64(-2), Value::I64(1)).unwrap();
	assert_eq!(
		reversed(&mut s, it),
		[Value::I64(0), Value::I64(-1), Value::I64(-2)]
	);
	s.release(it).unwrap();
	let it = s.range_iterator(Value::I64(3), Value::I64(1)).unwrap();
	assert!(reversed(&mut s, it).is_empty());
	s.release(it).unwrap();

	let range = s.range_values(Value::U64(1), Value::U64(3)).unwrap();
	let it = s.iterator(range).unwrap();
	assert_eq!(reversed(&mut s, it), [Value::U64(2), Value::U64(1)]);
	s.release(it).unwrap();
	s.release(range).unwrap();

	let none = s.variant(Type::Option, false, None).unwrap();
	let it = s.iterator(none).unwrap();
	assert!(reversed(&mut s, it).is_empty());
	s.release(it).unwrap();
	s.release(none).unwrap();
	let some = s.variant(Type::Option, true, Some(Value::I64(5))).unwrap();
	let it = s.iterator(some).unwrap();
	assert_eq!(reversed(&mut s, it), [Value::I64(5)]);
	s.release(it).unwrap();
	s.release(some).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn rev_skips_the_surrogates_and_spans_full_width_ranges() {
	let mut s = Store::new(Limits::default()).unwrap();
	let (a, b) = ('\u{d7fe}', '\u{e001}');
	let range = s.range_values(Value::Char(a), Value::Char(b)).unwrap();
	let it = s.iterator(range).unwrap();
	assert_eq!(
		reversed(&mut s, it),
		['\u{e000}', '\u{d7ff}', '\u{d7fe}'].map(Value::Char)
	);
	s.release(it).unwrap();
	s.release(range).unwrap();
	let inclusive = s
		.other_range(Type::RangeInclusive, &[Value::Char(a), Value::Char(b)])
		.unwrap();
	let it = s.iterator(inclusive).unwrap();
	assert_eq!(
		reversed(&mut s, it),
		['\u{e001}', '\u{e000}', '\u{d7ff}', '\u{d7fe}'].map(Value::Char)
	);
	s.release(it).unwrap();
	s.release(inclusive).unwrap();
	// `i64::MIN..=i64::MAX` holds 2^64 elements: the last offset still fits.
	let full = s
		.other_range(
			Type::RangeInclusive,
			&[Value::I64(i64::MIN), Value::I64(i64::MAX)],
		)
		.unwrap();
	let it = s.iterator(full).unwrap();
	assert!(!s.rev_empty(it).unwrap());
	assert_eq!(s.rev_last(it).unwrap(), u64::MAX);
	assert_eq!(
		s.rev_at(it, Value::U64(u64::MAX)).unwrap(),
		Value::I64(i64::MAX)
	);
	assert_eq!(s.rev_at(it, Value::U64(0)).unwrap(), Value::I64(i64::MIN));
	assert!(s.rev_at(it, Value::I64(0)).is_err());
	s.release(it).unwrap();
	s.release(full).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn rev_on_a_source_without_one_is_the_missing_method() {
	let mut s = Store::new(Limits::default()).unwrap();
	let from = s.other_range(Type::RangeFrom, &[Value::I64(1)]).unwrap();
	let it = s.iterator(from).unwrap();
	let e = s.rev_empty(it).unwrap_err();
	assert_eq!(e.kind, ErrorKind::Type);
	assert!(matches!(
		e.detail,
		Detail::MissingInstanceFunction {
			target: TypeName::RangeFromIter,
			..
		}
	));
	s.release(it).unwrap();
	s.release(from).unwrap();
	let object = s.record(Type::Object, &[]).unwrap();
	let it = s.iterator(object).unwrap();
	assert!(matches!(
		s.rev_last(it).unwrap_err().detail,
		Detail::MissingInstanceFunction {
			target: TypeName::ObjectIter,
			..
		}
	));
	s.release(it).unwrap();
	s.release(object).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn filter_map_results_and_into_iter_copy_stock_conversions() {
	let mut s = Store::new(Limits::default()).unwrap();
	let some = s.variant(Type::Option, true, Some(Value::I64(1))).unwrap();
	let none = s.variant(Type::Option, false, None).unwrap();
	let ok = s.variant(Type::Result, true, Some(Value::I64(1))).unwrap();
	assert!(s.option_some(some).unwrap());
	assert!(!s.option_some(none).unwrap());
	for (value, found) in [(ok, TypeName::Result), (Value::Bool(true), TypeName::Bool)] {
		let e = s.option_some(value).unwrap_err();
		assert_eq!(e.kind, ErrorKind::Type);
		assert_eq!(
			e.detail,
			Detail::ExpectedType {
				expected: TypeName::Option,
				found,
			}
		);
	}
	let it = s.protocol_iterator(some).unwrap();
	assert_eq!(s.iterator_next(it).unwrap(), Some(Value::I64(1)));
	s.release(it).unwrap();
	let it = s.protocol_iterator(Value::Unit).unwrap();
	assert_eq!(s.iterator_next(it).unwrap(), None);
	s.release(it).unwrap();
	let text = s.string("ab").unwrap();
	for (value, found) in [(Value::I64(5), TypeName::I64), (text, TypeName::String)] {
		let e = s.protocol_iterator(value).unwrap_err();
		assert_eq!(e.kind, ErrorKind::Type);
		assert_eq!(e.detail, Detail::MissingIntoIter { found });
	}
	for value in [some, none, ok, text] {
		s.release(value).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}
