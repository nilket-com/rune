//! Macro certification under replacement.
//!
//! The fresh core admits only the two std format macros (`::std::fmt::format` and
//! `::std::io::println`), certified when the std modules register them. A host may install
//! another module with a macro at the same path afterwards; the stock `Context` then stores
//! the replacing handler. The fresh core must refuse such a call at compile time, before the
//! uncertified handler can run.
//!
//! Each custom macro counts its calls, so a test can tell whether the host callback ran.
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
