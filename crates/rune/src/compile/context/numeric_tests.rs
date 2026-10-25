//! C6 Context certification controls. Compiles source; invokes no handler or VM.
use super::*;
use crate as rune;
use crate::experimental_core::{code::Op, error::Refusal, frontend};
use crate::function::numeric::{Call, Method, Pair};
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
		Call::MethodCmp => "pub fn main(){ (1).cmp(2) }",
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
		assert_eq!(cx.numeric_method(m), admit);
		compile(
			cx,
			Call::method(m),
			if admit { None } else { Some(Refusal::Method) },
		);
	}
	for (pair, admit) in Pair::DIRECT.into_iter().zip(pairs) {
		assert_eq!(cx.numeric_direct(pair), admit);
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
	f64.numeric = 0;
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
	for pair in Pair::DIRECT {
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

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn cmp_requires_all_providers_and_unshadowed_handler_aliases() {
	let cx = Context::with_default_modules().unwrap();
	assert!(cx.numeric_method(Method::Cmp));
	compile(&cx, Call::MethodCmp, None);
	let mut partial = core();
	partial
		.install(crate::modules::i64::module().unwrap())
		.unwrap();
	partial
		.install(crate::modules::u64::module().unwrap())
		.unwrap();
	partial
		.install(crate::modules::f64::module().unwrap())
		.unwrap();
	assert!(!partial.numeric_method(Method::Cmp));
	compile(&partial, Call::MethodCmp, Some(Refusal::Method));
	let mut cx = Context::with_default_modules().unwrap();
	cx.install(extra("cmp")).unwrap();
	assert!(cx.numeric_method(Method::Cmp));
	compile(&cx, Call::MethodCmp, None);
	let mut cx = Context::with_default_modules().unwrap();
	let mut m = Module::new();
	m.associated_function(&crate::runtime::Protocol::CMP, |a: i64, b: i64| a.cmp(&b))
		.unwrap();
	assert!(cx.install(m).is_err());
	assert!(!cx.numeric_method(Method::Cmp));
	compile(&cx, Call::MethodCmp, Some(Refusal::Method));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; cheap runtime helpers run under Miri"
)]
fn script_cmp_anywhere_in_the_unit_disables_the_intrinsic() {
	let cx = Context::with_default_modules().unwrap();
	for source in [
		"struct S{x} impl S {fn cmp(self,rhs){9}} pub fn main(){(1).cmp(2)}",
		"pub fn unused(a,b){a.cmp(b)} struct S{x} impl S {fn cmp(self,rhs){9}} pub fn main(){7}",
	] {
		let mut sources = Sources::new();
		sources.insert(Source::memory(source).unwrap()).unwrap();
		let error = frontend::compile(&mut sources, &cx, &mut Diagnostics::new(), 7)
			.err()
			.unwrap();
		assert_eq!(error.refusal, Some(Refusal::Method));
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}

// C9 receiver relevance: custom handlers are install-only, never invoked.
fn cmp_forbidden_i64(_: i64, _: i64) -> core::cmp::Ordering {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C9 i64 cmp handler must not execute")
}
fn cmp_forbidden_bool(_: bool, _: bool) -> core::cmp::Ordering {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C9 bool cmp handler must not execute")
}
fn cmp_forbidden_ordering(_: core::cmp::Ordering, _: core::cmp::Ordering) -> core::cmp::Ordering {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C9 Ordering cmp handler must not execute")
}
fn cmp_forbidden_vec(_: &crate::runtime::Vec, _: &crate::runtime::Vec) -> core::cmp::Ordering {
	INVOCATIONS.fetch_add(1, Ordering::SeqCst);
	panic!("C9 Vec cmp handler must not execute")
}
// Same dependency prefix as Context::with_config through String; then only u64/f64.
fn cmp_without_i64_and_vec() -> Context {
	let mut cx = Context::new();
	cx.install(crate::modules::iter::module().unwrap()).unwrap();
	cx.install(crate::modules::core::module().unwrap()).unwrap();
	cx.install(crate::modules::cmp::module().unwrap()).unwrap();
	cx.install(crate::modules::any::module().unwrap()).unwrap();
	cx.install(crate::modules::clone::module().unwrap())
		.unwrap();
	cx.install(crate::modules::num::module().unwrap()).unwrap();
	cx.install(crate::modules::hash::module().unwrap()).unwrap();
	cx.install(crate::modules::string::module().unwrap())
		.unwrap();
	cx.install(crate::modules::u64::module().unwrap()).unwrap();
	cx.install(crate::modules::f64::module().unwrap()).unwrap();
	cx
}
#[derive(Clone, Copy, Debug)]
enum CmpInstall {
	Named,
	Protocol,
	Trait,
}
fn cmp_i64_module(kind: CmpInstall) -> Module {
	let mut m = Module::with_item(["c9_relevance"]).unwrap();
	match kind {
		CmpInstall::Named => {
			m.function("cmp", cmp_forbidden_i64)
				.build_associated::<i64>()
				.unwrap();
		}
		CmpInstall::Protocol => {
			m.associated_function(&crate::runtime::Protocol::CMP, cmp_forbidden_i64)
				.unwrap();
		}
		CmpInstall::Trait => {
			m.define_trait(["CmpShadow"])
				.unwrap()
				.handler(|cx| {
					REGISTRATIONS.fetch_add(1, Ordering::SeqCst);
					cx.function("cmp", cmp_forbidden_i64)?;
					Ok(())
				})
				.unwrap();
			let item = m.item.join(["CmpShadow"]).unwrap();
			m.implement_trait::<i64>(&item).unwrap();
		}
	}
	m
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; pure certificate controls run under Miri"
)]
fn cmp_custom_i64_named_protocol_and_trait_fail_closed_in_both_orders() {
	for kind in [CmpInstall::Named, CmpInstall::Protocol, CmpInstall::Trait] {
		let mut default_first = Context::with_default_modules().unwrap();
		assert!(default_first.numeric_method(Method::Cmp));
		assert!(
			default_first.install(cmp_i64_module(kind)).is_err(),
			"{kind:?}"
		);
		assert!(!default_first.numeric_method(Method::Cmp), "{kind:?}");
		compile(&default_first, Call::MethodCmp, Some(Refusal::Method));
		let mut custom_first = cmp_without_i64_and_vec();
		assert!(!custom_first.numeric_method(Method::Cmp)); // i64 stock provider absent.
		custom_first.install(cmp_i64_module(kind)).unwrap();
		assert!(!custom_first.numeric_method(Method::Cmp), "{kind:?}");
		assert!(
			custom_first
				.install(crate::modules::i64::module().unwrap())
				.is_err(),
			"{kind:?}"
		);
		assert!(!custom_first.numeric_method(Method::Cmp), "{kind:?}");
		compile(&custom_first, Call::MethodCmp, Some(Refusal::Method));
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; pure certificate controls run under Miri"
)]
fn cmp_missing_receivers_close_on_names_and_protocols() {
	for protocol in [false, true] {
		let mut cx = Context::with_default_modules().unwrap();
		let mut m = Module::with_item(["c9_bool"]).unwrap();
		if protocol {
			m.associated_function(&crate::runtime::Protocol::CMP, cmp_forbidden_bool)
				.unwrap();
		} else {
			m.function("cmp", cmp_forbidden_bool)
				.build_associated::<bool>()
				.unwrap();
		}
		cx.install(m).unwrap();
		assert!(!cx.numeric_method(Method::Cmp));
		compile(&cx, Call::MethodCmp, Some(Refusal::Method));
		let mut cx = Context::with_default_modules().unwrap();
		let mut m = Module::with_item(["c9_ordering"]).unwrap();
		if protocol {
			m.associated_function(&crate::runtime::Protocol::CMP, cmp_forbidden_ordering)
				.unwrap();
		} else {
			m.function("cmp", cmp_forbidden_ordering)
				.build_associated::<core::cmp::Ordering>()
				.unwrap();
		}
		cx.install(m).unwrap();
		assert!(!cx.numeric_method(Method::Cmp));
		compile(&cx, Call::MethodCmp, Some(Refusal::Method));
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}
#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; pure runtime refusal helpers run under Miri"
)]
fn cmp_custom_vec_names_and_protocols_stay_open_and_refuse_at_runtime() {
	use crate::experimental_core::{
		error::ErrorKind,
		heap::{Limits, Store},
		value::{Type, Value},
	};
	for protocol in [false, true] {
		let mut cx = cmp_without_i64_and_vec();
		cx.install(crate::modules::i64::module().unwrap()).unwrap();
		assert!(cx.numeric_method(Method::Cmp));
		let mut m = Module::with_crate_item("std", ["vec"]).unwrap();
		m.ty::<crate::runtime::Vec>().unwrap();
		if protocol {
			m.associated_function(&crate::runtime::Protocol::CMP, cmp_forbidden_vec)
				.unwrap();
		} else {
			m.function("cmp", cmp_forbidden_vec)
				.build_associated::<crate::runtime::Vec>()
				.unwrap();
		}
		m.define_trait(["CmpPendingOther"])
			.unwrap()
			.handler(|cx| {
				let pending = cx.cx.numeric.begin_module(0, 0);
				assert_eq!(pending.0 & Method::Cmp.bit(), 0);
				assert!(cx.cx.numeric_method(Method::Cmp));
				cx.cx.numeric.finish_module(pending);
				Ok(())
			})
			.unwrap();
		let item = m.item.join(["CmpPendingOther"]).unwrap();
		m.implement_trait::<crate::runtime::Vec>(&item).unwrap();
		cx.install(m).unwrap();
		assert!(cx.numeric_method(Method::Cmp));
		compile(&cx, Call::MethodCmp, None);
		let mut store = Store::new(Limits::default()).unwrap();
		let value = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let before = store.state_for_test();
		let error = store.method_cmp(value, value).unwrap_err();
		assert_eq!(
			(error.kind, error.operation, error.refusal),
			(ErrorKind::Unsupported, "Method", None)
		);
		assert_eq!(store.state_for_test(), before);
		store.release(value).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; pure pending-mask controls run under Miri"
)]
fn cmp_relevant_pending_module_is_visible_during_trait_registration() {
	for protocol in [false, true] {
		let mut cx = Context::with_default_modules().unwrap();
		let mut m = Module::with_item(["c9_pending_bool"]).unwrap();
		if protocol {
			m.associated_function(&crate::runtime::Protocol::CMP, cmp_forbidden_bool)
				.unwrap();
		} else {
			m.function("cmp", cmp_forbidden_bool)
				.build_associated::<bool>()
				.unwrap();
		}
		m.define_trait(["CmpPendingRelevant"])
			.unwrap()
			.handler(|cx| {
				let pending = cx.cx.numeric.begin_module(0, 0);
				assert_ne!(pending.0 & Method::Cmp.bit(), 0);
				assert!(!cx.cx.numeric_method(Method::Cmp));
				cx.cx.numeric.finish_module(pending);
				Ok(())
			})
			.unwrap();
		let item = m.item.join(["CmpPendingRelevant"]).unwrap();
		m.implement_trait::<bool>(&item).unwrap();
		cx.install(m).unwrap();
		assert!(!cx.numeric_method(Method::Cmp));
		compile(&cx, Call::MethodCmp, Some(Refusal::Method));
	}
	assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 0);
}
