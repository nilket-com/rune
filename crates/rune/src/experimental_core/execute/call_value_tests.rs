//! D20 tests-first: CallValue's instruction-level contract that lazy operand
//! binding must preserve. These pass on D19 (CallValue rewritten into a whole-Op
//! Call temporary) and are the oracle for D20. Each failure is identified by its
//! exact Error (kind, operation, the CallValue's location); cleanup, idleness and
//! executor reuse are checked after every failure.

use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};

fn loc(n: u32) -> Location {
	Location {
		source: 20,
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

/// Function 1: `parameters` parameters, returns its first.
fn callee(parameters: u32) -> Function {
	let mut f = Function::new(parameters.max(1), parameters).unwrap();
	f.push(Op::Return { value: 0 }, loc(90)).unwrap();
	f
}

/// Function 0 (one parameter, slot 0): slot 1 = I64 5, slot 2 = "ab",
/// slot 3 = a literal token for function 1, then at pc 3 (location 30)
/// `CallValue { out: 4, value: token_slot, args: [2, 1][..args_len] }`, then
/// `Return { value: 4 }` (location 31).
fn program(token_slot: u32, args_len: u32, callee_parameters: u32) -> Code {
	let mut f = Function::new(5, 1).unwrap();
	let text = f.string("ab").unwrap();
	f.push(
		Op::Literal {
			out: 1,
			value: Value::I64(5),
		},
		loc(0),
	)
	.unwrap();
	f.push(Op::String { out: 2, text }, loc(1)).unwrap();
	f.push(
		Op::Literal {
			out: 3,
			value: Value::Function { code: 0, id: 1 },
		},
		loc(2),
	)
	.unwrap();
	let (args_start, args_len) = f.arguments(&[2, 1][..args_len as usize]).unwrap();
	f.push(
		Op::CallValue {
			out: 4,
			value: token_slot,
			args_start,
			args_len,
		},
		loc(30),
	)
	.unwrap();
	f.push(Op::Return { value: 4 }, loc(31)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.push(callee(callee_parameters)).unwrap();
	b.finish().unwrap()
}

fn fails(code: &Code, arg: Value, fuel: usize, expected: Error) {
	let mut store = store();
	let mut executor = Executor::new().unwrap();
	let failure = executor
		.invoke_args(code, &mut store, 0, Args::Values(&[arg]), fuel)
		.unwrap_err();
	assert_eq!(failure.error, expected);

	// The CallValue fails in the entry frame before publishing a callee frame.
	assert!(failure.call_chain.is_empty());
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());

	// The "ab" string and every argument transfer are released by cleanup.
	assert_eq!(store.live_entries(), 0);

	// Reuse after the failure: the executor and store still run a valid call.
	let ok = program(3, 2, 2);
	let value = executor
		.invoke_args(&ok, &mut store, 0, Args::Values(&[Value::Unit]), 1024)
		.unwrap();
	assert_eq!(store.text(value).unwrap(), "ab");
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
}

fn at_call(kind: ErrorKind) -> Error {
	Error::new(kind, "execute").at(loc(30))
}

#[test]
fn d20_dynamic_call_through_a_literal_token_succeeds_and_releases() {
	let code = program(3, 2, 2);
	let mut store = store();
	let mut executor = Executor::new().unwrap();

	for _ in 0..2 {
		let value = executor
			.invoke_args(&code, &mut store, 0, Args::Values(&[Value::Unit]), 1024)
			.unwrap();
		// The callee returned its first argument, the heap string.
		assert_eq!(store.text(value).unwrap(), "ab");
		store.release(value).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
	}
}

#[test]
fn d20_call_value_on_a_non_function_is_a_type_error_at_the_call() {
	// Slot 1 holds I64 5.
	fails(
		&program(1, 2, 2),
		Value::Unit,
		1024,
		at_call(ErrorKind::Type),
	);
}

#[test]
fn d20_call_value_with_a_foreign_owner_is_a_type_error() {
	let foreign = program(3, 2, 2);
	let code = program(0, 2, 2);
	assert_ne!(foreign.id(), code.id());
	fails(
		&code,
		Value::Function {
			code: foreign.id(),
			id: 1,
		},
		1024,
		at_call(ErrorKind::Type),
	);
}

#[test]
fn d20_call_value_with_the_wrong_arity_is_a_type_error() {
	// The callee takes one parameter; the call passes two.
	fails(
		&program(3, 2, 1),
		Value::Unit,
		1024,
		at_call(ErrorKind::Type),
	);
}

#[test]
fn d20_call_value_owner_check_precedes_the_callee_lookup() {
	// A foreign owner with an id that names no function in either program:
	// the owner check (Type) fires before the lookup (InvalidCode).
	let foreign = program(3, 2, 2);
	let code = program(0, 2, 2);
	fails(
		&code,
		Value::Function {
			code: foreign.id(),
			id: 99,
		},
		1024,
		at_call(ErrorKind::Type),
	);
}

#[test]
fn d20_call_value_callee_lookup_precedes_the_arity_check() {
	// A valid owner with an id that names no function, and an args_len that
	// matches no callee: the lookup (InvalidCode) fires before arity (Type).
	let code = program(0, 1, 2);
	fails(
		&code,
		Value::Function {
			code: code.id(),
			id: 99,
		},
		1024,
		at_call(ErrorKind::InvalidCode),
	);
}

#[test]
fn d20_call_value_non_function_with_wrong_arity_is_still_a_type_error() {
	// Both the token check and the arity check would report Type; this pins
	// the observable result only, not which check fired.
	fails(
		&program(1, 2, 1),
		Value::Unit,
		1024,
		at_call(ErrorKind::Type),
	);
}

/// Function 0 whose FIRST instruction (charged) is the CallValue: one parameter
/// (slot 0, the value called), `CallValue { out: 1, value: 0, args: [0] }` at
/// location 40, then `Return { value: 1 }`. Function 1 returns its argument.
fn charged_call() -> Code {
	let mut f = Function::new(2, 1).unwrap();
	let (args_start, args_len) = f.arguments(&[0]).unwrap();
	f.push(
		Op::CallValue {
			out: 1,
			value: 0,
			args_start,
			args_len,
		},
		loc(40),
	)
	.unwrap();
	f.push(Op::Return { value: 1 }, loc(41)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.push(callee(1)).unwrap();
	b.finish().unwrap()
}

#[test]
fn d20_charge_precedes_call_value_checks() {
	let code = charged_call();
	assert!(code.function(0).unwrap().cached_ref(0).unwrap().charged);
	let mut store = store();
	let mut executor = Executor::new().unwrap();

	// Fuel exhaustion at the charged CallValue, even though its value is not a function.
	let failure = executor
		.invoke_args(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 0)
		.unwrap_err();
	assert_eq!(
		failure.error,
		Error::new(ErrorKind::FuelExhausted, "execute").at(loc(40))
	);
	assert!(failure.call_chain.is_empty() && failure.cleanup_error.is_none());
	assert!(executor.is_idle());

	// A pending cancellation is consumed at the same charged CallValue.
	executor.request_cancel();
	let failure = executor
		.invoke_args(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 1024)
		.unwrap_err();
	assert_eq!(
		failure.error,
		Error::new(ErrorKind::Cancelled, "execute").at(loc(40))
	);
	assert!(executor.is_idle());

	// Consumed: the next invocation reaches the CallValue's own checks.
	let failure = executor
		.invoke_args(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 1024)
		.unwrap_err();
	assert_eq!(
		failure.error,
		Error::new(ErrorKind::Type, "execute").at(loc(40))
	);

	// And a valid token succeeds: the callee returns its argument, the token.
	let token = Value::Function {
		code: code.id(),
		id: 1,
	};

	let value = executor
		.invoke_args(&code, &mut store, 0, Args::Values(&[token]), 1024)
		.unwrap();
	assert_eq!(value, token);
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
}
