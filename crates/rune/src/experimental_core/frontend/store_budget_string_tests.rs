//! Existing typed host-string bridge across the historical payload budget.
use super::*;
use serde_json::{json, Value as Json};

fn large_text() -> std::string::String {
	"s".repeat(32 << 20)
}

fn context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["s5"]).unwrap();
	module.host_function("large_text", large_text).unwrap();
	context.install(module).unwrap();
	context
}

fn sources(row: &Json) -> Sources {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	sources
}

fn stock(row: &Json) -> Json {
	let context = context();
	let mut sources = sources(row);
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	let value = vm.call(["main"], ()).unwrap();
	json!({"value": crate::from_value::<i64>(value).unwrap(), "diagnostics":std::format!("{:?}",diagnostics.diagnostics())})
}

#[test]
#[ignore = "operator-only stock freeze; requires a new S5_STRING_ORACLE_OUT"]
fn store_budget_freeze_host_string_stock() {
	use std::io::Write;
	let mut row: Json =
		serde_json::from_str(include_str!("store_budget_string_contract.json")).unwrap();
	row["stock"] = stock(&row);
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("S5_STRING_ORACLE_OUT").unwrap())
		.unwrap();
	file.write_all(serde_json::to_string_pretty(&row).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn host_string_stock_and_source_are_frozen() {
	use sha2::{Digest, Sha256};
	let row: Json =
		serde_json::from_str(include_str!("store_budget_string_contract.json")).unwrap();
	let hash = Sha256::digest(row["source"].as_str().unwrap().as_bytes());
	let hash = hash
		.iter()
		.map(|b| std::format!("{b:02x}"))
		.collect::<std::string::String>();
	assert_eq!(hash, row["source_sha256"].as_str().unwrap());
	assert_eq!(stock(&row), row["stock"]);
}

#[test]
fn host_string_matches_stock_under_the_default_store_budget() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let row: Json =
		serde_json::from_str(include_str!("store_budget_string_contract.json")).unwrap();
	let context = context();
	let mut sources = sources(&row);
	let mut diagnostics = Diagnostics::new();
	let program = super::compile(&mut sources, &context, &mut diagnostics, 22).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let root = executor
		.invoke(&program, &mut store, main, &[], 10000)
		.unwrap();
	let Value::I64(value) = store.import(&root).unwrap() else {
		panic!("expected length");
	};
	assert_eq!(
		json!({"value":value,"diagnostics":std::format!("{:?}",diagnostics.diagnostics())}),
		row["stock"]
	);
	store.root_release(root).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	let mut sandbox = Store::new(Limits {
		entries: 65_536,
		bytes: 16 << 20,
	})
	.unwrap();
	let failure = executor
		.invoke(&program, &mut sandbox, main, &[], 10000)
		.unwrap_err();
	assert_eq!(
		failure.error.kind,
		crate::experimental_core::error::ErrorKind::StoreCapacity
	);
	assert_eq!((sandbox.live_entries(), sandbox.payload_bytes()), (0, 0));
	assert!(executor.is_idle());
}

#[test]
fn large_string_payload_bytes_and_contents_match_without_partial_publication() {
	use crate::experimental_core::{
		error::ErrorKind,
		heap::{Limits, Store},
	};
	use sha2::{Digest, Sha256};
	let text = large_text();
	let mut store = Store::new(Limits::default()).unwrap();
	let value = store.string(&text).unwrap();
	assert_eq!(store.payload_bytes(), 32 << 20);
	assert_eq!(
		Sha256::digest(store.text(value).unwrap().as_bytes()),
		Sha256::digest(text.as_bytes())
	);
	store.release(value).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	let mut sandbox = Store::new(Limits {
		entries: 65_536,
		bytes: 16 << 20,
	})
	.unwrap();
	assert_eq!(
		sandbox.string(&text).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!((sandbox.live_entries(), sandbox.payload_bytes()), (0, 0));
}
