//! C17d iterator terminal stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value as RuntimeValue};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("constructors/runtime_graph.rs");
const CASES: &str = include_str!("iter_terminals/cases.json");
const ROWS: usize = 580;

thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
fn context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c17d"]).unwrap();
	module.function("tap", tap).build().unwrap();
	context.install(module).unwrap();

	context
}
fn observed(value: &RuntimeValue) -> Json {
	let out = runtime_graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C17d value: {node}"
		);
	}
	out
}
fn error(error: &stock::VmError, sources: &Sources) -> Json {
	super::d1e_diagnostics::stock(error, sources);
	let kind = error.error().kind();
	let class = match kind {
		crate::runtime::VmErrorKind::Panic { .. } => "Panic".into(),
		crate::runtime::VmErrorKind::UnsupportedCallFn { .. } => "Type".into(),
		_ => runtime_error_class(kind),
	};
	let sites = error.stacktrace().iter().map(|at| {
		let inst = at.unit.debug_info().expect("debug unit").instruction_at(at.ip).expect("debug instruction");
		let range = inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	let chain = error
		.chain()
		.iter()
		.map(|e| json!({"kind":format!("{:?}",e.kind()),"display":e.to_string()}))
		.collect::<Vec<_>>();
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::runtime_error_display(error),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C17d stock-oracle child; explicit bounded external operator only"]
fn legacy_child() {
	let engine = std::env::var("C17D_ENGINE").unwrap();
	assert_eq!(engine, "stock");
	let index: usize = std::env::var("C17D_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let context = context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c17d.rn", source).unwrap())
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
				Err(e) => error(&e, &sources),
			};
			let reuse = if row["reuse_enabled"] == true {
				match crate::runtime::budget::with(200_000, || vm.call(["c17d_reuse"], ())).call() {
					Ok(value) => json!({"status":"value","graph":observed(&value)}),
					Err(e) => error(&e, &sources),
				}
			} else {
				Json::Null
			};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let logical = std::env::var_os("D1E_TRACE_OUT")
		.and_then(|p| fs::read(p).ok())
		.map(|b| serde_json::from_slice::<Json>(&b).unwrap())
		.unwrap_or(Json::Null);
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source":source,"origin":row["origin"],"logical":logical,"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let mut result = result;
	result["projected"] = projection(&result);
	let path = std::env::var_os("C17D_RESULT").unwrap();
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(path)
		.unwrap();
	use std::io::Write;
	file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
		.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

/// Masks `AnyTypeInfo::full_name` in a stock `Debug` string. It is a
/// process-local function pointer; every other byte, including the type hash
/// and the typed error shape, is kept.
fn stable_debug(raw: &str) -> String {
	let mut out = String::new();
	let mut rest = raw;
	const PREFIX: &str = "AnyTypeInfo { full_name: 0x";
	while let Some(at) = rest.find(PREFIX) {
		out.push_str(&rest[..at]);
		out.push_str(PREFIX);
		rest = &rest[at + PREFIX.len()..];
		let end = rest.find(',').unwrap();
		assert!(end > 0 && rest[..end].bytes().all(|b| b.is_ascii_hexdigit()));
		out.push_str("<process-local>");
		rest = &rest[end..];
	}
	out.push_str(rest);
	out
}

fn projection(record: &Json) -> Json {
	fn outcome(o: &Json) -> Json {
		if o["status"] == "error" {
			json!({"status":o["status"],"class":stable_debug(o["class"].as_str().unwrap()),"kind":stable_debug(o["kind"].as_str().unwrap()),"chain_kinds":o["chain"].as_array().unwrap().iter().map(|e|stable_debug(e["kind"].as_str().unwrap())).collect::<Vec<_>>(),"display":o["display"],"chain_displays":o["chain"].as_array().unwrap().iter().map(|e|e["display"].clone()).collect::<Vec<_>>(),"sites":o["sites"]})
		} else {
			o.clone()
		}
	}
	json!({"outcome":outcome(&record["observation"]["outcome"]),"events":record["observation"]["events"],"reuse":outcome(&record["observation"]["reuse"]),"diagnostics":record["diagnostics"],"logical":record["logical"]})
}

#[test]
fn observer_controls() {
	use sha2::{Digest, Sha256};
	let freeze: Json = serde_json::from_str(include_str!("iter_terminals/freeze.json")).unwrap();
	for (name, bytes) in [
		("cases.json", CASES),
		(
			"constructors/graph.rs",
			include_str!("constructors/runtime_graph.rs"),
		),
	] {
		assert_eq!(
			hex(Sha256::digest(
				crate::experimental_core::tests::name_projection::observer_bytes(name, bytes)
			)
			.as_ref()),
			freeze["files"][name]
		);
	}
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	assert_eq!(rows.len(), ROWS);
	let mut labels = std::collections::BTreeSet::new();
	for row in rows {
		assert!(labels.insert(row["label"].as_str().unwrap().to_owned()));
		assert_eq!(
			hex(Sha256::digest(row["source"].as_str().unwrap().as_bytes()).as_ref()),
			row["source_sha256"]
		);
	}
	// Float bits, integer kinds and chars are observable exactly.
	let one = |v: RuntimeValue| observed(&v);
	assert_ne!(
		one(RuntimeValue::from(0.0f64)),
		one(RuntimeValue::from(-0.0f64))
	);
	assert_ne!(
		one(RuntimeValue::from(f64::NAN)),
		one(RuntimeValue::from(-f64::NAN))
	);
	assert_ne!(one(RuntimeValue::from(1i64)), one(RuntimeValue::from(1u64)));
	assert_ne!(
		one(RuntimeValue::from(1i64)),
		one(RuntimeValue::from(1.0f64))
	);
	assert_ne!(one(RuntimeValue::from('a')), one(RuntimeValue::from(97i64)));
	assert_eq!(
		one(RuntimeValue::from(0.5f64)),
		one(RuntimeValue::from(0.5f64))
	);
	// Aliasing is observable: one shared element versus two equal elements.
	let vec = || crate::to_value(crate::runtime::Vec::new()).unwrap();
	let shared = vec();
	let alias = crate::to_value((shared.clone(), shared)).unwrap();
	let split = crate::to_value((vec(), vec())).unwrap();
	assert_ne!(observed(&alias), observed(&split));
	assert_eq!(
		stable_debug("AnyTypeInfo { full_name: 0x123, hash: 0x77 }"),
		stable_debug("AnyTypeInfo { full_name: 0xab, hash: 0x77 }")
	);
}

#[test]
#[ignore = "C17d admission inventory; bounded external operator only"]
fn compile_child() {
	let index: usize = std::env::var("C17D_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c17d"]).unwrap();
	module.host_function("tap", tap).unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c17d.rn", row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let result = match frontend::compile(&mut sources, &context, &mut diagnostics, 16) {
		Ok(_) => json!({"status":"compiled"}),
		Err(error) => {
			json!({"status":"refused","refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
		}
	};
	fs::write(
		std::env::var_os("C17D_RESULT").unwrap(),
		serde_json::to_vec_pretty(&result).unwrap(),
	)
	.unwrap();
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C17d execution contract against live stock; bounded children and exclusive GAP_OUT"]
fn contract() {
	let cases: Json = serde_json::from_str(CASES).unwrap();
	let cases = cases.as_array().unwrap();
	assert_eq!(cases.len(), ROWS);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	for (i, case) in cases.iter().enumerate() {
		assert!(labels.insert(case["label"].as_str().unwrap()));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, case, &folder.join("stock"));
		let fresh = launch("fresh", i, case, &folder.join("fresh"));
		verify_case(i, case, &stock, &fresh);
	}
	super::contract_completed("fresh", cases.len());
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, case: &Json, folder: &Path) -> super::live_oracle::Run {
	let index_text = index.to_string();
	let trace = folder.join("logical.json");
	let run = super::live_oracle::run_child(
		if engine == "stock" {
			"experimental_core::tests::gap_hunt::iter_terminals::legacy_child"
		} else {
			"experimental_core::tests::gap_hunt::iter_terminals::child"
		},
		case["label"].as_str().unwrap(),
		folder,
		"C17D_RESULT",
		&[
			("C17D_ENGINE", engine.as_ref()),
			("C17D_CASE", index_text.as_ref()),
			("D1E_TRACE_OUT", trace.as_os_str()),
		],
	);
	let observed = &run.observation;
	assert_eq!(observed["schema"], 1);
	assert_eq!(observed["engine"], engine);
	assert_eq!(observed["index"], index);
	assert_eq!(observed["fuel"], case["fuel"]);
	assert_eq!(observed["label"], case["label"]);
	assert_eq!(observed["source"], case["source"]);
	assert_eq!(observed["origin"], case["origin"]);
	run
}

include!("iter_terminals/run.rs");

include!("iter_terminals/contract.rs");
