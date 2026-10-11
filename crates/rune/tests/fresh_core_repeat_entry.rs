#![cfg(feature = "fresh-core")]
//! Repeat entry: one executor and store invoked K times through the normal checked path,
//! left clean after success, run-time error and fuel halt. Moved from the D21 dev crate
//! (`experimental/0184-dev/tests/repeat_entry.rs`); its two real-output tests are rewritten
//! as the in-crate `io_policy` cap test and `fresh_core_stdout.rs`.
use rune::experimental_core::{
	error::ErrorKind,
	execute::{Executor, Failure},
	frontend::{self, Program},
	heap::{Limits, Store},
	value::{Root, Scalar, Type},
};
use rune::{Context, Diagnostics, Hash, Source, Sources};

fn compile(text: &str) -> Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap()
}
fn new_store(entries: usize) -> Store {
	Store::new(Limits {
		entries,
		bytes: 65_536,
	})
	.unwrap()
}
/// Read a scalar result and release it.
fn scalar(store: &mut Store, root: Root) -> Scalar {
	let scalar = store.root_scalar(&root).unwrap().expect("scalar result");
	store.root_release(root).unwrap();
	scalar
}
/// Invoke, then release the result (the caller's one strong reference) and check the
/// executor and store are back where they started.
#[allow(clippy::result_large_err)] // `Failure` is the public error type of `invoke`.
fn once(
	executor: &mut Executor,
	program: &Program,
	store: &mut Store,
	fuel: usize,
) -> Result<Option<Scalar>, Failure> {
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	let result = executor.invoke(program, store, main, &[], fuel);
	assert!(executor.is_idle(), "executor not idle after invocation");
	let result = result.map(|root| {
		let scalar = store.root_scalar(&root).unwrap();
		store.root_release(root).unwrap();
		scalar
	});
	assert_eq!(
		(store.live_entries(), store.payload_bytes()),
		before,
		"store not back to its pre-invocation entries and bytes"
	);
	result
}
// Smaller return-value fixtures in the shape of the five diagnostic programs. They do
// not print; exact five-program output and completion checks are owed by the diagnostic
// precondition and are not claimed here.
const PROGRAMS: [(&str, &str); 5] = [
	(
		"numeric",
		"pub fn main() { let n = 0; let total = 0; while n < 1000 { total += n % 17; n += 1; } total }",
	),
	(
		"while",
		"pub fn main() { let n = 0; while n < 1000 { n += 1; } n }",
	),
	(
		"calls",
		"fn add(a, b) { a + b } pub fn main() { let total = 0; for i in 0..1000 { total = add(total, i); } total }",
	),
	(
		"fib",
		"fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } } pub fn main() { fib(12) }",
	),
	(
		"rows",
		"fn add(rows, key, value) { for row in rows { if row[0] == key { row[1] = row[1] + value; return; } } rows.push([key, value]); } pub fn main() { let rows = []; add(rows, \"a\", 2); add(rows, \"b\", 3); add(rows, \"a\", 5); let copy = rows.clone(); let alias = copy[0]; alias[1] = alias[1] + 1; copy.push([\"c\", 9]); rows[0][1] * 100 + rows.len() as i64 * 10 + copy.len() as i64 }",
	),
];

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn five_programs_repeat_identically_on_one_executor_and_store() {
	for (name, text) in PROGRAMS {
		let program = compile(text);
		let mut store = new_store(256);
		let mut executor = Executor::new().unwrap();
		let first = once(&mut executor, &program, &mut store, 10_000_000).unwrap();
		let mut fresh_store = new_store(256);
		let fresh = Executor::new()
			.unwrap()
			.run(
				&program,
				&mut fresh_store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				10_000_000,
			)
			.unwrap();
		let fresh = fresh_store.root_scalar(&fresh).unwrap();
		assert_eq!(first, fresh, "{name}: invoke differs from run");
		for k in 1..5 {
			let again = once(&mut executor, &program, &mut store, 10_000_000).unwrap();
			assert_eq!(again, first, "{name}: invocation {k} differs");
		}
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_runtime_error_leaves_the_executor_reusable() {
	let program = compile("pub fn main() { let v = [1, 2]; let s = \"x\" + \"y\"; v[0] / 0 }");
	let mut store = new_store(256);
	let mut executor = Executor::new().unwrap();
	for k in 0..5 {
		let failure = once(&mut executor, &program, &mut store, 10_000).unwrap_err();
		assert_eq!(
			failure.error.kind,
			ErrorKind::DivisionByZero,
			"invocation {k}"
		);
		assert!(failure.cleanup_error.is_none());
	}
	// The same executor then succeeds on another program.
	let ok = compile("pub fn main() { 7 }");
	assert_eq!(
		once(&mut executor, &ok, &mut store, 10_000).unwrap(),
		Some(Scalar::I64(7))
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_fuel_halt_leaves_the_executor_reusable() {
	let program = compile("pub fn main() { let v = [1]; let n = 0; loop { n += 1; v.push(n); } }");
	let mut store = new_store(256);
	let mut executor = Executor::new().unwrap();
	for k in 0..5 {
		let failure = once(&mut executor, &program, &mut store, 200).unwrap_err();
		assert_eq!(
			failure.error.kind,
			ErrorKind::FuelExhausted,
			"invocation {k}"
		);
		assert!(failure.error.location.is_some());
	}
	let ok = compile("pub fn main() { 7 }");
	assert_eq!(
		once(&mut executor, &ok, &mut store, 10_000).unwrap(),
		Some(Scalar::I64(7))
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn failing_and_succeeding_invocations_alternate() {
	let fails = compile("pub fn main() { let v = [1, 2, 3]; v[9] }");
	let works = compile("pub fn main() { let v = [1, 2, 3]; v[2] }");
	let mut store = new_store(256);
	let mut executor = Executor::new().unwrap();
	for _ in 0..4 {
		assert_eq!(
			once(&mut executor, &fails, &mut store, 10_000)
				.unwrap_err()
				.error
				.kind,
			ErrorKind::IndexOutOfBounds
		);
		assert_eq!(
			once(&mut executor, &works, &mut store, 10_000).unwrap(),
			Some(Scalar::I64(3))
		);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn no_invocation_leaks_an_entry_into_the_next() {
	// One entry in the store: an invocation that left anything behind would make
	// the next one fail at its first allocation.
	let program = compile("pub fn main() { let v = [1]; v.len() as i64 }");
	let mut store = new_store(1);
	let mut executor = Executor::new().unwrap();
	for _ in 0..5 {
		assert_eq!(
			once(&mut executor, &program, &mut store, 10_000).unwrap(),
			Some(Scalar::I64(1))
		);
	}
	let failing = compile("pub fn main() { let v = [1]; v[5] }");
	for _ in 0..3 {
		assert_eq!(
			once(&mut executor, &failing, &mut store, 10_000)
				.unwrap_err()
				.error
				.kind,
			ErrorKind::IndexOutOfBounds
		);
		assert_eq!(
			once(&mut executor, &program, &mut store, 10_000).unwrap(),
			Some(Scalar::I64(1))
		);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_returned_heap_value_is_the_callers_to_release() {
	let program = compile("pub fn main() { [1, 2] }");
	let mut store = new_store(4);
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let a = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap();
	let b = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap();
	assert_eq!(store.live_entries(), 2);
	assert!(executor.is_idle());
	store.root_release(a).unwrap();
	store.root_release(b).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_heap_result_is_released_on_every_repetition() {
	let program = compile("pub fn main() { let v = [1, 2]; v.push(3); v }");
	let mut store = new_store(2);
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());
	for _ in 0..5 {
		let value = executor
			.invoke(&program, &mut store, main, &[], 10_000)
			.unwrap();
		assert_eq!(store.live_entries(), 1);
		store.root_release(value).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
		assert!(executor.is_idle());
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_returned_alias_of_an_input_leaves_the_callers_ownership_intact() {
	let program = compile("pub fn main(v) { v.push(9); v }");
	let mut store = new_store(4);
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let one = store.root_i64(1);
	let root = store.root_list(Type::Vec, &[&one]).unwrap();
	for k in 1..=3 {
		let result = executor
			.invoke(&program, &mut store, main, &[&root], 10_000)
			.unwrap();
		// The result is the input alias: no new entry, and the script's mutation of
		// the caller-owned value is preserved, not rolled back, and visible through both.
		assert_eq!(store.live_entries(), 1);
		assert_eq!(store.root_len(&root).unwrap(), 1 + k);
		assert_eq!(store.root_len(&result).unwrap(), 1 + k);
		// Bytes after the script's intended mutation; releasing the alias changes nothing.
		let mutated = (store.live_entries(), store.payload_bytes());
		store.root_release(result).unwrap();
		assert_eq!(
			(store.live_entries(), store.payload_bytes()),
			mutated,
			"the caller's root survives releasing the result"
		);
		assert!(executor.is_idle());
	}
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_foreign_argument_is_refused_before_any_argument_is_retained() {
	use rune::experimental_core::value::{Scalar, Type};
	let program = compile("pub fn main(a, b) { a.len() as i64 }");
	let mut store = new_store(4);
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let one = store.root_i64(1);
	let root = store.root_list(Type::Vec, &[&one]).unwrap();
	// A root owned by another store, with the same slot and generation as `root`:
	// inside the core they are indistinguishable; the boundary tells them apart.
	let mut other = new_store(4);
	let two = other.root_i64(2);
	let bogus = other.root_list(Type::Vec, &[&two]).unwrap();
	for _ in 0..3 {
		let failure = executor
			.invoke(&program, &mut store, main, &[&root, &bogus], 10_000)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidHandle);
		assert!(failure.call_chain.is_empty());
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 1, "only the caller's root remains");
		assert_eq!(other.live_entries(), 1);
		let ok = executor
			.invoke(&program, &mut store, main, &[&root, &root], 10_000)
			.unwrap();
		assert_eq!(store.root_scalar(&ok).unwrap(), Some(Scalar::I64(1)));
		store.root_release(ok).unwrap();
		assert_eq!(store.live_entries(), 1);
	}
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
	other.root_release(bogus).unwrap();
	assert_eq!(other.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn an_entry_refusal_is_followed_by_valid_reuse() {
	let program = compile("pub fn main() { 5 }");
	let mut store = new_store(4);
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let failure = executor
		.invoke(&program, &mut store, 99, &[], 10_000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(executor.is_idle());
	let out = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap();
	assert_eq!(scalar(&mut store, out), Scalar::I64(5));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn run_and_invoke_report_the_same_failure() {
	let program =
		compile("fn inner(x) { x / 0 } fn outer(x) { inner(x) + 1 } pub fn main() { outer(4) }");
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = new_store(4);
	let by_run = Executor::new()
		.unwrap()
		.run(&program, &mut store, main, &[], 10_000)
		.unwrap_err();
	let mut executor = Executor::new().unwrap();
	let by_invoke = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap_err();
	let again = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap_err();
	for failure in [&by_invoke, &again] {
		assert_eq!(failure.error, by_run.error);
		assert_eq!(failure.call_chain, by_run.call_chain);
		assert!(failure.cleanup_error.is_none() && by_run.cleanup_error.is_none());
	}
	assert_eq!(by_run.error.kind, ErrorKind::DivisionByZero);
	assert!(by_run.error.location.is_some());
	assert_eq!(by_run.call_chain.len(), 2);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_pending_cancellation_is_consumed_only_by_a_charged_instruction() {
	let program = compile("pub fn main() { let n = 0; while n < 10 { n += 1; } n }");
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = new_store(4);
	let mut executor = Executor::new().unwrap();
	executor.request_cancel();
	let failure = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert!(executor.is_idle());
	let out = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap();
	assert_eq!(scalar(&mut store, out), Scalar::I64(10));
	// An entry refusal before the first charge does not swallow the request.
	executor.request_cancel();
	let refused = executor
		.invoke(&program, &mut store, 99, &[], 10_000)
		.unwrap_err();
	assert_eq!(refused.error.kind, ErrorKind::InvalidCode);
	let cancelled = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap_err();
	assert_eq!(cancelled.error.kind, ErrorKind::Cancelled);
	let out = executor
		.invoke(&program, &mut store, main, &[], 10_000)
		.unwrap();
	assert_eq!(scalar(&mut store, out), Scalar::I64(10));
}
