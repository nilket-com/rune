//! C12 frozen stock oracle and independent new observations.
use super::*;
use crate::runtime::{self as stock, Value as RuntimeValue};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("small_forms/runtime_graph.rs");
const C12_CASES: &str = include_str!("small_forms/cases.json");
const C12_ADMISSIONS: &str = include_str!("small_forms/admissions.json");
thread_local! {static C12_TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}
fn tap(tag: i64, value: i64) -> i64 {
	C12_TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
fn tap_bool(tag: i64, value: bool) -> bool {
	C12_TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"bool","value":value}))
	});
	value
}
fn c12_context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c12"]).unwrap();
	module.function("tap", tap).build().unwrap();
	module.function("tap_bool", tap_bool).build().unwrap();
	context.install(module).unwrap();
	context
}
fn c12_vec(values: impl IntoIterator<Item = RuntimeValue>) -> RuntimeValue {
	let mut out = crate::alloc::Vec::new();
	for value in values {
		out.try_push(value).unwrap();
	}
	RuntimeValue::vec(out).unwrap()
}
fn c12_input(name: &str) -> (RuntimeValue, RuntimeValue) {
	let state = c12_vec([]);
	let value = match name {
		"none" => RuntimeValue::unit(),
		"empty" => c12_vec([]),
		"one" => c12_vec([RuntimeValue::from(2i64)]),
		"ints" => c12_vec((1..=4i64).map(RuntimeValue::from)),
		"nested" => c12_vec([
			c12_vec([RuntimeValue::from(1i64)]),
			c12_vec([RuntimeValue::from(2i64)]),
		]),
		"cycle" => {
			let value = c12_vec([]);
			value
				.borrow_mut::<stock::Vec>()
				.unwrap()
				.push(value.clone())
				.unwrap();
			value
		}
		_ => panic!("unreviewed C12 fixture"),
	};
	(value, state)
}
fn c12_graph(value: &RuntimeValue) -> Json {
	let out = runtime_graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C12 output: {node}"
		);
	}
	out
}
fn legacy_c12_error(error: &stock::VmError) -> Json {
	let kind = error.error().kind();
	let sites=error.stacktrace().iter().map(|at|{
		let inst=at.unit.debug_info().expect("debug unit").instruction_at(at.ip).expect("debug instruction");
		let range=inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	let chain = error
		.chain()
		.iter()
		.map(|e| json!({"kind":format!("{:?}",e.kind()),"display":e.to_string()}))
		.collect::<Vec<_>>();
	json!({"status":"error","class":runtime_error_class(kind),"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::runtime_error_display(error),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}
fn c12_cleanup(name: &str, v: &RuntimeValue) -> Json {
	if name != "cycle" {
		return Json::Null;
	}
	match v.borrow_mut::<stock::Vec>() {
		Ok(mut values) => {
			values.clear();
			json!({"cycle_cleared":true})
		}
		Err(e) => json!({"cycle_cleared":false,"error":format!("{e:?}")}),
	}
}
#[test]
#[ignore = "C12 stock observer child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C12_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let index: usize = std::env::var("C12_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(C12_CASES).unwrap();
	let row = &cases[index];
	if engine == "fresh" {
		let mut result = c12_observe(row);
		result["schema"] = json!(1);
		result["engine"] = json!("fresh");
		result["index"] = json!(index);
		result["label"] = row["label"].clone();
		result["source_hex"] = json!(hex(row["source"].as_str().unwrap().as_bytes()));
		result["source_sha256"] = row["source_sha256"].clone();
		result["fixture"] = row["fixture"].clone();
		result["fuel"] = row["fuel"].clone();
		let path = std::env::var_os("C12_RESULT").unwrap();
		use std::io::Write;
		let mut file = std::fs::OpenOptions::new()
			.create_new(true)
			.write(true)
			.open(path)
			.unwrap();
		file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
			.unwrap();
		return;
	}
	let source = row["source"].as_str().unwrap();
	let fixture = row["fixture"].as_str().unwrap();
	C12_TAPE.with(|t| t.borrow_mut().clear());
	let context = c12_context();
	let (v, state) = c12_input(fixture);
	let before = json!({"receiver":c12_graph(&v),"state":c12_graph(&state)});
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c12.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut invoked = false;
	let (outcome, reuse) = match built {
		Err(e) => (
			json!({"status":"compile-error","error":format!("{e:?}")}),
			Json::Null,
		),
		Ok(mut vm) => {
			invoked = true;
			let result =
				crate::runtime::budget::with(row["fuel"].as_u64().unwrap() as usize, || {
					if fixture == "none" {
						vm.call(["main"], ())
					} else {
						vm.call(["main"], (v.clone(), state.clone()))
					}
				})
				.call();
			let outcome = match result {
				Ok(value) => json!({"status":"value","graph":c12_graph(&value)}),
				Err(e) => legacy_c12_error(&e),
			};
			let reuse =
				match crate::runtime::budget::with(200_000, || vm.call(["c12_reuse"], ())).call() {
					Ok(value) => json!({"status":"value","graph":c12_graph(&value)}),
					Err(e) => legacy_c12_error(&e),
				};
			(outcome, reuse)
		}
	};
	let post = json!({"receiver":c12_graph(&v),"state":c12_graph(&state)});
	let events = C12_TAPE.with(|t| t.borrow().clone());
	let cleanup = c12_cleanup(fixture, &v);
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fixture":fixture,"fuel":row["fuel"],"provisional":row["provisional"],"invoked":invoked,"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"before":before,"post":post,"events":events,"cleanup":cleanup,"reuse":reuse}});
	let path = std::env::var_os("C12_RESULT").unwrap();
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
	let cases: Vec<Json> = serde_json::from_str(C12_CASES).unwrap();
	assert_eq!(cases.len(), 111);
	assert_eq!(
		cases
			.iter()
			.map(|r| r["label"].as_str().unwrap())
			.collect::<std::collections::HashSet<_>>()
			.len(),
		111
	);
	assert_ne!(
		c12_graph(&RuntimeValue::from(1i64)),
		c12_graph(&RuntimeValue::from(2i64))
	);
	let a = RuntimeValue::new(stock::RangeInclusive::new(
		RuntimeValue::from(1i64),
		RuntimeValue::from(2i64),
	))
	.unwrap();
	let b = RuntimeValue::new(stock::RangeInclusive::new(
		RuntimeValue::from(1i64),
		RuntimeValue::from(3i64),
	))
	.unwrap();
	assert_ne!(c12_graph(&a), c12_graph(&b));
	let (v, _) = c12_input("cycle");
	let g = c12_graph(&v);
	assert_eq!(g["nodes"][0][2][0], g["root"]);
	assert_eq!(c12_cleanup("cycle", &v)["cycle_cleared"], true);
	C12_TAPE.with(|t| t.borrow_mut().clear());
	assert_eq!(tap(1, 7), 7);
	assert!(!tap_bool(2, false));
	C12_TAPE.with(|t| assert_eq!(t.borrow().len(), 2));
	C12_TAPE.with(|t| t.borrow_mut().clear());
}

include!("small_forms/p1.rs");
include!("small_forms/frozen.rs");

include!("small_forms/graph.rs");
include!("small_forms/error.rs");
include!("small_forms/run.rs");
include!("small_forms/contract.rs");

include!("small_forms/retained.rs");
