//! Live receiver-domain contracts for the conservative native fallback cut.
use super::*;
use crate as rune;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
	value::{Type, Value},
};
use std::{
	string::ToString,
	sync::{Arc, Mutex},
};

fn compile_source(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 19)
}

fn stock_i64(source: &str, context: &Context) -> i64 {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(context)
		.with_diagnostics(&mut Diagnostics::new())
		.build_vm()
		.unwrap();
	crate::from_value(vm.call(["main"], ()).unwrap()).unwrap()
}

fn executor(context: &Context) -> Executor {
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	executor
}

fn assert_i64(store: &Store, root: &crate::experimental_core::value::Root, expected: i64) {
	assert!(matches!(store.import(root).unwrap(), Value::I64(got) if got == expected));
}

#[test]
fn mixed_default_context_receivers_match_live_stock() {
	let source = r#"
		struct Local { empty }
		impl Local { fn is_empty(self) { self.empty } }
		enum Choice { Empty, Full }
		impl Choice { fn is_empty(self) { match self { Choice::Empty => true, Choice::Full => false } } }
		fn probe(value) { if value.is_empty() { 1 } else { 0 } }
		pub fn main() {
			probe([]) + 2 * probe([1]) + 4 * probe(()) + 8 * probe((1,))
			+ 16 * probe("") + 32 * probe("x") + 64 * probe(#{}) + 128 * probe(#{x: 1})
			+ 256 * probe(Local { empty: true }) + 512 * probe(Local { empty: false })
			+ 1024 * probe(Choice::Empty) + 2048 * probe(Choice::Full)
		}
	"#;
	let context = Context::with_default_modules().unwrap();
	let expected = stock_i64(source, &context);
	assert_eq!(expected, 1365);
	let program = compile_source(source, &context).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			100000,
		)
		.unwrap();
	assert_i64(&store, &root, expected);
	store.root_release(root).unwrap();
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn host_entry_accepts_represented_receiver_roots() {
	let source = "pub fn inspect(value){if value.is_empty(){1}else{0}}";
	let context = Context::with_default_modules().unwrap();
	let program = compile_source(source, &context).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	let scalar = store.root_i64(7);
	let roots = [
		(store.root_list(Type::Vec, &[]).unwrap(), 1),
		(store.root_list(Type::Vec, &[&scalar]).unwrap(), 0),
		(store.root_list(Type::Tuple, &[&scalar]).unwrap(), 0),
		(store.root_unit(), 1),
		(store.root_string("").unwrap(), 1),
		(store.root_string("x").unwrap(), 0),
		(store.root_record(None, &[]).unwrap(), 1),
		(store.root_record(None, &[("x", &scalar)]).unwrap(), 0),
	];
	for (input, expected) in roots {
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["inspect"])).unwrap(),
				&[&input],
				10000,
			)
			.unwrap();
		assert_i64(&store, &root, expected);
		store.root_release(root).unwrap();
		store.root_release(input).unwrap();
		assert!(executor.is_idle());
	}
	store.root_release(scalar).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn receiver_dependent_arity_preserves_receiver_and_argument_effects() {
	let events = Arc::new(Mutex::new(std::vec::Vec::new()));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["receiver_domain"]).unwrap();
	let tape = events.clone();
	module
		.fresh_host_function("tap", move |n: i64| -> i64 {
			tape.lock().unwrap().push(n);
			n
		})
		.unwrap();
	context.install(module).unwrap();
	for receiver in ["Local", "[]"] {
		let source = format!("struct Local;impl Local{{fn is_empty(self,a,b){{a+b}}}}fn probe(x){{x.is_empty(receiver_domain::tap(1),receiver_domain::tap(2))}}pub fn main(){{probe({{receiver_domain::tap(0);{receiver}}})}}pub fn again(){{7}}");
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(&source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut Diagnostics::new())
			.build_vm()
			.unwrap();
		let stock = vm.call(["main"], ());
		assert_eq!(*events.lock().unwrap(), [0, 1, 2]);
		events.lock().unwrap().clear();
		let program = compile_source(&source, &context).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = executor(&context);
		let fresh = executor.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			100000,
		);
		assert_eq!(*events.lock().unwrap(), [0, 1, 2]);
		events.lock().unwrap().clear();
		match stock {
			Ok(value) => {
				assert_eq!(crate::from_value::<i64>(value).unwrap(), 3);
				let root = fresh.unwrap();
				assert_i64(&store, &root, 3);
				store.root_release(root).unwrap();
			}
			Err(stock) => {
				let failure = fresh.unwrap_err();
				assert_ne!(failure.error.kind, ErrorKind::InvalidCode);
				assert_eq!(
					failure.message(&program).to_string(),
					stock.error().kind().to_string()
				);
				assert!(failure.cleanup_error.is_none());
			}
		}
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["again"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		assert_i64(&store, &root, 7);
		store.root_release(root).unwrap();
	}
}

#[derive(crate::Any)]
#[rune(item = ::receiver_domain)]
struct ContextOnly;

#[test]
fn context_only_native_provider_still_refuses_the_whole_site() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["receiver_domain"]).unwrap();
	module.ty::<ContextOnly>().unwrap();
	module
		.function("is_empty", |_: &ContextOnly| true)
		.build_associated::<ContextOnly>()
		.unwrap();
	context.install(module).unwrap();
	// ContextOnly never appears in source and cannot reach this call. The
	// absence of its unsupported handler from a source scan is not a proof.
	let source = "struct Local;impl Local{fn is_empty(self){true}}fn probe(x){x.is_empty()}pub fn main(){if probe(Local){1}else{0}}";
	assert_eq!(stock_i64(source, &context), 1);
	let error = compile_source(source, &context).err().unwrap();
	assert_eq!(error.refusal, Some(Refusal::Method));
}

#[test]
fn excluded_native_payload_construction_and_constants_remain_refused() {
	let context = Context::with_default_modules().unwrap();
	for (source, frontend_refusal) in [
		("pub fn main(){Bytes::new()}", false),
		(
			"use std::collections::HashMap;pub fn main(){HashMap::new()}",
			false,
		),
		(
			"use std::collections::HashSet;pub fn main(){HashSet::new()}",
			false,
		),
		("pub fn main(){b\"x\"}", false),
		("const PAYLOAD=b\"x\";pub fn main(){PAYLOAD}", true),
	] {
		// Check stock compilation live so a syntax or name-resolution mistake
		// cannot masquerade as a receiver-domain refusal.
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(source).unwrap())
			.unwrap();
		crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut Diagnostics::new())
			.build_vm()
			.unwrap();
		let error = compile_source(source, &context).err().unwrap();
		assert_eq!(error.kind, ErrorKind::Unsupported, "{source}");
		// Byte constants stop during frontend constant materialization;
		// the live stock compile above establishes that the source is valid.
		assert_eq!(
			error.refusal == Some(Refusal::FrontendDiagnostic),
			frontend_refusal,
			"{source}"
		);
	}
}

#[test]
fn ordinary_nominal_roots_can_cross_programs_with_the_same_published_shape() {
	let producer = "struct Shared{empty}pub fn main(){Shared{empty:true}}";
	let consumer = "struct Shared{empty}impl Shared{fn is_empty(self){self.empty}}pub fn inspect(value){if value.is_empty(){1}else{0}}";
	let context = Context::with_default_modules().unwrap();
	let mut producer_sources = Sources::new();
	producer_sources
		.insert(crate::Source::memory(producer).unwrap())
		.unwrap();
	let mut producer_vm = crate::prepare(&mut producer_sources)
		.with_context(&context)
		.with_diagnostics(&mut Diagnostics::new())
		.build_vm()
		.unwrap();
	let stock_input = producer_vm.call(["main"], ()).unwrap();
	let mut consumer_sources = Sources::new();
	consumer_sources
		.insert(crate::Source::memory(consumer).unwrap())
		.unwrap();
	let mut consumer_vm = crate::prepare(&mut consumer_sources)
		.with_context(&context)
		.with_diagnostics(&mut Diagnostics::new())
		.build_vm()
		.unwrap();
	let expected =
		crate::from_value::<i64>(consumer_vm.call(["inspect"], (stock_input,)).unwrap()).unwrap();
	assert_eq!(expected, 1);
	let producer = compile_source(producer, &context).unwrap();
	let consumer = compile_source(consumer, &context).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	let input = executor
		.invoke(
			&producer,
			&mut store,
			producer.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			10000,
		)
		.unwrap();
	let root = executor
		.invoke(
			&consumer,
			&mut store,
			consumer.function(Hash::type_hash(["inspect"])).unwrap(),
			&[&input],
			10000,
		)
		.unwrap();
	assert_i64(&store, &root, expected);
	store.root_release(root).unwrap();
	store.root_release(input).unwrap();
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}
