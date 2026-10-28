//! Bounded child runs for paired-engine contracts.
//!
//! A contract compares the execution engine against the legacy engine run live
//! on the same case, so no expected output is stored. Each engine runs one case
//! in a separate child process of this test binary, selected by its exact
//! `#[ignore]` child test name.
use super::*;
use serde_json::Value as Json;
use std::{ffi::OsStr, os::unix::process::CommandExt};

/// Byte limit for each captured stream and for the observation file.
pub(super) const CAP: usize = 2 << 20;
const WALL: Duration = Duration::from_secs(20);

/// One completed child: its observation envelope and captured streams.
pub(super) struct Run {
	pub(super) observation: Json,
	pub(super) stdout: Vec<u8>,
	pub(super) stderr: Vec<u8>,
}

/// Run the child test `selector` in a new `folder`, with a cleared environment
/// plus `env`, and return the observation it wrote to `folder/observation.json`,
/// whose path is passed in the variable `result_var`.
///
/// The child runs under `prlimit` in its own process group. It is killed if it
/// exceeds the wall-clock limit or either stream exceeds [`CAP`]; a killed or
/// failing child fails the calling test. Streams and a receipt are retained in
/// `folder`.
pub(super) fn run_child(
	selector: &str,
	label: &str,
	folder: &Path,
	result_var: &str,
	env: &[(&str, &OsStr)],
) -> Run {
	run_child_with_limits(selector, label, folder, result_var, env, 15, WALL)
}

pub(super) fn run_child_with_limits(
	selector: &str,
	label: &str,
	folder: &Path,
	result_var: &str,
	env: &[(&str, &OsStr)],
	cpu_seconds: u64,
	wall: Duration,
) -> Run {
	fs::create_dir_all(folder).unwrap();
	let result = folder.join("observation.json");
	let mut command = Command::new("/usr/bin/prlimit");
	command
		.arg(format!("--cpu={cpu_seconds}"))
		.args(["--as=2147483648", "--fsize=2097152", "--core=0", "--"])
		.arg(std::env::current_exe().unwrap())
		.args(["--exact", selector, "--ignored", "--nocapture", "--quiet"])
		.process_group(0)
		.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("LANG", "C.UTF-8")
		.env("TZ", "UTC")
		.env(result_var, &result)
		.current_dir(folder)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	for (key, value) in env {
		command.env(key, value);
	}
	let mut child = command.spawn().unwrap();
	let exceeded = Arc::new(AtomicBool::new(false));
	let flag = exceeded.clone();
	let pipe = child.stdout.take().unwrap();
	let stdout = std::thread::spawn(move || drain(pipe, flag));
	let flag = exceeded.clone();
	let pipe = child.stderr.take().unwrap();
	let stderr = std::thread::spawn(move || drain(pipe, flag));
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
		if exceeded.load(Ordering::Relaxed) || start.elapsed() >= wall {
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
	// wait() returns the cached status when try_wait() already reaped the child.
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
	assert!(
		!stop && status.success() && !exceeded.load(Ordering::Relaxed),
		"bounded child {selector}: {label}"
	);
	assert!(fs::metadata(&result).unwrap().len() <= CAP as u64);
	let observation = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
	Run {
		observation,
		stdout,
		stderr,
	}
}

fn drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	bytes
}
