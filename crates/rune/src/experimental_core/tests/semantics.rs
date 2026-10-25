//! Script-level semantics moved from the D21 acceptance tests
//! (`experimental/0184-dev/tests/acceptance.rs`), without the removed acceptance bridge.
//!
//! The `host::t(n, x)` and `host::mark(x)` effects are re-expressed as ordinary script
//! functions over a caller-owned witness vector `w`:
//! `fn t(w, n, x) { w.push(n); x }` and `fn mark(w, x) { w.push(0); x }`.
//! The witness survives a later failure, because it is the caller's root.

use crate::experimental_core::error::ErrorKind;
use crate::experimental_core::execute::Executor;
use crate::experimental_core::frontend::{self, Program};
use crate::experimental_core::heap::{Limits, Store};
use crate::experimental_core::value::{Root, Scalar, Type};
use crate::{Context, Diagnostics, Hash, Source, Sources};

const EFFECTS: &str = "fn t(w, n, x) { w.push(n); x } fn mark(w, x) { w.push(0); x }";

fn compile(text: &str) -> Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap()
}

fn main_of(program: &Program) -> u32 {
	program.function(Hash::type_hash(["main"])).unwrap()
}

fn witness(store: &mut Store, w: &Root) -> std::vec::Vec<i64> {
	let mut out = std::vec::Vec::new();
	for i in 0..store.root_len(w).unwrap() {
		let item = store.root_get(w, i).unwrap();
		match store.root_scalar(&item).unwrap() {
			Some(Scalar::I64(n)) => out.push(n),
			other => panic!("witness item {other:?}"),
		}

		store.root_release(item).unwrap();
	}

	out
}

/// Was `effects_keep_argument_order_aliases_failures_and_reset`.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn effects_keep_argument_order_aliases_failures_and_reset() {
	let mut store = Store::new(Limits::default()).unwrap();
	let mut ex = Executor::new().unwrap();
	let n = store.root_i64(7);

	for (text, events) in [
		(
			"pub fn main(a, w) { t(w, 1, a); t(w, 1, t(w, 2, a)) }",
			&[1, 2, 1][..],
		),
		(
			"pub fn main(a, w) { [t(w,1,a), t(w,2,a), t(w,3,a)] }",
			&[1, 2, 3][..],
		),
		(
			"pub fn main(a, w) { (t(w,1,a), t(w,2,a), t(w,3,a)) }",
			&[1, 2, 3][..],
		),
		(
			"pub fn main(a, w) { pick(t(w,1,a), t(w,2,a), t(w,3,a)) } fn pick(a,b,c) { c }",
			&[1, 2, 3][..],
		),
		(
			"pub fn main(a, w) { t(w,1,a) + t(w,2,a) * t(w,3,a) }",
			&[1, 2, 3][..],
		),
	] {
		let p = compile(&std::format!("{EFFECTS} {text}"));

		// A new witness per invocation is the reset.
		let w = store.root_list(Type::Vec, &[]).unwrap();
		let out = ex
			.invoke(&p, &mut store, main_of(&p), &[&n, &w], 1000)
			.unwrap();
		assert_eq!(witness(&mut store, &w), events, "{text}");
		store.root_release(out).unwrap();
		store.root_release(w).unwrap();
		assert!(ex.is_idle());
		assert_eq!(store.live_entries(), 0);
	}

	// The effect functions return their argument itself: an alias of the caller's list.
	let p = compile(&std::format!(
		"{EFFECTS} pub fn main(a, w) {{ mark(w, t(w, 1, a)) }}"
	));
	let item = store.root_i64(1);
	let list = store.root_list(Type::Vec, &[&item]).unwrap();

	// The baseline excludes the witness, which grows with the effects.
	let baseline = (store.live_entries(), store.payload_bytes());
	let w = store.root_list(Type::Vec, &[]).unwrap();
	let out = ex
		.invoke(&p, &mut store, main_of(&p), &[&list, &w], 1000)
		.unwrap();
	assert_eq!(witness(&mut store, &w), [1, 0]);
	store.root_release(w).unwrap();
	let changed = store.root_i64(9);
	store.root_set(&out, 0, &changed).unwrap();
	let got = store.root_get(&list, 0).unwrap();
	assert_eq!(store.root_scalar(&got).unwrap(), Some(Scalar::I64(9)));
	store.root_release(got).unwrap();
	store.root_release(out).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	store.root_release(list).unwrap();

	// Effects before a failure are kept.
	let p = compile(&std::format!(
		"{EFFECTS} pub fn main(a, w) {{ mark(w, a); t(w, 3, a); 1/0 }}"
	));
	let w = store.root_list(Type::Vec, &[]).unwrap();
	let failure = ex
		.invoke(&p, &mut store, main_of(&p), &[&n, &w], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
	assert!(failure.cleanup_error.is_none());
	assert!(ex.is_idle());
	assert_eq!(witness(&mut store, &w), [0, 3]);
	store.root_release(w).unwrap();
	assert_eq!(store.live_entries(), 0);
}

/// Was `short_circuit_records_only_executed_rhs`. The execution core's `&&` is strict-bool
/// (a declared difference): a non-bool left operand is a Type error.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn short_circuit_records_only_executed_rhs() {
	let p = compile(&std::format!(
		"{EFFECTS} pub fn main(a, b, w) {{ a && mark(w, b) }}"
	));
	let mut store = Store::new(Limits::default()).unwrap();
	let mut ex = Executor::new().unwrap();
	let b = store.root_bool(true);

	for (left, marks) in [(false, 0), (true, 1)] {
		let a = store.root_bool(left);
		let w = store.root_list(Type::Vec, &[]).unwrap();
		let out = ex
			.invoke(&p, &mut store, main_of(&p), &[&a, &b, &w], 1000)
			.unwrap();
		assert_eq!(witness(&mut store, &w).len(), marks);
		assert_eq!(store.root_scalar(&out).unwrap(), Some(Scalar::Bool(left)));
		store.root_release(out).unwrap();
		store.root_release(w).unwrap();
	}

	let a = store.root_i64(1);
	let w = store.root_list(Type::Vec, &[]).unwrap();
	let failure = ex
		.invoke(&p, &mut store, main_of(&p), &[&a, &b, &w], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	assert!(witness(&mut store, &w).is_empty());
	store.root_release(w).unwrap();
	assert_eq!(store.live_entries(), 0);
}

/// Moved unchanged apart from the context (default modules, no host module).
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn missing_display_forms_are_type_errors_through_format_and_template() {
	for expression in [r#"format!("{}", a)"#, r#"`${a}`"#] {
		let p = compile(&std::format!("pub fn main(a) {{ {expression} }}"));
		let main = main_of(&p);
		let mut store = Store::new(Limits::default()).unwrap();
		let mut ex = Executor::new().unwrap();
		let unit = store.root_unit();
		let vector = store.root_list(Type::Vec, &[&unit]).unwrap();
		let baseline = (store.live_entries(), store.payload_bytes());

		for arg in [&unit, &vector] {
			let failure = ex.invoke(&p, &mut store, main, &[arg], 1000).unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::Type);
			assert!(failure.cleanup_error.is_none());
			assert!(ex.is_idle());
			assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
		}

		store.root_release(vector).unwrap();
		store.root_release(unit).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
