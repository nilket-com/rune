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
	json!({"status":"error","missing":missing,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::stock_display(error),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C15 stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C15_ENGINE").unwrap();
	assert!(engine == "stock" || engine == "fresh");
	let index: usize = std::env::var("C15_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let mut row = cases[index].clone();
	if let Ok(fuel) = std::env::var("C15_COUNTERFACTUAL_FUEL") {
		assert_eq!(engine, "stock");
		assert_eq!(index, 125);
		assert_eq!(fuel, "200000");
		row["fuel"] = json!(200000);
	}
	let row = &row;
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());

	if engine == "fresh" {
		let result = observe_fresh(row);
		let path = std::env::var_os("C15_RESULT").unwrap();
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

#[test]
fn single_field_tuple_struct_slice() {
	use crate::experimental_core::value::{Scalar, Type, Value as FreshValue};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("struct T(a);pub fn main(){T(7).0}pub fn reuse(){7}").unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
	let mut store = Store::new(Limits {
		entries: 32,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	for name in ["main", "reuse"] {
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash([name])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(7)));
		store.root_release(root).unwrap();
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
	{
		let source = "struct T(a);pub fn main(){let f=T;f(7).0}";
		let mut sources = Sources::new();
		sources.insert(Source::memory(source).unwrap()).unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14).unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(7)));
		store.root_release(root).unwrap();
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
	assert_eq!(std::mem::size_of::<Type>(), 8);
	assert_eq!(std::mem::size_of::<FreshValue>(), 16);
	println!("C15 SLICE: T(7).0=7; lifecycle closed; reuse7; C16f constructor value admitted; unit/pair widened; Type8 Value16; heap static asserts Payload40 Entry72 Slot80");
}

#[test]
fn nominal_shape_survives_foreign_program_arguments() {
	let c = Context::with_default_modules().unwrap();
	let compile = |text: &str| {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		frontend::compile(&mut sources, &c, &mut Diagnostics::new(), 14).unwrap()
	};
	let unit = compile("struct Foreign;pub fn main(){Foreign}");
	let tuple = compile("struct Foreign();pub fn main(){Foreign()}");
	let render = compile("pub fn main(x){format!(\"{:?}\",x)}");
	let mut store = Store::new(Limits {
		entries: 64,
		bytes: 4096,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	let mut types = Vec::new();
	for (p, expected) in [(&unit, "Foreign"), (&tuple, "Foreign ()")] {
		let value = executor
			.invoke(
				p,
				&mut store,
				p.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		types.push(store.root_tag(&value).unwrap());
		let out = executor
			.invoke(
				&render,
				&mut store,
				render.function(Hash::type_hash(["main"])).unwrap(),
				&[&value],
				200000,
			)
			.unwrap();
		assert_eq!(store.root_text(&out).unwrap(), expected);
		store.root_release(out).unwrap();
		store.root_release(value).unwrap();
	}
	assert_eq!(types[0], types[1]);
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

include!("constructors/fresh_graph.rs");
include!("constructors/fresh.rs");
include!("constructors/contract.rs");
include!("constructors/bounded.rs");

#[test]
fn type_observation_registry_controls() {
	use crate::function::fresh_pipeline as a;
	let c = Context::with_default_modules().unwrap();
	for hash in [a::TYPE_OF, a::TYPE_OF_ASSOC, a::TYPE_NAME] {
		println!(
			"ANY REGISTRY {hash} present={} certified={}",
			c.lookup_function(hash).is_some(),
			c.fresh_any_function(hash)
		);
	}
	assert!(c.fresh_any_function(a::TYPE_NAME));
	assert!(c.fresh_any_function(a::TYPE_OF_ASSOC) || c.fresh_any_function(a::TYPE_OF));
	use crate::experimental_core::error::TypeName as N;
	for name in [
		N::Unit,
		N::Bool,
		N::I64,
		N::U64,
		N::F64,
		N::Char,
		N::Ordering,
		N::String,
		N::Vec,
		N::Tuple,
		N::Object,
		N::Option,
		N::Result,
		N::Range,
		N::RangeFrom,
		N::RangeFull,
		N::RangeInclusive,
		N::RangeToInclusive,
		N::RangeTo,
		N::Function,
		N::Type,
		N::VecIter,
		N::ObjectIter,
		N::OptionIter,
		N::StringSplit,
		N::CharSplit,
		N::Chars,
		N::Skip,
		N::Take,
		N::RangeIter,
		N::UnsignedRangeIter,
		N::CharRangeIter,
		N::RangeInclusiveIter,
		N::UnsignedRangeInclusiveIter,
		N::CharRangeInclusiveIter,
		N::RangeFromIter,
		N::UnsignedRangeFromIter,
		N::CharRangeFromIter,
	] {
		let hash = name.builtin_hash().unwrap();
		let actual = c
			.get_const_value(Hash::associated_function(
				hash,
				&crate::runtime::Protocol::INTO_TYPE_NAME,
			))
			.and_then(|v| v.as_string().ok());
		println!("TYPE NAME REGISTRY {name}: {actual:?}");
	}
}

#[test]
fn nominal_missing_method_proof_rejects_unclosed_receiver_sets() {
	let context = Context::with_default_modules().unwrap();
	for (source, admitted) in [
		(
			"struct T(a);pub fn main(){let t=T(7);let u=t;u.is_empty()}",
			true,
		),
		(
			"struct T(a);pub fn main(){let t=T(7);t=[];t.is_empty()}",
			false,
		),
		("struct T(a);pub fn main(t){t.is_empty()}", false),
		(
			"struct T(a);impl T{fn is_empty(self){true}}pub fn main(){T(7).is_empty()}",
			false,
		),
		("struct T(a);pub fn main(){T(7).collect::<T>()}", false),
	] {
		let mut sources = Sources::new();
		sources.insert(Source::memory(source).unwrap()).unwrap();
		let result = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 14);
		assert_eq!(result.is_ok(), admitted, "{source}");
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: compiles Rune source in bounded 2MiB subprocesses; native CI covers this compiler stack boundary"
)]
fn nested_tuple_constructors_keep_the_normal_stack_boundary() {
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c15-stack-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let out = scratch.0.join("out");
	fs::create_dir(&out).unwrap();
	for arity in [1, 2] {
		for depth in [126, 127, 128] {
			let mut expression = String::from("1");
			for _ in 0..depth {
				expression = if arity == 1 {
					format!("T({expression})")
				} else {
					format!("T({expression},0)")
				};
			}
			let id = format!("{arity}-{depth}");
			let path = out.join(format!("{id}.rn"));
			let declaration = if arity == 1 {
				"struct T(v);"
			} else {
				"struct T(v,w);"
			};
			fs::write(
				&path,
				format!("{declaration} pub fn main(){{{expression};1}}"),
			)
			.unwrap();
			let observed = launch(&path, &path, "stack-main", "fresh", &scratch.0, &out, &id);
			assert!(observed.complete, "{id}: {}", observed.record);
			let fields: Vec<_> = observed.record.split('\t').collect();
			if depth < 128 {
				assert_eq!(fields[0], "value", "{id}: {}", observed.record);
				assert_eq!(unhex(fields[4]), "i64:1:1");
			} else {
				assert_eq!(fields[0], "refused", "{id}: {}", observed.record);
				assert!(
					fields[1] == "Some(DepthLimit)"
						|| (fields[1] == "Some(FrontendDiagnostic)"
							&& unhex(fields[3]).contains("MaxItemDepth { max: 128 }")),
					"{id}: {}",
					observed.record
				);
			}
		}
	}
}
