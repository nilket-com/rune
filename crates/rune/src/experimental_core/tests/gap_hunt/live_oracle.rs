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
	let group = child.id();
	let exceeded = Arc::new(AtomicBool::new(false));
	let flag = exceeded.clone();
	let pipe = child.stdout.take().unwrap();
	let stdout = std::thread::spawn(move || drain(pipe, flag, group));
	let flag = exceeded.clone();
	let pipe = child.stderr.take().unwrap();
	let stderr = std::thread::spawn(move || drain(pipe, flag, group));
	let start = Instant::now();

	let timed_out = Arc::new(AtomicBool::new(false));
	let deadline_flag = timed_out.clone();
	let (completed, exit) = std::sync::mpsc::channel();
	let timer = std::thread::spawn(move || {
		if matches!(
			exit.recv_timeout(wall),
			Err(std::sync::mpsc::RecvTimeoutError::Timeout)
		) {
			deadline_flag.store(true, Ordering::Relaxed);
			kill_group(group);
		}
	});
	// Exit and stream EOF drive completion; the one-shot deadline also covers
	// descendants that retain a pipe after their immediate parent exits.
	let status = child.wait().unwrap();
	let stdout = stdout.join().unwrap();
	let stderr = stderr.join().unwrap();
	let _ = completed.send(());
	timer.join().unwrap();
	let stop = timed_out.load(Ordering::Relaxed) || exceeded.load(Ordering::Relaxed);
	fs::write(folder.join("stdout"), &stdout).unwrap();
	fs::write(folder.join("stderr"), &stderr).unwrap();
	fs::write(
		folder.join("receipt.txt"),
		format!(
			"case:{label}\nrc:{status}\nstop:{stop}\nelapsed:{:?}\ntimeout:{}\nstream_limit:{}\n",
			start.elapsed(),
			timed_out.load(Ordering::Relaxed),
			exceeded.load(Ordering::Relaxed)
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

fn kill_group(group: u32) {
	let _ = Command::new("/usr/bin/kill")
		.args(["-KILL", "--", &format!("-{group}")])
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status();
}

fn drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>, group: u32) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room && !exceeded.swap(true, Ordering::Relaxed) {
			kill_group(group);
		}
	}
	bytes
}

#[test]
#[ignore = "internal child for blocking bounds controls"]
fn blocking_wait_child() {
	match std::env::var("V4_ORACLE_MODE").unwrap().as_str() {
		"timeout" => {
			let (_sender, receiver) = std::sync::mpsc::channel::<()>();
			receiver.recv().unwrap();
		}
		"stream" => {
			use std::io::Write as _;
			let _ = std::io::stdout().write_all(&std::vec![b'x'; CAP + 4096]);
		}
		"success" => {
			fs::write(
				std::env::var_os("V4_ORACLE_RESULT").unwrap(),
				b"{\"ok\":true}",
			)
			.unwrap();
		}
		_ => panic!("invalid bounds control"),
	}
}

#[test]
#[ignore = "explicit bounds regression for the native contract supervisor"]
fn blocking_wait_controls() {
	let root = std::path::PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	let selector = "experimental_core::tests::gap_hunt::live_oracle::blocking_wait_child";
	let success = run_child_with_limits(
		selector,
		"success",
		&root.join("success"),
		"V4_ORACLE_RESULT",
		&[("V4_ORACLE_MODE", "success".as_ref())],
		5,
		Duration::from_secs(5),
	);
	assert_eq!(success.observation, serde_json::json!({"ok":true}));
	for (mode, wall) in [
		("timeout", Duration::from_millis(250)),
		("stream", Duration::from_secs(5)),
	] {
		let folder = root.join(mode);
		let failed = std::panic::catch_unwind(|| {
			run_child_with_limits(
				selector,
				mode,
				&folder,
				"V4_ORACLE_RESULT",
				&[("V4_ORACLE_MODE", mode.as_ref())],
				5,
				wall,
			)
		});
		assert!(failed.is_err(), "{mode}: required stop missing");
		let receipt = fs::read_to_string(folder.join("receipt.txt")).unwrap();
		assert!(receipt.contains("stop:true\n"));
		assert!(receipt.contains(if mode == "timeout" {
			"timeout:true\nstream_limit:false\n"
		} else {
			"timeout:false\nstream_limit:true\n"
		}));
		for stream in ["stdout", "stderr"] {
			assert!(fs::metadata(folder.join(stream)).unwrap().len() <= CAP as u64);
		}
	}
	std::println!("CONTRACT ENGINE both");
	std::println!("CONTRACT ROWS 3");
}
