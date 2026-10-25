// Strict cross-engine assertions. Only complete VM/Failure audit envelopes
// are outside the language observation; raw typed kind, values and sites remain.
fn c12_without_raw(mut value: Json) -> Json {
	if let Some(map) = value.as_object_mut() {
		map.remove("raw");
		for value in map.values_mut() {
			*value = c12_without_raw(value.take());
		}
	} else if let Some(array) = value.as_array_mut() {
		for value in array {
			*value = c12_without_raw(value.take());
		}
	}
	value
}
fn c12_compare(i: usize, frozen: &Json, actual: &Json, counts: &Json) {
	for key in ["label", "source_sha256", "fixture", "fuel"] {
		assert_eq!(actual[key], frozen[key], "row{i}/{key}");
	}
	assert_eq!(actual["source_hex"], frozen["stock"]["source_hex"]);
	let audit = &actual["fresh_audit"];
	assert!(audit["cleanup_error"].is_null());
	assert_eq!(audit["row_idle"], true);
	assert_eq!(audit["executor_idle"], true);
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"live_entries",
		"payload_bytes",
	] {
		assert_eq!(audit[key], 0, "row{i}/{key}");
	}
	let source = unhex(actual["source_hex"].as_str().unwrap());
	let d1_observed = super::method_diagnostics::intentional_json_before(
		"small_forms",
		actual["label"].as_str().unwrap(),
		&source,
		"fresh",
		&actual["observation"],
	);
	let observed = &d1_observed;
	let expected = &frozen["stock"]["observation"];
	assert_eq!(observed["before"], expected["before"]);
	assert_eq!(observed["cleanup"], expected["cleanup"]);
	assert_eq!(observed["reuse"], expected["reuse"]);
	match frozen["policy"].as_str().unwrap() {
		"Refused-surface" => {
			assert_eq!(actual["invoked"], false);
			assert_eq!(observed["outcome"]["status"], "refused");
			assert_eq!(
				observed["outcome"]["refusal"],
				frozen["fresh_expectation"]["refusal"]
			);
			assert_eq!(observed["post"], observed["before"]);
			assert_eq!(observed["events"], json!([]));
		}
		"Retained-fuel" => {
			assert_eq!(actual["invoked"], true);
			assert_eq!(observed["outcome"]["status"], "error");
			assert_eq!(observed["outcome"]["class"], "FuelExhausted");
			c12_enforce_resource(i, frozen, observed).unwrap();
			assert_eq!(observed["outcome"]["chain"], expected["outcome"]["chain"]);
			for key in ["events", "post", "reuse"] {
				assert_eq!(observed[key], frozen["fresh_expectation"][key]);
			}
		}
		"Exact" => {
			assert_eq!(actual["invoked"], true);
			let mut allowed = counts.get(format!("{i:03}")).cloned().unwrap_or(json!({}));
			allowed
				.as_object_mut()
				.unwrap()
				.retain(|path, _| !path.ends_with("/raw"));
			let projected = project_p1(
				&json!({"observation":c12_without_raw(observed.clone())}),
				&allowed,
			)
			.unwrap();
			let effective = c12_enforce_diagnostics(i, frozen, &projected["observation"]).unwrap();
			assert_eq!(
				effective,
				c12_without_raw(expected.clone()),
				"C12 execution row{i}"
			);
		}
		other => panic!("unreviewed policy {other}"),
	}
}

fn c12_with_admissions(frozen: &[Json]) -> Vec<Json> {
	let overlay: Json = serde_json::from_str(C12_ADMISSIONS).unwrap();
	assert_eq!(overlay["schema"], 1);
	let admissions = overlay["admissions"].as_array().unwrap();
	let mut effective = frozen.to_vec();
	let mut seen = std::collections::BTreeSet::new();
	for admission in admissions {
		let index = admission["id"].as_str().unwrap().parse::<usize>().unwrap();
		assert!(seen.insert(index), "duplicate C12 admission {index}");
		let row = effective.get_mut(index).expect("C12 admission row");
		assert_eq!(row["label"], admission["label"]);
		assert_eq!(row["source_sha256"], admission["source_sha256"]);
		assert_eq!(row["policy"], admission["from_policy"]);
		assert_eq!(row["policy"], "Refused-surface");
		assert_eq!(row["stock"]["observation"], admission["expected"]);
		row["policy"] = admission["policy"].clone();
		row["fresh_expectation"] = admission["expected"].clone();
	}
	assert_eq!(seen, std::collections::BTreeSet::from([101, 109]));
	effective
}
#[test]
#[ignore = "C12 strict execution contract; explicit new C12_OUT, bounded children"]
fn contract() {
	use std::process::{Command, Stdio};
	use std::sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(std::env::var_os("C12_OUT").expect("new C12_OUT"));
	fs::create_dir(&out).unwrap();
	let (cases, frozen, counts) = c12_data();
	let effective = c12_with_admissions(&frozen);
	let start = Instant::now();
	for (i, (_case, expected)) in cases.iter().zip(&effective).enumerate() {
		assert!(start.elapsed() < Duration::from_secs(1700));
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let result = dir.join("observation.json");
		let mut child = Command::new("/usr/bin/prlimit")
			.args([
				"--cpu=15",
				"--as=2147483648",
				"--fsize=8388608",
				"--core=0",
				"--",
			])
			.arg(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::small_forms::child",
				"--ignored",
				"--nocapture",
				"--test-threads=1",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("C12_ENGINE", "fresh")
			.env("C12_CASE", i.to_string())
			.env("C12_RESULT", &result)
			.current_dir("/tmp")
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let exceeded = Arc::new(AtomicBool::new(false));
		let so = child.stdout.take().unwrap();
		let se = child.stderr.take().unwrap();
		let a = exceeded.clone();
		let stdout = std::thread::spawn(move || c12_drain(so, a));
		let a = exceeded.clone();
		let stderr = std::thread::spawn(move || c12_drain(se, a));
		let launched = Instant::now();
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			if launched.elapsed() > Duration::from_secs(45)
				|| start.elapsed() > Duration::from_secs(1750)
				|| exceeded.load(Ordering::SeqCst)
			{
				child.kill().unwrap();
				break child.wait().unwrap();
			}
			std::thread::sleep(Duration::from_millis(5));
		};
		let stdout = stdout.join().unwrap();
		let stderr = stderr.join().unwrap();
		fs::write(dir.join("stdout"), &stdout).unwrap();
		fs::write(dir.join("stderr"), &stderr).unwrap();
		fs::write(dir.join("receipt.json"),serde_json::to_vec_pretty(&json!({"index":i,"status":status.to_string(),"seconds":launched.elapsed().as_secs_f64(),"output_exceeded":exceeded.load(Ordering::SeqCst)})).unwrap()).unwrap();
		assert!(
			status.success() && !exceeded.load(Ordering::SeqCst),
			"C12 child STOP {i}"
		);
		let mut actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		let source = unhex(actual["source_hex"].as_str().unwrap());
		actual["observation"]["outcome"] = super::d1c_diagnostics::before("small_forms", actual["label"].as_str().unwrap(), &source, "fresh", &actual["observation"]["outcome"]);
		c12_compare(i, expected, &actual, &counts);
		assert_eq!(
			hex(&subject_stdout(&stdout).unwrap()),
			expected["stdout_subject_hex"]
		);
		assert_eq!(hex(&stderr), expected["stderr_hex"]);
	}
	fs::write(out.join("COMPLETE"), "execution111 strict; P1 pointers only\n").unwrap();
	println!("\nCONTRACT ENGINE fresh\nCONTRACT ROWS {}", cases.len());
}
