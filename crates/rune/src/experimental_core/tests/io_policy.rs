//! IO policy: refuse unless the certified stdio builtin is installed.
//!
//! Invariant under test: the core admits `println!` iff
//! `Context::fresh_stdio_println()` is true, which holds iff the handler stored at
//! `::std::io::println` was inserted by an item of a module marked by
//! `modules::io::module(true)`. Functions are insert-once (`ConflictingFunction`), and a
//! module whose `unique` id was already installed is skipped entirely, so the marker cannot
//! certify a handler that some other module inserted.

use crate::experimental_core::error::Refusal;
use crate::experimental_core::execute::Executor;
use crate::experimental_core::frontend;
use crate::experimental_core::heap::{Limits, Store};
use crate::{Context, Diagnostics, Hash, Module, Source, Sources};

const PRINTS: &str = "pub fn main() { println!(\"first\"); println!(\"{:?}\", [1, 2]); 1 / 0 }";

/// `println!` in value position (the lowering's `call` path, not the statement path).
const PRINT_VALUE: &str = "pub fn main() { let x = println!(\"first\"); x }";

/// A direct call, for contexts without the std `println!` macro.
const PRINT_DIRECT: &str = "pub fn main() { std::io::println(\"first\"); 1 }";

fn outcome(context: &Context, text: &str) -> Result<(), Option<Refusal>> {
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();

	match frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1) {
		Ok(_) => Ok(()),
		Err(e) => Err(e.refusal),
	}
}

/// Whether both `println!` positions are admitted. A refusal must be the IO policy's
/// (`Native`), the same in both positions.
fn admitted(context: &Context) -> bool {
	let statement = outcome(context, PRINTS);
	let value = outcome(context, PRINT_VALUE);
	assert_eq!(statement, value, "statement and value positions agree");

	match statement {
		Ok(()) => true,
		Err(refusal) => {
			assert_eq!(
				refusal,
				Some(Refusal::Native),
				"refused for the IO policy only"
			);
			false
		}
	}
}

/// With no `::std::io::println` function installed, the Rune front end itself rejects
/// the macro's call as unresolved, before lowering: a `FrontendDiagnostic` refusal.
fn refused_as_unresolved(context: &Context) {
	for text in [PRINTS, PRINT_VALUE] {
		assert_eq!(
			outcome(context, text),
			Err(Some(Refusal::FrontendDiagnostic)),
			"{text}"
		);
	}
}

fn custom_println() -> Module {
	let mut m = Module::with_crate_item("std", ["io"]).unwrap();
	m.function("println", |_: &str| {}).build().unwrap();
	m
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn default_modules_certify_the_stdio_builtin() {
	let context = Context::with_default_modules().unwrap();
	assert!(context.fresh_stdio_println());
	assert!(admitted(&context));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn no_printer_installed_is_refused() {
	let context = Context::with_config(false).unwrap();
	assert!(!context.fresh_stdio_println());
	refused_as_unresolved(&context);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn unique_skip_does_not_install_or_certify() {
	// with_config(false) already installed io::module(false) under unique "std::io";
	// installing io::module(true) afterwards is skipped entirely (Ok, nothing changes).
	let mut context = Context::with_config(false).unwrap();
	context
		.install(crate::modules::io::module(true).unwrap())
		.unwrap();
	assert!(!context.fresh_stdio_println());
	refused_as_unresolved(&context);
}

#[cfg(feature = "capture-io")]
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn capture_is_refused() {
	let io = crate::modules::capture_io::CaptureIo::new();
	let mut context = Context::with_config(false).unwrap();
	context
		.install(crate::modules::capture_io::module(&io).unwrap())
		.unwrap();
	assert!(!admitted(&context));
	assert!(io.is_empty());
}

#[cfg(feature = "disable-io")]
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn disable_is_refused() {
	let mut context = Context::with_config(false).unwrap();
	context
		.install(crate::modules::disable_io::module().unwrap())
		.unwrap();
	assert!(!admitted(&context));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn custom_printer_is_refused() {
	let mut context = Context::with_config(false).unwrap();
	context.install(custom_println()).unwrap();
	assert!(!admitted(&context));
}

#[test]
fn custom_first_then_builtin_conflicts_and_stays_uncertified() {
	let mut context = Context::new();
	context.install(custom_println()).unwrap();
	assert!(context
		.install(crate::modules::io::module(true).unwrap())
		.is_err());
	assert!(!context.fresh_stdio_println());

	// `Context::new()` has no `println!` macro, and the partial install stopped before
	// it; call the custom function directly.
	assert_eq!(outcome(&context, PRINT_DIRECT), Err(Some(Refusal::Native)));
}

#[cfg(feature = "capture-io")]
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn builtin_first_then_capture_conflicts_and_builtin_remains() {
	let io = crate::modules::capture_io::CaptureIo::new();
	let mut context = Context::with_default_modules().unwrap();
	assert!(context
		.install(crate::modules::capture_io::module(&io).unwrap())
		.is_err());
	assert!(context.fresh_stdio_println());
	assert!(admitted(&context));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn refusal_happens_before_any_execution_or_output() {
	// Compile-time: no Program exists, so no executor can run and nothing can print.
	let context = Context::with_config(false).unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(PRINTS).unwrap()).unwrap();
	assert!(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).is_err());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn output_order_and_partial_output_on_failure_via_the_test_seam() {
	// Earlier lines survive a later failure. The seam is per executor.
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(PRINTS).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor.test_output = Some(std::vec::Vec::new());
	let failure = executor
		.invoke(&program, &mut store, main, &[], 1_000)
		.unwrap_err();
	assert_eq!(
		failure.error.kind,
		crate::experimental_core::error::ErrorKind::DivisionByZero
	);
	assert_eq!(
		executor.test_output.as_deref(),
		Some(&b"first\n[1, 2]\n"[..])
	);
}

fn compile_program(text: &str) -> frontend::Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap()
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn output_has_no_cumulative_cap() {
	let program = compile_program(
		r#"pub fn main(a) { let i = 0; while i < 256 { println!("{}", a); i += 1; } 42 }"#,
	);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let arg = store.root_string(&"a".repeat(4095)).unwrap();
	let mut executor = Executor::new().unwrap();
	executor.test_output = Some(std::vec::Vec::new());
	let result = executor
		.invoke(&program, &mut store, main, &[&arg], 100_000)
		.unwrap();
	assert_eq!(
		store.root_scalar(&result).unwrap(),
		Some(super::super::value::Scalar::I64(42))
	);
	assert_eq!(
		executor.test_output.as_deref(),
		Some(format!("{}\n", "a".repeat(4095)).repeat(256).as_bytes())
	);
	assert!(executor.is_idle());
	store.root_release(result).unwrap();
	store.root_release(arg).unwrap();
	assert_eq!(store.live_entries(), 0);
}

/// Repeated invocations preserve output order and reusable executor state.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn repeated_output_preserves_order() {
	let program = compile_program(
        "pub fn main(_a) { let s = \"x\"; let i = 0; while i < 16 { s += s; i += 1; } println!(\"{}\", s); }",
    );
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let unit = store.root_unit();
	let mut executor = Executor::new().unwrap();
	executor.test_output = Some(std::vec::Vec::new());
	let baseline = (store.live_entries(), store.payload_bytes());

	for _ in 0..3 {
		let out = executor
			.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
			.unwrap();
		store.root_release(out).unwrap();
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	}

	let mut line = "x".repeat(65536);
	line.push('\n');
	assert_eq!(
		executor.test_output.as_deref(),
		Some(line.repeat(3).as_bytes())
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn print_macro_is_refused_before_execution() {
	let context = Context::with_default_modules().unwrap();
	assert_eq!(
		outcome(&context, r#"pub fn main() { print!("a"); 42 }"#),
		Err(Some(Refusal::FrontendDiagnostic))
	);
}
