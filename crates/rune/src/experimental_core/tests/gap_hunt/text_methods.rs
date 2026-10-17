//! C14 stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("text_methods/graph.rs");
const CASES: &str = include_str!("text_methods/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
include!("text_methods/fixture.rs");

fn observed(value: &Value) -> Json {
	let out = graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C14 value: {node}"
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
#[ignore = "C14 stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C14_ENGINE").unwrap();
	assert_eq!(engine, "stock");
	let index: usize = std::env::var("C14_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());

	let context = context(row["fixture"].as_str().unwrap());
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c14.rn", source).unwrap())
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
				match crate::runtime::budget::with(200_000, || vm.call(["c14_reuse"], ())).call() {
					Ok(value) => json!({"status":"value","graph":observed(&value)}),
					Err(e) => error(&e),
				};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fixture":row["fixture"],"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let path = std::env::var_os("C14_RESULT").unwrap();
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
	assert_eq!(rows.len(), 190);
	let mut labels = std::collections::HashSet::new();
	for (i, r) in rows.iter().enumerate() {
		assert_eq!(r["id"], format!("{i:03}"));
		assert!(labels.insert(r["label"].as_str().unwrap()));
		let s = r["source"].as_str().unwrap();
		assert_eq!(
			hex(Sha256::digest(s.as_bytes()).as_ref()),
			r["source_sha256"]
		);
		assert!(s.contains("pub fn c14_reuse(){7}"));
	}
	assert_ne!(observed(&Value::from(1i64)), observed(&Value::from(2i64)));
	assert_ne!(
		observed(&Value::try_from("a").unwrap()),
		observed(&Value::try_from("b").unwrap())
	);
	assert_ne!(
		error(&stock::VmError::new(stock::VmErrorKind::Overflow)),
		error(&stock::VmError::new(stock::VmErrorKind::Underflow))
	);
	TAPE.with(|t| t.borrow_mut().clear());
	tap(1, 2);
	TAPE.with(|t| assert_eq!(t.borrow().len(), 1));
	TAPE.with(|t| t.borrow_mut().clear());
	for fixture in ["default", "i64-provider", "raw-i64-provider"] {
		let _ = context(fixture);
	}
}
