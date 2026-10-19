//! C11c move-capture stock oracle. This module is test-only; it does not alter product code.
use super::*;
use crate::runtime::{self as stock, Value};
use serde_json::{json, Value as Json};
use std::cell::RefCell;
include!("captures/graph.rs");
include!("move_captures/fresh.rs");
include!("move_captures/fresh_contract.rs");
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
		_ => stock_class(kind),
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
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::stock_display(error),"sites":sites,"chain":chain,"raw":format!("{error:?}")})
}

#[test]
#[ignore = "C11c stock-oracle child; explicit bounded external operator only"]
fn child() {
	let engine = std::env::var("C11C_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let index: usize = std::env::var("C11C_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = match std::env::var_os("C11C_CASES_FILE") {
		Some(path) => serde_json::from_slice(&fs::read(path).unwrap()).unwrap(),
		None => serde_json::from_str(CASES).unwrap(),
	};
	let row = &cases[index];
	if engine == "fresh" {
		fresh_child(index, row);
	}
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let context = context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c11b.rn", source).unwrap())
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
	let result = json!({"schema":1,"engine":"stock","index":index,"label":row["label"],"source":source,"origin":row["origin"],"logical":logical,"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"events":events,"reuse":reuse}});
	let mut result = result;
	result["projected"] = projection(&result);
	if std::env::var_os("C11C_REPLAY").is_some() {
		let frozen: Json = serde_json::from_str(if std::env::var_os("C11C_CASES_FILE").is_some() {
			include_str!("move_captures/function_tokens_stock.json")
		} else {
			include_str!("move_captures/stock.json")
		})
		.unwrap();
		assert_eq!(result["projected"], frozen[index]["projected"]);
	}
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
const REPLAY_CAP: usize = 2 << 20;
fn replay_drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = REPLAY_CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	bytes
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C11c frozen stock replay; bounded children and exclusive GAP_OUT"]
fn stock_contract() {
	run_contract("stock");
}

#[cfg(target_os = "linux")]
fn run_contract(engine: &str) {
	run_contract_cases(engine, false);
}

#[cfg(target_os = "linux")]
fn run_contract_cases(engine: &str, function_copy: bool) {
	assert!(matches!(engine, "stock" | "fresh"));
	let cases_text = if function_copy {
		include_str!("move_captures/function_tokens_cases.json")
	} else {
		include_str!("move_captures/cases.json")
	};
	let cases: Json = serde_json::from_str(cases_text).unwrap();
	let mut frozen: Json = serde_json::from_str(if function_copy {
		include_str!("move_captures/function_tokens_stock.json")
	} else {
		include_str!("move_captures/stock.json")
	})
	.unwrap();
	if function_copy {
		for row in frozen.as_array_mut().unwrap() {
			let case = row["case"].as_object().unwrap().clone();
			row.as_object_mut().unwrap().extend(case);
		}
	}
	let cases = cases.as_array().unwrap();
	let frozen = frozen.as_array().unwrap();
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
	assert_eq!(cases.len(), frozen.len());
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let supplemental_cases = out.join("function_tokens_cases.json");
	if function_copy {
		fs::write(&supplemental_cases, cases_text).unwrap();
	}
	let mut labels = std::collections::BTreeSet::new();
	for (i, (case, expected)) in cases.iter().zip(frozen).enumerate() {
		let label = case["label"].as_str().unwrap();
		assert!(labels.insert(label));
		assert_eq!(case["label"], expected["label"]);
		assert_eq!(case["source"], expected["source"]);
		let folder = out.join(format!("{i:03}"));
		fs::create_dir(&folder).unwrap();
		let result = folder.join("observation.json");
		use std::os::unix::process::CommandExt;
		let mut command = Command::new("/usr/bin/prlimit");
		command
			.args([
				"--cpu=15",
				"--as=2147483648",
				"--fsize=2097152",
				"--core=0",
				"--",
			])
			.arg(std::env::current_exe().unwrap());
		command.process_group(0);
		command.env_clear();
		if function_copy {
			command.env("C11C_CASES_FILE", &supplemental_cases);
		}
		let mut child = command
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::move_captures::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env("C11C_ENGINE", engine)
			.env("C11C_REPLAY", "1")
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C11C_CASE", i.to_string())
			.env("C11C_RESULT", &result)
			.env("D1E_TRACE_OUT", folder.join("logical.json"))
			.current_dir(&folder)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let exceeded = Arc::new(AtomicBool::new(false));
		let flag = exceeded.clone();
		let pipe = child.stdout.take().unwrap();
		let stdout = std::thread::spawn(move || replay_drain(pipe, flag));
		let flag = exceeded.clone();
		let pipe = child.stderr.take().unwrap();
		let stderr = std::thread::spawn(move || replay_drain(pipe, flag));
		let start = Instant::now();
		let mut stop = false;
		let mut exited = false;
		loop {
			if child.try_wait().unwrap().is_some() {
				exited = true;
			}
			if !exceeded.load(Ordering::Relaxed)
				&& exited && stdout.is_finished()
				&& stderr.is_finished()
			{
				break;
			}
			if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(20) {
				stop = true;
				let _ = Command::new("/usr/bin/kill")
					.args(["-KILL", "--", &format!("-{}", child.id())])
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.status();
				break;
			}
			std::thread::sleep(Duration::from_millis(10));
		}
		// wait() also returns the cached status when try_wait() already reaped
		// the child. Every exit path explicitly collects it before joining.
		let status = child.wait().unwrap();
		let stdout = stdout.join().unwrap();
		let stderr = stderr.join().unwrap();
		fs::write(folder.join("stdout"), &stdout).unwrap();
		fs::write(folder.join("stderr"), &stderr).unwrap();
		fs::write(
			folder.join("receipt.txt"),
			format!(
				"case:{label}\nrc:{status}\nstop:{stop}\nelapsed:{:?}\n",
				start.elapsed()
			),
		)
		.unwrap();
		assert!(!stop && status.success() && !exceeded.load(Ordering::Relaxed));
		assert!(fs::metadata(&result).unwrap().len() <= REPLAY_CAP as u64);

		let observed: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		assert_eq!(observed["schema"], 1);
		assert_eq!(observed["engine"], engine);
		assert_eq!(observed["index"], i);
		assert_eq!(observed["fuel"], case["fuel"]);
		assert_eq!(observed["label"], case["label"]);
		assert_eq!(observed["source"], case["source"]);
		assert_eq!(observed["origin"], case["origin"]);
		if engine == "stock" {
			assert_eq!(observed["projected"], expected["projected"], "{label}");
		} else if function_copy {
			verify_function_copy_record(&observed, case, expected, &disposition["rows"][i]);
		} else {
			verify_fresh_record(&observed, case, expected, &disposition["rows"][i]);
		}
		assert_eq!(hex(&stdout), expected["stdout_hex"]);
		assert_eq!(hex(&stderr), expected["stderr_hex"]);
	}
	super::contract_completed(engine, cases.len());
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
	assert_ne!(observed(&Value::from(1i64)), observed(&Value::from(2i64)));
}
