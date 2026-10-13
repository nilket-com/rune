use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};

fn loc(n: u32) -> Location {
	Location {
		source: 0,
		revision: 1,
		start: n,
		end: n + 1,
	}
}
fn panic_program(nested: bool) -> Program {
	let mut b = Builder::default();
	let mut f = Function::new(2, 1).unwrap();
	if nested {
		let (args_start, args_len) = f.arguments(&[0]).unwrap();
		f.push(
			Op::Call {
				out: 1,
				function: 1,
				args_start,
				args_len,
			},
			loc(1),
		)
		.unwrap();
		f.push(Op::Return { value: 1 }, loc(2)).unwrap();
	} else {
		f.push(Op::Panic { value: 0 }, loc(3)).unwrap();
	}
	b.push(f).unwrap();
	if nested {
		let mut f = Function::new(1, 1).unwrap();
		f.push(Op::Panic { value: 0 }, loc(3)).unwrap();
		b.push(f).unwrap();
	}
	Program::for_diagnostics_test(b.finish().unwrap())
}

#[test]
fn full_panic_payload_survives_cleanup_store_drop_and_executor_reuse() {
	let p = panic_program(true);
	let text = std::format!("{}終", "α".repeat(1024));
	let mut s = Store::new(Limits::default()).unwrap();
	let v = s.root_string(&text).unwrap();
	let mut e = Executor::new().unwrap();
	let first = e.invoke(&p, &mut s, 0, &[&v], 100).unwrap_err();
	assert_eq!(first.error.kind, ErrorKind::Panic);
	assert_eq!(first.error.detail, Detail::PanicMessage);
	assert_eq!(first.error.location, Some(loc(3)));
	assert_eq!(first.call_chain.as_slice(), &[loc(1)]);
	assert!(first.cleanup_error.is_none());
	assert!(e.is_idle());
	for _ in 0..3 {
		let next = e.invoke(&p, &mut s, 0, &[&v], 100).unwrap_err();
		assert_eq!(next.panic_message.as_deref(), Some(text.as_str()));
		assert!(e.is_idle());
	}
	s.root_release(v).unwrap();
	assert_eq!(s.live_entries(), 0);
	drop(s);
	drop(e);
	assert_eq!(first.panic_message.as_deref(), Some(text.as_str()));
	assert_eq!(
		std::format!("{}", first.message(&p)),
		std::format!("Panicked: {text}")
	);
	assert!(size_of::<Error>() <= 96);
	assert!(size_of::<Detail>() <= 32);
}

#[test]
fn panic_copy_allocation_failure_is_primary_and_never_partial() {
	let p = panic_program(false);
	let mut s = Store::new(Limits::default()).unwrap();
	let v = s.root_string("complete required text").unwrap();
	let mut e = Executor::new().unwrap();
	let warm = e.invoke(&p, &mut s, 0, &[&v], 100).unwrap_err();
	assert_eq!(warm.error.kind, ErrorKind::Panic);
	drop(warm);
	// Failure transfers diagnostics to the caller; reserve its one root frame
	// outside the injected budget so only the message copy needs allocation.
	e.diagnostics.try_reserve(1).unwrap();
	let error = rune_alloc::limit::with(0, || {
		let fail = e.invoke(&p, &mut s, 0, &[&v], 100).unwrap_err();
		assert!(fail.panic_message.is_none());
		fail.error
	})
	.call();
	assert_eq!(error.kind, ErrorKind::Allocation);
	assert_eq!(error.location, Some(loc(3)));
	assert!(e.is_idle());
	let next = e.invoke(&p, &mut s, 0, &[&v], 100).unwrap_err();
	assert_eq!(
		next.panic_message.as_deref(),
		Some("complete required text")
	);
	s.root_release(v).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn invalid_message_is_checked_before_any_copy_allocation() {
	let mut s = Store::new(Limits::default()).unwrap();
	let text = s.string("x").unwrap();
	s.release(text).unwrap();
	let invalid =
		rune_alloc::limit::with(0, || copy_panic_message(&s, text).unwrap_err().kind).call();
	assert_eq!(invalid, ErrorKind::InvalidHandle);
	let wrong = rune_alloc::limit::with(0, || {
		copy_panic_message(&s, Value::I64(1)).unwrap_err().kind
	})
	.call();
	assert_eq!(wrong, ErrorKind::Type);
}

#[test]
fn string_copy_byte_budget_precedes_allocation_and_accepts_exact_length() {
	let mut exact = Store::new(Limits {
		entries: 4,
		bytes: 6,
	})
	.unwrap();
	let source = exact.string("abc").unwrap();
	let copy = exact.string_from(source).unwrap();
	assert_eq!(exact.text(copy).unwrap(), "abc");
	exact.release(source).unwrap();
	assert_eq!(exact.text(copy).unwrap(), "abc");
	exact.release(copy).unwrap();
	assert_eq!(exact.live_entries(), 0);
	let mut short = Store::new(Limits {
		entries: 4,
		bytes: 5,
	})
	.unwrap();
	let source = short.string("abc").unwrap();
	let error = rune_alloc::limit::with(0, || short.string_from(source).unwrap_err().kind).call();
	assert_eq!(error, ErrorKind::StoreCapacity);
	assert_eq!(short.text(source).unwrap(), "abc");
	short.release(source).unwrap();
	assert_eq!(short.live_entries(), 0);
}
