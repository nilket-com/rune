//! C15 stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("constructors/graph.rs");
const CASES: &str = include_str!("constructors/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
include!("constructors/fixture.rs");

fn observed(value: &Value) -> Json {
	let out = graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C15 value: {node}"
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
#[ignore = "C15 stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C15_ENGINE").unwrap();
	assert!(engine == "stock");
	let index: usize = std::env::var("C15_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = cases[index].clone();
	let row = &row;
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());

	let context = context(row["fixture"].as_str().unwrap());
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c15.rn", source).unwrap())
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
				match crate::runtime::budget::with(200_000, || vm.call(["c15_reuse"], ())).call() {
					Ok(value) => json!({"status":"value","graph":observed(&value)}),
					Err(e) => error(&e),
				};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fixture":row["fixture"],"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let path = std::env::var_os("C15_RESULT").unwrap();
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
	assert_eq!(rows.len(), 133);
	let mut labels = std::collections::HashSet::new();
	for (i, r) in rows.iter().enumerate() {
		assert_eq!(r["id"], format!("{i:03}"));
		assert!(labels.insert(r["label"].as_str().unwrap()));
		assert_eq!(
			hex(Sha256::digest(r["source"].as_str().unwrap().as_bytes()).as_ref()),
			r["source_sha256"]
		);
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
	let _ = context("default");
}

#[test]
fn nominal_observer_controls() {
	fn run(source: &str) -> Json {
		let c = context("default");
		let mut sources = Sources::new();
		sources
			.insert(Source::new("control.rn", source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&c)
			.build_vm()
			.unwrap();
		let value = vm.call(["main"], ()).unwrap();
		observed(&value)
	}
	let a = run("struct T(a);pub fn main(){T(1)}");
	let b = run("struct T(a);pub fn main(){T(2)}");
	let c = run("struct Other(a);pub fn main(){Other(1)}");
	assert_ne!(a, b);
	assert_ne!(a, c);
	assert_eq!(a["nodes"][0][0], "Nominal");
	let shared = run("struct T(a);pub fn main(){let t=T(1);(t,t)}");
	let separate = run("struct T(a);pub fn main(){(T(1),T(1))}");
	assert_ne!(shared, separate);
	let cycle = run("struct T(a);pub fn main(){let t=T([]);t.0.push(t);t}");
	assert_eq!(cycle["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn stock_replay_controls() {
	use sha2::{Digest, Sha256};
	let freeze: Json = serde_json::from_str(include_str!("constructors/freeze.json")).unwrap();
	for (name, data) in [
		("cases.json", CASES),
		(
			"dispositions.json",
			include_str!("constructors/dispositions.json"),
		),
		("stock.json", include_str!("constructors/stock.json")),
		(
			"stock_raw.json",
			include_str!("constructors/stock_raw.json"),
		),
		("p1.json", include_str!("constructors/p1.json")),
	] {
		assert_eq!(
			hex(Sha256::digest(data.as_bytes()).as_ref()),
			freeze["files"][name]
		);
	}
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(include_str!("constructors/stock.json")).unwrap();
	let raw: Json = serde_json::from_str(include_str!("constructors/stock_raw.json")).unwrap();
	let dispositions: Json =
		serde_json::from_str(include_str!("constructors/dispositions.json")).unwrap();
	let allowances: Json = serde_json::from_str(include_str!("constructors/p1.json")).unwrap();
	fn project(v: &Json, path: &str, leaves: &mut Vec<Json>) -> Json {
		match v {
			Json::Object(m) => Json::Object(
				m.iter()
					.filter(|(k, _)| k.as_str() != "raw")
					.map(|(k, v)| {
						(
							k.clone(),
							project(
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
					.map(|(i, v)| project(v, &format!("{path}.{i}"), leaves))
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
	for r in rows {
		let id = r["id"].as_str().unwrap();
		let mut leaves = Vec::new();
		assert_eq!(project(&raw[id], "", &mut leaves), frozen[id]);
		assert_eq!(
			json!(leaves),
			allowances.get(id).cloned().unwrap_or(json!([]))
		);
		for key in ["source_sha256", "fixture", "fuel"] {
			assert_eq!(raw[id][key], r[key]);
			assert_eq!(dispositions[id][key], r[key]);
		}
		assert_eq!(
			raw[id]["source_hex"],
			hex(r["source"].as_str().unwrap().as_bytes())
		);
		if raw[id]["observation"]["outcome"]["status"] == "compile-error" {
			assert_eq!(raw[id]["observation"]["reuse"], Json::Null);
			assert_eq!(dispositions[id]["disposition"], "Refused-frontend-arity");
		} else {
			assert_eq!(raw[id]["observation"]["reuse"]["status"], "value");
		}
	}
	let mut bad = raw["018"].clone();
	bad["observation"]["outcome"]["graph"]["nodes"][1][2] = json!("8");
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["018"]);
	let mut bad = raw["131"].clone();
	bad["observation"]["outcome"]["graph"]["nodes"][0][2] = json!([1, 2]);
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["131"]);
	let mut bad = raw["059"].clone();
	let kind = bad["observation"]["outcome"]["kind"]
		.as_str()
		.unwrap()
		.replace("0x1cad9186c9641c4f", "0x1111111111111111");
	bad["observation"]["outcome"]["kind"] = json!(kind);
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["059"]);
}
