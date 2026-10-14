//! PRINT_RAW ownership and fail-closed controls, without process stdout under Miri.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function, PRINT_RAW},
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
fn finish(f: Function) -> Code {
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}
fn program(out: u32) -> Program {
	let mut f = Function::new(2, 1).unwrap();
	let (args_start, args_len) = f.arguments(&[0]).unwrap();
	f.push(
		Op::Call {
			out,
			function: PRINT_RAW,
			args_start,
			args_len,
		},
		loc(10),
	)
	.unwrap();
	f.push(Op::Return { value: out }, loc(11)).unwrap();
	Program::for_diagnostics_test(finish(f))
}
#[test]
fn raw_print_preserves_argument_and_returns_unit_even_when_output_aliases_it() {
	for out in [0, 1] {
		let p = program(out);
		let mut s = Store::new(Limits::default()).unwrap();
		let a = s.root_string("α\n\0").unwrap();
		let mut e = Executor::new().unwrap();
		e.test_output = Some(std::vec::Vec::new());
		for _ in 0..2 {
			let result = e.invoke(&p, &mut s, 0, &[&a], 100).unwrap();
			assert_eq!(
				s.root_scalar(&result).unwrap(),
				Some(super::super::value::Scalar::Unit)
			);
			assert_eq!(s.root_format(&a, false).unwrap(), "α\n\0");
			s.root_release(result).unwrap();
			assert!(e.is_idle());
		}
		assert_eq!(e.test_output.as_deref(), Some("α\n\0α\n\0".as_bytes()));
		s.root_release(a).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
}
#[test]
fn raw_print_type_and_moved_failures_preserve_location_and_allow_reuse() {
	let p = program(1);
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	e.test_output = Some(std::vec::Vec::new());
	let a = s.root_i64(7);
	let failure = e.invoke(&p, &mut s, 0, &[&a], 100).unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	assert_eq!(
		failure.error.detail,
		Detail::ExpectedType {
			expected: TypeName::String,
			found: TypeName::I64
		}
	);
	assert_eq!(failure.error.location, Some(loc(10)));
	assert!(failure.cleanup_error.is_none());
	assert!(e.is_idle());
	let a = s.root_string("moved").unwrap();
	s.root_invalidate(&a).unwrap();
	let failure = e.invoke(&p, &mut s, 0, &[&a], 100).unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
	assert_eq!(failure.error.location, Some(loc(10)));
	assert!(e.test_output.as_ref().unwrap().is_empty());
	s.root_release(a).unwrap();
	let a = s.root_string("ok").unwrap();
	let result = e.invoke(&p, &mut s, 0, &[&a], 100).unwrap();
	assert_eq!(e.test_output.as_deref(), Some(&b"ok"[..]));
	s.root_release(result).unwrap();
	s.root_release(a).unwrap();
	assert!(e.is_idle());
	assert_eq!(s.live_entries(), 0);
}
#[test]
fn certified_call_seal_checks_arity_range_argument_and_output() {
	for (out, start, len, arg, valid) in [
		(1, 0, 1, 0, true),
		(1, 0, 0, 0, false),
		(1, 0, 2, 0, false),
		(1, u32::MAX, 1, 0, false),
		(1, 0, 1, 99, false),
		(2, 0, 1, 0, false),
	] {
		let mut f = Function::new(2, 1).unwrap();
		f.arguments(&[arg]).unwrap();
		f.push(
			Op::Call {
				out,
				function: PRINT_RAW,
				args_start: start,
				args_len: len,
			},
			loc(0),
		)
		.unwrap();
		f.push(Op::Return { value: 0 }, loc(1)).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		assert_eq!(b.finish().is_ok(), valid);
	}
}
#[test]
fn direct_cold_helper_rejects_malformed_operands_before_output() {
	let p = program(1);
	let c = &p.code;
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	e.test_output = Some(std::vec::Vec::new());
	let v = s.string("x").unwrap();
	e.slots.try_push(v).unwrap();
	e.slots.try_push(Value::Unit).unwrap();
	let f = c.function(0).unwrap();
	for (base, out, start, len) in [
		(0, 1, 0, 0),
		(0, 1, 0, 2),
		(0, 1, u32::MAX, 1),
		(usize::MAX, 1, 0, 1),
		(0, u32::MAX, 0, 1),
	] {
		assert_eq!(
			e.intrinsic(c, &mut s, f, base, out, PRINT_RAW, start, len)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidCode
		);
		assert!(e.test_output.as_ref().unwrap().is_empty());
		assert_eq!(s.text(v).unwrap(), "x");
	}
	let mut malformed = Function::new(2, 1).unwrap();
	malformed.arguments(&[99]).unwrap();
	assert_eq!(
		e.intrinsic(c, &mut s, &malformed, 0, 1, PRINT_RAW, 0, 1)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
	assert!(e.test_output.as_ref().unwrap().is_empty());
	e.slots.clear();
	s.release(v).unwrap();
}
#[test]
fn old_output_is_released_after_the_write_with_no_success_allocation() {
	let p = program(1);
	let c = &p.code;
	let f = c.function(0).unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let input = s.string("input").unwrap();
	let old = s.string("old").unwrap();
	e.slots.try_push(input).unwrap();
	e.slots.try_push(old).unwrap();
	let mut captured = std::vec::Vec::with_capacity(16);
	captured.clear();
	e.test_output = Some(captured);
	rune_alloc::limit::with(0, || {
		e.intrinsic(c, &mut s, f, 0, 1, PRINT_RAW, 0, 1)
			.map_err(|e| e.kind)
	})
	.call()
	.unwrap();
	assert_eq!(e.test_output.as_deref(), Some(&b"input"[..]));
	assert_eq!(e.slots[1], Value::Unit);
	assert_eq!(s.live_entries(), 1);
	assert_eq!(s.text(input).unwrap(), "input");
	e.slots.clear();
	s.release(input).unwrap();
}
#[test]
fn output_is_not_retried_when_unit_put_drains_a_bad_pending_release() {
	let p = program(1);
	let c = &p.code;
	let f = c.function(0).unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let stale = s.string("gone").unwrap();
	s.release(stale).unwrap();
	let input = s.string("once").unwrap();
	e.slots.try_push(input).unwrap();
	e.slots.try_push(Value::Unit).unwrap();
	e.test_output = Some(std::vec::Vec::new());
	s.push_work_for_test(stale);
	assert!(e.intrinsic(c, &mut s, f, 0, 1, PRINT_RAW, 0, 1).is_err());
	assert_eq!(e.test_output.as_deref(), Some(&b"once"[..]));
	assert_eq!(e.slots[1], Value::Unit);
	assert_eq!(s.queued_work_for_test(), 0);
	e.slots.clear();
	s.release(input).unwrap();
	assert_eq!(s.live_entries(), 0);
}
#[test]
fn nested_type_failure_keeps_original_intrinsic_and_ordinary_call_sites() {
	let mut main = Function::new(2, 0).unwrap();
	main.push(
		Op::Literal {
			out: 0,
			value: Value::I64(7),
		},
		loc(1),
	)
	.unwrap();
	let (args_start, args_len) = main.arguments(&[0]).unwrap();
	main.push(
		Op::Call {
			out: 1,
			function: 1,
			args_start,
			args_len,
		},
		loc(2),
	)
	.unwrap();
	main.push(Op::Return { value: 1 }, loc(3)).unwrap();
	let mut child = Function::new(2, 1).unwrap();
	let (args_start, args_len) = child.arguments(&[0]).unwrap();
	child
		.push(
			Op::Call {
				out: 1,
				function: PRINT_RAW,
				args_start,
				args_len,
			},
			loc(10),
		)
		.unwrap();
	child.push(Op::Return { value: 1 }, loc(11)).unwrap();
	let mut b = Builder::default();
	b.push(main).unwrap();
	b.push(child).unwrap();
	let c = b.finish().unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	e.test_output = Some(std::vec::Vec::new());
	let failure = e
		.invoke_args_for_test(&c, &mut s, 0, Args::Values(&[]), 100)
		.unwrap_err();
	assert_eq!(failure.error.location, Some(loc(10)));
	assert_eq!(failure.call_chain.as_slice(), &[loc(2)]);
	assert!(failure.cleanup_error.is_none());
	assert!(e.is_idle());
	assert!(e.test_output.as_ref().unwrap().is_empty());
}
