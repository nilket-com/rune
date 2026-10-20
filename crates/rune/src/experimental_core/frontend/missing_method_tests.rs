use super::*;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
};
use std::sync::{Arc, Mutex};

fn compile(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory(source)?).unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 13)
}

#[test]
fn missing_arguments_and_runtime_context_preflight() {
	use crate::TypeHash;
	let events = Arc::new(Mutex::new(std::vec::Vec::new()));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c13_test"]).unwrap();
	let tape = events.clone();
	module
		.fresh_host_function("tap", move |n: i64| -> i64 {
			tape.lock().unwrap().push(n);
			n
		})
		.unwrap();
	context.install(module).unwrap();
	let program = compile("pub fn main(){c13_test::tap(42).unrecognized(c13_test::tap(7),c13_test::tap(8))} pub fn again(){7}", &context).unwrap();
	let runtime = crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap();
	let mut store = Store::new(Limits {
		entries: 128,
		bytes: 65536,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	executor.set_host_runtime(Some(runtime.clone())).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let failure = executor
		.invoke(&program, &mut store, main, &[], 10000)
		.unwrap_err();
	assert_eq!(*events.lock().unwrap(), [42, 7, 8]);
	let missing = failure.missing_instance_function().unwrap();
	assert_eq!(missing.instance, i64::HASH);
	assert_eq!(
		missing.hash,
		Hash::associated_function(i64::HASH, Hash::ident("unrecognized"))
	);
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	events.lock().unwrap().clear();
	let handler = context
		.lookup_function(Hash::type_hash(["c13_test", "tap"]))
		.unwrap()
		.clone();
	context
		.c13_insert_raw_fixture(missing.hash, handler)
		.unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mismatch = executor
		.invoke(&program, &mut store, main, &[], 10000)
		.unwrap_err();
	assert_eq!(mismatch.error.kind, ErrorKind::InvalidCode);
	assert!(events.lock().unwrap().is_empty());
	assert_eq!(store.live_entries(), 0);
	executor.set_host_runtime(Some(runtime)).unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["again"])).unwrap(),
			&[],
			1000,
		)
		.unwrap();
	store.root_release(root).unwrap();
}

#[test]
fn registered_and_script_names_refuse_before_execution() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"pub fn main(){42.extend()}",
		"pub fn main(){42.next_back()}",
		"pub fn main(){\"42\".parse::<i64>()}",
		"struct Local; impl Local{fn unrecognized(self){23}} pub fn main(){42.unrecognized()}",
		"mod hidden{pub struct Local; impl Local{fn unrecognized(self){23}}} pub fn main(){42.unrecognized()}",
	] {
		assert_eq!(compile(source, &context).err().unwrap().refusal, Some(Refusal::Method), "{source}");
	}
	assert!(compile(
		"fn unrecognized(){23} pub fn main(){42.unrecognized::<i64>()}",
		&context
	)
	.is_ok());
}
