//! C17c numeric providers: results, operand conversion and failure paths,
//! independent of the stock oracle.
use super::*;
use crate::experimental_core::error::{Detail, HostCause, IntTarget, IntValue, TypeName};
use crate::function::number::{Method, Pair, Receiver};

fn pair(m: Method, r: Receiver) -> Pair {
	Pair::new(m, r).unwrap()
}

/// The Option payload, releasing the Option.
fn option(s: &mut Store, v: Value) -> Option<Value> {
	let (tag, success, payload) = s.variant_parts(v).unwrap();
	assert_eq!(tag, Type::Option);
	assert_eq!(success, payload.is_some());
	s.release(v).unwrap();
	payload
}

/// The string text, releasing the string.
fn text(s: &mut Store, v: Value) -> std::string::String {
	let t = std::string::String::from(s.text(v).unwrap());
	s.release(v).unwrap();
	t
}

fn bits(v: Value) -> u64 {
	let Value::F64(x) = v else {
		panic!("expected f64, found {v:?}");
	};
	x.to_bits()
}

/// Exact bits. Miri chooses NaN bits and the sign of a min/max zero tie
/// nondeterministically, so under Miri those compare by class only.
fn same(v: Value, expected: f64) -> bool {
	let actual = f64::from_bits(bits(v));
	if cfg!(miri) && (expected.is_nan() || expected == 0.0) {
		return actual.is_nan() == expected.is_nan() && (actual == 0.0) == (expected == 0.0);
	}
	actual.to_bits() == expected.to_bits()
}

const EDGES: [f64; 8] = [
	0.0,
	-0.0,
	1.5,
	-2.5,
	f64::INFINITY,
	f64::NEG_INFINITY,
	f64::NAN,
	5e-324,
];

/// Correctly rounded operations: identical bits on every platform and under Miri.
#[test]
fn float_results_are_the_rust_bits() {
	let mut s = Store::new(Limits::default()).unwrap();
	for x in EDGES {
		for (m, expected) in [
			(Method::Sqrt, x.sqrt()),
			(Method::Floor, x.floor()),
			(Method::Ceil, x.ceil()),
		] {
			let v = s.number(pair(m, Receiver::F64), &[Value::F64(x)]).unwrap();
			assert!(same(v, expected), "{m:?} {x}");
		}
		for y in EDGES {
			for (m, expected) in [(Method::Min, x.min(y)), (Method::Max, x.max(y))] {
				let v = s
					.number(pair(m, Receiver::F64), &[Value::F64(x), Value::F64(y)])
					.unwrap();
				assert!(same(v, expected), "{m:?} {x} {y}");
			}
			let v = s
				.number(
					pair(Method::PartialCmp, Receiver::F64),
					&[Value::F64(x), Value::F64(y)],
				)
				.unwrap();
			assert_eq!(option(&mut s, v), x.partial_cmp(&y).map(Value::Ordering));
		}
	}
	assert_eq!(s.live_entries(), 0);
}

/// `exp`, `ln` and `powi` are the platform's; Miri perturbs them, so this
/// test is native only.
#[test]
fn transcendental_results_are_the_rust_bits() {
	let mut s = Store::new(Limits::default()).unwrap();
	for x in EDGES {
		for (m, expected) in [(Method::Exp, x.exp()), (Method::Ln, x.ln())] {
			let v = s.number(pair(m, Receiver::F64), &[Value::F64(x)]).unwrap();
			assert_eq!(bits(v), expected.to_bits(), "{m:?} {x}");
		}
		let v = s
			.number(
				pair(Method::Powi, Receiver::F64),
				&[Value::F64(x), Value::I64(-3)],
			)
			.unwrap();
		assert_eq!(bits(v), x.powi(-3).to_bits());
	}
}

#[test]
fn integer_results_match_rust() {
	let mut s = Store::new(Limits::default()).unwrap();
	let i = |m| pair(m, Receiver::I64);
	let u = |m| pair(m, Receiver::U64);
	let two = |s: &mut Store, p, a, b| s.number(p, &[a, b]).unwrap();
	assert_eq!(
		two(
			&mut s,
			i(Method::WrappingAdd),
			Value::I64(i64::MAX),
			Value::I64(1)
		),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		two(&mut s, u(Method::WrappingSub), Value::U64(0), Value::U64(1)),
		Value::U64(u64::MAX)
	);
	assert_eq!(
		two(
			&mut s,
			i(Method::SaturatingMul),
			Value::I64(-2),
			Value::I64(i64::MAX)
		),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		two(
			&mut s,
			i(Method::WrappingDiv),
			Value::I64(i64::MIN),
			Value::I64(-1)
		),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		two(
			&mut s,
			i(Method::WrappingRem),
			Value::I64(i64::MIN),
			Value::I64(-1)
		),
		Value::I64(0)
	);
	assert_eq!(
		two(
			&mut s,
			i(Method::SaturatingPow),
			Value::I64(-2),
			Value::I64(63)
		),
		Value::I64(i64::MIN)
	);
	assert_eq!(
		two(
			&mut s,
			u(Method::WrappingPow),
			Value::U64(3),
			Value::I64(41)
		),
		Value::U64(3u64.wrapping_pow(41))
	);
	// The argument converts to the receiver type, as stock FromValue does.
	assert_eq!(
		two(&mut s, u(Method::Max), Value::U64(5), Value::I64(3)),
		Value::U64(5)
	);
	assert_eq!(
		two(&mut s, i(Method::Min), Value::I64(5), Value::U64(3)),
		Value::I64(3)
	);
	for (p, a, b, expected) in [
		(
			i(Method::CheckedAdd),
			Value::I64(i64::MAX),
			Value::I64(1),
			None,
		),
		(
			i(Method::CheckedDiv),
			Value::I64(i64::MIN),
			Value::I64(-1),
			None,
		),
		(i(Method::CheckedRem), Value::I64(7), Value::I64(0), None),
		(
			i(Method::CheckedMul),
			Value::I64(6),
			Value::I64(7),
			Some(Value::I64(42)),
		),
		(u(Method::CheckedSub), Value::U64(0), Value::U64(1), None),
		(u(Method::CheckedPow), Value::U64(2), Value::I64(64), None),
		(
			i(Method::CheckedPow),
			Value::I64(-2),
			Value::I64(63),
			Some(Value::I64(i64::MIN)),
		),
	] {
		let v = s.number(p, &[a, b]).unwrap();
		assert_eq!(option(&mut s, v), expected, "{p:?}");
	}
	for (m, x, expected) in [
		(Method::WrappingAbs, i64::MIN, i64::MIN),
		(Method::SaturatingAbs, i64::MIN, i64::MAX),
		(Method::Signum, -7, -1),
	] {
		assert_eq!(
			s.number(i(m), &[Value::I64(x)]).unwrap(),
			Value::I64(expected)
		);
	}
	let v = s
		.number(i(Method::CheckedAbs), &[Value::I64(i64::MIN)])
		.unwrap();
	assert_eq!(option(&mut s, v), None);
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn text_and_char_results() {
	let mut s = Store::new(Limits::default()).unwrap();
	// The longest f64 Display texts, and both integer extremes.
	for x in [
		f64::MIN,
		-5e-324,
		-f64::MIN_POSITIVE / 4.0,
		-0.0,
		f64::NAN,
		1.0,
	] {
		let v = s
			.number(pair(Method::ToString, Receiver::F64), &[Value::F64(x)])
			.unwrap();
		assert_eq!(text(&mut s, v), std::format!("{x}"));
	}
	let v = s
		.number(
			pair(Method::ToString, Receiver::I64),
			&[Value::I64(i64::MIN)],
		)
		.unwrap();
	assert_eq!(text(&mut s, v), std::format!("{}", i64::MIN));
	let v = s
		.number(
			pair(Method::ToString, Receiver::U64),
			&[Value::U64(u64::MAX)],
		)
		.unwrap();
	assert_eq!(text(&mut s, v), std::format!("{}", u64::MAX));
	assert_eq!(
		s.number(
			pair(Method::ToI64, Receiver::Char),
			&[Value::Char('\u{10FFFF}')]
		)
		.unwrap(),
		Value::I64(0x10FFFF)
	);
	let from = pair(Method::FromI64, Receiver::Char);
	for (n, expected) in [
		(97, Some(Value::Char('a'))),
		(0xD800, None),
		(0x11_0000, None),
		(i64::from(u32::MAX), None),
	] {
		let v = s.number(from, &[Value::I64(n)]).unwrap();
		assert_eq!(option(&mut s, v), expected);
	}
	let e = s.number(from, &[Value::I64(-1)]).unwrap_err();
	assert_eq!((e.kind, e.detail), (ErrorKind::Overflow, Detail::Underflow));
	let e = s.number(from, &[Value::I64(1 << 32)]).unwrap_err();
	assert_eq!((e.kind, e.detail), (ErrorKind::Overflow, Detail::None));
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn failures_copy_stock_conversion_and_change_nothing() {
	let mut s = Store::new(Limits::default()).unwrap();
	let before = s.state_for_test();
	let fail = |s: &mut Store, p: Pair, args: &[Value]| {
		let e = s.number(p, args).unwrap_err();
		(e.kind, e.detail)
	};
	assert_eq!(
		fail(
			&mut s,
			pair(Method::Max, Receiver::F64),
			&[Value::F64(1.0), Value::I64(2)]
		),
		(
			ErrorKind::Type,
			Detail::NumericArgumentType {
				cause: HostCause::ExpectedType,
				arg: 1,
				expected: TypeName::F64,
				found: TypeName::I64,
			}
		)
	);
	assert_eq!(
		fail(
			&mut s,
			pair(Method::ToI64, Receiver::Char),
			&[Value::I64(97)]
		),
		(
			ErrorKind::Type,
			Detail::NumericArgumentType {
				cause: HostCause::ExpectedType,
				arg: 0,
				expected: TypeName::Char,
				found: TypeName::I64,
			}
		)
	);
	assert_eq!(
		fail(
			&mut s,
			pair(Method::Pow, Receiver::I64),
			&[Value::I64(2), Value::F64(2.0)]
		),
		(
			ErrorKind::Type,
			Detail::NumericArgumentNumber {
				cause: HostCause::ExpectedNumber,
				arg: 1,
				found: TypeName::F64,
			}
		)
	);
	for (p, b, value, to) in [
		(
			pair(Method::Pow, Receiver::I64),
			Value::I64(-1),
			IntValue::Signed(-1),
			IntTarget::U32,
		),
		(
			pair(Method::Pow, Receiver::I64),
			Value::I64(1 << 32),
			IntValue::Signed(1 << 32),
			IntTarget::U32,
		),
		(
			pair(Method::Powi, Receiver::F64),
			Value::U64(1 << 31),
			IntValue::Unsigned(1 << 31),
			IntTarget::I32,
		),
		(
			pair(Method::CheckedAdd, Receiver::I64),
			Value::U64(u64::MAX),
			IntValue::Unsigned(u64::MAX),
			IntTarget::I64,
		),
		(
			pair(Method::Max, Receiver::U64),
			Value::I64(-1),
			IntValue::Signed(-1),
			IntTarget::U64,
		),
	] {
		let a = match p.method() {
			Method::Powi => Value::F64(2.0),
			_ if p == pair(Method::Max, Receiver::U64) => Value::U64(5),
			_ => Value::I64(2),
		};
		assert_eq!(
			fail(&mut s, p, &[a, b]),
			(
				ErrorKind::IntegerCoercion,
				Detail::NumericArgumentCoerce {
					cause: HostCause::IntegerCoercion,
					arg: 1,
					value,
					to,
				}
			)
		);
	}
	assert_eq!(
		fail(
			&mut s,
			pair(Method::Pow, Receiver::I64),
			&[Value::I64(2), Value::I64(64)]
		),
		(ErrorKind::Overflow, Detail::None)
	);
	for m in [Method::WrappingDiv, Method::WrappingRem] {
		assert_eq!(
			fail(
				&mut s,
				pair(m, Receiver::U64),
				&[Value::U64(7), Value::U64(0)]
			),
			(ErrorKind::DivisionByZero, Detail::None)
		);
	}
	// A wrong operand count is invalid code, never a partial call.
	assert_eq!(
		fail(&mut s, pair(Method::Sqrt, Receiver::F64), &[]).0,
		ErrorKind::InvalidCode
	);
	assert_eq!(
		fail(&mut s, pair(Method::Max, Receiver::F64), &[Value::F64(1.0)]).0,
		ErrorKind::InvalidCode
	);
	assert_eq!(s.state_for_test(), before);
}
