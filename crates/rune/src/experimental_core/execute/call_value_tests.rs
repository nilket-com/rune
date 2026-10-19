//! D20 tests-first: CallValue's instruction-level contract that lazy operand
//! binding must preserve. These pass on D19 (CallValue rewritten into a whole-Op
//! Call temporary) and are the oracle for D20. Each failure is identified by its
//! exact Error (kind, operation, the CallValue's location); cleanup, idleness and
//! executor reuse are checked after every failure.

use super::*;
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function, CLOSURE_CREATE},
	heap::Limits,
};

/// Exercise the captured-closure call protocol without compiling Rune source or
/// constructing the standard-module Context. The scalar is copied into each
/// activation; the Vec remains shared. The final arity error must unwind the
/// caller and release its closure/environment owners.
#[test]
fn c11b_hidden_closure_arguments_activation_and_rollback() {
	let mut caller = Function::new(12, 1).unwrap();
	caller
		.push(
			Op::Literal {
				out: 1,
				value: Value::I64(0),
			},
			loc(0),
		)
		.unwrap();
	let (captures_start, captures_len) = caller.arguments(&[1, 0]).unwrap();
	caller
		.push(
			Op::List {
				out: 2,
				tag: super::super::value::Type::Tuple,
				args_start: captures_start,
				args_len: captures_len,
			},
			loc(1),
		)
		.unwrap();
	caller
		.push(
			Op::Literal {
				out: 3,
				value: Value::U64(1),
			},
			loc(2),
		)
		.unwrap();
	let (create_start, create_len) = caller.arguments(&[3, 2]).unwrap();
	caller
		.push(
			Op::Call {
				out: 4,
				function: CLOSURE_CREATE,
				args_start: create_start,
				args_len: create_len,
			},
			loc(3),
		)
		.unwrap();
	let (no_args_start, no_args_len) = caller.arguments(&[]).unwrap();
	caller
		.push(
			Op::CallValue {
				out: 5,
				value: 4,
				args_start: no_args_start,
				args_len: no_args_len,
			},
			loc(4),
		)
		.unwrap();
	caller
		.push(
			Op::CallValue {
				out: 6,
				value: 4,
				args_start: no_args_start,
				args_len: no_args_len,
			},
			loc(5),
		)
		.unwrap();
	caller
		.push(
			Op::Binary {
				out: 7,
				op: Binary::Add,
				lhs: 5,
				rhs: 6,
			},
			loc(6),
		)
		.unwrap();
	caller
		.push(
			Op::Literal {
				out: 8,
				value: Value::I64(2),
			},
			loc(7),
		)
		.unwrap();
	caller
		.push(
			Op::Binary {
				out: 9,
				op: Binary::Eq,
				lhs: 7,
				rhs: 8,
			},
			loc(8),
		)
		.unwrap();
	caller
		.push(
			Op::Branch {
				condition: 9,
				yes: 10,
				no: 13,
			},
			loc(9),
		)
		.unwrap();
	caller
		.push(
			Op::Literal {
				out: 11,
				value: Value::I64(7),
			},
			loc(10),
		)
		.unwrap();
	let (wrong_args_start, wrong_args_len) = caller.arguments(&[11]).unwrap();
	caller
		.push(
			Op::CallValue {
				out: 10,
				value: 4,
				args_start: wrong_args_start,
				args_len: wrong_args_len,
			},
			loc(11),
		)
		.unwrap();
	caller.push(Op::Return { value: 10 }, loc(12)).unwrap();
	let message = caller.string("captured activation mismatch").unwrap();
	caller
		.push(
			Op::String {
				out: 10,
				text: message,
			},
			loc(13),
		)
		.unwrap();
	caller.push(Op::Panic { value: 10 }, loc(14)).unwrap();

	let mut closure = Function::new(6, 2).unwrap();
	closure
		.push(
			Op::Literal {
				out: 2,
				value: Value::U64(0),
			},
			loc(20),
		)
		.unwrap();
	closure
		.push(
			Op::Literal {
				out: 3,
				value: Value::I64(1),
			},
			loc(21),
		)
		.unwrap();
	closure
		.push(
			Op::Binary {
				out: 4,
				op: Binary::Add,
				lhs: 0,
				rhs: 3,
			},
			loc(22),
		)
		.unwrap();
	closure
		.push(Op::Copy { out: 0, value: 4 }, loc(23))
		.unwrap();
	closure
		.push(
			Op::Get {
				out: 4,
				value: 1,
				index: 2,
			},
			loc(24),
		)
		.unwrap();
	closure
		.push(
			Op::Binary {
				out: 5,
				op: Binary::Add,
				lhs: 4,
				rhs: 3,
			},
			loc(25),
		)
		.unwrap();
	closure
		.push(
			Op::Set {
				value: 1,
				index: 2,
				new: 5,
			},
			loc(26),
		)
		.unwrap();
	closure.push(Op::Return { value: 0 }, loc(27)).unwrap();
	let mut builder = Builder::default();
	builder.push(caller).unwrap();
	builder.push(closure).unwrap();
	let code = builder.finish().unwrap();

	let mut store = Store::new(Limits::default()).unwrap();
	let vector = store
		.list(super::super::value::Type::Vec, &[Value::I64(0)])
		.unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	let mut executor = Executor::new().unwrap();
	let failure = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[vector]), 1000)
		.unwrap_err();
	// C11c replaces the historical Type classification with real ArgumentCount.
	assert_eq!(
		failure.error.kind,
		ErrorKind::ArgumentCount,
		"{:#?}",
		failure.error
	);
	assert!(matches!(
		failure.error.detail,
		Detail::Arity {
			given: 1,
			expected: 0
		}
	));
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	assert_eq!(store.get(vector, 0).unwrap(), Value::I64(2));
	store.release(vector).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

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
		.invoke_args_for_test(code, &mut store, 0, Args::Values(&[arg]), fuel)
		.unwrap_err();
	// D2 adds detail; the legacy oracle still checks every pre-D2 field.
	assert_eq!(
		failure.error.with_detail(Detail::None),
		expected.with_detail(Detail::None)
	);

	// The CallValue fails in the entry frame before publishing a callee frame.
	assert!(failure.call_chain.is_empty());
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());

	// The "ab" string and every argument transfer are released by cleanup.
	assert_eq!(store.live_entries(), 0);

	// Reuse after the failure: the executor and store still run a valid call.
	let ok = program(3, 2, 2);
	let value = executor
		.invoke_args_for_test(&ok, &mut store, 0, Args::Values(&[Value::Unit]), 1024)
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
			.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[Value::Unit]), 1024)
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
fn d20_call_value_with_the_wrong_arity_is_argument_count() {
	// The callee takes one parameter; the call passes two.
	fails(
		&program(3, 2, 1),
		Value::Unit,
		1024,
		at_call(ErrorKind::ArgumentCount),
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
	// matches no callee: the lookup (InvalidCode) fires before arity (ArgumentCount).
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
	// C11c keeps token Type precedence over the later ArgumentCount check; this pins
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
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 0)
		.unwrap_err();
	assert_eq!(
		failure.error.with_detail(Detail::None),
		Error::new(ErrorKind::FuelExhausted, "execute").at(loc(40))
	);
	assert!(failure.call_chain.is_empty() && failure.cleanup_error.is_none());
	assert!(executor.is_idle());

	// A pending cancellation is consumed at the same charged CallValue.
	executor.request_cancel();
	let failure = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 1024)
		.unwrap_err();
	assert_eq!(
		failure.error.with_detail(Detail::None),
		Error::new(ErrorKind::Cancelled, "execute").at(loc(40))
	);
	assert!(executor.is_idle());

	// Consumed: the next invocation reaches the CallValue's own checks.
	let failure = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[Value::I64(7)]), 1024)
		.unwrap_err();
	assert_eq!(
		failure.error.with_detail(Detail::None),
		Error::new(ErrorKind::Type, "execute").at(loc(40))
	);

	// And a valid token succeeds: the callee returns its argument, the token.
	let token = Value::Function {
		code: code.id(),
		id: 1,
	};

	let value = executor
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[token]), 1024)
		.unwrap();
	assert_eq!(value, token);
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn p3_moved_set_context_is_vec_only() {
	use crate::experimental_core::value::Type;
	let mut builder = Builder::default();
	let mut function = Function::new(3, 0).unwrap();
	function.push(Op::Return { value: 0 }, loc(0)).unwrap();
	builder.push(function).unwrap();
	let code = builder.finish().unwrap();
	for tag in [Type::Vec, Type::Tuple, Type::Object] {
		let mut store = store();
		let original = if tag == Type::Object {
			store.record(tag, &[("x", Value::I64(7))]).unwrap()
		} else {
			store.list(tag, &[Value::I64(7)]).unwrap()
		};
		let taken = store.take_capture(original).unwrap();
		let error = store.set(original, 0, Value::I64(9)).unwrap_err();
		assert_eq!(error.kind, ErrorKind::AccessConflict);
		assert_eq!(
			error.detail,
			Detail::Moved {
				access: super::super::error::MovedAccess::Write
			}
		);
		let enriched = enrich_error(
			error,
			&store,
			&Op::Set {
				value: 0,
				index: 1,
				new: 2,
			},
			&code,
			0,
			&[original],
			0,
		);
		let expected = if tag == Type::Vec {
			error.with_detail(Detail::MovedArgument {
				access: super::super::error::MovedAccess::Write,
				arg: 0,
			})
		} else {
			error
		};
		assert_eq!(
			enriched, expected,
			"only Vec Set has native argument context"
		);
		store.release(original).unwrap();
		store.release(taken).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
