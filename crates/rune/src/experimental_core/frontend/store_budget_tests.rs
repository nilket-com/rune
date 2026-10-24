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
