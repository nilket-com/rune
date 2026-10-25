//! C17a Option and Result methods: ownership, texts and failure paths, independent
//! of the stock oracle.
use super::*;
use crate::experimental_core::error::Detail;
use crate::function::variant::Method;
use rune_alloc::limit;

fn run(
	s: &mut Store,
	m: Method,
	receiver: Value,
	message: Option<Value>,
) -> Result<VariantOutcome> {
	s.variant_method(m, receiver, message, &FormatLimits::default())
}

fn panic_text(outcome: Result<VariantOutcome>) -> std::string::String {
	match outcome {
		Ok(VariantOutcome::Panic(text)) => std::string::String::from(&*text),
		_ => panic!("expected a panic"),
	}
}

#[test]
fn unwrap_and_expect_return_the_payload_handle() {
	for m in [Method::Unwrap, Method::Expect] {
		for tag in [Type::Option, Type::Result] {
			let mut s = Store::new(Limits::default()).unwrap();
			let payload = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
			let variant = s.variant(tag, true, Some(payload)).unwrap();
			s.release(payload).unwrap();
			let message = (m == Method::Expect).then_some(Value::I64(5));
			let Ok(VariantOutcome::Value(value)) = run(&mut s, m, variant, message) else {
				panic!("expected the payload");
			};
			assert_eq!(value, payload);
			// The returned handle owns a reference of its own.
			s.release(variant).unwrap();
			assert_eq!(s.len(value).unwrap(), 1);
			s.release(value).unwrap();
			assert_eq!(s.live_entries(), 0);
		}
	}
}

#[test]
fn variant_tests_and_wrong_families() {
	let mut s = Store::new(Limits::default()).unwrap();
	let some = s.variant(Type::Option, true, Some(Value::I64(1))).unwrap();
	let none = s.variant(Type::Option, false, None).unwrap();
	let ok = s.variant(Type::Result, true, Some(Value::Unit)).unwrap();
	let err = s.variant(Type::Result, false, Some(Value::Unit)).unwrap();
	let flag = |s: &mut Store, m, v| match run(s, m, v, None) {
		Ok(VariantOutcome::Value(Value::Bool(b))) => b,
		_ => panic!("expected a flag"),
	};
	assert!(flag(&mut s, Method::IsSome, some) && !flag(&mut s, Method::IsSome, none));
	assert!(!flag(&mut s, Method::IsNone, some) && flag(&mut s, Method::IsNone, none));
	assert!(flag(&mut s, Method::IsOk, ok) && !flag(&mut s, Method::IsOk, err));
	assert!(!flag(&mut s, Method::IsErr, ok) && flag(&mut s, Method::IsErr, err));
	let vec = s.list(Type::Vec, &[]).unwrap();
	for (m, receiver) in [
		(Method::IsOk, some),
		(Method::IsErr, none),
		(Method::IsSome, ok),
		(Method::IsNone, err),
		(Method::Unwrap, vec),
		(Method::Unwrap, Value::I64(1)),
		(Method::IsSome, Value::Unit),
	] {
		let error = match run(&mut s, m, receiver, None) {
			Err(error) => error,
			Ok(_) => panic!("expected a missing method"),
		};
		assert_eq!(error.kind, ErrorKind::Type);
		let Detail::MissingInstanceFunction { hash, instance, .. } = error.detail else {
			panic!("expected a missing method detail");
		};
		assert_eq!(hash, crate::Hash::associated_function(instance, m.hash()));
	}
	for v in [some, none, ok, err, vec] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn panic_texts_match_stock() {
	let mut s = Store::new(Limits::default()).unwrap();
	let none = s.variant(Type::Option, false, None).unwrap();
	let text = s.string("boom").unwrap();
	let err = s.variant(Type::Result, false, Some(text)).unwrap();
	let message = s.string("ctx").unwrap();
	assert_eq!(
		panic_text(run(&mut s, Method::Unwrap, none, None)),
		"Called `Option::unwrap()` on a `None` value"
	);
	assert_eq!(
		panic_text(run(&mut s, Method::Unwrap, err, None)),
		"Called `Result::unwrap()` on an `Err` value: \"boom\""
	);
	assert_eq!(
		panic_text(run(&mut s, Method::Expect, none, Some(message))),
		"ctx"
	);
	assert_eq!(
		panic_text(run(&mut s, Method::Expect, err, Some(message))),
		"ctx: \"boom\""
	);
	assert_eq!(
		panic_text(run(&mut s, Method::Expect, err, Some(Value::I64(2)))),
		"2: \"boom\""
	);
	// The message is formatted with Display, so a list fails before any panic.
	let list = s.list(Type::Vec, &[]).unwrap();
	let error = match run(&mut s, Method::Expect, none, Some(list)) {
		Err(error) => error,
		Ok(_) => panic!("expected a display failure"),
	};
	assert!(matches!(error.detail, Detail::NoDisplay { .. }));
	for v in [none, text, err, message, list] {
		s.release(v).unwrap();
	}
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn allocation_failures_leave_the_store_unchanged() {
	let mut failures = 0;
	let mut successes = 0;
	for budget in 0..64 {
		let mut s = Store::new(Limits::default()).unwrap();
		let text = s.string("boom").unwrap();
		let err = s.variant(Type::Result, false, Some(text)).unwrap();
		let message = s.string("ctx").unwrap();
		let before = (s.live, s.bytes, s.edges, std::format!("{:?}", s.slots));
		match limit::with(budget, || run(&mut s, Method::Expect, err, Some(message))).call() {
			Ok(VariantOutcome::Panic(text)) => {
				successes += 1;
				assert_eq!(&*text, "ctx: \"boom\"");
			}
			Ok(VariantOutcome::Value(_)) => panic!("expected a panic"),
			Err(e) => {
				failures += 1;
				assert_eq!(e.kind, ErrorKind::Allocation);
			}
		}
		assert_eq!(
			(s.live, s.bytes, s.edges, std::format!("{:?}", s.slots)),
			before
		);
		for v in [text, err, message] {
			s.release(v).unwrap();
		}
		assert_eq!(s.live_entries(), 0);
	}
	assert!(failures > 0 && successes > 0);
}
