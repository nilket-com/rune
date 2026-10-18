//! C15c nominal clone observations. The operator binds each stock binary and source.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;

include!("constructors/graph.rs");
const CASES: &str = include_str!("nominal_clone/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}

fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|tape| tape.borrow_mut().push(json!({"tag":tag,"value":value})));
	value
}

fn context(fresh: bool) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	for name in ["c15", "c15b", "c15c"] {
		let mut module = crate::Module::with_item([name]).unwrap();
		if fresh {
			module.fresh_host_function("tap", tap).unwrap();
		} else {
			module.function("tap", tap).build().unwrap();
		}
		context.install(module).unwrap();
	}
	context
}

fn observed(value: &Value) -> Json {
	let graph = graph(value);
	for node in graph["nodes"].as_array().unwrap() {
		assert!(
			!matches!(node[0].as_str(), Some("Dynamic" | "OpaqueAny")),
			"unmapped C15c node: {node}"
		);
	}
	graph
}

fn error(error: &stock::VmError) -> Json {
	let kind = error.error().kind();
	let sites = error.stacktrace().iter().map(|at| {
		let instruction = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
		let range = instruction.span.range();
		json!({"source":instruction.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	json!({"status":"error","kind":format!("{kind:?}"),"display":kind.to_string(),"sites":sites,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C15c stock oracle child; bounded external operator only"]
fn child() {
	let index: usize = std::env::var("C15C_CASE").unwrap().parse().unwrap();
	let phase = std::env::var("C15C_PHASE").unwrap();
	assert!(["before", "after"].contains(&phase.as_str()));
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &rows[index];
	let source = row["source"].as_str().unwrap();
	let engine = std::env::var("C15C_ENGINE").unwrap_or_else(|_| "stock".to_owned());
	assert!(["stock", "fresh"].contains(&engine.as_str()));
	if engine == "fresh" {
		let result = observe_fresh(row, &phase);
		write_result(&result);
		return;
	}
	let context = context(false);
	let mut sources = Sources::new();
	sources
		.insert(Source::new(row["source_name"].as_str().unwrap(), source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	TAPE.with(|tape| tape.borrow_mut().clear());
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let (outcome, reuse) = match built {
		Err(error) => (
			json!({"status":"compile-error","error":format!("{error:?}")}),
			Json::Null,
		),
		Ok(mut vm) => {
			let outcome = match stock::budget::with(row["fuel"].as_u64().unwrap() as usize, || {
				vm.call(["main"], ())
			})
			.call()
			{
				Ok(value) => json!({"status":"value","graph":observed(&value)}),
				Err(failure) => error(&failure),
			};
			let reuse = match stock::budget::with(200_000, || {
				vm.call([row["reuse"].as_str().unwrap()], ())
			})
			.call()
			{
				Ok(value) => json!({"status":"value","graph":observed(&value)}),
				Err(failure) => error(&failure),
			};
			(outcome, reuse)
		}
	};
	let events = TAPE.with(|tape| tape.borrow().clone());
	let result = json!({"schema":1,"engine":"stock","phase":phase,"index":index,"label":row["label"],"source_sha256":row["source_sha256"],"source_hex":hex(source.as_bytes()),"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	write_result(&result);
}

fn write_result(result: &Json) {
	let path = std::env::var_os("C15C_RESULT").unwrap();
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
fn oracle_sources_are_unique_and_pinned() {
	use sha2::{Digest, Sha256};
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	assert_eq!(rows.len(), 73);
	let mut labels = std::collections::HashSet::new();
	for (index, row) in rows.iter().enumerate() {
		assert_eq!(row["id"], format!("{index:03}"));
		assert!(labels.insert(row["label"].as_str().unwrap()));
		assert_eq!(
			hex(&Sha256::digest(row["source"].as_str().unwrap().as_bytes())),
			row["source_sha256"]
		);
	}
}

// The original observations stay frozen. Only these source-bound rows change.
pub(super) fn intentional_json(family: &str, case: &Json, old: &Json) -> Option<Json> {
	let rules: Json = serde_json::from_str(include_str!("nominal_clone/intentional.json")).unwrap();
	let rule = rules["original_json_rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|rule| rule["family"] == family && rule["id"] == case["id"])?;
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	let before: Json =
		serde_json::from_str(include_str!("nominal_clone/stock_before.json")).unwrap();
	let after: Json = serde_json::from_str(include_str!("nominal_clone/stock_after.json")).unwrap();
	let id = rule["oracle_id"].as_str().unwrap();
	assert_eq!(
		*old, before[id]["observation"],
		"C15c before {family} {}",
		case["id"]
	);
	Some(after[id]["observation"].clone())
}

pub(super) fn intentional_record(family: &str, label: &str, source: &str, old: String) -> String {
	use sha2::{Digest, Sha256};
	let rules: Json = serde_json::from_str(include_str!("nominal_clone/intentional.json")).unwrap();
	let Some(rule) = rules["original_tsv_rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|rule| rule["family"] == family && rule["label"] == label)
	else {
		return old;
	};
	assert_eq!(
		rule["source_sha256"],
		hex(&Sha256::digest(source.as_bytes()))
	);
	assert_eq!(rule["before"], hex(old.as_bytes()));
	super::unhex(rule["after"].as_str().unwrap())
}

#[test]
#[ignore = "C15c stock contract; new C15C_STOCK_OUT and source-bound native binary"]
fn stock_contract() {
	contract("stock", false);
}

#[test]
#[ignore = "C15c fresh contract; bounded source-bound native children"]
fn fresh_contract() {
	contract("fresh", false);
}

#[test]
#[ignore = "C15c tuple-shaped nominal slice; bounded source-bound native children"]
fn slice_contract() {
	contract("fresh", true);
}

fn contract(engine: &str, slice: bool) {
	use sha2::{Digest, Sha256};
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(
		std::env::var_os(if engine == "stock" {
			"C15C_STOCK_OUT"
		} else {
			"C15C_FRESH_OUT"
		})
		.unwrap(),
	);
	fs::create_dir(&out).unwrap();
	let receipt_path = PathBuf::from(std::env::var_os("C15C_NATIVE_RECEIPT").unwrap());
	let receipt_hash = std::env::var("C15C_NATIVE_RECEIPT_SHA256").unwrap();
	let source = std::env::var("C15C_NATIVE_SOURCE").unwrap();
	let executable = std::env::current_exe().unwrap();
	let binary_hash = hex(&Sha256::digest(fs::read(&executable).unwrap()));
	let rows: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let expected: Json =
		serde_json::from_str(include_str!("nominal_clone/stock_after.json")).unwrap();
	let started = Instant::now();
	let mut count = 0;
	for (index, row) in rows.iter().enumerate() {
		if slice && ![0, 1, 2, 3, 5, 6, 18, 21, 24, 27].contains(&index) {
			continue;
		}
		count += 1;
		assert!(started.elapsed() < Duration::from_secs(1700));
		let receipt_bytes = fs::read(&receipt_path).unwrap();
		assert_eq!(hex(&Sha256::digest(&receipt_bytes)), receipt_hash);
		let receipt: Json = serde_json::from_slice(&receipt_bytes).unwrap();
		assert_eq!(receipt["expected_repo"], source);
		assert_eq!(receipt["binary_sha256"], binary_hash);
		let units = receipt["units"].as_array().unwrap();
		assert!(!units.is_empty());
		assert!(units.iter().all(|unit| unit["recorded_cwd"] == source));
		let dir = out.join(format!("{index:03}"));
		fs::create_dir(&dir).unwrap();
		let result = dir.join("observation.json");
		let stdout = dir.join("stdout");
		let stderr = dir.join("stderr");
		let mut child = Command::new("/usr/bin/prlimit")
			.args(["--cpu=15", "--as=2147483648", "--fsize=8388608", "--"])
			.arg(&executable)
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::nominal_clone::child",
				"--ignored",
				"--nocapture",
				"--test-threads=1",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("C15C_ENGINE", engine)
			.env("C15C_PHASE", "after")
			.env("C15C_CASE", index.to_string())
			.env("C15C_RESULT", &result)
			.current_dir("/tmp")
			.stdin(Stdio::null())
			.stdout(fs::File::create(&stdout).unwrap())
			.stderr(fs::File::create(&stderr).unwrap())
			.spawn()
			.unwrap();
		let launched = Instant::now();
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			if launched.elapsed() > Duration::from_secs(45) {
				child.kill().unwrap();
				panic!("C15c child timeout {index}: {:?}", child.wait());
			}
			std::thread::sleep(Duration::from_millis(5));
		};
		assert!(status.success(), "C15c child {index}: {status}");
		let stdout = fs::read_to_string(&stdout).unwrap();
		assert_eq!(
			stdout
				.matches("test result: ok. 1 passed; 0 failed;")
				.count(),
			1
		);
		assert_eq!(stdout.matches("test result:").count(), 1);
		assert!(fs::read(&stderr).unwrap().is_empty());
		let mut actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		if engine == "fresh" {
			let audit = actual
				.as_object_mut()
				.unwrap()
				.remove("fresh_audit")
				.unwrap();
			assert_eq!(audit["post_code_drop"], true);
			for key in ["row_idle", "executor_idle"] {
				assert_eq!(audit[key], true);
			}
			for key in [
				"row_live_entries",
				"row_payload_bytes",
				"live_entries",
				"payload_bytes",
			] {
				assert_eq!(audit[key], 0);
			}
			actual["engine"] = json!("stock");
		}
		assert_eq!(
			actual,
			expected[row["id"].as_str().unwrap()],
			"C15c stock {index}"
		);
	}
	assert_eq!(
		binary_hash,
		hex(&Sha256::digest(fs::read(&executable).unwrap()))
	);
	fs::write(
		out.join("COMPLETE"),
		format!("{engine}{count} exact; source and test counts checked per child\n"),
	)
	.unwrap();
	println!("CONTRACT ENGINE {engine}\nCONTRACT ROWS {count}");
}

#[test]
fn intentional_rows_preserve_original_observations() {
	let rules: Json = serde_json::from_str(include_str!("nominal_clone/intentional.json")).unwrap();
	assert_eq!(rules["original_json_rows"].as_array().unwrap().len(), 15);
	for (family, cases, stock) in [
		(
			"constructors",
			include_str!("constructors/cases.json"),
			include_str!("constructors/stock.json"),
		),
		(
			"enum_variants",
			include_str!("enum_variants/cases.json"),
			include_str!("enum_variants/stock.json"),
		),
	] {
		let cases: Vec<Json> = serde_json::from_str(cases).unwrap();
		let stock: Json = serde_json::from_str(stock).unwrap();
		for case in cases {
			let id = case["id"].as_str().unwrap();
			if let Some(after) = intentional_json(family, &case, &stock[id]["observation"]) {
				assert_ne!(after, stock[id]["observation"]);
			}
		}
	}
	assert_eq!(rules["original_tsv_rows"].as_array().unwrap().len(), 3);
	let mut changed = 0;
	for (family, data) in [
		(
			"len_clone_stock.tsv",
			include_str!("../len_clone_stock.tsv"),
		),
		(
			"global_clone_stock.tsv",
			include_str!("../global_clone_stock.tsv"),
		),
	] {
		for line in data.lines().skip(1) {
			let row: Vec<_> = line.split('\t').collect();
			let source = super::unhex(row[1]);
			let old = super::unhex(row[2]);
			let after = intentional_record(family, row[0], &source, old.clone());
			changed += usize::from(after != old);
		}
	}
	assert_eq!(changed, 3);
}

// Independent observations: this function never reads frozen expectations.
include!("enum_variants/fresh_graph.rs");
fn observe_fresh(row: &Json, phase: &str) -> Json {
	let source = row["source"].as_str().unwrap();
	let context = context(true);
	let mut sources = Sources::new();
	sources
		.insert(Source::new(row["source_name"].as_str().unwrap(), source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 15).unwrap();
	let mut store = Store::new(Limits {
		entries: 4096,
		bytes: 4 << 20,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	TAPE.with(|tape| tape.borrow_mut().clear());
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			row["fuel"].as_u64().unwrap() as usize,
		)
		.unwrap();
	let value = store.import(&root).unwrap();
	let outcome = json!({"status":"value","graph":fresh_graph(&mut store, value)});
	store.root_release(root).unwrap();
	let events = TAPE.with(|tape| tape.borrow().clone());
	let row_idle = executor.is_idle();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program
				.function(Hash::type_hash([row["reuse"].as_str().unwrap()]))
				.unwrap(),
			&[],
			200_000,
		)
		.unwrap();
	let value = store.import(&root).unwrap();
	let reuse = json!({"status":"value","graph":fresh_graph(&mut store, value)});
	store.root_release(root).unwrap();
	// Keep a fresh result alive across compiler metadata destruction. Compare
	// actual observations from the same script, without consulting the oracle.
	let held = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			row["fuel"].as_u64().unwrap() as usize,
		)
		.unwrap();
	drop(program);
	let value = store.import(&held).unwrap();
	assert_eq!(fresh_graph(&mut store, value), outcome["graph"]);
	store.root_release(held).unwrap();
	json!({"schema":1,"engine":"fresh","phase":phase,"index":row["id"].as_str().unwrap().parse::<usize>().unwrap(),"label":row["label"],"source_sha256":row["source_sha256"],"source_hex":hex(source.as_bytes()),"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"events":events,"reuse":reuse},"fresh_audit":{"post_code_drop":true,"row_idle":row_idle,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes()}})
}
