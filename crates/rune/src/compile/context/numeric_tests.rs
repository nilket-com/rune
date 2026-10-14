//! C6 Context certification controls. Compiles source; invokes no handler or VM.
use super::*;
use crate as rune;
use crate::experimental_core::{code::Op, error::Refusal, frontend};
use crate::function::fresh_numeric::{Call, Method, Pair};
use crate::{Diagnostics, Source, Sources};
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(crate::Any)]
#[rune(item = ::c6)]
struct Extra;
#[derive(crate::Any)]
#[rune(item = ::c6)]
struct Missing;
static INVOCATIONS: AtomicUsize = AtomicUsize::new(0);
static REGISTRATIONS: AtomicUsize = AtomicUsize::new(0);
fn forbidden(_: &Extra) -> f64 {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C6 custom handler must not execute at compile time")
}
fn late(_: &Missing) -> f64 {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C6 late handler must not execute")
}
fn core() -> Context {
	let mut cx = Context::new();
	cx.install(crate::modules::core::module().unwrap()).unwrap();
	cx.install(crate::modules::cmp::module().unwrap()).unwrap();
	cx.install(crate::modules::clone::module().unwrap())
		.unwrap();
	cx
}
fn numeric() -> Context {
	let mut cx = core();
	cx.install(crate::modules::i64::module().unwrap()).unwrap();
	cx.install(crate::modules::f64::module().unwrap()).unwrap();
	cx
}
fn extra(name: &'static str) -> Module {
	let mut m = Module::with_item(["c6"]).unwrap();
	m.ty::<Extra>().unwrap();
	m.function(name, forbidden)
		.build_associated::<Extra>()
		.unwrap();
	m
}
fn source(call: Call) -> &'static str {
	match call {
		Call::MethodAbs => "pub fn main(){ (-2.5).abs() }",
		Call::MethodRound => "pub fn main(){ (1.5).round() }",
		Call::MethodPowf => "pub fn main(){ (2.0).powf(3.0) }",
		Call::F64Abs => "pub fn main(){ ::std::f64::abs(-2.5) }",
		Call::F64Round => "pub fn main(){ ::std::f64::round(1.5) }",
		Call::F64Powf => "pub fn main(){ ::std::f64::powf(2.0,3.0) }",
		Call::I64Abs => "pub fn main(){ ::std::i64::abs(-3) }",
	}
}
fn compile(cx: &Context, call: Call, expected: Option<Refusal>) {
	let mut sources = Sources::new();
	sources
		.insert(Source::memory(source(call)).unwrap())
		.unwrap();
	let result = frontend::compile(&mut sources, cx, &mut Diagnostics::new(), 7);
	match expected {
		Some(reason) => assert_eq!(
			result.err().unwrap().refusal,
			Some(reason),
			"source {}",
			source(call)
		),
		None => {
			let program = result.unwrap();
			let id = program.function(Hash::type_hash(["main"])).unwrap();
			let f = program.code.function(id).unwrap();
			let calls: std::vec::Vec<_> = f
				.instructions()
				.iter()
				.filter_map(|i| match f.decode(*i).unwrap() {
					Op::Call {
						function, args_len, ..
					} => Some((function, args_len)),
					_ => None,
				})
				.collect();
			assert_eq!(calls, [(call.id(), call.name().arity())]);
		}
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}
fn matrix(cx: &Context, methods: [bool; 3], pairs: [bool; 4]) {
	for (m, admit) in [Method::Abs, Method::Round, Method::Powf]
		.into_iter()
		.zip(methods)
	{
		assert_eq!(cx.fresh_numeric_method(m), admit);
		compile(
			cx,
			Call::method(m),
			if admit { None } else { Some(Refusal::Method) },
		);
	}
	for (pair, admit) in Pair::ALL.into_iter().zip(pairs) {
		assert_eq!(cx.fresh_numeric_direct(pair), admit);
		compile(
			cx,
			Call::direct(pair),
			if admit { None } else { Some(Refusal::Native) },
		);
	}
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn default_extension_trait_and_custom_first_contexts_preserve_exact_sets() {
	matrix(
		&Context::with_default_modules().unwrap(),
		[true; 3],
		[true; 4],
	);
	let mut cx = Context::with_default_modules().unwrap();
	cx.install(extra("abs")).unwrap();
	matrix(&cx, [false, true, true], [true; 4]);
	let mut cx = core();
	cx.install(extra("abs")).unwrap();
	cx.install(crate::modules::i64::module().unwrap()).unwrap();
	cx.install(crate::modules::f64::module().unwrap()).unwrap();
	matrix(&cx, [false, true, true], [true; 4]);
	let mut cx = Context::with_default_modules().unwrap();
	let mut m = Module::with_item(["c6"]).unwrap();
	m.ty::<Extra>().unwrap();
	m.define_trait(["RoundTrait"])
		.unwrap()
		.handler(|cx| {
			REGISTRATIONS.fetch_add(1, Ordering::SeqCst);
			cx.function("round", forbidden)?;
			Ok(())
		})
		.unwrap();
	let trait_item = m.item.join(["RoundTrait"]).unwrap();
	m.implement_trait::<Extra>(&trait_item).unwrap();
	cx.install(m).unwrap();
	let before = REGISTRATIONS.load(Ordering::SeqCst);
	matrix(&cx, [true, false, true], [true; 4]);
	assert_eq!(REGISTRATIONS.load(Ordering::SeqCst), before);
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn uncertified_and_partial_numeric_modules_fail_closed() {
	let mut cx = core();
	cx.install(crate::modules::i64::module().unwrap()).unwrap();
	let mut f64 = crate::modules::f64::module().unwrap();
	f64.fresh_numeric = 0;
	cx.install(f64).unwrap();
	matrix(&cx, [false; 3], [false, false, false, true]);
	let mut cx = core();
	cx.install(crate::modules::i64::module().unwrap()).unwrap();
	let mut f64 = crate::modules::f64::module().unwrap();
	f64.function("late_failure", late)
		.build_associated::<Missing>()
		.unwrap();
	assert!(cx.install(f64).is_err());
	// Every f64 pair remains pending after whole-module failure, even though
	// earlier associated registrations reached certification.
	matrix(&cx, [false; 3], [false, false, false, true]);
	let mut cx = Context::with_default_modules().unwrap();
	let mut m = extra("abs");
	m.function("late_failure", late)
		.build_associated::<Missing>()
		.unwrap();
	assert!(cx.install(m).is_err());
	matrix(&cx, [false, true, true], [true; 4]);
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn duplicate_function_insert_and_nonfunction_alias_close_selected_names() {
	use crate::runtime::TypeOf;
	let mut cx = numeric();
	let hash = Pair::F64Abs.item();
	let handler = cx.functions.get(&hash).unwrap().clone();
	assert!(cx
		.insert_native_fn(&f64::type_info(), hash, &handler, None)
		.is_err());
	matrix(&cx, [false, true, true], [false, true, true, true]);
	let mut cx = Context::with_default_modules().unwrap();
	let mut m = Module::with_item(["c6"]).unwrap();
	m.ty::<Extra>().unwrap();
	m.constant("abs", 1i64).build_associated::<Extra>().unwrap();
	cx.install(m).unwrap();
	matrix(&cx, [false, true, true], [true; 4]);
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn absent_and_partial_numeric_modules_are_not_implicitly_certified() {
	let mut cx = core();
	for m in [Method::Abs, Method::Round, Method::Powf] {
		compile(&cx, Call::method(m), Some(Refusal::Method));
	}
	for pair in Pair::ALL {
		compile(&cx, Call::direct(pair), Some(Refusal::FrontendDiagnostic));
	}
	cx.install(crate::modules::i64::module().unwrap()).unwrap();
	for m in [Method::Abs, Method::Round, Method::Powf] {
		compile(&cx, Call::method(m), Some(Refusal::Method));
	}
	compile(&cx, Call::I64Abs, None);
	for pair in [Pair::F64Abs, Pair::F64Round, Pair::F64Powf] {
		compile(&cx, Call::direct(pair), Some(Refusal::FrontendDiagnostic));
	}
}
