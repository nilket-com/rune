//! Development checks on hand-built owned code; no frontend/conformance claim.

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function, Op},
	error::{ErrorKind, Location},
	execute::Executor,
	heap::{Limits, Store},
	value::{Type, Value},
};

fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

fn store() -> Store {
	Store::new(Limits {
		entries: 128,
		bytes: 32_768,
	})
	.unwrap()
}

#[test]
fn loop_backedge_charges_and_runs() {
	let mut f = Function::new(5, 0).unwrap();
	for (out, value) in [(0, 0), (1, 0), (2, 100), (3, 1)] {
		f.push(
			Op::Literal {
				out,
				value: Value::I64(value),
			},
			loc(out),
		)
		.unwrap();
	}

	f.push(
		Op::Binary {
			out: 4,
			op: Binary::Lt,
			lhs: 0,
			rhs: 2,
		},
		loc(4),
	)
	.unwrap();
	f.push(
		Op::Branch {
			condition: 4,
			yes: 6,
			no: 9,
		},
		loc(5),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 1,
			op: Binary::Add,
			lhs: 1,
			rhs: 0,
		},
		loc(6),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 0,
			op: Binary::Add,
			lhs: 0,
			rhs: 3,
		},
		loc(7),
	)
	.unwrap();
	f.push(Op::Jump { target: 4 }, loc(8)).unwrap();
	f.push(Op::Return { value: 1 }, loc(9)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	assert!(code.function(0).unwrap().instructions()[4].is_charged());
	assert!(!code.function(0).unwrap().instructions()[6].is_charged());
	assert_eq!(
		Executor::new()
			.unwrap()
			.run_values(&code, &mut store(), 0, &[], 1000)
			.ok(),
		Some(Value::I64(4950))
	);
	let mut s = store();
	let error = Executor::new()
		.unwrap()
		.run_values(&code, &mut s, 0, &[], 1)
		.err()
		.unwrap();
	assert_eq!(error.error.kind, ErrorKind::FuelExhausted);
	assert_eq!(error.error.location, Some(loc(4)));
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn explicit_recursive_frames_and_return() {
	let mut f = Function::new(6, 1).unwrap();
	f.push(
		Op::Literal {
			out: 1,
			value: Value::I64(0),
		},
		loc(0),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 2,
			op: Binary::Eq,
			lhs: 0,
			rhs: 1,
		},
		loc(1),
	)
	.unwrap();
	f.push(
		Op::Branch {
			condition: 2,
			yes: 3,
			no: 4,
		},
		loc(2),
	)
	.unwrap();
	f.push(Op::Return { value: 1 }, loc(3)).unwrap();
	f.push(
		Op::Literal {
			out: 3,
			value: Value::I64(1),
		},
		loc(4),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 4,
			op: Binary::Sub,
			lhs: 0,
			rhs: 3,
		},
		loc(5),
	)
	.unwrap();
	let (args_start, args_len) = f.arguments(&[4]).unwrap();
	f.push(
		Op::Call {
			out: 5,
			function: 0,
			args_start,
			args_len,
		},
		loc(6),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 5,
			op: Binary::Add,
			lhs: 5,
			rhs: 3,
		},
		loc(7),
	)
	.unwrap();
	f.push(Op::Return { value: 5 }, loc(8)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	assert!(!code.function(0).unwrap().instructions()[6].is_charged());
	assert!(!code.function(0).unwrap().instructions()[7].is_charged());
	assert_eq!(
		Executor::new()
			.unwrap()
			.run_values(&code, &mut store(), 0, &[Value::I64(32)], 1000)
			.ok(),
		Some(Value::I64(32))
	);
	let error = Executor::new()
		.unwrap()
		.run_values(&code, &mut store(), 0, &[Value::I64(1100)], 5000)
		.err()
		.unwrap();
	assert_eq!(error.error.kind, ErrorKind::FrameCapacity);
	assert_eq!(error.call_chain.len(), 1023);
}

#[test]
fn writes_survive_error_but_local_roots_are_released() {
	let mut s = store();
	let rows = s.list(Type::Vec, &[Value::I64(2)]).unwrap();
	let mut f = Function::new(5, 1).unwrap();
	f.push(
		Op::Literal {
			out: 1,
			value: Value::I64(0),
		},
		loc(0),
	)
	.unwrap();
	f.push(
		Op::Literal {
			out: 2,
			value: Value::I64(9),
		},
		loc(1),
	)
	.unwrap();
	f.push(
		Op::Set {
			value: 0,
			index: 1,
			new: 2,
		},
		loc(2),
	)
	.unwrap();
	f.push(
		Op::Literal {
			out: 3,
			value: Value::Bool(false),
		},
		loc(3),
	)
	.unwrap();
	f.push(
		Op::Binary {
			out: 4,
			op: Binary::Add,
			lhs: 2,
			rhs: 3,
		},
		loc(4),
	)
	.unwrap();
	f.push(Op::Return { value: 4 }, loc(5)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	let error = Executor::new()
		.unwrap()
		.run_values(&code, &mut s, 0, &[rows], 1000)
		.err()
		.unwrap();
	assert_eq!(error.error.kind, ErrorKind::Type);
	assert_eq!(error.error.location, Some(loc(4)));
	assert_eq!(s.get(rows, 0).unwrap(), Value::I64(9));
	assert_eq!(s.live_entries(), 1);
	s.release(rows).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn long_forward_path_is_split_and_invalid_code_refused() {
	let mut f = Function::new(1, 0).unwrap();
	for i in 0..900 {
		f.push(
			Op::Literal {
				out: 0,
				value: Value::I64(i as i64),
			},
			loc(i),
		)
		.unwrap();
	}

	f.push(Op::Return { value: 0 }, loc(900)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	assert_eq!(
		Executor::new()
			.unwrap()
			.run_values(&code, &mut store(), 0, &[], 1000)
			.ok(),
		Some(Value::I64(899))
	);
	let error = Executor::new()
		.unwrap()
		.run_values(&code, &mut store(), 0, &[], 1)
		.err()
		.unwrap();
	assert_eq!(error.error.kind, ErrorKind::FuelExhausted);
	let mut bad = Function::new(1, 0).unwrap();
	bad.push(Op::Return { value: 2 }, loc(0)).unwrap();
	let mut b = Builder::default();
	b.push(bad).unwrap();
	assert!(b.finish().is_err());
}

#[test]
fn cancellation_and_closed_store_are_named_errors() {
	let mut f = Function::new(1, 0).unwrap();
	f.push(Op::Return { value: 0 }, loc(0)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	let mut executor = Executor::new().unwrap();
	executor.request_cancel();
	assert_eq!(
		executor
			.run_values(&code, &mut store(), 0, &[], 1000)
			.err()
			.unwrap()
			.error
			.kind,
		ErrorKind::Cancelled
	);
	let mut s = store();
	s.close().unwrap();
	assert_eq!(
		Executor::new()
			.unwrap()
			.run_values(&code, &mut s, 0, &[], 1000)
			.err()
			.unwrap()
			.error
			.kind,
		ErrorKind::ClosedStore
	);
}

#[test]
fn compact_instruction_and_literal_pool_round_trip() {
	assert_eq!(
		core::mem::size_of::<crate::experimental_core::code::Instruction>(),
		16
	);
	let mut f = Function::new(1, 0).unwrap();
	f.push(
		Op::Literal {
			out: 0,
			value: Value::I64(i64::MIN),
		},
		loc(0),
	)
	.unwrap();
	f.push(Op::Return { value: 0 }, loc(1)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	assert!(matches!(
		code.function(0).unwrap().operation(0).unwrap(),
		Op::Literal {
			out: 0,
			value: Value::I64(i64::MIN)
		}
	));
	assert_eq!(
		Executor::new()
			.unwrap()
			.run_values(&code, &mut store(), 0, &[], 10)
			.unwrap(),
		Value::I64(i64::MIN)
	);
}
