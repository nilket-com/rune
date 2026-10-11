//! Local source-to-core construction tests, not the reviewed A-E conformance run.

use crate::experimental_core::{
	error::{ErrorKind, Refusal},
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::Value,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::string::ToString;

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
		.run_values(&program.code, &mut store, main, args, 10_000)
		.unwrap();
	assert_eq!(store.live_entries(), 0);
	result
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_arguments_locals_and_if() {
	assert_eq!(
		run(
			"pub fn main(n) { let x = n + 2; if x > 3 { x * 4 } else { 0 } }",
			&[Value::I64(5)]
		),
		Value::I64(28)
	);
	assert_eq!(
		run(
			"pub fn main(n) { let x = n + 2; if x > 3 { x * 4 } else { 0 } }",
			&[Value::I64(0)]
		),
		Value::I64(0)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn direct_arithmetic_operands_keep_snapshot_and_live_local_rules() {
	assert_eq!(
		run("pub fn main(x) { -x + x }", &[Value::I64(7)]),
		Value::I64(0)
	);
	assert_eq!(
		run(
			"pub fn main() { let x = 3; let y = x + { x = 7; x }; y * 10 + x }",
			&[]
		),
		Value::I64(107)
	);
	assert_eq!(
		run("pub fn main() { let x = 3; ({ x = 7; 2 }) + x }", &[]),
		Value::I64(9)
	);
	assert_eq!(
		run("pub fn main() { let x = 3; x + x; x + 1 }", &[]),
		Value::I64(4)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_forward_and_recursive_functions() {
	assert_eq!(
		run(
			"pub fn main() { add(20, recurse(2)) } fn add(a,b) { a + b } fn recurse(n) { if n == 0 { 20 } else { recurse(n-1) + 1 } }",
			&[]
		),
		Value::I64(42)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_vector_alias_and_short_circuit() {
	assert_eq!(
		run(
			"pub fn main() { let a = [1,2]; let b = a; b[0] = 42; a[0] }",
			&[]
		),
		Value::I64(42)
	);
	assert_eq!(
		run("pub fn main() { false && (1 / 0 == 0) }", &[]),
		Value::Bool(false)
	);
	assert_eq!(
		run("pub fn main() { true || (1 / 0 == 0) }", &[]),
		Value::Bool(true)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn construction_refuses_constants_closures_and_macros() {
	let context = Context::with_default_modules().unwrap();
	for text in [
		"const A = [42]; pub fn main() { A }",
		"pub fn main() { let f = || 42; f() }",
		"pub fn main() { format!(\"{:x}\",42) }",
	] {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		assert_eq!(
			frontend::compile(&mut sources, &context, &mut diagnostics, 7)
				.err()
				.unwrap()
				.kind,
			ErrorKind::Unsupported
		);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_restricted_scalar_constants() {
	assert_eq!(
		run(
			"const A = 20; const B = A + 1; pub fn main() { B * 2 }",
			&[]
		),
		Value::I64(42)
	);
	assert_eq!(
		run("const A = false && (1 / 0 == 0); pub fn main() { A }", &[]),
		Value::Bool(false)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_loops_break_continue_and_labels() {
	assert_eq!(
		run(
			"pub fn main() { let n=0; let sum=0; while n<10 { n+=1; if n==5 { continue; } sum+=n; } sum }",
			&[]
		),
		Value::I64(50)
	);
	assert_eq!(
		run(
			"pub fn main() { let n=0; let result='outer: loop { n+=1; loop { if n==3 { break 'outer 42; } else { continue 'outer; } } }; result }",
			&[]
		),
		Value::I64(42)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_allowlisted_methods_and_type_operations() {
	assert_eq!(
		run(
			"pub fn main() { let a = [1]; let b = a.clone(); b.push(2); a.len()*10+b.len() }",
			&[]
		),
		Value::U64(12)
	);
	assert_eq!(
		run(
			"pub fn main() { let a=\"a\"; let b=a; a+=\"b\"; b==\"ab\" }",
			&[]
		),
		Value::Bool(true)
	);
	assert_eq!(
		run("pub fn main() { (42 as f64) is f64 }", &[]),
		Value::Bool(true)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_bundle_outlives_frontend_and_keeps_revision() {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	let text = "pub fn main(n) { n / 0 }";
	sources
		.insert(Source::new("cell-2.rn", text).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 17).unwrap();
	drop(sources);
	assert_eq!(program.sources[0].name, "cell-2.rn");
	assert_eq!(program.sources[0].text, text);
	assert_eq!(program.sources[0].revision, 17);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let error = Executor::new()
		.unwrap()
		.run_values(&program.code, &mut store, main, &[Value::I64(42)], 100)
		.unwrap_err();
	assert_eq!(error.error.kind, ErrorKind::DivisionByZero);
	let location = error.error.location.unwrap();
	assert_eq!(location.revision, 17);
	assert_eq!(
		&program.sources[0].text[location.start as usize..location.end as usize],
		"n / 0"
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_range_and_vector_for_cleanup() {
	assert_eq!(
		run(
			"pub fn main() { let sum=0; for n in 0..10 { if n==5 { continue; } sum+=n; } sum }",
			&[]
		),
		Value::I64(40)
	);
	assert_eq!(
		run(
			"pub fn main() { let sum=0; for n in [10,20,12] { sum+=n; } sum }",
			&[]
		),
		Value::I64(42)
	);
	assert_eq!(
		run(
			"pub fn main() { let a=[1,2]; for n in a { break; } a.push(42); a[2] }",
			&[]
		),
		Value::I64(42)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_vector_lease_conflict_unwinds_before_reuse() {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(a) { for n in a { a.push(n); } }").unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 1).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let value = store
		.list(crate::experimental_core::value::Type::Vec, &[Value::I64(7)])
		.unwrap();
	let failure = Executor::new()
		.unwrap()
		.run_values(&program.code, &mut store, main, &[value], 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
	assert!(failure.cleanup_error.is_none());
	store.push(value, Value::I64(42)).unwrap();
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_named_function_tokens_use_new_frames_only() {
	assert_eq!(
		run(
			"fn add(a,b) { a+b } pub fn main() { let f=add; f(20,22) }",
			&[]
		),
		Value::I64(42)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_builtin_variants_and_question_propagation() {
	assert_eq!(run("pub fn main() { Ok(42)? }", &[]), Value::I64(42));
	assert_eq!(run("pub fn main() { Some(42)? }", &[]), Value::I64(42));

	for (text, tag, payload) in [
		(
			"fn f() { Err(7)? } pub fn main() { f()?; 42 }",
			crate::experimental_core::value::Type::Result,
			Some(Value::I64(7)),
		),
		(
			"fn f() { None? } pub fn main() { f()?; 42 }",
			crate::experimental_core::value::Type::Option,
			None,
		),
	] {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		let program = frontend::compile(&mut sources, &context, &mut diagnostics, 1).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits {
			entries: 32,
			bytes: 4096,
		})
		.unwrap();
		let value = Executor::new()
			.unwrap()
			.run_values(&program.code, &mut store, main, &[], 100)
			.unwrap();
		assert_eq!(store.variant_parts(value).unwrap(), (tag, false, payload));
		store.release(value).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_object_and_struct_shared_fields_and_outer_clone() {
	assert_eq!(
		run(
			"struct A { x } struct B { x } pub fn main() { let a=A { x:42 }; (a is A) && (a is not B) }",
			&[]
		),
		Value::Bool(true)
	);
	assert_eq!(
		run(
			"pub fn main() { let a=#{x: 1, inner: [2]}; let b=a; b.x=4; let c=a.clone(); c.x=10; c.inner[0]=3; a.x+a.inner[0]+c.x }",
			&[]
		),
		Value::I64(17)
	);
	assert_eq!(
		run(
			"struct Row { amount, count } pub fn main() { let a=Row { count: 2, amount: 40 }; let b=a; b.count=3; a.amount+a.count }",
			&[]
		),
		Value::I64(43)
	);
	assert_eq!(
		run(
			"pub fn main() { let a=(1,2); let b=a; b.0=40; a.0+a.1 }",
			&[]
		),
		Value::I64(42)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_assignment_preserves_target_and_rhs_evaluation_order() {
	assert_eq!(
		run("pub fn main() { let n=1; n += { n=10; 2 }; n }", &[]),
		Value::I64(12)
	);
	assert_eq!(
		run(
			"pub fn main() { let a=[1]; let b=[2]; let target=a; target[0] = { target=b; 42 }; a[0]*10+b[0] }",
			&[]
		),
		Value::I64(52)
	);

	// A call target is materialised before the RHS and keeps the earlier handle.
	assert_eq!(
		run(
			"fn identity(x) { x } pub fn main() { let a=[1]; let b=[2]; let target=a; identity(target)[0] = { target=b; 42 }; a[0]*10+b[0] }",
			&[]
		),
		Value::I64(422)
	);

	// The source-reviewed adjacent-shape reference follow-up confirms this too.
	assert_eq!(
		run(
			"pub fn main() { let a=#{x:1}; let b=#{x:2}; let target=a; target.x = { target=b; 42 }; a.x*10+b.x }",
			&[]
		),
		Value::I64(52)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn candidate_local_program_and_refused_format_spec() {
	assert_eq!(
		run(
			"const A=21; pub fn main() { let r=#{value:A}; helper(r.value)*2 } fn helper(n) { let sum=0; for x in [n] { sum+=x; } sum }",
			&[]
		),
		Value::I64(42)
	);
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main() { format!(\"{:x}\",42) }").unwrap())
		.unwrap();
	assert!(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 9).is_err());
}

#[test]
fn a_custom_macro_at_the_builtin_path_is_not_trusted() {
	use crate::alloc::prelude::*;
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let mut context = Context::new();
	let mut module = crate::Module::with_crate_item("std", ["fmt"]).unwrap();
	module
		.macro_(
			"format",
			move |_: &mut crate::macros::MacroContext<'_, '_, '_>,
			      input: &crate::macros::TokenStream| {
				seen.fetch_add(1, Ordering::SeqCst);
				Ok(input.try_clone()?)
			},
		)
		.unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main() { ::std::fmt::format!(\"{}\",42) }").unwrap())
		.unwrap();
	assert!(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).is_err());
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn trusted_format_expansion_refuses_nonliteral_and_dynamic_precision() {
	let context = Context::with_default_modules().unwrap();
	for (text, message) in [
		(
			"const F=\"{}\"; pub fn main() { format!(F,42) }",
			"format string must be a literal",
		),
		(
			"pub fn main() { format!(\"{:.*}\",2,1.25) }",
			"dynamic precision is not admitted",
		),
	] {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		assert!(frontend::compile(&mut sources, &context, &mut diagnostics, 1).is_err());
		assert!(
			diagnostics.diagnostics().iter().any(|d| match d {
				crate::diagnostics::Diagnostic::Fatal(error) => error.to_string().contains(message),
				_ => false,
			}),
			"missing refusal: {message}"
		);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn deferred_local_index_and_compound_assignment_regressions() {
	for (source, expected) in [
		(
			"pub fn main() { let v=[0,0]; let i=0; v[i]={i=1;100}; v[0]*1000+v[1] }",
			100,
		),
		(
			"pub fn main() { let v=[0,0]; let w=[0,0]; let orig=v; let i=0; v[i]={v=w;i=1;100}; orig[0]*1000000+orig[1]*10000+w[0]*1000+w[1] }",
			100,
		),
		(
			"pub fn main() { let v=[1]; let w=[0]; let orig=v; v[0]+={v=w;100}; orig[0]*1000+w[0] }",
			1100,
		),
		(
			"pub fn main() { let o=#{x:1}; let p=#{x:0}; let orig=o; o.x+={o=p;100}; orig.x*1000+p.x }",
			1100,
		),
		(
			"fn key(counter) { counter[0]+=1; 0 } pub fn main() { let n=[0]; let v=[1]; v[key(n)]+=2; n[0]*100+v[0] }",
			103,
		),
		(
			"fn identity(v) { v } pub fn main() { let a=[1]; let b=[0]; let target=a; identity(target)[0]+={target=b;100}; a[0]*1000+b[0] }",
			101000,
		),
	] {
		assert_eq!(run(source, &[]), Value::I64(expected));
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn source_literal_formatting_uses_core_renderer() {
	assert_eq!(
		run(
			"pub fn main() { format!(\"{} {:?}\",42,[1,2]) == \"42 [1, 2]\" }",
			&[]
		),
		Value::Bool(true)
	);
	assert_eq!(
		run(
			"pub fn main() { let x=42; `value=${x}` == \"value=42\" }",
			&[]
		),
		Value::Bool(true)
	);
	assert_eq!(
		run(
			"pub fn main() { format!(\"{:?}\",Some('a')) == \"Some('a')\" }",
			&[]
		),
		Value::Bool(true)
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn all_template_arguments_run_before_a_format_error() {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(n) { format!(\"{} {}\",(),{ n[0]+=1; 1 }) }").unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let n = store
		.list(crate::experimental_core::value::Type::Vec, &[Value::I64(0)])
		.unwrap();
	let failure = Executor::new()
		.unwrap()
		.run_values(&program.code, &mut store, main, &[n], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	assert_eq!(store.get(n, 0).unwrap(), Value::I64(1));
	assert_eq!(store.live_entries(), 1);
	store.release(n).unwrap();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn named_refusals_keep_source_revision_and_syntax_span() {
	let context = Context::with_default_modules().unwrap();
	for (body, operation, syntax) in [
		("match n { _ => 0 }", "Match", "match"),
		("let f = || 42; f()", "CallClosure", "||"),
		("let f = async { 42 }; 0", "AsyncBlock", "async"),
	] {
		let text = format!("pub fn main(n) {{ {body} }}");
		let mut sources = Sources::new();
		sources.insert(Source::memory(&text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		let error = frontend::compile(&mut sources, &context, &mut diagnostics, 91)
			.err()
			.unwrap();
		assert_eq!(error.kind, ErrorKind::Unsupported);
		assert_eq!(error.operation, operation);
		assert_eq!(error.refusal.unwrap().name(), operation);
		let at = error.location.unwrap();
		assert_eq!(at.source, 0);
		assert_eq!(at.revision, 91);
		assert!(
			text[at.start as usize..at.end as usize].contains(syntax),
			"{operation}: {:?}",
			&text[at.start as usize..at.end as usize]
		);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn unadmitted_native_call_is_refused_at_its_call_site() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["host"]).unwrap();
	module.function("forbidden", || 42i64).build().unwrap();
	context.install(module).unwrap();

	for body in ["host::forbidden()", "let f = host::forbidden; f()"] {
		let text = format!("pub fn main() {{ {body} }}");
		let mut sources = Sources::new();
		sources.insert(Source::memory(&text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		let error = frontend::compile(&mut sources, &context, &mut diagnostics, 92)
			.err()
			.unwrap();
		assert_eq!(error.kind, ErrorKind::Unsupported);
		assert_eq!(error.operation, "Native");
		assert_eq!(error.refusal, Some(Refusal::Native));
		let at = error.location.unwrap();
		assert_eq!(at.revision, 92);
		assert!(text[at.start as usize..at.end as usize].contains("host::forbidden"));
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn type_tests_on_function_and_type_values_are_boolean() {
	assert_eq!(
		run(
			"pub fn main() { let f = foo; f is i64 } fn foo() { 42 }",
			&[]
		),
		Value::Bool(false)
	);
	assert_eq!(run("pub fn main() { i64 is i64 }", &[]), Value::Bool(false));
	assert_eq!(
		run("pub fn main() { i64 is not i64 }", &[]),
		Value::Bool(true)
	);
}
