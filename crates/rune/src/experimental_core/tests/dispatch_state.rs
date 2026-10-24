//! Tests-first transfer contracts on the unchanged 145bb453 executor.
//! Source only until review: no claim of baseline execution yet.

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function, Op},
	error::{ErrorKind, Location},
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::{Type, Value},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};

fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

fn compiled(text: &str) -> (frontend::Program, u32) {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	(program, main)
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn exhausted_iterator_and_dynamic_calls_resume_the_correct_caller() {
	let (program, main) = compiled(
		r#"
		fn first(rows) {
			for item in [] { return 999; }
			for item in rows { return item; }
			77
		}
		fn invoke(rows) { let f = first; f(rows) + 1 }
		pub fn main() {
			let a = invoke([6]);
			let b = invoke([]);
			a * 100 + b
		}
	"#,
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());

	for _ in 0..3 {
		let out = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap();
		assert_eq!(out.value, Value::I64(778));
		store.root_release(out).unwrap();
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn failed_try_returns_a_heap_alias_without_running_the_tail() {
	let (program, main) = compiled(
		r#"
		fn fail(rows) { Err(rows)?; rows[0] = 999; Ok(rows) }
		fn bridge(rows) { let f = fail; let result = f(rows); rows[0] += 1; result }
		pub fn main() { let rows = [7]; bridge(rows) }
	"#,
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());

	for _ in 0..3 {
		let out = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap();
		let (tag, success, payload) = store.variant_parts(out.value).unwrap();
		assert_eq!(tag, Type::Result);
		assert!(!success);
		assert_eq!(store.get(payload.unwrap(), 0).unwrap(), Value::I64(8));
		store.root_release(out).unwrap();
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn successful_try_resumes_the_callee_then_the_parent() {
	let (program, main) = compiled(
		r#"
		fn unwrap_add(n) { let x = Ok(n)?; x + 2 }
		fn bridge(n) { unwrap_add(n) * 3 + 1 }
		pub fn main() { bridge(5) + bridge(10) }
	"#,
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let out = executor
		.invoke(&program, &mut store, main, &[], 1000)
		.unwrap();
	assert_eq!(out.value, Value::I64(59));
	store.root_release(out).unwrap();
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn callee_errors_keep_the_instruction_pc_callsite_and_reusable_state() {
	// Parent slots [0..3), callee [3..7), return destination parent slot 2.
	let mut caller = Function::new(3, 0).unwrap();
	caller
		.push(
			Op::Literal {
				out: 0,
				value: Value::I64(8),
			},
			loc(10),
		)
		.unwrap();
	let (args_start, args_len) = caller.arguments(&[0]).unwrap();
	caller
		.push(
			Op::Call {
				out: 2,
				function: 1,
				args_start,
				args_len,
			},
			loc(11),
		)
		.unwrap();
	caller.push(Op::Return { value: 2 }, loc(12)).unwrap();
	let mut callee = Function::new(4, 1).unwrap();
	callee
		.push(
			Op::Literal {
				out: 1,
				value: Value::I64(0),
			},
			loc(100),
		)
		.unwrap();
	callee
		.push(
			Op::Binary {
				out: 3,
				op: Binary::Div,
				lhs: 0,
				rhs: 1,
			},
			loc(101),
		)
		.unwrap();
	callee.push(Op::Return { value: 3 }, loc(102)).unwrap();
	let mut success = Function::new(1, 0).unwrap();
	success
		.push(
			Op::Literal {
				out: 0,
				value: Value::I64(42),
			},
			loc(200),
		)
		.unwrap();
	success.push(Op::Return { value: 0 }, loc(201)).unwrap();
	let mut builder = Builder::default();
	builder.push(caller).unwrap();
	builder.push(callee).unwrap();
	builder.push(success).unwrap();
	let code = builder.finish().unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();

	for (fuel, kind, where_) in [
		(1000, ErrorKind::DivisionByZero, loc(101)),
		(1, ErrorKind::FuelExhausted, loc(100)),
	] {
		let failure = executor
			.invoke_code(&code, &mut store, 0, &[], fuel)
			.unwrap_err();
		assert_eq!(failure.error.kind, kind);
		assert_eq!(failure.error.location, Some(where_));
		assert_eq!(failure.call_chain.as_slice(), &[loc(11)]);
		assert!(failure.cleanup_error.is_none());
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		let out = executor
			.invoke_code(&code, &mut store, 2, &[], 1000)
			.unwrap();
		assert_eq!(out.value, Value::I64(42));
		store.root_release(out).unwrap();
		assert!(executor.is_idle());
	}
}

#[test]
fn call_frame_capacity_is_at_the_call_and_cleanup_allows_reuse() {
	let mut recursive = Function::new(1, 0).unwrap();
	let (args_start, args_len) = recursive.arguments(&[]).unwrap();
	recursive
		.push(
			Op::Call {
				out: 0,
				function: 0,
				args_start,
				args_len,
			},
			loc(300),
		)
		.unwrap();
	recursive.push(Op::Return { value: 0 }, loc(301)).unwrap();
	let mut success = Function::new(1, 0).unwrap();
	success
		.push(
			Op::Literal {
				out: 0,
				value: Value::I64(42),
			},
			loc(400),
		)
		.unwrap();
	success.push(Op::Return { value: 0 }, loc(401)).unwrap();
	let mut builder = Builder::default();
	builder.push(recursive).unwrap();
	builder.push(success).unwrap();
	let code = builder.finish().unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::with_limits(super::super::execute::ExecutorLimits {
		frames: 1024,
		slots: 65_536,
	})
	.unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());
	let failure = executor
		.invoke_code(&code, &mut store, 0, &[], 10_000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::FrameCapacity);
	assert_eq!(failure.error.location, Some(loc(300)));

	// There are 1024 published frames. Entry has no callsite; the refused
	// next callee is never published, leaving 1023 identical recursive sites.
	assert_eq!(failure.call_chain.len(), 1023);
	assert!(failure.call_chain.iter().all(|site| *site == loc(300)));
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	let out = executor
		.invoke_code(&code, &mut store, 1, &[], 1000)
		.unwrap();
	assert_eq!(out.value, Value::I64(42));
	store.root_release(out).unwrap();
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
}
