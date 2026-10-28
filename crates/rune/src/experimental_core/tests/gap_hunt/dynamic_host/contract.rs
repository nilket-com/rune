// Paired bounded children compare live stock with the execution engine.
const C10_REPLAY_CAP: usize = 64 << 20;
const C10_FRESH_EXPECTATIONS: &str = include_str!("fresh_expectations.json");

fn c10_expectations() -> Json {
	serde_json::from_str(C10_FRESH_EXPECTATIONS).unwrap()
}

fn c10_data_seals() {
	assert_eq!(CASES.len(), COUNT);
	assert_eq!(COUNT, 126);
	let rules = c10_expectations();
	assert_eq!(rules.as_object().unwrap().len(), 33);
	let mut counts = BTreeMap::new();
	let mut labels = std::collections::BTreeSet::new();
	for &(label, source, policy) in CASES {
		assert!(labels.insert(label), "duplicate C10 label {label}");
		*counts.entry(policy).or_insert(0usize) += 1;
		if policy == "Exact" {
			assert!(rules.get(label).is_none());
		} else {
			let rule = &rules[label];
			assert_eq!(rule["policy"], policy);
			assert_eq!(rule["source_sha256"], c10_sha(source.as_bytes()));
			assert!(rule["fresh"].is_object());
		}
	}
	assert_eq!(
		counts,
		BTreeMap::from([
			("Exact", 93),
			("Refused-runtime", 28),
			("Retained-arity", 1),
			("Retained-isolation", 4)
		])
	);
}

fn c10_drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = C10_REPLAY_CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	bytes
}

fn c10_launch(engine: &str, i: usize, folder: &Path) -> (Json, Vec<u8>, Vec<u8>) {
	use std::os::unix::process::CommandExt;
	let (label, source, policy) = CASES[i];
	fs::create_dir_all(folder).unwrap();
	let result = folder.join("observation.json");
	let mut child = Command::new("/usr/bin/prlimit")
		.args([
			"--cpu=30",
			"--as=2147483648",
			"--fsize=67108864",
			"--core=0",
			"--",
		])
		.arg(std::env::current_exe().unwrap())
		.process_group(0)
		.args([
			"--exact",
			"experimental_core::tests::gap_hunt::dynamic_host::child",
			"--ignored",
			"--nocapture",
			"--quiet",
		])
		.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("HOME", fs::canonicalize(folder).unwrap())
		.env("LANG", "C.UTF-8")
		.env("TZ", "UTC")
		.env("C10_ENGINE", engine)
		.env("C10_CASE", i.to_string())
		.env("C10_RESULT", &result)
		.current_dir(folder)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let exceeded = Arc::new(AtomicBool::new(false));
	let flag = exceeded.clone();
	let pipe = child.stdout.take().unwrap();
	let stdout = std::thread::spawn(move || c10_drain(pipe, flag));
	let flag = exceeded.clone();
	let pipe = child.stderr.take().unwrap();
	let stderr = std::thread::spawn(move || c10_drain(pipe, flag));
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
		if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(45) {
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
	let status = child.wait().unwrap();
	let stdout = stdout.join().unwrap();
	let stderr = stderr.join().unwrap();
	fs::write(folder.join("stdout"), &stdout).unwrap();
	fs::write(folder.join("stderr"), &stderr).unwrap();
	fs::write(folder.join("receipt.json"), serde_json::to_vec_pretty(&json!({"index":i,"label":label,"engine":engine,"success":status.success(),"stop":stop,"seconds":start.elapsed().as_secs_f64(),"stdout_sha256":c10_sha(&stdout),"stderr_sha256":c10_sha(&stderr)})).unwrap()).unwrap();
	assert!(
		!stop && status.success() && !exceeded.load(Ordering::Relaxed),
		"row {label} raw at {}",
		folder.display()
	);
	assert!(fs::metadata(&result).unwrap().len() <= C10_REPLAY_CAP as u64);
	let envelope: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
	assert_eq!(envelope["schema"], 1);
	assert_eq!(envelope["engine"], engine);
	assert_eq!(envelope["index"], i);
	assert_eq!(envelope["label"], label);
	assert_eq!(envelope["source_hex"], hex(source.as_bytes()));
	assert_eq!(envelope["policy"], policy);
	assert_eq!(envelope["fuel"], FUEL);
	(envelope, stdout, stderr)
}

#[test]
fn contract() {
	c10_data_seals();
	let rules = c10_expectations();
	let certificate: Json =
		serde_json::from_str(include_str!("../s7b1_host_outputs.json")).unwrap();
	let owned = std::env::var_os("C10_REPLAY_OUT").is_none();
	let out = std::env::var_os("C10_REPLAY_OUT")
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			std::env::temp_dir().join(format!(
				"rune-c10-live-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			))
		});
	fs::create_dir(&out).unwrap();
	let total = Instant::now();
	for (i, &(label, source, policy)) in CASES.iter().enumerate() {
		assert!(
			total.elapsed() < Duration::from_secs(3600),
			"C10 whole replay bound"
		);
		let dir = out.join(format!("{i:03}"));
		let (mut stock, stock_stdout, stock_stderr) = c10_launch("stock", i, &dir.join("stock"));
		let (mut fresh, stdout, stderr) = c10_launch("fresh", i, &dir.join("fresh"));
		assert_eq!(stdout, stock_stdout, "stdout {label}");
		assert_eq!(stderr, stock_stderr, "stderr {label}");
		assert_eq!(stdout, b"\nrunning 1 test\n", "child framing {label}");
		assert!(stderr.is_empty(), "child stderr {label}");
		let legacy_warnings = stock["observation"]
			.as_object_mut()
			.unwrap()
			.remove("warnings")
			.expect("legacy warning multiset");
		let execution_warnings = fresh["observation"]
			.as_object_mut()
			.unwrap()
			.remove("warnings")
			.expect("execution warning multiset");
		assert!(legacy_warnings.is_array() && execution_warnings.is_array());
		if stock["observation"]["invoked"] == true && fresh["observation"]["invoked"] == true {
			assert_eq!(
				execution_warnings, legacy_warnings,
				"compile warning multiset {label}"
			);
		}
		// Runtime policies retain every original nonwarning field. The warning
		// metadata above is verified independently against the live compiler.
		let portable = c10_project(&stock["observation"], i).unwrap();
		let stock_common = c10_common(&portable).unwrap();
		let rule = &certificate["rows"][i];
		assert_eq!(rule["index"], i);
		assert_eq!(rule["label"], label);
		assert_eq!(rule["source_sha256"], c10_sha(source.as_bytes()));
		if rule["policy"] == "Refused-compile" {
			let observed = &fresh["observation"];
			assert_eq!(
				observed["outcome"],
				json!({"status":"refused","kind":"Unsupported","operation":"Native","refusal":"Native","primary":rule["primary"]}),
				"{i} {label}"
			);
			assert_eq!(observed["invoked"], false);
			assert_eq!(observed["events"], json!([]));
			assert_eq!(observed["events_after_reuse"], json!([]));
			assert_eq!(observed["host_calls"], 0);
			assert_eq!(observed["protocol_calls"], 0);
			assert_eq!(observed["reuse"]["status"], "value");
			assert_eq!(
				observed["reuse"]["graph"],
				runtime_graph(&RuntimeValue::from(7i64))
			);
			c10_audit(&fresh["audit"], "Refused-compile", label);
			continue;
		}
		c10_audit(&fresh["audit"], policy, label);
		let expected = if policy == "Exact" {
			stock_common
		} else {
			c10_arity_expected(
				i,
				label,
				&hex(source.as_bytes()),
				policy,
				rules[label]["fresh"].clone(),
			)
			.unwrap()
		};
		c10_compare(&fresh["observation"], &expected, label).unwrap_or_else(|e| {
			panic!(
				"strict execution row {i} {label}: {e}; raw at {}",
				dir.display()
			)
		});
	}
	contract_completed("fresh", COUNT);
	if owned {
		fs::remove_dir_all(out).unwrap();
	}
}
// seq15065: one exact Retained-arity diagnostic pin, never a diagnostic filter.
fn c10_arity_expected(
	index: usize,
	label: &str,
	source_hex: &str,
	policy: &str,
	mut expected: Json,
) -> Result<Json, String> {
	if index != 124 && label != "argument/wrong-arity" {
		return Ok(expected);
	}
	if index != 124 || label != "argument/wrong-arity" || policy != "Retained-arity" {
		return Err("arity diagnostic pin row/policy mismatch".into());
	}
	let rules: Json = serde_json::from_str(include_str!("../../dynamic_host_arity.json")).unwrap();
	if rules.as_object().unwrap().len() != 1 {
		return Err("arity pin count".into());
	}
	let rule = &rules[label];
	let source = c10_hex_bytes(source_hex)?;
	if rule["index"] != index
		|| rule["source_sha256"] != c10_sha(&source)
		|| expected["outcome"] != rule["fresh_expectation"]
		|| expected["diagnostics"] != "[]"
	{
		return Err("arity diagnostic source/frozen expectation drift".into());
	}
	let primary = &rule["fresh_expectation"]["primary"];
	let start = primary["start"].as_u64().unwrap() as usize;
	let end = primary["end"].as_u64().unwrap() as usize;
	if source.get(start..end).map(hex).as_deref() != primary["span_hex"].as_str() {
		return Err("arity diagnostic span bytes drift".into());
	}
	expected["diagnostics"] = rule["fresh_top_level_diagnostics"].clone();
	Ok(expected)
}
