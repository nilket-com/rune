//! Test-only child for the versioned AWFY verifier. Real default stdio stays installed.
use crate::experimental_core::{
	execute::{Executor, ExecutorLimits},
	frontend,
	heap::{Limits, Store},
	value::Scalar,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fs, io::Write as _, path::PathBuf, vec::Vec};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
	path: PathBuf,
	origin: std::string::String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
	#[serde(default)]
	engine: Option<std::string::String>,
	num: i64,
	inner: i64,
	inputs: Vec<Input>,
	stdout: PathBuf,
	control_mode: std::string::String,
	control_environment: BTreeMap<std::string::String, std::string::String>,
}
const HISTORICAL: &str =
	"inactive environment control: S7A_* names unread by the S6_* child; ran under ordinary limits";
fn controls(c: &Config) -> Result<(Option<usize>, Option<usize>), std::string::String> {
	let mut actual = BTreeMap::new();
	for (key, value) in std::env::vars() {
		if key == "RUNE_VERIFY_AWFY_CONFIG" || key == "RUNE_VERIFY_AWFY_OUTPUT" {
			continue;
		}
		if key.starts_with("S6_")
			|| key.starts_with("S7A_")
			|| key.starts_with("RUNE_VERIFY_AWFY_")
			|| key.ends_with("_FRAME_LIMIT")
			|| key.ends_with("_SLOT_LIMIT")
			|| c.control_environment.contains_key(&key)
		{
			actual.insert(key, value);
		}
	}
	if actual != c.control_environment {
		return Err(std::format!(
			"declared/actual control environment differs: declared={:?}, actual={actual:?}",
			c.control_environment
		));
	}
	let mut frames = None;
	let mut slots = None;
	match c.control_mode.as_str() {
		"ordinary" if actual.is_empty() => {}
		"active" if !actual.is_empty() => {
			for (key, value) in actual {
				let n = value
					.parse::<usize>()
					.map_err(|_| std::format!("invalid limit {key}={value}"))?;
				match key.as_str() {
					"S6_FRAME_LIMIT" => frames = Some(n),
					"S6_SLOT_LIMIT" => slots = Some(n),
					_ => return Err(std::format!("unknown or unused active control {key}")),
				}
			}
		}
		HISTORICAL if !actual.is_empty() => {
			for (key, value) in actual {
				if key != "S7A_FRAME_LIMIT" && key != "S7A_SLOT_LIMIT" {
					return Err(std::format!("not a historical inactive knob: {key}"));
				}
				value
					.parse::<usize>()
					.map_err(|_| std::format!("invalid historical limit {key}={value}"))?;
			}
		}
		_ => {
			return Err(std::format!(
				"invalid or unused control mode {:?}",
				c.control_mode
			))
		}
	}
	Ok((frames, slots))
}
// Only this ignored, single-test process redirects fd1. It cannot affect ordinary
// library tests; restoration also runs on unwinding. No registration is replaced.
struct StdoutCapture(i32);
extern "C" {
	fn dup(fd: i32) -> i32;
	fn dup2(old: i32, new: i32) -> i32;
	fn close(fd: i32) -> i32;
}
impl StdoutCapture {
	fn new(path: &std::path::Path) -> Self {
		use std::os::fd::AsRawFd;
		std::io::stdout().flush().expect("flush framework stdout");
		let file = fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(path)
			.expect("new wire stdout");
		// SAFETY: dup retains the valid process stdout; dup2 uses the live file fd.
		unsafe {
			let saved = dup(1);
			assert!(saved >= 0);
			assert_eq!(dup2(file.as_raw_fd(), 1), 1);
			Self(saved)
		}
	}
}
impl Drop for StdoutCapture {
	fn drop(&mut self) {
		std::io::stdout().flush().expect("flush wire stdout");
		// SAFETY: saved descriptor is uniquely owned by this guard and restored once.
		unsafe {
			assert_eq!(dup2(self.0, 1), 1);
			assert_eq!(close(self.0), 0);
		}
	}
}
fn observe(
	c: &Config,
	frame_limit: Option<usize>,
	slot_limit: Option<usize>,
) -> (i32, std::string::String) {
	use std::fmt::Write as _;
	let mut stderr = std::string::String::new();
	let mut sources = Sources::new();
	for input in &c.inputs {
		let text = fs::read_to_string(&input.path).expect("frozen source");
		sources
			.insert(Source::new(&input.origin, &text).expect("source"))
			.expect("insert source");
	}
	let context = Context::with_default_modules().expect("context");
	let mut diagnostics = Diagnostics::new();
	if c.engine.as_deref() == Some("stock") {
		assert!(
			frame_limit.is_none() && slot_limit.is_none(),
			"stock controls are not comparable"
		);
		let mut vm = match crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut diagnostics)
			.build_vm()
		{
			Ok(vm) => vm,
			Err(error) => return (3, std::format!("stock compile: {error:?}")),
		};
		let result = match vm.call(["main"], (c.num, c.inner)) {
			Ok(value) => value,
			Err(error) => return (7, std::format!("stock runtime: {error:?}")),
		};
		let ok: bool = match crate::from_value(result) {
			Ok(value) => value,
			Err(error) => return (7, std::format!("stock result: {error:?}")),
		};
		std::println!("RESULT {}", if ok { "ok" } else { "FAILED" });
		return (if ok { 0 } else { 1 }, stderr);
	}
	assert!(
		c.engine.is_none() || c.engine.as_deref() == Some("fresh"),
		"unknown engine"
	);
	let program = match frontend::compile(&mut sources, &context, &mut diagnostics, 1) {
		Ok(p) => p,
		Err(error) => {
			let refusal = error.refusal.map_or("-", |r| r.name());
			let mut site = std::string::String::from("-");
			if let Some(l) = error.location {
				if let Some(input) = c.inputs.get(l.source as usize) {
					let text = fs::read_to_string(&input.path).expect("source");
					let start = l.start as usize;
					let line = text[..start].matches('\n').count() + 1;
					let column = start - text[..start].rfind('\n').map_or(0, |i| i + 1) + 1;
					let snippet: std::string::String = text
						[start..(l.end as usize).min(text.len())]
						.chars()
						.take(160)
						.collect();
					site = std::format!("{}:{line}:{column} {snippet:?}", input.origin);
				}
			}
			std::println!(
				"REFUSED kind={refusal} operation={} error={:?} site={site}",
				error.operation,
				error.kind
			);
			if diagnostics.has_error() {
				let mut buffer = crate::termcolor::Buffer::no_color();
				diagnostics
					.emit(&mut buffer, &sources)
					.expect("diagnostics");
				stderr.push_str(std::str::from_utf8(buffer.as_slice()).expect("UTF8 diagnostics"));
			}
			return (3, stderr);
		}
	};
	let main = program.function(Hash::type_hash(["main"])).expect("main");
	let mut store = Store::new(Limits::default()).expect("store");
	let mut limits = ExecutorLimits::default();
	if let Some(n) = frame_limit {
		limits.frames = n;
	}
	if let Some(n) = slot_limit {
		limits.slots = n;
	}
	let mut executor = Executor::with_limits(limits).expect("executor");
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().expect("runtime")).expect("runtime arc"),
		))
		.expect("host runtime");
	let a = store.root_i64(c.num);
	let b = store.root_i64(c.inner);
	let result = match executor.invoke(&program, &mut store, main, &[&a, &b], usize::MAX) {
		Ok(root) => root,
		Err(failure) => {
			writeln!(stderr, "error: {:?}", failure.error).expect("format");
			if frame_limit.is_some() || slot_limit.is_some() {
				writeln!(
					stderr,
					"S6_CONTROL callers={} idle={} cleanup_error={}",
					failure.call_chain.len(),
					executor.is_idle(),
					failure.cleanup_error.is_some()
				)
				.expect("format");
			}
			if let Some(message) = &failure.panic_message {
				writeln!(stderr, "panic: {message:?}").expect("format");
			}
			return (7, stderr);
		}
	};
	let ok = match store.root_scalar(&result).expect("scalar") {
		Some(Scalar::Bool(ok)) => ok,
		other => {
			writeln!(stderr, "error: non-bool result {other:?}").expect("format");
			return (7, stderr);
		}
	};
	writeln!(stderr, "fresh live_entries={}", store.live_entries()).expect("format");
	std::println!("RESULT {}", if ok { "ok" } else { "FAILED" });
	(if ok { 0 } else { 1 }, stderr)
}
#[test]
#[ignore = "versioned xtask AWFY child; requires explicit frozen input/output paths"]
fn child() {
	let config: Config = serde_json::from_slice(
		&fs::read(std::env::var_os("RUNE_VERIFY_AWFY_CONFIG").expect("config path"))
			.expect("config bytes"),
	)
	.expect("config JSON");
	let output = PathBuf::from(std::env::var_os("RUNE_VERIFY_AWFY_OUTPUT").expect("output path"));
	let (frame_limit, slot_limit) = controls(&config).expect("AWFY_CONTROL_CONFIGURATION");
	let guard = StdoutCapture::new(&config.stdout);
	let (code, stderr) = observe(&config, frame_limit, slot_limit);
	drop(guard);
	let stdout = fs::read_to_string(&config.stdout).expect("wire stdout");
	let mut file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(output)
		.expect("new wire JSON");
	serde_json::to_writer_pretty(
		&mut file,
		&json!({"code":code,"stdout":stdout,"stderr":stderr}),
	)
	.expect("wire JSON");
	file.write_all(b"\n").expect("newline");
}
