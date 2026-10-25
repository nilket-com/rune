//! C16d labeled-block oracle and fix-both contracts.
use super::*;
use serde_json::{json, Value as Json};
include!("tuple_let/observe.rs");
fn legacy_observe(source: &str, mode: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c8.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			return json!({"status":"compile-error","error":format!("{e:?}"),"diagnostics":format!("{:?}",diagnostics.diagnostics())})
		}
	};
	assert!(matches!(mode, "c8-main" | "c8-small-store"));
	let result = crate::runtime::budget::with(FUEL, || vm.call(["main"], ())).call();
	match result {
		Ok(value) => {
			if value.into_unit().is_ok() {
				return json!({"status":"value","type":"Unit"});
			}
			use crate::runtime::Inline;
			match value.as_inline() {
				Some(Inline::Float(n)) => {
					json!({"status":"value","type":"F64","bits":format!("{:016x}",n.to_bits())})
				}
				Some(Inline::Signed(n)) => {
					json!({"status":"value","type":"I64","value":n.to_string()})
				}
				Some(Inline::Unsigned(n)) => {
					json!({"status":"value","type":"U64","value":n.to_string()})
				}
				Some(Inline::Bool(n)) => json!({"status":"value","type":"Bool","value":n}),
				_ => panic!("C16d subject returned non-scalar"),
			}
		}
		Err(e) => {
			super::d1e_diagnostics::stock(&e, &sources);
			// Capture metadata directly inside the crate, no parsing Display/Debug.
			let kind = e.error().kind();
			let class = match kind {
				crate::runtime::VmErrorKind::Panic { .. } => "Panic".into(),
				_ => runtime_error_class(kind),
			};
			let chain = e
				.chain()
				.iter()
				.map(|at| json!({"kind":format!("{:?}",at.kind()),"display":at.to_string()}))
				.collect::<Vec<_>>();
			let sites = e.stacktrace().iter().map(|at| {
				let inst = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
				let range = inst.span.range();
				json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
			}).collect::<Vec<_>>();
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::runtime_error_display(&e),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

fn exact_keys(o: &serde_json::Map<String, Json>, names: &[&str]) {
	assert_eq!(o.len(), names.len());
	for name in names {
		assert!(o.contains_key(*name));
	}
}
fn has_address(text: &str) -> bool {
	text.as_bytes().windows(2).enumerate().any(|(i, p)| {
		p == b"0x"
			&& text.as_bytes()[i + 2..]
				.iter()
				.take_while(|b| b.is_ascii_hexdigit())
				.count() >= 9
	})
}
fn project(o: &Json) -> Json {
	let obj = o.as_object().unwrap();
	match o["status"].as_str().unwrap() {
		"value" => {
			let t = o["type"].as_str().unwrap();
			match t {
				"Unit" => exact_keys(obj, &["status", "type"]),
				"F64" => {
					exact_keys(obj, &["status", "type", "bits"]);
					let b = o["bits"].as_str().unwrap();
					assert_eq!(b.len(), 16);
					assert!(b
						.bytes()
						.all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
				}
				"I64" | "U64" => {
					exact_keys(obj, &["status", "type", "value"]);
					let n = o["value"].as_str().unwrap();
					if t == "I64" {
						n.parse::<i64>().unwrap();
					} else {
						n.parse::<u64>().unwrap();
					}
				}
				"Bool" => {
					exact_keys(obj, &["status", "type", "value"]);
					o["value"].as_bool().unwrap();
				}
				_ => panic!("unmapped scalar"),
			}
			o.clone()
		}
		"error" => {
			exact_keys(
				obj,
				&[
					"status", "class", "kind", "display", "chain", "sites", "raw",
				],
			);
			assert!(!has_address(o["class"].as_str().unwrap()));
			assert!(!has_address(o["display"].as_str().unwrap()));
			let chain = o["chain"]
				.as_array()
				.unwrap()
				.iter()
				.map(|e| {
					exact_keys(e.as_object().unwrap(), &["kind", "display"]);
					assert!(!has_address(e["display"].as_str().unwrap()));
					e["display"].clone()
				})
				.collect::<Vec<_>>();
			for site in o["sites"].as_array().unwrap() {
				exact_keys(
					site.as_object().unwrap(),
					&["source", "start", "end", "frames"],
				);
				for k in ["source", "start", "end"] {
					site[k].as_u64().unwrap();
				}
				assert!(site["start"].as_u64().unwrap() <= site["end"].as_u64().unwrap());
				assert!(!has_address(site["frames"].as_str().unwrap()));
			}
			json!({"status":o["status"],"class":o["class"],"display":o["display"],"chain_displays":chain,"sites":o["sites"]})
		}
		"compile-error" => {
			exact_keys(obj, &["status", "error", "diagnostics"]);
			o.clone()
		}
		_ => panic!("unreviewed status"),
	}
}

#[test]
#[ignore = "C16d oracle child; external bounded controller only"]
fn child() {
	let rows: Json = serde_json::from_str(include_str!("labeled_blocks/cases.json")).unwrap();
	let index: usize = std::env::var("C16_CASE").unwrap().parse().unwrap();
	let row = &rows[index];
	let source = row["source"].as_str().unwrap();
	let engine = std::env::var("C16_ENGINE").unwrap_or_else(|_| "stock".into());
	let (observation, audit) = match engine.as_str() {
		"stock" => (legacy_observe(source, "c8-main"), None),
		"fresh" => {
			let (observation, audit) = observe(source, "c8-main");
			(observation, Some(audit))
		}
		_ => panic!("unknown C16 engine"),
	};
	if engine == "stock"
		&& std::env::var_os("C16_REPLAY").is_some()
		&& !matches!(index, 41 | 42 | 57 | 58 | 69)
	{
		let frozen: Json = serde_json::from_str(include_str!("labeled_blocks/stock.json")).unwrap();
		assert_eq!(frozen[index]["label"], row["label"]);
		assert_eq!(frozen[index]["source"], row["source"]);
		assert_eq!(project(&observation), frozen[index]["projected"]);
	}
	let projected = if engine == "stock" {
		project(&observation)
	} else {
		observation.clone()
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":row["label"],"policy":row["policy"],"source":source,"fuel":FUEL,"outcome":observation,"projected":projected,"fresh_audit":audit});
	fs::write(
		std::env::var_os("C16_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
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
#[ignore = "C16d frozen stock replay; bounded children and exclusive GAP_OUT"]
fn legacy_contract() {
	run_contract("stock");
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C16d strict execution replay; bounded children and exclusive GAP_OUT"]
fn contract() {
	run_contract("fresh");
}

#[cfg(target_os = "linux")]
fn run_contract(engine: &str) {
	assert!(matches!(engine, "stock" | "fresh"));
	let cases: Json = serde_json::from_str(include_str!("labeled_blocks/cases.json")).unwrap();
	let frozen: Json = serde_json::from_str(include_str!("labeled_blocks/stock.json")).unwrap();
	let cases = cases.as_array().unwrap();
	let frozen = frozen.as_array().unwrap();
	assert_eq!(cases.len(), 97);
	assert_eq!(cases.len(), frozen.len());
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let fixes: Json = serde_json::from_str(include_str!("labeled_blocks/fix_both.json")).unwrap();
	for (i, (case, old_expected)) in cases.iter().zip(frozen).enumerate() {
		let mut expected = old_expected.clone();
		if let Some(fix) = fixes.as_array().unwrap().iter().find(|f| f["index"] == i) {
			assert_eq!(fix["old"], *old_expected);
			expected["outcome"] = fix["new"].clone();
			expected["projected"] = fix["new"].clone();
			expected["stdout_hex"] = fix["stdout_hex"].clone();
			expected["stderr_hex"] = fix["stderr_hex"].clone();
		}
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
		let mut child = command
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::labeled_blocks::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("C16_ENGINE", engine)
			.env("C16_REPLAY", "1")
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C16_CASE", i.to_string())
			.env("C16_RESULT", &result)
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
		assert_eq!(observed["fuel"], FUEL);
		assert_eq!(observed["label"], case["label"]);
		assert_eq!(observed["source"], case["source"]);
		assert_eq!(observed["policy"], case["policy"]);
		let logical: Json =
			serde_json::from_str(include_str!("labeled_blocks/logical.json")).unwrap();
		if engine == "stock" {
			assert_eq!(observed["projected"], expected["projected"], "{label}");
		} else {
			let audit = &observed["fresh_audit"];
			for key in [
				"no_cleanup_error",
				"clean_after_row",
				"row_idle",
				"reuse",
				"idle",
			] {
				assert_eq!(audit[key], true, "{label}: {key}");
			}
			for key in [
				"row_live_entries",
				"row_payload_bytes",
				"live_entries",
				"payload_bytes",
			] {
				assert_eq!(audit[key], 0, "{label}: {key}");
			}
			let actual = &observed["outcome"];
			if matches!(i, 80 | 81 | 85) {
				// The HIR accepts these targets; new lowering refuses them before invocation.
				assert_eq!(expected["outcome"]["status"], "compile-error");
				assert_eq!(actual["status"], "refused", "{label}");
				assert_eq!(actual["refusal"], "LoopControl", "{label}");
				assert_eq!(audit["invoked"], false);
			} else if matches!(i, 76 | 77) {
				// Fuel counts and instruction locations are engine-specific resource facts.
				assert_eq!(actual["status"], "error");
				assert_eq!(audit["typed_failure_kind"], "FuelExhausted");
			} else if expected["outcome"]["status"] == "compile-error" {
				assert_eq!(actual["status"], "compile-error", "{label}");
				assert_eq!(
					actual["diagnostics"], expected["outcome"]["diagnostics"],
					"{label}"
				);
				assert_eq!(audit["invoked"], false);
			} else if case["policy"] == "Retain-refusal" {
				assert_eq!(actual["status"], "refused", "{label}");
				assert_eq!(actual["refusal"], "LabelledBlock", "{label}");
				assert_eq!(audit["invoked"], false);
			} else if logical.get(label).is_some() && !matches!(i, 58 | 76 | 77) {
				let actual = project(actual);
				let expected = &expected["projected"];
				for key in ["status", "class", "display", "chain_displays"] {
					assert_eq!(actual[key], expected[key], "{label}: {key}");
				}
				let sites = actual["sites"].as_array().unwrap();
				let oracle_sites = expected["sites"].as_array().unwrap();
				assert_eq!(sites.len(), oracle_sites.len());
				for (a, b) in sites.iter().zip(oracle_sites) {
					for key in ["source", "start", "end"] {
						assert_eq!(a[key], b[key]);
					}
				}
			} else {
				assert_eq!(audit["invoked"], true, "{label}");
				assert_eq!(project(actual), expected["projected"], "{label}");
			}
		}
		if let Some(expected) = logical.get(label).filter(|_| !matches!(i, 58 | 76 | 77)) {
			let actual: Json =
				serde_json::from_slice(&fs::read(folder.join("logical.json")).unwrap()).unwrap();
			assert_eq!(
				actual, *expected,
				"{label}: logical frames/context/rendered"
			);
		}
		assert_eq!(hex(&stdout), expected["stdout_hex"]);
		assert_eq!(hex(&stderr), expected["stderr_hex"]);
	}
	super::contract_completed(engine, cases.len());
}

// These witnesses belong to the fix-both slice, not the frozen pre-fix oracle.
const CLEANUP_WITNESSES: [(&str, &str, &str); 4] = [
	("break-iterator", "pub fn main(){let v=[1,2];for x in v{'b:{break;}}v[0]=9;v[0]}", "9"),
	("break-nested-blocks", "pub fn main(){let v=[1,2];for x in v{'a:{'b:{let alias=v;break;}}}v[0]=9;v[0]}", "9"),
	("continue-iterator", "pub fn main(){let v=[1,2];let n=0;for x in v{'b:{n+=x;continue;}n+=100;}v[0]=9;n+v[0]}", "12"),
	("continue-nested-blocks", "pub fn main(){let v=[1,2];let n=0;for x in v{'a:{'b:{let alias=v;n+=x;continue;}}n+=100;}v[0]=9;n+v[0]}", "12"),
];

#[test]
#[ignore = "C16d fix-both cleanup child; external bounded controller only"]
fn cleanup_child() {
	let index: usize = std::env::var("C16_WITNESS").unwrap().parse().unwrap();
	let (label, source, expected) = CLEANUP_WITNESSES[index];
	let engine = std::env::var("C16_ENGINE").unwrap();
	let (observation, audit) = match engine.as_str() {
		"stock" => (legacy_observe(source, "c8-main"), None),
		"fresh" => {
			let (observation, audit) = observe(source, "c8-main");
			(observation, Some(audit))
		}
		_ => panic!("unknown C16 engine"),
	};
	fs::write(std::env::var_os("C16_RESULT").unwrap(), serde_json::to_vec_pretty(
		&json!({"label":label,"source":source,"engine":engine,"outcome":observation,"fresh_audit":audit})
	).unwrap()).unwrap();
	assert_eq!(observation["status"], "value", "{label}: {observation}");
	assert_eq!(observation["type"], "I64");
	assert_eq!(observation["value"], expected);
}
