//! S5 stock observations beyond the historical default Store budgets.
use super::*;
use serde_json::{json, Value as Json};

fn stock(row: &Json) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	let value = vm.call(["main"], ()).unwrap();
	json!({"value": crate::from_value::<i64>(value).unwrap(),
		"diagnostics": std::format!("{:?}", diagnostics.diagnostics())})
}

#[test]
#[ignore = "operator-only stock freeze; requires a new S5_ORACLE_OUT"]
fn store_budget_freeze_stock() {
	use std::io::Write;
	let mut contract: Json =
		serde_json::from_str(include_str!("store_budget_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		std::println!("S5_STOCK {} {}", row["name"], row["stock"]);
	}
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("S5_ORACLE_OUT").unwrap())
		.unwrap();
	file.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn store_budget_sources_and_stock_are_frozen() {
	use sha2::{Digest, Sha256};
	let contract: Json = serde_json::from_str(include_str!("store_budget_contract.json")).unwrap();
	assert_eq!(contract["rows"].as_array().unwrap().len(), 4);
	for row in contract["rows"].as_array().unwrap() {
		let hash = Sha256::digest(row["source"].as_str().unwrap().as_bytes());
		assert_eq!(
			hash.iter()
				.map(|b| std::format!("{b:02x}"))
				.collect::<std::string::String>(),
			row["source_sha256"].as_str().unwrap()
		);
		assert_eq!(stock(row), row["stock"], "{}", row["name"]);
	}
}

#[test]
fn store_budget_progressions_are_exactly_the_two_frozen_budget_gaps() {
	let original: Json = serde_json::from_str(include_str!("vec_resize_contract.json")).unwrap();
	let overlay: Json =
		serde_json::from_str(include_str!("store_budget_progressions.json")).unwrap();
	let rows = overlay["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 2);
	for name in ["budget-boundary", "entry-boundary"] {
		let row = original["rows"]
			.as_array()
			.unwrap()
			.iter()
			.find(|r| r["name"] == name)
			.unwrap();
		let progression = rows.iter().find(|r| r["name"] == name).unwrap();
		assert_eq!(row["classification"], "Known-gap");
		assert_eq!(progression["classification"], "Exact");
		assert_eq!(progression["source_sha256"], row["source_sha256"]);
		assert_eq!(progression["stock"], row["stock"]);
	}
}

#[test]
fn default_store_matches_stock_beyond_both_historical_limits() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let contract: Json = serde_json::from_str(include_str!("store_budget_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
			.unwrap();
		let mut diagnostics = Diagnostics::new();
		let compiled = super::compile(&mut sources, &context, &mut diagnostics, 22);
		let controls: Json =
			serde_json::from_str(include_str!("store_budget_refusals.json")).unwrap();
		if let Some(control) = controls["rows"]
			.as_array()
			.unwrap()
			.iter()
			.find(|c| c["name"] == row["name"] && c["source_sha256"] == row["source_sha256"])
		{
			let error = compiled.err().unwrap();
			assert_eq!(std::format!("{:?}", error.refusal), "Some(Method)");
			let at = error.location.unwrap();
			assert_eq!(json!([at.start, at.end]), control["span"]);
			continue;
		}
		let program = compiled.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let result = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				200_000_000,
			)
			.unwrap_or_else(|e| panic!("{}: {e:?}", row["name"]));
		let Value::I64(value) = store.import(&result).unwrap() else {
			panic!("{}", row["name"]);
		};
		assert_eq!(
			json!({"value":value,"diagnostics":std::format!("{:?}", diagnostics.diagnostics())}),
			row["stock"],
			"{}",
			row["name"]
		);
		store.root_release(result).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
