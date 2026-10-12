//! D18 tests-first: exact Template oracles that a Template-local slot read must
//! preserve. These pass on D14 (Template maps its parts through the shared `get`
//! closure) and are the oracle for D18. Output covers every mode flag and part
//! order; the failure cases put two distinguishable runtime formatting failures
//! in both orders, so the first failing part is identified by its exact error.

use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};

fn loc(n: u32) -> Location {
	Location {
		source: 18,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

fn store() -> Store {
	Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap()
}

fn code(f: Function) -> Code {
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}

/// Slots 0..=5 hold I64 42, Char 'x', Bool true, Unit, the string "ab" and a
/// function token; the template at pc 6 (location 60) formats `parts` into slot
/// 6, which is returned.
fn template(parts: &[u32]) -> Code {
	let mut f = Function::new(7, 0).unwrap();
	let text = f.string("ab").unwrap();

	for (out, value) in [
		(0, Value::I64(42)),
		(1, Value::Char('x')),
		(2, Value::Bool(true)),
		(3, Value::Unit),
	] {
		f.push(Op::Literal { out, value }, loc(out)).unwrap();
	}

	f.push(Op::String { out: 4, text }, loc(4)).unwrap();
	f.push(
		Op::Literal {
			out: 5,
			value: Value::Function { code: 0, id: 0 },
		},
		loc(5),
	)
	.unwrap();
	let (args_start, args_len) = f.arguments(parts).unwrap();
	f.push(
		Op::Template {
			out: 6,
			args_start,
			args_len,
		},
		loc(60),
	)
	.unwrap();
	f.push(Op::Return { value: 6 }, loc(61)).unwrap();
	code(f)
}

#[test]
fn d18_template_formats_every_mode_in_part_order() {
	// (slot, debug) pairs: display and debug forms, a repeated slot, the heap
	// string between scalars, and the same value in both modes.
	let code = template(&[0, 0, 1, 1, 2, 0, 3, 1, 4, 0, 1, 0, 0, 1]);
	let mut store = store();
	let mut executor = Executor::new().unwrap();
	let value = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[]), 1024)
		.unwrap();
	assert_eq!(store.text(value).unwrap(), "42'x'true()abx42");
	assert!(executor.is_idle());
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}

fn first_failure(parts: &[u32], expected: Error) {
	let code = template(parts);
	let mut store = store();
	let mut executor = Executor::new().unwrap();
	let failure = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[]), 1024)
		.unwrap_err();
	assert_eq!(failure.error, expected);
	assert!(failure.call_chain.is_empty());
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn d18_template_reports_the_first_failing_part_unit_display_first() {
	// The string part renders first, then Unit has no display form (Type) before
	// the function token (Unsupported) is reached.
	first_failure(
		&[4, 0, 3, 0, 5, 0],
		Error::new(ErrorKind::Type, "missing display form").at(loc(60)),
	);
}

#[test]
fn d18_template_reports_the_first_failing_part_function_first() {
	// Same two failures in the other order: the function token fails first.
	first_failure(
		&[4, 0, 5, 0, 3, 0],
		Error::new(ErrorKind::Unsupported, "value formatting form").at(loc(60)),
	);
}

#[test]
fn d18_template_executor_is_reusable_after_a_formatting_failure() {
	let failing = template(&[3, 0]);
	let ok = template(&[0, 0, 4, 0]);
	let mut store = store();
	let mut executor = Executor::new().unwrap();
	let failure = executor
		.invoke_args_for_test(&failing, &mut store, 0, Args::Values(&[]), 1024)
		.unwrap_err();
	assert_eq!(
		failure.error,
		Error::new(ErrorKind::Type, "missing display form").at(loc(60))
	);
	assert!(executor.is_idle());
	let value = executor
		.invoke_args_for_test(&ok, &mut store, 0, Args::Values(&[]), 1024)
		.unwrap();
	assert_eq!(store.text(value).unwrap(), "42ab");
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}
