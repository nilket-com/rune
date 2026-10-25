fn c15b_drain(
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
#[ignore = "C15b strict execution contract; explicit new C15B_OUT, bounded children"]
fn contract() {
	use std::process::{Command, Stdio};
	use std::sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(std::env::var_os("C15B_OUT").expect("new C15B_OUT"));
	fs::create_dir(&out).unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	let start = Instant::now();
	for (i, case) in cases.iter().enumerate() {
		assert!(start.elapsed() < Duration::from_secs(1700));
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let result = dir.join("observation.json");
		let mut child = Command::new("/usr/bin/prlimit")
			.args(["--cpu=15", "--as=2147483648", "--fsize=8388608", "--"])
			.arg(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::enum_variants::child",
				"--ignored",
				"--nocapture",
				"--test-threads=1",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("C15B_ENGINE", "fresh")
			.env("C15B_CASE", i.to_string())
			.env("C15B_RESULT", &result)
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
		let stdout = std::thread::spawn(move || c15b_drain(so, a));
		let a = exceeded.clone();
		let stderr = std::thread::spawn(move || c15b_drain(se, a));
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
			"C15b child STOP {i}"
		);
		let mut actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		actual["observation"]["outcome"] = super::d1c_diagnostics::before("enum_variants", case["label"].as_str().unwrap(), case["source"].as_str().unwrap(), "fresh", &actual["observation"]["outcome"]);
		compare(case, &frozen[format!("{i:03}")], &actual);
		assert!(stderr.is_empty(), "C15b stderr {i}");
	}
	fs::write(
		out.join("COMPLETE"),
		"execution166 strict; frozen P1, pre-effect refusal and fuel policy only\n",
	)
	.unwrap();
	println!("\nCONTRACT ENGINE fresh\nCONTRACT ROWS {}", cases.len());
}

#[test]
#[ignore = "C15b strict stock VM contract; explicit new C15B_STOCK_OUT, bounded children"]
fn legacy_contract() {
	use std::process::{Command, Stdio};
	use std::sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	};
	use std::time::{Duration, Instant};
	let out = PathBuf::from(std::env::var_os("C15B_STOCK_OUT").expect("new C15B_STOCK_OUT"));
	fs::create_dir(&out).unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();

	let start = Instant::now();
	for (i, case) in cases.iter().enumerate() {
		assert!(start.elapsed() < Duration::from_secs(1700));
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let result = dir.join("observation.json");
		let mut child = Command::new("/usr/bin/prlimit")
			.args(["--cpu=15", "--as=2147483648", "--fsize=8388608", "--"])
			.arg(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::enum_variants::child",
				"--ignored",
				"--nocapture",
				"--test-threads=1",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("C15B_ENGINE", "stock")
			.env("C15B_CASE", i.to_string())
			.env("C15B_RESULT", &result)
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
		let stdout = std::thread::spawn(move || c15b_drain(so, a));
		let a = exceeded.clone();
		let stderr = std::thread::spawn(move || c15b_drain(se, a));
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
			"C15b child STOP {i}"
		);
		let mut actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		actual["observation"]["outcome"] = super::d1c_diagnostics::before("enum_variants", case["label"].as_str().unwrap(), case["source"].as_str().unwrap(), "stock", &actual["observation"]["outcome"]);
		compare_stock(case, &actual);
		assert!(stderr.is_empty(), "C15b stderr {i}");
	}
	fs::write(
		out.join("COMPLETE"),
		"stock166 strict; frozen P1, pre-effect refusal and fuel policy only\n",
	)
	.unwrap();
	println!("\nCONTRACT ENGINE fresh\nCONTRACT ROWS {}", cases.len());
}
