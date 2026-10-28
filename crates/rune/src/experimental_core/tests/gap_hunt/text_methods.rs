//! C14 stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value as RuntimeValue};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("text_methods/runtime_graph.rs");
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
include!("text_methods/graph.rs");
include!("text_methods/run.rs");

fn observed(value: &RuntimeValue) -> Json {
	let out = runtime_graph(value);
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
	json!({"status":"error","missing":missing,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::runtime_error_display(error),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

/// Observe `row` (case `index`) on the stock VM in this process.
fn legacy_observe(index: usize, row: &Json) -> Json {
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
	json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fixture":row["fixture"],"observation":{"outcome":outcome,"events":events,"reuse":reuse}})
}

/// Drop `raw` debug evidence and mask each `AnyTypeInfo::full_name` pointer in
/// a typed kind (P1), recording each masked leaf's path and count in `leaves`.
fn stock_projection(v: &Json, path: &str, leaves: &mut Vec<Json>) -> Json {
	match v {
		Json::Object(m) => Json::Object(
			m.iter()
				.filter(|(k, _)| k.as_str() != "raw")
				.map(|(k, v)| {
					(
						k.clone(),
						stock_projection(
							v,
							&if path.is_empty() {
								k.clone()
							} else {
								format!("{path}.{k}")
							},
							leaves,
						),
					)
				})
				.collect(),
		),
		Json::Array(a) => Json::Array(
			a.iter()
				.enumerate()
				.map(|(i, v)| stock_projection(v, &format!("{path}.{i}"), leaves))
				.collect(),
		),
		Json::String(s) if s.contains("full_name:") => {
			assert!(path.ends_with("kind"));
			let mut tail = s.as_str();
			let mut out = String::new();
			let mut count = 0;
			while let Some(i) = tail.find("full_name:") {
				assert!(tail[..i].ends_with("AnyTypeInfo { "));
				out.push_str(&tail[..i]);
				let digits = tail[i..].strip_prefix("full_name: 0x").unwrap();
				let n = digits.bytes().take_while(|b| b.is_ascii_hexdigit()).count();
				assert!(n > 0 && digits[n..].starts_with(", hash: 0x"));
				out.push_str("full_name: <pointer>");
				tail = &digits[n..];
				count += 1;
			}
			out.push_str(tail);
			leaves.push(json!({"path":path,"count":count}));
			json!(out)
		}
		_ => v.clone(),
	}
}

/// The comparable stock observation for `case`: the live stock observation `raw` with
/// D1 displays restored to their original text and the P1 projection applied
/// to exactly the leaves `p1.json` allows.
fn expected_stock(case: &Json, raw: &Json) -> Json {
	let id = case["id"].as_str().unwrap();
	let label = case["label"].as_str().unwrap();
	let source = case["source"].as_str().unwrap();
	for key in ["label", "source_sha256", "fixture", "fuel"] {
		assert_eq!(raw[key], case[key], "{id} {key}");
	}
	assert_eq!(raw["source_hex"], hex(source.as_bytes()));
	let mut raw = raw.clone();
	raw["observation"]["outcome"] = super::d1c_diagnostics::before(
		"text_methods",
		label,
		source,
		"stock",
		&raw["observation"]["outcome"],
	);
	raw["observation"] = super::method_diagnostics::intentional_json_before(
		"text_methods",
		label,
		source,
		"stock",
		&raw["observation"],
	);
	let allowances: Json = serde_json::from_str(include_str!("text_methods/p1.json")).unwrap();
	let mut leaves = Vec::new();
	let projected = stock_projection(&raw["observation"], "", &mut leaves);
	assert_eq!(
		json!(leaves),
		allowances.get(id).cloned().unwrap_or(json!([])),
		"P1 {id}"
	);
	projected
}

#[test]
#[ignore = "C14 stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C14_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let index: usize = std::env::var("C14_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let mut row = cases[index].clone();
	if let Ok(fuel) = std::env::var("C14_COUNTERFACTUAL_FUEL") {
		assert_eq!(engine, "stock");
		assert_eq!(index, 184);
		assert_eq!(fuel, "200000");
		row["fuel"] = json!(200000);
	}
	let row = &row;
	TAPE.with(|t| t.borrow_mut().clear());

	if engine == "fresh" {
		let result = observe(row);
		let path = std::env::var_os("C14_RESULT").unwrap();
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
	let result = legacy_observe(index, row);
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
	assert_ne!(
		observed(&RuntimeValue::from(1i64)),
		observed(&RuntimeValue::from(2i64))
	);
	assert_ne!(
		observed(&RuntimeValue::try_from("a").unwrap()),
		observed(&RuntimeValue::try_from("b").unwrap())
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

include!("text_methods/contract.rs");
include!("text_methods/bounded.rs");
