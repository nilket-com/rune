//! C11b stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("captures/graph.rs");
const CASES: &str = include_str!("captures/cases.json");
const DISPOSITIONS: &str = include_str!("captures/dispositions.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| t.borrow_mut().push(json!({"tag":tag,"kind":"i64","value":value.to_string()})));
	value
}
fn tap_bool(tag: i64, value: bool) -> bool {
	TAPE.with(|t| t.borrow_mut().push(json!({"tag":tag,"kind":"bool","value":value})));
	value
}
fn context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c11b"]).unwrap();
	module.function("tap", tap).build().unwrap();
	module.function("tap_bool", tap_bool).build().unwrap();
	context.install(module).unwrap();
	context
}
fn observed(value: &Value) -> Json {
	let out = graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")), "unmapped C11b value: {node}");
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
	let chain = error.chain().iter().map(|e| json!({"kind":format!("{:?}",e.kind()),"display":e.to_string()})).collect::<Vec<_>>();
	json!({"status":"error","kind":format!("{kind:?}"),"display":kind.to_string(),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C11b stock-oracle child; explicit bounded external operator only"]
fn child() {
	assert_eq!(std::env::var("C11B_ENGINE").unwrap(), "stock");
	let index: usize = std::env::var("C11B_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let context = context();
	let mut sources = Sources::new();
	sources.insert(Source::new("c11b.rn", source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources).with_context(&context).with_diagnostics(&mut diagnostics).build_vm();
	let (outcome, reuse) = match built {
		Err(e) => (json!({"status":"compile-error","error":format!("{e:?}")}), Json::Null),
		Ok(mut vm) => {
			let result = crate::runtime::budget::with(row["fuel"].as_u64().unwrap() as usize, || vm.call(["main"], ())).call();
			let outcome = match result { Ok(value) => json!({"status":"value","graph":observed(&value)}), Err(e) => error(&e) };
			let reuse = match crate::runtime::budget::with(200_000, || vm.call(["c11b_reuse"], ())).call() {
				Ok(value) => json!({"status":"value","graph":observed(&value)}), Err(e) => error(&e)
			};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let path = std::env::var_os("C11B_RESULT").unwrap();
	let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path).unwrap();
	use std::io::Write;
	file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes()).unwrap();
}

#[test]
fn observer_controls() {
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	assert_eq!(cases.len(), 37);
	let dispositions: serde_json::Map<String, Json> = serde_json::from_str(DISPOSITIONS).unwrap();
	assert_eq!(dispositions.len(), cases.len());
	for (index, case) in cases.iter().enumerate() {
		let id = case["id"].as_str().unwrap();
		let disposition = &dispositions[id];
		assert_eq!(disposition["index"].as_u64(), Some(index as u64));
		assert_eq!(disposition["label"], case["label"]);
		assert_eq!(disposition["source_sha256"], case["source_sha256"]);
		assert!(disposition["stock_observation_sha256"].as_str().is_some_and(|s| s.len() == 64));
	}
	for id in ["000", "013", "020", "027", "029", "031", "033", "034", "036"] {
		assert_eq!(dispositions[id]["disposition"], "Exact");
		assert!(dispositions[id]["fresh_expectation"].is_object());
	}
	for (id, expected) in [("014", "Exact-or-Retained-diag"), ("028", "Exact-or-Retained-diag"), ("018", "Retained-fuel"), ("032", "Refused-surface"), ("035", "Retained-format")] {
		assert_eq!(dispositions[id]["disposition"], expected);
	}
	assert_eq!(cases.iter().map(|r| r["label"].as_str().unwrap()).collect::<std::collections::HashSet<_>>().len(), cases.len());
	assert_ne!(observed(&Value::from(1i64)), observed(&Value::from(2i64)));
	TAPE.with(|t| t.borrow_mut().clear());
	assert_eq!(tap(1, 7), 7);
	assert!(!tap_bool(2, false));
	TAPE.with(|t| assert_eq!(t.borrow().len(), 2));
	TAPE.with(|t| t.borrow_mut().clear());
}
