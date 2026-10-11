//! A1 lowering-shape regressions: direct local operands under the writes_nothing rule,
//! snapshot where a later operand writes, stack slot reuse, single lifetime-based releases,
//! effect-only statements, and loop/branch cleanup. Local construction checks, not evidence.

use crate::experimental_core::{
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::Value,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::string::{String, ToString};

fn run(text: &str, args: &[Value]) -> Value {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap();
	let result = Executor::new()
		.unwrap()
		.run_values(&program.code, &mut store, main, args, 100_000)
		.unwrap();
	// Every temporary and local was released: no entry survives the invocation.
	assert_eq!(store.live_entries(), 0, "live entries after {text}");
	result
}

fn int(text: &str) -> i64 {
	match run(text, &[]) {
		Value::I64(n) => n,
		other => panic!("expected an integer from {text}, got {other:?}"),
	}
}

fn text(src: &str) -> String {
	// Strings are heap values; read them back through a length-and-sum so the
	// helper's zero-live-entries check still applies to the whole program.
	match run(src, &[]) {
		Value::I64(n) => n.to_string(),
		Value::U64(n) => n.to_string(),
		other => panic!("expected an encoded string from {src}, got {other:?}"),
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn call_argument_snapshots_when_a_later_argument_writes_the_local() {
	// a must see 1 even though b rebinds x to 2 before the call runs.
	assert_eq!(
		int("fn f(a, b) { a * 10 + b } pub fn main() { let x = 1; f(x, { x = 2; x }) }"),
		12
	);

	// With a later argument that writes nothing the local is read in place.
	assert_eq!(
		int("fn f(a, b) { a * 10 + b } pub fn main() { let x = 1; f(x, 5) }"),
		15
	);

	// A later call argument is not assumed pure either.
	assert_eq!(
		int("fn g() { 3 } fn f(a, b) { a * 10 + b } pub fn main() { let x = 1; f(x, g()) }"),
		13
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn collection_and_template_operands_snapshot_before_a_writing_operand() {
	assert_eq!(
		int("pub fn main() { let x = 1; let v = [x, { x = 2; x }]; v[0] * 10 + v[1] }"),
		12
	);
	assert_eq!(
		int("pub fn main() { let x = 1; let t = (x, { x = 2; x }); t.0 * 10 + t.1 }"),
		12
	);
	assert_eq!(
		int("pub fn main() { let x = 1; let o = #{a: x, b: { x = 2; x }}; o.a * 10 + o.b }"),
		12
	);
	assert_eq!(
		text("pub fn main() { let x = 1; let s = format!(\"{}{}\", x, { x = 2; x }); s.len() }"),
		"2"
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn index_read_materialises_its_target_before_a_writing_index() {
	// The read target is evaluated in source order: the original vector is read.
	assert_eq!(
		int("pub fn main() { let v = [7, 8]; v[{ v = [9]; 0 }] }"),
		7
	);

	// A bare index is read in place.
	assert_eq!(int("pub fn main() { let v = [7, 8]; let i = 1; v[i] }"), 8);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn assignment_to_an_existing_local_is_not_written_early() {
	// The right side reads x after a short-circuit would have overwritten it.
	assert_eq!(
		run(
			"pub fn main() { let x = false; let y = true; x = y && x; x }",
			&[]
		),
		Value::Bool(false)
	);

	// A let binding may be written directly: nothing can read it first.
	assert_eq!(
		run(
			"pub fn main() { let x = false; let y = true; let z = y && x; z }",
			&[]
		),
		Value::Bool(false)
	);
	assert_eq!(
		run("pub fn main() { let x = false; let z = x || true; z }", &[]),
		Value::Bool(true)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn conditions_read_locals_in_place() {
	assert_eq!(
		int(
			"pub fn main() { let b = true; let n = 0; while b { n += 1; if n == 3 { b = false; } } n }"
		),
		3
	);
	assert_eq!(
		int("pub fn main() { let c = false; if c { 1 } else { 2 } }"),
		2
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn block_locals_and_temporaries_are_released_once_and_slots_are_reused() {
	assert_eq!(
		int(
			"pub fn main() { let v = [1]; let n = { let w = [2, 3]; w.len() as i64 }; n + v.len() as i64 }"
		),
		3
	);
	assert_eq!(
		int("pub fn main() { let x = 1; let x = x + 1; let x = x * 10; x }"),
		20
	);
	assert_eq!(
		int(
			"pub fn main() { let s = 0; let i = 0; while i < 50 { let t = [i, i]; s += t[1]; i += 1; } s }"
		),
		1225
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn loop_values_breaks_and_continues_release_body_slots() {
	assert_eq!(int("pub fn main() { let r = loop { break 7; }; r }"), 7);
	assert_eq!(
		int(
			"pub fn main() { let s = 0; for x in [1, 2, 3, 4] { let t = [x]; if x == 2 { continue; } if x == 4 { break; } s += t[0]; } s }"
		),
		4
	);
	assert_eq!(
		int(
			"pub fn main() { let s = 0; 'outer: for x in [1, 2] { for y in [10, 20] { let t = [y]; if y == 20 { break 'outer; } s += x + t[0]; } } s }"
		),
		11
	);
	assert_eq!(
		int("pub fn main() { let n = 0; loop { n += 1; if n < 5 { continue; } break; } n }"),
		5
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn iteration_leases_end_on_every_exit_path() {
	assert_eq!(
		int(
			"pub fn main() { let v = [1, 2]; let s = 0; for x in v { s += x; } v.push(3); s + v.len() as i64 }"
		),
		6
	);
	assert_eq!(
		int("pub fn main() { let v = [1, 2]; for x in v { break; } v.push(9); v.len() as i64 }"),
		3
	);
	assert_eq!(
		int(
			"pub fn main() { let v = [1, 2]; for x in v { if x == 1 { continue; } } v.push(9); v.len() as i64 }"
		),
		3
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn returns_from_nested_positions_and_tail_locals() {
	assert_eq!(
		int("pub fn main() { let a = if true { return 5; } else { 1 }; a }"),
		5
	);
	assert_eq!(int("pub fn main() { let x = 5; x }"), 5);
	assert_eq!(
		int(
			"fn f(v) { for x in v { if x == 2 { return x * 100; } } 0 } pub fn main() { f([1, 2, 3]) }"
		),
		200
	);
	assert_eq!(
		int("pub fn main() { let v = [1, 2]; let n = v.len() as i64; return n; }"),
		2
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn effect_statements_produce_no_unit_temporaries() {
	assert_eq!(
		int(
			"pub fn main() { let v = []; v.push(1); v.push(2); let s = 0; for x in v { s += x; } s }"
		),
		3
	);
	assert_eq!(
		int("pub fn main() { let n = 0; n += 2; n -= 1; n *= 10; n }"),
		10
	);
	assert_eq!(int("pub fn main() { let v = [1]; drop(v); 3 }"), 3);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn string_concatenation_results_are_released() {
	assert_eq!(
		text("pub fn main() { let a = \"ab\"; let b = a + \"cd\"; let c = b + a; c.len() }"),
		"6"
	);
	assert_eq!(
		text(
			"pub fn main() { let s = \"\"; let i = 0; while i < 20 { s += \"x\"; i += 1; } s.len() }"
		),
		"20"
	);
}

/// A store with room for exactly one entry: a heap value whose scope has ended
/// must have been released before the next one is admitted. The executor's
/// error cleanup empties the store, so only an in-run admission check catches
/// a lifetime that overran its scope.
fn one_entry(text: &str) -> Value {
	entries(1, text)
}

fn entries(entries: usize, text: &str) -> Value {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits {
		entries,
		bytes: 65_536,
	})
	.unwrap();
	let result = Executor::new()
		.unwrap()
		.run_values(&program.code, &mut store, main, &[], 100_000)
		.unwrap_or_else(|failure| panic!("{text}: {failure:?}"));
	assert_eq!(store.live_entries(), 0, "live entries after {text}");
	result
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn heap_values_written_on_one_path_are_released_when_their_scope_ends() {
	// Codex's reproducers (review of 5270735b): a scalar on the other path must
	// not hide the vector when the enclosing scope is released.
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = if true { [1] } else { 0 }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = if false { 0 } else { [1] }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = loop { if true { break [1]; } break 0; }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = loop { if false { break 0; } break [1]; }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);

	// Nested alternatives, both shapes.
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = if true { if false { 0 } else { [1] } } else { 0 }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = if false { 0 } else { if true { [1] } else { 0 } }; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);

	// A pre-existing scalar local that a branch rebinds to a vector.
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = 0; if true { v = [1]; } } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);

	// Zero iterations the other way round: the local holds a vector and the body
	// that would have written a scalar never ran; its flag must not be cleared.
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = [1]; while false { v -= 1; } } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);

	// Zero iterations: the loop body never ran, so the local still holds its scalar.
	assert_eq!(
		one_entry(
			"pub fn main() { { let v = 0; while false { v = [1]; } } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);

	// A back edge: the vector from the previous iteration is still bound while the
	// next one is built (two entries at that instant, as on the reference), is
	// released by the assignment, and the scope's release follows the loop: the
	// final vector needs the only entry left for `w`.
	assert_eq!(
		entries(
			2,
			"pub fn main() { { let v = 0; let i = 0; while i < 3 { v = [i]; i += 1; } } let w = [2]; let x = [3]; w.len() as i64 + x.len() as i64 }"
		),
		Value::I64(2)
	);

	// Per-iteration temporaries and a value kept across iterations.
	assert_eq!(
		one_entry(
			"pub fn main() { let i = 0; while i < 3 { let t = [i]; i += 1; } let w = [2]; w.len() as i64 }"
		),
		Value::I64(1)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_called_function_local_is_read_after_its_arguments() {
	// Reference: compile/v2/assemble.rs:2894 (SeqTail::CallVar) reads the callee
	// local after the argument run is assembled, so an argument that rebinds
	// the local selects the new callee. Not a snapshot.
	assert_eq!(
		int("fn g(x) { x } fn h(x) { x * 10 } pub fn main() { let f = g; f({ f = h; 2 }) }"),
		20
	);
	assert_eq!(int("fn g(x) { x } pub fn main() { let f = g; f(2) }"), 2);
}
