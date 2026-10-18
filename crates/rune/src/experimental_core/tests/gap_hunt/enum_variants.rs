//! C15b stock VM oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("enum_variants/graph.rs");
const CASES: &str = include_str!("enum_variants/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
include!("enum_variants/fixture.rs");

fn observed(value: &Value) -> Json {
	let out = graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C15b value: {node}"
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

include!("enum_variants/fresh_graph.rs");
include!("enum_variants/fresh.rs");
include!("enum_variants/contract.rs");
include!("enum_variants/bounded.rs");

#[test]
#[ignore = "C15b stock VM oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C15B_ENGINE").unwrap();
	assert!(engine == "stock" || engine == "fresh");
	let index: usize = std::env::var("C15B_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let mut row = cases[index].clone();
	if let Ok(fuel) = std::env::var("C15B_COUNTERFACTUAL_FUEL") {
		assert_eq!(engine, "stock");
		assert_eq!(index, 148);
		assert_eq!(fuel, "200000");
		row["fuel"] = json!(200000);
	}
	let row = &row;
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());

	if engine == "fresh" {
		let result = observe_fresh(row);
		let path = std::env::var_os("C15B_RESULT").unwrap();
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
		.insert(Source::new("c15b.rn", source).unwrap())
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
			let reuse = match crate::runtime::budget::with(200_000, || vm.call(["c15b_reuse"], ()))
				.call()
			{
				Ok(value) => json!({"status":"value","graph":observed(&value)}),
				Err(e) => error(&e),
			};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fixture":row["fixture"],"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let path = std::env::var_os("C15B_RESULT").unwrap();
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
	assert_eq!(rows.len(), 166);
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
	fn run(body: &str) -> Json {
		let source = format!("enum E{{U,Z(),T(a),N{{x}}}} enum F{{T(a)}} pub fn main(){{{body}}}");
		let c = context("default");
		let mut sources = Sources::new();
		sources
			.insert(Source::new("control.rn", source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&c)
			.build_vm()
			.unwrap();
		observed(&vm.call(["main"], ()).unwrap())
	}
	let a = run("E::T(1)");
	let b = run("E::T(2)");
	let other = run("F::T(1)");
	assert_ne!(a, b);
	assert_ne!(a, other);
	assert_eq!(a["nodes"][0][0], "Nominal");
	let n = run("E::N{x:1}");
	let u = run("E::U");
	let z = run("E::Z()");
	assert_eq!(a["nodes"][0][1], n["nodes"][0][1]);
	assert_eq!(a["nodes"][0][1], u["nodes"][0][1]);
	assert_ne!(a["nodes"][0][2], n["nodes"][0][2]);
	assert_ne!(u, z);
	assert_ne!(a, n);
	assert_ne!(a["nodes"][0][1], other["nodes"][0][1]);
	let shared = run("let x=E::T(1);(x,x)");
	let cloned = run("let x=E::T(1);(x,std::clone::clone(x))");
	let distinct = run("(E::T(1),E::T(1))");
	assert_ne!(shared, distinct);
	assert_eq!(cloned, distinct);
	let shared_child = run("let x=E::T([1]);(x,std::clone::clone(x))");
	let distinct_children = run("(E::T([1]),E::T([1]))");
	assert_ne!(shared_child, distinct_children);
}

#[test]
fn tuple_variant_match_slice() {
	use crate::experimental_core::value::{Scalar, Type, Value as FreshValue};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory("enum E{U,T(a)} enum F{T(a)} pub fn main(){match E::T(7){E::U=>0,E::T(x)=>x,_=>9}} pub fn repeat(){let value=E::T(7);let sum=0;for _ in 0..1000{sum+=match value{E::U=>0,E::T(x)=>x,_=>9};}sum} pub fn wrong_type(){match F::T(7){E::U=>0,E::T(x)=>x,_=>9}} pub fn scalar(){match 7{E::U=>0,E::T(x)=>x,_=>9}} pub fn reuse(){7}").unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	for (name, expected) in [
		("main", 7),
		("repeat", 7000),
		("wrong_type", 9),
		("scalar", 9),
		("reuse", 7),
	] {
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash([name])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		assert_eq!(
			store.root_scalar(&root).unwrap(),
			Some(Scalar::I64(expected)),
			"{name}"
		);
		store.root_release(root).unwrap();
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}

	assert_eq!(std::mem::size_of::<Type>(), 8);
	assert_eq!(std::mem::size_of::<FreshValue>(), 16);
	println!("C15b slice: two-arm match=7; 1000 misses and hits=7000; wrong types miss; lifecycle and reuse pass; Type8 Value16");
}

#[test]
fn variant_metadata_survives_code_drop_and_changed_declarations() {
	use crate::experimental_core::{nominal, value::Type};
	let context = Context::with_default_modules().unwrap();
	let compile = |text: &str| {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap()
	};
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	let mut roots = Vec::new();
	for text in [
		"enum C15bLife{V{x}} pub fn main(){C15bLife::V{x:7}}",
		"enum C15bLife{U,V{y}} pub fn main(){C15bLife::V{y:9}}",
		"enum C15bLife{V{z}} pub fn main(){C15bLife::V{z:11}}",
	] {
		let program = compile(text);
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		drop(program);
		roots.push(root);
	}
	let mut ids = Vec::new();
	for (root, field, ordinal) in [
		(&roots[0], "x", 0),
		(&roots[1], "y", 1),
		(&roots[2], "z", 0),
	] {
		let value = store.import(root).unwrap();
		let Type::Struct(n) = store.tag(value).unwrap() else {
			panic!("variant descriptor")
		};
		let rtti = nominal::variant_rtti(n).unwrap();
		assert_eq!(rtti.fields.get(field), Some(&0));
		assert_eq!(rtti.variant_index, ordinal);
		ids.push(n);
	}
	assert_ne!(ids[0].0, ids[1].0);
	assert!(nominal::same_variant(ids[0], ids[1]));
	assert_ne!(ids[0].0, ids[2].0);
	assert!(nominal::same_variant(ids[0], ids[2]));
	for root in roots {
		store.root_release(root).unwrap();
	}
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn variant_miss_does_not_read_a_moved_child() {
	use crate::experimental_core::value::Scalar;
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory("enum C15bMove{U,T(a)} pub fn make(){C15bMove::T([7])} pub fn probe(v){match v{C15bMove::U=>0,_=>1}} pub fn hit(v){match v{C15bMove::T(x)=>x[0],_=>0}}").unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["make"])).unwrap(),
			&[],
			200000,
		)
		.unwrap();
	let value = store.import(&root).unwrap();
	let child = store.host_observe_values(value).unwrap()[0];
	store.invalidate(child).unwrap();
	let result = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["probe"])).unwrap(),
			&[&root],
			200000,
		)
		.unwrap();
	assert_eq!(store.root_scalar(&result).unwrap(), Some(Scalar::I64(1)));
	store.root_release(result).unwrap();
	assert!(executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["hit"])).unwrap(),
			&[&root],
			200000
		)
		.is_err());
	store.root_release(root).unwrap();
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn context_variants_are_refused_before_effects() {
	use crate as rune;
	#[derive(crate::Any)]
	enum HostEnum {
		#[rune(constructor)]
		V(#[rune(get)] i64),
	}
	let mut context = context_for("default", true);
	let mut module = crate::Module::new();
	module.ty::<HostEnum>().unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(){c15b::tap(1,1);HostEnum::V(7)}").unwrap())
		.unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let mut stock_sources = Sources::new();
	stock_sources
		.insert(Source::memory("pub fn main(){c15b::tap(1,1);HostEnum::V(7)}").unwrap())
		.unwrap();
	assert!(crate::prepare(&mut stock_sources)
		.with_context(&context)
		.build_vm()
		.is_ok());
	let error = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14)
		.err()
		.expect("native context variant must be refused");
	assert_eq!(
		error.refusal,
		Some(crate::experimental_core::error::Refusal::Native)
	);
	TAPE.with(|t| assert!(t.borrow().is_empty()));
}

#[test]
fn same_variant_compares_across_tuple_and_named_programs() {
	use crate::experimental_core::value::Scalar;
	let context = Context::with_default_modules().unwrap();
	let texts = [
		"enum C15bShape{V(a)} pub fn main(){C15bShape::V(7)}",
		"enum C15bShape{V{x}} pub fn main(){C15bShape::V{x:7}}",
	];
	let mut roots = Vec::new();
	let mut stock_values = Vec::new();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	for text in texts {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
		roots.push(
			executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					200000,
				)
				.unwrap(),
		);
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		stock_values.push(vm.call(["main"], ()).unwrap());
	}
	assert!(stock::Value::partial_eq(&stock_values[0], &stock_values[1]).unwrap());
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(a,b){a==b}").unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
	let result = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[&roots[0], &roots[1]],
			200000,
		)
		.unwrap();
	assert_eq!(
		store.root_scalar(&result).unwrap(),
		Some(Scalar::Bool(true))
	);
	store.root_release(result).unwrap();
	for root in roots {
		store.root_release(root).unwrap();
	}
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
fn stock_replay_controls() {
	use sha2::{Digest, Sha256};
	let freeze: Json = serde_json::from_str(include_str!("enum_variants/freeze.json")).unwrap();
	for (name, data) in [
		("cases.json", CASES),
		(
			"dispositions.json",
			include_str!("enum_variants/dispositions.json"),
		),
		("stock.json", include_str!("enum_variants/stock.json")),
		(
			"stock_raw.json",
			include_str!("enum_variants/stock_raw.json"),
		),
		("p1.json", include_str!("enum_variants/p1.json")),
		(
			"intentional.json",
			include_str!("enum_variants/intentional.json"),
		),
		(
			"resource_complete.json",
			include_str!("enum_variants/resource_complete.json"),
		),
	] {
		assert_eq!(
			hex(Sha256::digest(data.as_bytes()).as_ref()),
			freeze["files"][name]
		);
	}
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(include_str!("enum_variants/stock.json")).unwrap();
	let raw: Json = serde_json::from_str(include_str!("enum_variants/stock_raw.json")).unwrap();
	let dispositions: Json =
		serde_json::from_str(include_str!("enum_variants/dispositions.json")).unwrap();
	let allowances: Json = serde_json::from_str(include_str!("enum_variants/p1.json")).unwrap();
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
			assert_eq!(dispositions[id]["disposition"], "Refused-frontend");
		} else {
			assert_eq!(raw[id]["observation"]["reuse"]["status"], "value");
		}
	}
	let mut bad = raw["014"].clone();
	bad["observation"]["outcome"]["graph"]["nodes"][1][2] = json!("8");
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["014"]);
	let mut bad = raw["035"].clone();
	bad["observation"]["outcome"]["graph"]["nodes"][0][2] = json!([1, 2]);
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["035"]);
	let mut bad = raw["045"].clone();
	let kind = bad["observation"]["outcome"]["kind"]
		.as_str()
		.unwrap()
		.replace("0x1cad9186c9641c4f", "0x1111111111111111");
	bad["observation"]["outcome"]["kind"] = json!(kind);
	assert_ne!(project(&bad, "", &mut Vec::new()), frozen["045"]);
	let intentional: Json =
		serde_json::from_str(include_str!("enum_variants/intentional.json")).unwrap();
	assert_eq!(intentional.as_object().unwrap().len(), 4);
	for id in ["036", "039", "040", "052"] {
		let rule = &intentional[id];
		assert_eq!(rule["engines"], json!(["stock", "fresh"]));
		assert_eq!(rule["source_sha256"], dispositions[id]["source_sha256"]);
		assert_eq!(
			dispositions[id]["disposition"],
			"Intentional-declaration-order"
		);
		assert_eq!(rule["stock_before_observation"], frozen[id]["observation"]);
		let values = if id == "052" {
			vec![true, true, false]
		} else {
			vec![false, true, true, true, false, false]
		};
		let mut expected = frozen[id]["observation"].clone();
		for (node, value) in expected["outcome"]["graph"]["nodes"]
			.as_array_mut()
			.unwrap()
			.iter_mut()
			.skip(1)
			.zip(values)
		{
			node[2] = json!(value);
		}
		assert_eq!(rule["expected_observation"], expected);
	}
	assert_ne!(
		intentional["036"]["expected_observation"],
		frozen["036"]["observation"]
	);
	let complete: Json =
		serde_json::from_str(include_str!("enum_variants/resource_complete.json")).unwrap();
	let rule = &complete["148"];
	assert_eq!(rule["source_sha256"], dispositions["148"]["source_sha256"]);
	assert_eq!(rule["fresh_fuel"], 32);
	assert_eq!(rule["stock_fuel"], 200000);
	assert_eq!(rule["observation"]["outcome"]["status"], "value");
	let tape = rule["observation"]["events"].as_array().unwrap();
	assert_eq!(tape.len(), 100);
	assert_eq!(frozen["148"]["observation"]["events"], json!(&tape[..1]));
}
