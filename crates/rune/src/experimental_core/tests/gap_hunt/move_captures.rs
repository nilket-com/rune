//! C11c move-capture stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value as RuntimeValue};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("captures/runtime_graph.rs");
include!("move_captures/run.rs");
include!("move_captures/contract.rs");
const CASES: &str = include_str!("move_captures/cases.json");

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
fn legacy_context() -> Context {
	let mut legacy_context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c11b"]).unwrap();
	module.function("tap", tap).build().unwrap();
	module.function("tap_bool", tap_bool).build().unwrap();
	legacy_context.install(module).unwrap();
	legacy_context
}
fn observed(value: &RuntimeValue) -> Json {
	let out = runtime_graph(value);
	for node in out["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C11b value: {node}"
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
#[ignore = "C11c stock-oracle child; explicit bounded external operator only"]
fn child_driver() {
	let engine = std::env::var("C11C_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let index: usize = std::env::var("C11C_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = match std::env::var_os("C11C_CASES_FILE") {
		Some(path) => serde_json::from_slice(&fs::read(path).unwrap()).unwrap(),
		None => serde_json::from_str(CASES).unwrap(),
	};
	let row = &cases[index];
	if engine == "fresh" {
		child(index, row);
	}
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let legacy_context = legacy_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c11b.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&legacy_context)
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
				match crate::runtime::budget::with(200_000, || vm.call(["c11b_reuse"], ())).call() {
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
	let path = std::env::var_os("C11C_RESULT").unwrap();
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

fn projection(record: &Json) -> Json {
	fn outcome(o: &Json) -> Json {
		if o["status"] == "error" {
			json!({"status":o["status"],"class":o["class"],"display":o["display"],"chain_displays":o["chain"].as_array().unwrap().iter().map(|e|e["display"].clone()).collect::<Vec<_>>(),"sites":o["sites"]})
		} else {
			o.clone()
		}
	}
	json!({"outcome":outcome(&record["observation"]["outcome"]),"events":record["observation"]["events"],"reuse":outcome(&record["observation"]["reuse"]),"diagnostics":record["diagnostics"],"logical":record["logical"]})
}
/// Run every case on live stock, then on the execution engine, and compare.
#[cfg(target_os = "linux")]
fn run_contract_cases(function_copy: bool) {
	let cases_text = if function_copy {
		include_str!("move_captures/function_tokens_cases.json")
	} else {
		include_str!("move_captures/cases.json")
	};
	let cases: Json = serde_json::from_str(cases_text).unwrap();
	let cases = cases.as_array().unwrap();
	assert_eq!(cases.len(), if function_copy { 8 } else { 71 });
	let disposition: Json = serde_json::from_str(if function_copy {
		include_str!("move_captures/function_tokens_disposition.json")
	} else {
		include_str!("move_captures/disposition.json")
	})
	.unwrap();
	assert_eq!(disposition["rows"].as_array().unwrap().len(), cases.len());
	let mut counts = std::collections::BTreeMap::new();
	for row in disposition["rows"].as_array().unwrap() {
		*counts
			.entry(row["classification"].as_str().unwrap())
			.or_insert(0usize) += 1;
	}
	if function_copy {
		assert_eq!(counts.get("Intentional"), Some(&8));
	} else {
		assert_eq!(counts.get("exact"), Some(&66));
		assert_eq!(counts.get("inherited-refusal"), Some(&1));
		assert_eq!(counts.get("policy-presentation"), Some(&4));
		assert_eq!(counts.values().sum::<usize>(), 71);
	}
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let supplemental_cases = out.join("function_tokens_cases.json");
	if function_copy {
		fs::write(&supplemental_cases, cases_text).unwrap();
	}
	let cases_file = function_copy.then_some(supplemental_cases.as_path());
	let mut labels = std::collections::BTreeSet::new();
	for (i, case) in cases.iter().enumerate() {
		let label = case["label"].as_str().unwrap();
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, case, &folder.join("stock"), cases_file);
		let expected = &stock.observation;
		let fresh = launch("fresh", i, case, &folder.join("fresh"), cases_file);
		if function_copy {
			verify_function_copy_record(
				&fresh.observation,
				case,
				expected,
				&disposition["rows"][i],
			);
		} else {
			verify_record(&fresh.observation, case, expected, &disposition["rows"][i]);
		}
		assert_eq!(hex(&fresh.stdout), hex(&stock.stdout), "stdout: {label}");
		assert_eq!(hex(&fresh.stderr), hex(&stock.stderr), "stderr: {label}");
	}
	super::contract_completed("fresh", cases.len());
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
#[cfg(target_os = "linux")]
fn launch(
	engine: &str,
	index: usize,
	case: &Json,
	folder: &Path,
	cases_file: Option<&Path>,
) -> super::live_oracle::Run {
	let index_text = index.to_string();
	let trace = folder.join("logical.json");
	let mut env = vec![
		("C11C_ENGINE", engine.as_ref()),
		("C11C_CASE", index_text.as_ref()),
		("D1E_TRACE_OUT", trace.as_os_str()),
	];
	if let Some(path) = cases_file {
		env.push(("C11C_CASES_FILE", path.as_os_str()));
	}
	let run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::move_captures::child_driver",
		case["label"].as_str().unwrap(),
		folder,
		"C11C_RESULT",
		&env,
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

#[test]
fn oracle_case_bindings() {
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let seed: Vec<Json> = serde_json::from_str(include_str!("../c11c_stopgap/seeds.json")).unwrap();
	let old: Vec<Json> = serde_json::from_str(include_str!("captures/cases.json")).unwrap();
	assert_eq!(rows.len(), 71);
	assert_eq!(seed.len(), 10);
	assert_eq!(old.len(), 37);
	for (row, expected) in rows[..10].iter().zip(seed) {
		assert_eq!(row["source"], expected["source"]);
	}
	for (row, expected) in rows[34..].iter().zip(old) {
		assert_eq!(row["source"], expected["source"]);
		assert_eq!(row["fuel"], expected["fuel"]);
		assert_eq!(row["source_sha256"], expected["source_sha256"]);
	}
	let mut labels = std::collections::BTreeSet::new();
	for row in rows {
		assert!(labels.insert(row["label"].as_str().unwrap().to_string()));
	}
	assert_ne!(
		observed(&RuntimeValue::from(1i64)),
		observed(&RuntimeValue::from(2i64))
	);
}
