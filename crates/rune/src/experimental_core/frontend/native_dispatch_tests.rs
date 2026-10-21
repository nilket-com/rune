//! Script and native providers of the same plain method name.
use super::*;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
};
use std::string::ToString;

const CASES: &[(&str, i64)] = &[
	("struct S{x}impl S{fn unwrap(self){100+self.x}}fn f(x){x.unwrap()}pub fn main(){f(S{x:2})+f(Some(3))+f(Ok(4))}", 109),
	("struct S;impl S{fn is_some(self){true}}fn f(x){x.is_some()}pub fn main(){if f(S)&&f(Some(2))&&!f(None){1}else{0}}", 1),
	("struct S;impl S{fn is_none(self){false}}fn f(x){x.is_none()}pub fn main(){if !f(S)&&!f(Some(2))&&f(None){1}else{0}}", 1),
	("struct S;impl S{fn is_ok(self){true}}fn f(x){x.is_ok()}pub fn main(){if f(S)&&f(Ok(2))&&!f(Err(3)){1}else{0}}", 1),
	("struct S;impl S{fn is_err(self){false}}fn f(x){x.is_err()}pub fn main(){if !f(S)&&!f(Ok(2))&&f(Err(3)){1}else{0}}", 1),
	("struct S{x}impl S{fn expect(self,m){self.x+m.len()}}fn f(x){x.expect(\"ok\")}pub fn main(){f(S{x:8})+f(Some(3))+f(Ok(4))}", 17),
	("struct S;impl S{fn is_some(self,x){x}}fn f(x){x.is_some(7)}pub fn main(){f(S)}", 7),
	("struct S;impl S{fn trim(self){\"script\"}}fn f(x){x.trim()}pub fn main(){(f(S).len()+f(\" hi \").len()) as i64}", 8),
	("struct S;impl S{fn sqrt(self){5.0}}fn f(x){x.sqrt()}pub fn main(){(f(S)+f(16.0)) as i64}", 9),
];

#[test]
fn proven_native_receivers_keep_their_frame_requirement() {
	use crate::experimental_core::execute::ExecutorLimits;
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(
			crate::Source::memory(
				"struct S;impl S{fn sqrt(self){5.0}}pub fn main(){4.0.sqrt() as i64}",
			)
			.unwrap(),
		)
		.unwrap();
	let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::with_limits(ExecutorLimits {
		frames: 1,
		slots: 64,
	})
	.unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			1000,
		)
		.unwrap();
	assert!(matches!(
		store.import(&root).unwrap(),
		crate::experimental_core::value::Value::I64(2)
	));
	store.root_release(root).unwrap();
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn stock_contract() {
	for &(source, expected) in CASES {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut Diagnostics::new())
			.build_vm()
			.unwrap();
		let value = vm.call(["main"], ()).unwrap();
		assert_eq!(
			crate::from_value::<i64>(value).unwrap(),
			expected,
			"{source}"
		);
	}
}

#[test]
fn mixed_providers() {
	for &(source, expected) in CASES {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(source).unwrap())
			.unwrap();
		let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18).unwrap();
		use crate::TypeHash;
		assert!(program
			.function(Hash::associated_function(
				<Option<crate::runtime::Value>>::HASH,
				Hash::ident("unwrap")
			))
			.is_none());
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				100000,
			)
			.unwrap();
		let crate::experimental_core::value::Value::I64(got) = store.import(&root).unwrap() else {
			panic!("expected i64")
		};
		assert_eq!(got, expected, "{source}");
		store.root_release(root).unwrap();
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}

#[test]
fn callable_split_remains_a_compile_time_refusal() {
	let source = "struct S;impl S{fn split(self,p){1}}fn f(x,p){x.split(p)}pub fn main(){f(\"abc\",|c|true);1}";
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut Diagnostics::new())
		.build_vm()
		.unwrap();
	assert_eq!(
		crate::from_value::<i64>(vm.call(["main"], ()).unwrap()).unwrap(),
		1
	);
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	let error = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18)
		.err()
		.unwrap();
	assert_eq!(error.refusal, Some(Refusal::Method));
}

#[test]
fn native_string_function_argument_errors_match_stock() {
	use crate::runtime::InvocationContext;
	for pattern in ["|x|x", "{let n=1;move |x|x+n}"] {
		let source = format!("struct H{{h}}impl H{{fn ends_with(self,a){{\"script\"}}}}pub fn main(){{\"abc\".ends_with({pattern})}}");
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(&source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut Diagnostics::new())
			.build_vm()
			.unwrap();
		let stock = vm.call(["main"], ()).unwrap_err();
		let trace = stock.stacktrace().first().unwrap();
		let stock_span = trace
			.unit
			.debug_info()
			.unwrap()
			.instruction_at(trace.ip)
			.unwrap()
			.span
			.range();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(&source).unwrap())
			.unwrap();
		let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				10000,
			)
			.unwrap_err();
		assert_eq!(
			failure.message(&program).to_string(),
			stock.error().kind().to_string()
		);
		let at = failure.error.location.unwrap();
		assert_eq!(at.start as usize..at.end as usize, stock_span);
		assert!(failure.call_chain.is_empty());
		let contexts = failure.invocation_context().collect::<std::vec::Vec<_>>();
		assert!(matches!(
			&contexts[..],
			[InvocationContext::BadArgument { arg: 1 }]
		));
		assert!(failure.cleanup_error.is_none());
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}

#[test]
fn native_provider_is_revalidated_before_execution() {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory("struct S;impl S{fn is_some(self){true}}fn f(x){x.is_some()}pub fn main(){let x=1/0;f(Some(x))}").unwrap()).unwrap();
	let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(Context::new().runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let failure = executor
		.invoke(&program, &mut store, main, &[], 100000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(failure.error.location.is_none());
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let failure = executor
		.invoke(&program, &mut store, main, &[], 100000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
}

#[test]
fn unsupported_native_provider_keeps_the_site_refused() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c18b3"]).unwrap();
	module
		.function("is_some", |_: crate::runtime::Value| false)
		.build_associated::<crate::runtime::Vec>()
		.unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(
			crate::Source::memory(
				"struct S;impl S{fn is_some(self){true}}fn f(x){x.is_some()}pub fn main(){f(S)}",
			)
			.unwrap(),
		)
		.unwrap();
	let error = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18)
		.err()
		.unwrap();
	assert_eq!(error.refusal, Some(Refusal::Method));
}

#[test]
fn failed_native_and_missing_calls_leave_executor_reusable() {
	for source in [
		"struct S;impl S{fn unwrap(self){1}}fn f(x){x.unwrap()}pub fn main(){f(None)}pub fn again(){7}",
		"struct S;impl S{fn unwrap(self){1}}fn f(x){x.unwrap()}pub fn main(){f(42)}pub fn again(){7}",
		"struct S;impl S{fn is_some(self,x){x}}fn f(x){x.is_some(7)}pub fn main(){f(Some(1))}pub fn again(){7}",
	] {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources.insert(crate::Source::memory(source).unwrap()).unwrap();
		let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 18).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor.invoke(&program, &mut store, program.function(Hash::type_hash(["main"])).unwrap(), &[], 100000).unwrap_err();
		assert_ne!(failure.error.kind, ErrorKind::InvalidCode, "{source}");
		assert!(failure.cleanup_error.is_none());
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		let root = executor.invoke(&program, &mut store, program.function(Hash::type_hash(["again"])).unwrap(), &[], 1000).unwrap();
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}
