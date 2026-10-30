//! Admission must not acquire ownership or alter heap bookkeeping.
use super::*;
use crate::experimental_core::{
	execute::{Executor, PrepareError},
	frontend,
	value::Root,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::string::String;

fn snapshot(store: &Store, args: &[&Root]) -> String {
	// Payload Debug prints child handles, never recursively follows heap edges.
	// Include vacant slot generations and capacities as well as live payloads.
	std::format!(
		"{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
		(store.id, store.closed, store.limits),
		(&store.slots, store.slots.capacity()),
		(&store.free, store.free.capacity()),
		(&store.work, store.work.capacity()),
		(
			&store.opaque_names,
			store.opaque_names.capacity(),
			store.opaque_sweep_at
		),
		(
			store.edges,
			store.first,
			store.last,
			store.live,
			store.bytes
		),
		store.format_capacity_extra,
		args,
	)
}

#[test]
fn refused_and_invalid_prepare_preserve_every_store_and_argument_field() {
	let mut context = Context::with_default_modules().unwrap();
	let effects = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
	let count = effects.clone();
	let mut module = crate::Module::with_crate("probe").unwrap();
	module
		.host_function("touch", move || {
			count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
		})
		.unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(
		"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(a,b){probe::touch();a}"
	).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let internal = program.function(Hash::type_hash(["S", "run"])).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let retired = store.root_string("retired").unwrap();
	let stale = Root {
		store: retired.store,
		value: retired.value,
	};
	store.root_release(retired).unwrap();
	let text = store.root_string("replacement").unwrap();
	let list = store.root_list(Type::Vec, &[&text, &text]).unwrap();
	let foreign_store = Store::new(Limits::default()).unwrap();
	let foreign = foreign_store.root_unit();
	let foreign_before = snapshot(&foreign_store, &[&foreign]);

	// Refusal is reached only after all argument imports have been validated.
	let before = snapshot(&store, &[&list]);
	assert!(matches!(
		executor.prepare(&program, &mut store, internal, &[&list]),
		Err(PrepareError::Refused(_))
	));
	assert_eq!(snapshot(&store, &[&list]), before);
	for args in [&[&list, &foreign][..], &[&list, &stale][..], &[&list][..]] {
		let before = snapshot(&store, args);
		assert!(matches!(
			executor.prepare(&program, &mut store, main, args),
			Err(PrepareError::Invalid(_))
		));
		assert_eq!(snapshot(&store, args), before);
		assert!(executor.is_idle());
	}
	assert_eq!(snapshot(&foreign_store, &[&foreign]), foreign_before);
	// Host-runtime invalidity occurs after valid argument imports as well.
	executor.set_host_runtime(None).unwrap();
	let args = [&list, &text];
	let before = snapshot(&store, &args);
	assert!(matches!(
		executor.prepare(&program, &mut store, main, &args),
		Err(PrepareError::Invalid(_))
	));
	assert_eq!(snapshot(&store, &args), before);
	assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 0);
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	// A fully valid prepare token may also be dropped without retaining args.
	let args = [&list, &text];
	let before = snapshot(&store, &args);
	drop(executor.prepare(&program, &mut store, main, &args).unwrap());
	assert_eq!(snapshot(&store, &args), before);
	store.root_release(list).unwrap();
	store.root_release(text).unwrap();
	assert_eq!(store.live_entries(), 0);
	store.close().unwrap();
}
