//! C13 stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("methods/graph.rs");
const CASES: &str = include_str!("methods/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
fn tap_bool(tag: i64, value: bool) -> bool {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"bool","value":value}))
	});
	value
}
include!("methods/fixture.rs");
include!("methods/run.rs");
include!("methods/contract.rs");
include!("methods/bounded.rs");

fn observed(value: &Value) -> Json {
	let out = graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C13 value: {node}"
		);
	}
	out
}
fn error(error: &stock::VmError) -> Json {
	let kind = error.error().kind();
	let sites = error.stacktrace().iter().map(|at| {
		let inst = at.unit.debug_info().expect("debug unit").instruction_at(at.ip).expect("debug instruction");
		let range = inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	let missing = error.missing_instance_function().map(|e|json!({"hash":e.hash.to_string(),"instance":e.instance.to_string(),"instance_name":e.instance_name().to_string()}));
	let chain = error
		.chain()
		.iter()
		.map(|e| json!({"kind":format!("{:?}",e.kind()),"display":e.to_string()}))
		.collect::<Vec<_>>();
	json!({"status":"error","missing":missing,"kind":format!("{kind:?}"),"display":kind.to_string(),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C13 stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C13_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let index: usize = std::env::var("C13_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	if engine == "fresh" {
		let result = observe(row);
		let path = std::env::var_os("C13_RESULT").unwrap();
		let mut file = std::fs::OpenOptions::new()
			.create_new(true)
			.write(true)
			.open(path)
			.unwrap();
		use std::io::Write;
		file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
			.unwrap();
		return;
	}
	let context = context(row["fixture"].as_str().unwrap());
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c13.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let (outcome, reuse) = match built {
		Err(e) => (
			json!({"status":"compile-error","error":format!("{e:?}")}),
			Json::Null,
		),
		Ok(mut vm) => {
			let result =
				crate::runtime::budget::with(row["fuel"].as_u64().unwrap() as usize, || {
					vm.call(["main"], ())
				})
				.call();
			let outcome = match result {
				Ok(value) => json!({"status":"value","graph":observed(&value)}),
				Err(e) => error(&e),
			};
			let reuse =
				match crate::runtime::budget::with(200_000, || vm.call(["c13_reuse"], ())).call() {
					Ok(value) => json!({"status":"value","graph":observed(&value)}),
					Err(e) => error(&e),
				};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fixture":row["fixture"],"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let path = std::env::var_os("C13_RESULT").unwrap();
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(path)
		.unwrap();
	use std::io::Write;
	file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn observer_controls() {
	use sha2::{Digest, Sha256};
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	assert_eq!(rows.len(), 84);
	let mut labels = std::collections::HashSet::new();
	for (index, row) in rows.iter().enumerate() {
		assert_eq!(row["id"], format!("{index:03}"));
		assert!(labels.insert(row["label"].as_str().unwrap()));
		let source = row["source"].as_str().unwrap();
		assert_eq!(
			hex(Sha256::digest(source.as_bytes()).as_ref()),
			row["source_sha256"]
		);
		assert!(source.contains("pub fn c13_reuse(){7}"));
		assert_eq!(row["fuel"], 200_000);
	}
	assert_ne!(observed(&Value::from(1i64)), observed(&Value::from(2i64)));
	let missing = |hash| {
		stock::VmError::new(stock::VmErrorKind::MissingInstanceFunction {
			hash: Hash::new(hash),
			instance: Value::from(1i64).type_info(),
		})
	};
	assert_ne!(error(&missing(1))["missing"], error(&missing(2))["missing"]);
	assert_eq!(
		error(&stock::VmError::new(stock::VmErrorKind::Overflow))["missing"],
		Json::Null
	);
	TAPE.with(|t| t.borrow_mut().clear());
	assert_eq!(tap(1, 7), 7);
	assert!(!tap_bool(2, false));
	TAPE.with(|t| assert_eq!(t.borrow().len(), 2));
	TAPE.with(|t| t.borrow_mut().clear());
	for fixture in [
		"default",
		"i64-provider",
		"bool-provider",
		"unit-provider",
		"raw-i64-provider",
		"raw-local-provider",
		"failed-install-provider",
		"opaque-provider",
	] {
		let _ = context(fixture);
	}
}
