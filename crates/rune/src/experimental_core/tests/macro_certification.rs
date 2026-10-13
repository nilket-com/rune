//! Macro certification under replacement.
//!
//! The fresh core admits selected std formatting and assertion macros, certified
//! when the std modules register their actual handlers. A host may install
//! another module with a macro at the same path afterwards; the stock `Context` then stores
//! the replacing handler. The fresh core must refuse such a call at compile time, before the
//! uncertified handler can run.
//!
//! Each custom macro counts its calls, so a test can tell whether the host callback ran.

use crate as rune;
use crate::alloc::prelude::*;
use crate::experimental_core::frontend;
use crate::{Context, Diagnostics, Module, Source, Sources};
use std::sync::{
	atomic::{AtomicUsize, Ordering},
	Arc,
};

/// A module at `std::<path>` with a counting macro `name` that expands to its input.
fn counting_macro(path: &str, name: &'static str, calls: &Arc<AtomicUsize>) -> Module {
	let seen = calls.clone();
	let mut module = Module::with_crate_item("std", [path]).unwrap();
	module
		.macro_(
			name,
			move |_: &mut crate::macros::MacroContext<'_, '_, '_>,
			      input: &crate::macros::TokenStream| {
				seen.fetch_add(1, Ordering::SeqCst);
				Ok(input.try_clone()?)
			},
		)
		.unwrap();
	module
}

fn fresh_compile(context: &Context, text: &str) -> bool {
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1).is_ok()
}

const FORMAT: &str = "pub fn main() { format!(\"{}\", 42) }";
const PRINTLN: &str = "pub fn main() { println!(\"{}\", 42); 1 }";

fn output_source(name: &str) -> std::string::String {
	let path = if name == "panic" {
		"::std"
	} else {
		"::std::io"
	};
	let args = if name == "dbg" { "1,2" } else { "\"x\"" };
	std::format!("pub fn main() {{ {path}::{name}!({args}); 7 }}")
}

fn output_macro(name: &'static str, calls: Option<&Arc<AtomicUsize>>) -> Module {
	let mut module = if name == "panic" {
		Module::with_crate("std").unwrap()
	} else {
		Module::with_crate_item("std", ["io"]).unwrap()
	};
	if let Some(calls) = calls {
		let seen = calls.clone();
		module
			.macro_(
				name,
				move |_: &mut crate::macros::MacroContext<'_, '_, '_>,
				      input: &crate::macros::TokenStream| {
					seen.fetch_add(1, Ordering::SeqCst);
					Ok(input.try_clone()?)
				},
			)
			.unwrap();
	} else {
		match name {
			"panic" => {
				module
					.macro_meta(crate::modules::core::panic_macro)
					.unwrap();
			}
			"print" => {
				module.macro_meta(crate::modules::io::print_macro).unwrap();
			}
			"dbg" => {
				module.macro_meta(crate::modules::io::dbg_macro).unwrap();
			}
			_ => unreachable!(),
		}
		module.fresh_mark_builtin_macro();
	}
	module
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI checks output macro identity"
)]
fn output_macro_replacement_restore_partial_failure_and_unique_skip() {
	use crate::experimental_core::error::Refusal;
	{
		let name = "panic";
		let calls = Arc::new(AtomicUsize::new(0));
		let source = output_source(name);
		let mut context = Context::with_default_modules().unwrap();
		assert!(fresh_compile(&context, &source));
		context.install(output_macro(name, Some(&calls))).unwrap();
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		if name == "panic" {
			context
				.install(crate::modules::core::module().unwrap())
				.unwrap();
		} else {
			context
				.install(crate::modules::io::module(true).unwrap())
				.unwrap();
		}
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		context.install(output_macro(name, None)).unwrap();
		assert!(fresh_compile(&context, &source));
		let mut collision = if name == "panic" {
			Module::with_crate("std").unwrap()
		} else {
			Module::with_crate_item("std", ["io"]).unwrap()
		};
		collision.function("c2_collision", || {}).build().unwrap();
		context.install(collision).unwrap();
		let mut partial = output_macro(name, Some(&calls));
		partial.function("c2_collision", || {}).build().unwrap();
		assert!(context.install(partial).is_err());
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
}

#[test]
#[cfg_attr(miri, ignore = "std-module installation; native CI checks identity")]
fn output_custom_macro_first_is_overwritten_only_by_actual_builtin() {
	{
		let name = "panic";
		let calls = Arc::new(AtomicUsize::new(0));
		let mut context = Context::new();
		context.install(output_macro(name, Some(&calls))).unwrap();
		let hash = if name == "panic" {
			crate::Hash::type_hash(crate::item!(::std::panic))
		} else if name == "print" {
			crate::Hash::type_hash(crate::item!(::std::io::print))
		} else {
			crate::Hash::type_hash(crate::item!(::std::io::dbg))
		};
		assert!(!context.fresh_builtin_macro(hash));
		context.install(output_macro(name, None)).unwrap();
		assert!(context.fresh_builtin_macro(hash));
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
}

fn output_callee(name: &'static str, calls: &Arc<AtomicUsize>) -> Module {
	let mut module = if name == "panic" {
		Module::with_crate("std").unwrap()
	} else {
		Module::with_crate_item("std", ["io"]).unwrap()
	};
	let seen = calls.clone();
	module
		.function(name, move |_: crate::runtime::Value| {
			seen.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	module
}

#[test]
#[cfg_attr(
	miri,
	ignore = "std-module compiler; native CI checks insert-once IO callees"
)]
fn output_callee_custom_first_and_failed_replacement_stay_identity_bound() {
	use crate::experimental_core::error::Refusal;
	{
		let name = "panic";
		let calls = Arc::new(AtomicUsize::new(0));
		let mut first = Context::new();
		first.install(output_callee(name, &calls)).unwrap();
		if name == "panic" {
			assert!(first
				.install(crate::modules::core::module().unwrap())
				.is_err());
		} else {
			assert!(first
				.install(crate::modules::io::module(true).unwrap())
				.is_err());
		}
		first.install(output_macro(name, None)).unwrap();
		fresh_refusal(&first, &output_source(name), Refusal::Native);
		assert!(!match name {
			"panic" => first.fresh_panic(),
			"print" => false,
			_ => unreachable!(),
		});
		let mut after = Context::with_default_modules().unwrap();
		assert!(after.install(output_callee(name, &calls)).is_err());
		assert!(fresh_compile(&after, &output_source(name)));
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
}

#[cfg(all(feature = "capture-io", feature = "disable-io"))]
#[test]
#[cfg_attr(
	miri,
	ignore = "std-module compiler; native CI checks alternate output contexts"
)]
fn output_panic_admitted_but_alternate_print_and_dbg_contexts_refuse_before_output() {
	use crate::experimental_core::error::Refusal;
	for disabled in [true, false] {
		let mut context = Context::with_config(false).unwrap();
		let capture = crate::modules::capture_io::CaptureIo::new();
		if disabled {
			context
				.install(crate::modules::disable_io::module().unwrap())
				.unwrap();
		} else {
			context
				.install(crate::modules::capture_io::module(&capture).unwrap())
				.unwrap();
		}
		assert!(fresh_compile(&context, &output_source("panic")));
		for name in ["print", "dbg"] {
			fresh_refusal(&context, &output_source(name), Refusal::FrontendDiagnostic);
		}
		fresh_refusal(
			&context,
			"pub fn main(){print!(\"before\");panic!(\"after\");}",
			Refusal::FrontendDiagnostic,
		);
		assert!(capture.is_empty());
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn builtin_then_custom_format_is_refused_before_the_custom_handler_runs() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();

	// Stock semantics: the replacing install succeeds.
	context
		.install(counting_macro("fmt", "format", &calls))
		.unwrap();
	assert!(
		!fresh_compile(&context, FORMAT),
		"fresh compile admitted it"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0, "the custom handler ran");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn builtin_then_custom_println_is_refused_before_the_custom_handler_runs() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	context
		.install(counting_macro("io", "println", &calls))
		.unwrap();
	assert!(
		!fresh_compile(&context, PRINTLN),
		"fresh compile admitted it"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0, "the custom handler ran");
}

/// The replacing module fails part-way: its macro item is installed, then a later item
/// conflicts with the builtin `println` function. The stored macro handler is the custom one.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_partial_replacing_install_is_refused_before_the_custom_handler_runs() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = counting_macro("io", "println", &calls);
	module.function("println", |_: &str| {}).build().unwrap();
	assert!(
		context.install(module).is_err(),
		"the conflicting install succeeded"
	);
	assert!(
		!fresh_compile(&context, PRINTLN),
		"fresh compile admitted it"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0, "the custom handler ran");
}

/// Custom first, builtin second: the builtin replaces the custom handler and is the one
/// stored, so the certified expansion is admitted and the custom handler never runs.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn custom_then_builtin_format_admits_the_builtin() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::new();
	context
		.install(counting_macro("fmt", "format", &calls))
		.unwrap();
	// The std modules up to `fmt`, in `Context::with_config`'s order (later ones need the
	// types and traits of earlier ones).
	use crate::modules as m;

	for module in [
		m::iter::module(),
		m::core::module(),
		m::cmp::module(),
		m::any::module(),
		m::clone::module(),
		m::num::module(),
		m::hash::module(),
		m::string::module(),
		m::bytes::module(),
		m::collections::module(),
		m::collections::hash_map::module(),
		m::collections::hash_set::module(),
		m::collections::vec_deque::module(),
		m::char::module(),
		m::f64::module(),
		m::f64::consts::module(),
		m::tuple::module(),
		m::fmt::module(),
	] {
		context.install(module.unwrap()).unwrap();
	}

	// No std prelude on this Context, so the macro is named by its full path.
	let text = "pub fn main() { ::std::fmt::format!(\"{}\", 42) }";
	assert!(
		fresh_compile(&context, text),
		"fresh compile refused the builtin"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0, "the custom handler ran");
}

/// Custom first, builtin second, for `println!`: the std io module (stdio) replaces the
/// custom macro and is certified, so `println!` is admitted again.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn custom_then_builtin_println_admits_the_builtin() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::new();
	context
		.install(counting_macro("io", "println", &calls))
		.unwrap();
	use crate::modules as m;

	for module in [
		m::iter::module(),
		m::core::module(),
		m::cmp::module(),
		m::any::module(),
		m::clone::module(),
		m::num::module(),
		m::hash::module(),
		m::string::module(),
		m::bytes::module(),
		m::collections::module(),
		m::collections::hash_map::module(),
		m::collections::hash_set::module(),
		m::collections::vec_deque::module(),
		m::char::module(),
		m::f64::module(),
		m::f64::consts::module(),
		m::tuple::module(),
		m::fmt::module(),
		m::future::module(),
		m::i64::module(),
		m::u64::module(),
		m::io::module(true),
	] {
		context.install(module.unwrap()).unwrap();
	}

	let text = "pub fn main() { ::std::io::println!(\"{}\", 42); 1 }";
	assert!(
		fresh_compile(&context, text),
		"fresh compile refused the builtin"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0, "the custom handler ran");
}

/// Stock behaviour, recorded and unchanged: the ordinary (non-fresh) build expands the call
/// with the replacing custom handler.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn stock_build_uses_the_replacing_custom_macro() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	context
		.install(counting_macro("fmt", "format", &calls))
		.unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(FORMAT).unwrap()).unwrap();
	let _ = crate::prepare(&mut sources).with_context(&context).build();
	assert!(
		calls.load(Ordering::SeqCst) > 0,
		"stock build did not call it"
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default module compilation; native CI covers certificates"
)]
fn assertion_callees_are_certified_only_as_actual_std_registrations() {
	let context = Context::with_default_modules().unwrap();
	assert!(context.fresh_panic());
	assert!(context.fresh_string_from());
}

fn assertion_source(name: &str) -> std::string::String {
	let args = match name {
		"assert" => "true",
		"assert_eq" => "1,1",
		"assert_ne" => "1,2",
		_ => unreachable!(),
	};
	std::format!("pub fn main() {{ ::std::test::{name}!({args}); 7 }}")
}
fn fresh_refusal(
	context: &Context,
	text: &str,
	expected: crate::experimental_core::error::Refusal,
) {
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let error = frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1)
		.err()
		.expect("must refuse");
	assert_eq!(error.refusal, Some(expected), "{error:?}");
}
fn builtin_assertion(name: &str) -> Module {
	let mut m = Module::with_crate_item("std", ["test"]).unwrap();
	match name {
		"assert" => {
			m.macro_meta(crate::modules::test::assert).unwrap();
		}
		"assert_eq" => {
			m.macro_meta(crate::modules::test::assert_eq).unwrap();
		}
		"assert_ne" => {
			m.macro_meta(crate::modules::test::assert_ne).unwrap();
		}
		_ => unreachable!(),
	}
	m.fresh_mark_builtin_macro();
	m
}
#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler identity matrix; native CI covers it"
)]
fn assertion_macro_identity_replacement_partial_install_restore_and_unique_skip() {
	use crate::experimental_core::error::Refusal;
	for name in ["assert", "assert_eq", "assert_ne"] {
		let source = assertion_source(name);
		let calls = Arc::new(AtomicUsize::new(0));
		let mut context = Context::with_default_modules().unwrap();
		assert!(fresh_compile(&context, &source));
		context
			.install(counting_macro("test", name, &calls))
			.unwrap();
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		// The unique std::test module was already installed: skipping it cannot
		// recertify the replacing callback now stored at the macro hash.
		context
			.install(crate::modules::test::module().unwrap())
			.unwrap();
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		context.install(builtin_assertion(name)).unwrap();
		assert!(fresh_compile(&context, &source));
		let mut collision = Module::with_crate_item("std", ["test"]).unwrap();
		collision.function("collision", || {}).build().unwrap();
		context.install(collision).unwrap();
		let mut partial = counting_macro("test", name, &calls);
		partial.function("collision", || {}).build().unwrap();
		assert!(context.install(partial).is_err());
		fresh_refusal(&context, &source, Refusal::FrontendDiagnostic);
		assert_eq!(calls.load(Ordering::SeqCst), 0, "custom {name} executed");
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler identity matrix; native CI covers it"
)]
fn insert_once_panic_custom_first_is_never_certified() {
	use crate::experimental_core::error::Refusal;
	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let mut context = Context::new();
	let mut custom = Module::with_crate("std").unwrap();
	custom
		.function("panic", move |_: crate::runtime::Value| {
			seen.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	context.install(custom).unwrap();
	assert!(context
		.install(crate::modules::core::module().unwrap())
		.is_err());
	context
		.install(crate::modules::test::module().unwrap())
		.unwrap();
	assert!(!context.fresh_panic());
	// The failed core installation already consumed its unique-module id.
	// Skipping it later must not certify the custom handler that remains stored.
	context
		.install(crate::modules::core::module().unwrap())
		.unwrap();
	assert!(!context.fresh_panic());
	fresh_refusal(&context, &assertion_source("assert"), Refusal::Native);
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler identity matrix; native CI covers it"
)]
fn insert_once_panic_custom_after_builtin_preserves_builtin_identity() {
	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let mut context = Context::with_default_modules().unwrap();
	let mut custom = Module::with_crate("std").unwrap();
	custom
		.function("panic", move |_: crate::runtime::Value| {
			seen.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	assert!(context.install(custom).is_err());
	assert!(context.fresh_panic());
	assert!(fresh_compile(&context, &assertion_source("assert")));
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI covers NaN macro operand semantics"
)]
fn nan_assertion_operands_work_without_admitting_new_source_constants() {
	use crate::experimental_core::{
		error::ErrorKind,
		execute::Executor,
		heap::{Limits, Store},
		value::Scalar,
	};
	let context = Context::with_default_modules().unwrap();
	for name in ["assert_eq", "assert_ne"] {
		let source = std::format!("pub fn target(a,b) {{ {name}!(a,b); 17 }}");
		let (mut stock, program) =
			super::differential::compile_unit(&context, "assert-nan", &source, "target");
		let expected = stock.call(["target"], (f64::NAN, f64::NAN));
		let mut store = Store::new(Limits::default()).unwrap();
		let a = store.root_f64(f64::NAN);
		let b = store.root_f64(f64::NAN);
		let mut executor = Executor::new().unwrap();
		let actual = executor.invoke(
			&program,
			&mut store,
			program
				.function(crate::Hash::type_hash(["target"]))
				.unwrap(),
			&[&a, &b],
			100,
		);
		if name == "assert_eq" {
			let expected = std::format!("{}", expected.unwrap_err().into_kind());
			let actual = actual.unwrap_err();
			assert_eq!(actual.error.kind, ErrorKind::Panic);
			assert_eq!(std::format!("{}", actual.message(&program)), expected);
		} else {
			assert_eq!(crate::from_value::<i64>(expected.unwrap()).unwrap(), 17);
			let actual = actual.unwrap();
			assert_eq!(store.root_scalar(&actual).unwrap(), Some(Scalar::I64(17)));
			store.root_release(actual).unwrap();
		}
		store.root_release(a).unwrap();
		store.root_release(b).unwrap();
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI covers source shadow resolution"
)]
fn source_std_spelling_never_intercepts_the_certified_crate_function() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Scalar,
	};
	let context = Context::with_default_modules().unwrap();
	let source =
		"mod std { pub fn panic(message) { 9 } }\npub fn target() { std::panic(\"local\") }";
	let (mut stock, program) =
		super::differential::compile_unit(&context, "assert-shadow", source, "target");
	assert_eq!(
		crate::from_value::<i64>(stock.call(["target"], ()).unwrap()).unwrap(),
		9
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program
				.function(crate::Hash::type_hash(["target"]))
				.unwrap(),
			&[],
			100,
		)
		.unwrap();
	assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(9)));
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI covers constructor identity"
)]
fn string_constructor_custom_after_builtin_preserves_builtin_identity() {
	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let mut context = Context::with_default_modules().unwrap();
	let mut custom = Module::with_crate_item("std", ["string", "String"]).unwrap();
	custom
		.function("from", move |_: crate::runtime::Value| {
			seen.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	let error = context.install(custom).unwrap_err();
	assert!(
		matches!(error, crate::ContextError::ConflictingFunction { .. }),
		"{error:?}"
	);
	assert!(context.fresh_string_from());
	assert!(fresh_compile(&context, &assertion_source("assert_eq")));
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI covers constructor identity"
)]
fn string_constructor_custom_first_failed_builtin_never_certifies() {
	use crate::experimental_core::error::Refusal;
	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let mut context = Context::new();
	context
		.install(crate::modules::core::module().unwrap())
		.unwrap();
	let mut custom = Module::with_crate_item("std", ["string", "String"]).unwrap();
	custom
		.function("from", move |_: crate::runtime::Value| {
			seen.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	context.install(custom).unwrap();
	let error = context
		.install(crate::modules::string::module().unwrap())
		.unwrap_err();
	assert!(
		matches!(error, crate::ContextError::ConflictingFunction { .. }),
		"{error:?}"
	);
	assert!(!context.fresh_string_from());
	fresh_refusal(
		&context,
		"pub fn main() { ::std::string::String::from(\"x\") }",
		Refusal::Native,
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "default-module compiler; native CI covers constructor copying"
)]
fn certified_string_constructor_copies_and_checks_its_argument() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
	};
	let context = Context::with_default_modules().unwrap();
	let source = "pub fn target() { let a=\"a\"; let b=String::from(a); b+=\"b\"; (a,b) }";
	let (mut stock, program) =
		super::differential::compile_unit(&context, "string-copy", source, "target");
	let expected = stock.call(["target"], ()).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program
				.function(crate::Hash::type_hash(["target"]))
				.unwrap(),
			&[],
			100,
		)
		.unwrap();
	let fresh = store.root_format(&root, true).unwrap();
	let mut text = crate::alloc::String::new();
	stock
		.with(|| crate::runtime::Formatter::format_with(&mut text, |f| expected.debug_fmt(f)))
		.unwrap();
	assert_eq!(fresh, text);
	assert_eq!(fresh.as_str(), "(\"a\", \"ab\")");
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
	for operand in ["1", "()", "[1]"] {
		let source = std::format!("pub fn target() {{ String::from({operand}) }}");
		let (mut stock, program) =
			super::differential::compile_unit(&context, "string-type", &source, "target");
		let expected = std::format!("{}", stock.call(["target"], ()).unwrap_err().into_kind());
		let actual = executor
			.invoke(
				&program,
				&mut store,
				program
					.function(crate::Hash::type_hash(["target"]))
					.unwrap(),
				&[],
				100,
			)
			.unwrap_err();
		assert_eq!(std::format!("{}", actual.message(&program)), expected);
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
	}
}
