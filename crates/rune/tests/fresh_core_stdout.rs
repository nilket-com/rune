#![cfg(feature = "fresh-core")]
//! Real-stdout behaviour of the fresh core's `println!`.
//!
//! The in-crate `test_output` seam proves only the ordering and cap logic; this file
//! checks the real stdout path. The test binary re-runs itself with an environment
//! flag selecting a child scenario, so the parent reads the child's real stdout. No
//! extra bin target or harness crate is needed. libtest's own lines around the child
//! test are excluded by sentinels.
//!
//! Moved from the D21 dev crate: `tests/output.rs` (`format_sink` ok, error and limit)
//! and `tests/repeat_entry.rs::repeated_real_output_is_the_single_output_repeated`
//! (`repeat_sink`). Both bins stay outside this crate.

use std::process::Command;

use rune::experimental_core::execute::Executor;
use rune::experimental_core::frontend;
use rune::experimental_core::heap::{Limits, Store};
use rune::experimental_core::value::Scalar;
use rune::{Context, Diagnostics, Hash, Source, Sources};

const CHILD: &str = "RUNE_FRESH_CORE_STDOUT_CHILD";
const BEGIN: &str = "<<fresh-core-stdout-begin>>\n";
const END: &str = "<<fresh-core-stdout-end>>";

/// The child scenario. Only meaningful when spawned by a parent test below; in an
/// ordinary run it returns at once.
#[test]
#[cfg_attr(
	miri,
	ignore = "child-process scenario; parent spawn tests are native-only"
)]
fn child_prints_through_the_fresh_core() {
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
		"limit" => (
			"pub fn main(a) { println!(\"{}\",a); println!(\"{}\",a); println!(\"{}\",a); }",
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
	let arg = if mode == "limit" {
		store.root_string(&"a".repeat(65535)).unwrap()
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
				if mode == "ok" {
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

/// Runs one child scenario and returns exactly the bytes the fresh core wrote to the
/// real stdout, plus the child's outcome line.
fn child(mode: &str) -> (Vec<u8>, String) {
	let out = Command::new(std::env::current_exe().unwrap())
		.args([
			"--exact",
			"child_prints_through_the_fresh_core",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD, mode)
		.output()
		.unwrap();
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

/// Declared difference from the stock `println`: a 128 KiB per-invocation cap,
/// checked before each write, so the third 64 KiB line is refused unwritten.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: re-runs the test binary as a child process (current_exe + Command::output); Miri cannot spawn processes and isolation refuses current_exe; native CI only"
)]
fn invocation_stdout_cap_refuses_before_the_next_write() {
	let (out, outcome) = child("limit");
	assert_eq!(out.len(), 128 << 10);
	assert_eq!(out[65535], b'\n');
	assert_eq!(out[131071], b'\n');
	assert!(out.iter().all(|b| *b == b'a' || *b == b'\n'));
	assert_eq!(outcome, "error:FormatLimit invocations=1");
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
