fn c13_drain(
	mut input: impl std::io::Read,
	exceeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<u8> {
	let mut out = Vec::new();
	let mut buffer = [0u8; 4096];
	loop {
		let n = input.read(&mut buffer).unwrap();
		if n == 0 {
			break;
		}
		let available = (128 * 1024usize).saturating_sub(out.len());
		out.extend_from_slice(&buffer[..n.min(available)]);
		if n > available {
			exceeded.store(true, std::sync::atomic::Ordering::SeqCst);
		}
	}
	out
}

// Compare program output while ignoring only libtest's elapsed-time summary.
fn c13_stdout_body(stdout: &[u8]) -> &[u8] {
    let prefix = b"\nrunning 1 test\n";
    let suffix = b".\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; ";
    let rest = stdout.strip_prefix(prefix).expect("C13 libtest prefix");
    let at = rest.windows(suffix.len()).rposition(|window| window == suffix).expect("C13 libtest summary");
    let summary = std::str::from_utf8(&rest[at..]).expect("C13 libtest summary UTF-8");
    assert!(summary.contains(" filtered out; finished in ") && summary.ends_with("s\n\n"));
    &rest[..at]
}

// The stock engine is run for this source and fixture on every contract pass.
fn c13_live_expected(case: &Json, index: usize, row: &Json, dir: &Path) -> (Json, Vec<u8>, Vec<u8>) {
    let index_text = index.to_string();
    let stock = super::live_oracle::run_child_with_limits(
        "experimental_core::tests::gap_hunt::methods::child", case["label"].as_str().unwrap(),
        dir, "C13_RESULT", &[("C13_CASE", index_text.as_ref()), ("C13_ENGINE", "stock".as_ref())],
        15, Duration::from_secs(45));
    assert_eq!(stock.observation["schema"], 1);
    assert_eq!(stock.observation["engine"], "stock");
    assert_eq!(stock.observation["index"], index);
    assert_eq!(stock.observation["source_sha256"], case["source_sha256"]);
    assert_eq!(stock.observation["fixture"], case["fixture"]);
    let label = case["label"].as_str().unwrap();
    let source = case["source"].as_str().unwrap();
    let mut observation = stock.observation["observation"].clone();
    observation["outcome"] = super::d1c_diagnostics::d1d_before("methods", label, source, "stock", &observation["outcome"]);
    observation = super::method_diagnostics::intentional_json_before("methods", label, source, "stock", &observation);
    let observation = project_observation(&observation, &observation, "");
    let mut expected = row.clone();
    expected["stock_observation"] = observation.clone();
    expected["fresh_expectation"] = observation;
    (expected, stock.stdout, stock.stderr)
}

#[test]
#[ignore = "C13 strict execution contract; explicit new C13_OUT, bounded children"]
fn contract() {
	use std::process::{Command, Stdio};
	use std::sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(std::env::var_os("C13_OUT").expect("new C13_OUT"));
	fs::create_dir(&out).unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	let start = Instant::now();
	for (i, case) in cases.iter().enumerate() {
		assert!(start.elapsed() < Duration::from_secs(1700));
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let (live_expected, stock_stdout, stock_stderr) = c13_live_expected(case, i, &frozen[format!("{i:03}")], &dir.join("stock"));
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
				"experimental_core::tests::gap_hunt::methods::child",
				"--ignored",
				"--nocapture",
                "--quiet",
				"--test-threads=1",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", fs::canonicalize(&dir).unwrap())
			.env("LANG", "C.UTF-8")
			.env("C13_ENGINE", "fresh")
			.env("C13_CASE", i.to_string())
			.env("C13_RESULT", &result)
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
		let stdout = std::thread::spawn(move || c13_drain(so, a));
		let a = exceeded.clone();
		let stderr = std::thread::spawn(move || c13_drain(se, a));
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
			"C13 child STOP {i}"
		);
		let mut actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		actual["observation"]["outcome"] = super::d1c_diagnostics::d1d_before("methods", case["label"].as_str().unwrap(), case["source"].as_str().unwrap(), "fresh", &actual["observation"]["outcome"]);
		compare(case, &live_expected, &actual);
        assert_eq!(c13_stdout_body(&stdout), c13_stdout_body(&stock_stdout), "C13 stdout {i}");
        assert_eq!(stderr, stock_stderr, "C13 stderr {i}");
	}
	fs::write(
		out.join("COMPLETE"),
		"execution84 strict; P1 pointers and frozen frame leaves only\n",
	)
	.unwrap();
	println!("\nCONTRACT ENGINE fresh\nCONTRACT ROWS {}", cases.len());
}
