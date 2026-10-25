const C12_STOCK: &str = include_str!("stock.json");
const C12_P1_COUNTS: &str = include_str!("p1_counts.json");
fn c12_data() -> (Vec<Json>, Vec<Json>, Json) {
	(
		serde_json::from_str(C12_CASES).unwrap(),
		serde_json::from_str(C12_STOCK).unwrap(),
		serde_json::from_str(C12_P1_COUNTS).unwrap(),
	)
}
#[test]
fn frozen_schema() {
	let (cases, stock, counts) = c12_data();
	assert_eq!(cases.len(), 111);
	assert_eq!(stock.len(), 111);
	let mut policies = std::collections::BTreeMap::new();
	let mut addresses = 0;
	for (i, (case, frozen)) in cases.iter().zip(&stock).enumerate() {
		assert_eq!(case["id"], format!("{i:03}"));
		for key in ["id", "label", "source_sha256", "fixture", "fuel"] {
			assert_eq!(case[key], frozen[key], "{i}/{key}");
		}
		assert_eq!(
			hex(case["source"].as_str().unwrap().as_bytes()),
			frozen["stock"]["source_hex"]
		);
		assert_eq!(case["source_sha256"], frozen["stock"]["source_sha256"]);
		assert_eq!(frozen["stock"]["engine"], "stock");
		assert_eq!(frozen["stock"]["index"], i);
		assert_eq!(frozen["stdout_subject_hex"], "");
		assert_eq!(frozen["stderr_hex"], "");
		*policies
			.entry(frozen["policy"].as_str().unwrap())
			.or_insert(0usize) += 1;
	}
	assert_eq!(
		policies,
		std::collections::BTreeMap::from([
			("Exact", 107),
			("Refused-surface", 3),
			("Retained-fuel", 1)
		])
	);
	assert_eq!(counts.as_object().unwrap().len(), 10);
	for paths in counts.as_object().unwrap().values() {
		for (path, count) in paths.as_object().unwrap() {
			assert!(p1_path(path));
			addresses += count.as_u64().unwrap();
		}
	}
	assert_eq!(addresses, 36);
	assert_eq!(stock[101]["fresh_expectation"]["refusal"], "CallClosure");
	assert_eq!(stock[109]["fresh_expectation"]["refusal"], "CallClosure");
	assert_eq!(stock[110]["fresh_expectation"]["refusal"], "ByteStr");
}

#[test]
fn stale_capture_refusals_have_stock_exact_admission_overlay() {
	let (_, stock, _) = c12_data();
	let overlay: Json = serde_json::from_str(C12_ADMISSIONS).unwrap();
	assert_eq!(overlay["schema"], 1);
	let rows = overlay["admissions"].as_array().unwrap();
	assert_eq!(rows.len(), 2);
	for (admission, index) in rows.iter().zip([101usize, 109]) {
		assert_eq!(admission["id"], format!("{index:03}"));
		assert_eq!(stock[index]["policy"], "Refused-surface");
		assert_eq!(stock[index]["fresh_expectation"]["refusal"], "CallClosure");
		assert_eq!(admission["from_policy"], stock[index]["policy"]);
		assert_eq!(admission["label"], stock[index]["label"]);
		assert_eq!(admission["source_sha256"], stock[index]["source_sha256"]);
		assert_eq!(admission["policy"], "Exact");
		assert_eq!(admission["expected"], stock[index]["stock"]["observation"]);
	}
}
#[cfg(feature = "sha2")]
#[test]
fn frozen_sha256_seals() {
	use sha2::Digest;
	for (text, expected) in [
		(
			C12_CASES,
			"55b4989f1b6cb3d659ed1d5b8658805ece32be050f05f585d859391e7b9f7cd6",
		),
		(
			C12_STOCK,
			"937c24ffe8107abb634e6a14bb6097fb3786c56c2765752865c35c18b123d61d",
		),
		(
			C12_P1_COUNTS,
			"4a8ad5999eb760296d479cc3b13c15d7c64f4e5abb19dfee870d9cc176895703",
		),
	] {
		assert_eq!(hex(&sha2::Sha256::digest(text.as_bytes())), expected);
	}
	let (cases, _, _) = c12_data();
	for case in cases {
		assert_eq!(
			hex(&sha2::Sha256::digest(
				case["source"].as_str().unwrap().as_bytes()
			)),
			case["source_sha256"]
		);
	}
}
fn subject_stdout(bytes: &[u8]) -> Result<Vec<u8>, String> {
	let text = std::str::from_utf8(bytes).map_err(|_| "non-UTF8 test stdout")?;
	let rest = text
		.strip_prefix(
			"\nrunning 1 test\ntest experimental_core::tests::gap_hunt::small_forms::child ... ",
		)
		.ok_or("test harness prefix")?;
	let (subject, footer) = rest
		.rsplit_once("ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; ")
		.ok_or("test harness suffix")?;
	let (filtered, time) = footer
		.split_once(" filtered out; finished in ")
		.ok_or("test harness footer")?;
	filtered
		.parse::<usize>()
		.map_err(|_| "test harness count")?;
	let seconds = time
		.strip_suffix("s\n\n")
		.ok_or("test harness timing suffix")?
		.parse::<f64>()
		.map_err(|_| "test harness time")?;
	if !seconds.is_finite() || seconds < 0.0 {
		return Err("invalid test time".into());
	}
	Ok(subject.as_bytes().to_vec())
}
#[test]
fn stdout_framing_preserves_subject_bytes() {
	let prefix =
		"\nrunning 1 test\ntest experimental_core::tests::gap_hunt::small_forms::child ... ";
	let suffix="ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.02s\n\n";
	assert_eq!(
		subject_stdout(format!("{prefix}hello\n{suffix}").as_bytes()).unwrap(),
		b"hello\n"
	);
	assert!(subject_stdout(b"hello\n").is_err());
}
fn c12_drain(
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
#[test]
#[ignore = "C12 frozen stock replay; explicit new C12_REPLAY_OUT, bounded children"]
fn legacy_contract() {
	use std::process::{Command, Stdio};
	use std::sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(std::env::var_os("C12_REPLAY_OUT").expect("new C12_REPLAY_OUT"));
	fs::create_dir(&out).unwrap();
	let (cases, frozen, counts) = c12_data();
	let start = Instant::now();
	for (i, (case, expected)) in cases.iter().zip(&frozen).enumerate() {
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
			.env("C12_ENGINE", "stock")
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
		actual["observation"]["outcome"] = super::d1c_diagnostics::before("small_forms", actual["label"].as_str().unwrap(), &source, "stock", &actual["observation"]["outcome"]);
		actual["observation"] = super::method_diagnostics::intentional_json_before(
			"small_forms",
			actual["label"].as_str().unwrap(),
			&source,
			"stock",
			&actual["observation"],
		);
		let empty = json!({});
		let count = counts.get(format!("{i:03}")).unwrap_or(&empty);
		let portable = project_p1(&actual, count).unwrap_or_else(|e| panic!("P1 row{i}: {e}"));
		assert_eq!(
			portable, expected["stock"],
			"C12 stock row {i}: {}",
			case["label"]
		);
		assert_eq!(
			hex(&subject_stdout(&stdout).unwrap()),
			expected["stdout_subject_hex"]
		);
		assert_eq!(hex(&stderr), expected["stderr_hex"]);
	}
	fs::write(
		out.join("COMPLETE"),
		"stock111 replay exact; P1 pointers only\n",
	)
	.unwrap();
	println!("\nCONTRACT ENGINE stock\nCONTRACT ROWS {}", cases.len());
}
