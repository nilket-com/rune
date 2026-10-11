#![cfg(feature = "fresh-core")]
//! External integration test of the fresh core: it uses only the public API, as a host
//! would.

use rune::experimental_core::error::{Error, ErrorKind, Refusal};
use rune::experimental_core::execute::Executor;
use rune::experimental_core::frontend::{self, Program};
use rune::experimental_core::heap::{Limits, Store};
use rune::experimental_core::value::{Scalar, Type};
use rune::{Context, Diagnostics, Hash, Module, Source, Sources};

fn compile(context: &Context, text: &str) -> Result<Program, Error> {
    let mut sources = Sources::new();
    sources.insert(Source::memory(text).unwrap()).unwrap();
    frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1)
}

fn main_of(program: &Program) -> u32 {
    program.function(Hash::type_hash(["main"])).unwrap()
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn compile_invoke_read_back_and_release_through_roots() {
    let context = Context::with_default_modules().unwrap();
    let program = compile(&context, "pub fn main(a, b) { let v = [a]; v.push(b); v }").unwrap();
    let mut store = Store::new(Limits::default()).unwrap();
    let a = store.root_i64(40);
    let b = store.root_string("x").unwrap();
    let mut executor = Executor::new().unwrap();
    let out = executor
        .invoke(&program, &mut store, main_of(&program), &[&a, &b], 1_000)
        .unwrap();
    assert_eq!(store.root_tag(&out).unwrap(), Type::Vec);
    assert_eq!(store.root_len(&out).unwrap(), 2);
    let first = store.root_get(&out, 0).unwrap();
    assert_eq!(store.root_scalar(&first).unwrap(), Some(Scalar::I64(40)));
    let second = store.root_get(&out, 1).unwrap();
    assert_eq!(store.root_text(&second).unwrap(), "x");
    for root in [first, second, out, a, b] {
        store.root_release(root).unwrap();
    }
    assert_eq!(store.live_entries(), 0);
    assert!(executor.is_idle());
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn run_time_error_is_a_located_failure_and_the_executor_is_reusable() {
    let context = Context::with_default_modules().unwrap();
    let program = compile(&context, "pub fn main(a) { 1 / a }").unwrap();
    assert!(!program.sources().is_empty());
    let mut store = Store::new(Limits::default()).unwrap();
    let zero = store.root_i64(0);
    let mut executor = Executor::new().unwrap();
    let failure = executor
        .invoke(&program, &mut store, main_of(&program), &[&zero], 1_000)
        .unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
    assert!(failure.error.location.is_some());
    let one = store.root_i64(1);
    let out = executor
        .invoke(&program, &mut store, main_of(&program), &[&one], 1_000)
        .unwrap();
    assert_eq!(store.root_scalar(&out).unwrap(), Some(Scalar::I64(1)));
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn fuel_halt_is_reported_and_run_matches_invoke() {
    let context = Context::with_default_modules().unwrap();
    let program = compile(&context, "pub fn main() { loop {} }").unwrap();
    let mut store = Store::new(Limits::default()).unwrap();
    let failure = Executor::new()
        .unwrap()
        .run(&program, &mut store, main_of(&program), &[], 10)
        .unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::FuelExhausted);
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn outside_the_subset_is_a_compile_time_refusal() {
    let context = Context::with_default_modules().unwrap();
    for text in [
        "pub fn main() { let f = |x| x; f(1) }",
        "pub async fn main() { 1 }",
        "pub fn main() { std::string::String::new() }",
    ] {
        let error = compile(&context, text).err().unwrap();
        assert_eq!(error.kind, ErrorKind::Unsupported, "{text}");
        assert!(error.refusal.is_some(), "{text}");
    }
}

#[cfg(feature = "capture-io")]
#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn println_under_a_capture_context_is_refused_not_redirected() {
    let io = rune::modules::capture_io::CaptureIo::new();
    let mut context = Context::with_config(false).unwrap();
    context
        .install(rune::modules::capture_io::module(&io).unwrap())
        .unwrap();
    let error = compile(&context, "pub fn main() { println!(\"x\"); 1 }")
        .err()
        .unwrap();
    assert_eq!(error.refusal, Some(Refusal::Native));
    assert!(io.is_empty());
}

/// Moved from the D21 dev crate (`acceptance.rs::ordinary_compile_refuses_the_native_bridge`):
/// a call to an installed native function is refused at compile time, so the handler can
/// never run on the fresh core.
#[test]
#[cfg_attr(
    miri,
    ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_native_function_call_is_refused_at_compile_time() {
    fn handler(_: i64) -> i64 {
        panic!("a native handler must never be reached from the fresh core")
    }
    let mut context = Context::with_default_modules().unwrap();
    let mut module = Module::with_crate("host").unwrap();
    module.function("mark", handler).build().unwrap();
    context.install(module).unwrap();
    for text in [
        "pub fn main(a) { host::mark(a) }",
        "pub fn main(a) { let f = host::mark; f(a) }",
    ] {
        let error = compile(&context, text).err().unwrap();
        assert_eq!(error.refusal, Some(Refusal::Native), "{text}");
        assert!(error.location.is_some(), "{text}");
    }
}
