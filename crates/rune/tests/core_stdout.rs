#![cfg(feature = "execution")]

//! Real-stdout behavior of the execution core's `println!`.
//!
//! The in-crate `test_output` seam proves only the ordering logic; this file
//! checks the real stdout path. The test binary re-runs itself with an environment
//! flag selecting a child scenario, so the parent reads the child's real stdout. No
//! extra bin target or harness crate is needed. libtest's own lines around the child
//! test are excluded by sentinels.
//!
//! Moved from the D21 dev crate: `tests/output.rs` (`format_sink` ok, error and limit)
//! and `tests/repeat_entry.rs::repeated_real_output_is_the_single_output_repeated`
//! (`repeat_sink`). Both bins stay outside this crate.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rune::experimental_core::execute::Executor;
use rune::experimental_core::frontend;
use rune::experimental_core::heap::{Limits, Store};
use rune::experimental_core::value::Scalar;
use rune::{Context, Diagnostics, Hash, Source, Sources};

const CHILD: &str = "RUNE_CORE_STDOUT_CHILD";
const BEGIN: &str = "<<execution-stdout-begin>>\n";
const END: &str = "<<execution-stdout-end>>";

/// The child scenario. Only meaningful when spawned by a parent test below; in an
/// ordinary run it returns at once.
#[test]
#[cfg_attr(
	miri,
	ignore = "child-process scenario; parent spawn tests are native-only"
)]
fn child_prints_through_the_core() {
	let Some(mode) = std::env::var_os(CHILD) else {
		return;
	};

	let mode = mode.into_string().unwrap();
	let (text, invocations) = match mode.as_str() {
		"ok" => (
			"pub fn main(_a) { println!(\"first\"); println!(\"{:?}\",[1,2]); 42 }",
			1,
		),
		"error" => (
			"pub fn main(_a) { println!(\"before\"); println!(\"{:?}\",[1,2]); 1/0 }",
			1,
		),
		"volume" => (
			"pub fn main(a) { let i = 0; while i < 256 { println!(\"{}\",a); i += 1; } 42 }",
			1,
		),
		"repeat1" | "repeat5" => (
			"pub fn main(_a) { let n = 0; while n < 3 { n += 1; } println!(\"n {}\", n); }",
			if mode == "repeat1" { 1 } else { 5 },
		),
		other => panic!("unknown child scenario {other}"),
	};

	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let arg = if mode == "volume" {
		store.root_string(&"a".repeat(4095)).unwrap()
	} else {
		store.root_unit()
	};

	let mut executor = Executor::new().unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());
	print!("{BEGIN}");
	let mut outcome = String::from("ok");

	for _ in 0..invocations {
		match executor.invoke(&program, &mut store, main, &[&arg], 1_000_000) {
			Ok(root) => {
				if mode == "ok" || mode == "volume" {
					assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(42)));
				}

				store.root_release(root).unwrap();
			}

			Err(failure) => {
				outcome = format!("error:{:?}", failure.error.kind);
				break;
			}
		}

		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	}

	println!("{END} {outcome} invocations={invocations}");
	store.root_release(arg).unwrap();
	assert_eq!(store.live_entries(), 0);
}

/// Runs one child scenario and returns exactly the bytes the execution core wrote to the
/// real stdout, plus the child's outcome line.
fn child(mode: &str) -> (Vec<u8>, String) {
	let mut child = Command::new(std::env::current_exe().unwrap())
		.args([
			"--exact",
			"child_prints_through_the_core",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD, mode)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let mut stdout = child.stdout.take().unwrap();
	let mut stderr = child.stderr.take().unwrap();
	let out_reader = std::thread::spawn(move || {
		let mut out = Vec::new();
		stdout.read_to_end(&mut out).unwrap();
		out
	});
	let err_reader = std::thread::spawn(move || {
		let mut out = Vec::new();
		stderr.read_to_end(&mut out).unwrap();
		out
	});
	let started = Instant::now();
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break status;
		}
		if started.elapsed() > Duration::from_secs(30) {
			let _ = child.kill();
			let _ = child.wait();
			panic!("child deadline: {mode}");
		}
		std::thread::sleep(Duration::from_millis(10));
	};
	let out = std::process::Output {
		status,
		stdout: out_reader.join().unwrap(),
		stderr: err_reader.join().unwrap(),
	};
	assert!(out.status.success(), "{mode}: {out:?}");
	let stdout = out.stdout;
	let begin = find(&stdout, BEGIN.as_bytes()).expect("child ran") + BEGIN.len();
	let end = begin + find(&stdout[begin..], END.as_bytes()).expect("child finished");
	let tail = &stdout[end + END.len()..];
	let line = tail.split(|b| *b == b'\n').next().unwrap();
	(
		stdout[begin..end].to_vec(),
		String::from_utf8(line.to_vec()).unwrap().trim().to_owned(),
	)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: re-runs the test binary as a child process (current_exe + Command::output); Miri cannot spawn processes and isolation refuses current_exe; native CI only"
)]
fn output_is_real_ordered_and_survives_failure() {
	let (out, outcome) = child("ok");
	assert_eq!(out, b"first\n[1, 2]\n");
	assert_eq!(outcome, "ok invocations=1");
	let (out, outcome) = child("error");
	assert_eq!(out, b"before\n[1, 2]\n");
	assert_eq!(outcome, "error:DivisionByZero invocations=1");
}

/// A MiB of output in bounded lines has no cumulative cap.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: re-runs the test binary as a child process (current_exe + Command::output); Miri cannot spawn processes and isolation refuses current_exe; native CI only"
)]
fn invocation_stdout_has_no_cumulative_cap() {
	let (out, outcome) = child("volume");
	assert_eq!(
		out,
		format!("{}\n", "a".repeat(4095)).repeat(256).as_bytes()
	);
	assert!(out.iter().all(|b| *b == b'a' || *b == b'\n'));
	assert_eq!(outcome, "ok invocations=1");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: re-runs the test binary as a child process (current_exe + Command::output); Miri cannot spawn processes and isolation refuses current_exe; native CI only"
)]
fn repeated_real_output_is_the_single_output_repeated() {
	let (one, outcome) = child("repeat1");
	assert_eq!(one, b"n 3\n");
	assert_eq!(outcome, "ok invocations=1");
	let (five, outcome) = child("repeat5");
	assert_eq!(five, b"n 3\nn 3\nn 3\nn 3\nn 3\n");
	assert_eq!(outcome, "ok invocations=5");
}
