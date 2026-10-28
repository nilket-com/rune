//! Bounded, exact-selection Miri execution from committed inputs.
use crate::common::{self, Context, Result};
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
	collections::BTreeSet,
	fs,
	path::{Path, PathBuf},
	process::Command,
	sync::{
		atomic::{AtomicBool, Ordering},
		Mutex,
	},
};
#[path = "miri_trace.rs"]
mod trace;
const MANIFEST: &str = include_str!("../manifests/miri.json");
#[derive(Deserialize)]
struct Manifest {
	toolchain: String,
	features: String,
	helpers: Vec<Helper>,
	retired: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Helper {
	name: String,
	selector: String,
	ignored: bool,
	fixture_source: String,
	fixture_sha256: String,
	first_bound: u64,
}
fn relative(root: &Path, name: &str) -> Result<PathBuf> {
	let path = Path::new(name);
	ensure!(
		!path.is_absolute()
			&& path
				.components()
				.all(|c| matches!(c, std::path::Component::Normal(_))),
		"unsafe manifest path {name}"
	);
	Ok(root.join(path))
}
fn manifest(ctx: &Context) -> Result<Manifest> {
	let m: Manifest = serde_json::from_str(MANIFEST)?;
	ensure!(m.helpers.len() == 73, "expected 73 Miri helpers");
	let mut names = BTreeSet::new();
	let mut selectors = BTreeSet::new();
	for h in &m.helpers {
		ensure!(
			!h.name.is_empty()
				&& h.name
					.bytes()
					.all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
			"invalid helper name"
		);
		ensure!(
			names.insert(&h.name) && selectors.insert(&h.selector),
			"duplicate Miri name or selector"
		);
		ensure!(h.first_bound > 0, "zero first bound");
		ensure!(
			common::hash_file(&relative(&ctx.repo, &h.fixture_source)?)? == h.fixture_sha256,
			"fixture changed: {}",
			h.fixture_source
		);
	}
	for retired in &m.retired {
		ensure!(
			!relative(&ctx.repo, retired)?.exists(),
			"retired fixture resurrected: {retired}"
		);
	}
	Ok(m)
}
pub fn check_manifest(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	Ok(
		json!({"helpers":m.helpers.len(),"toolchain":m.toolchain,"features":m.features,"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes())}),
	)
}
fn cargo(ctx: &Context, m: &Manifest) -> Command {
	let mut c = ctx.cargo_toolchain(&m.toolchain);
	c.args([
		"miri",
		"test",
		"--locked",
		"-p",
		"rune",
		"--lib",
		"--features",
		&m.features,
	])
	.env("CARGO_TARGET_DIR", ctx.out.join("target-miri"))
	.env("CARGO_BUILD_JOBS", "2")
	.env_remove("MIRIFLAGS")
	.env_remove("MIRI_SYSROOT");
	c
}
fn traced(ctx: &Context, label: &str, command: &Command) -> Command {
	let mut c = Command::new("/usr/bin/strace");
	c.args(["-ff", "-ttt", "-e", "trace=execve,exit_group", "-o"])
		.arg(ctx.out.join(format!("{label}.trace")))
		.arg(command.get_program())
		.args(command.get_args())
		.current_dir(command.get_current_dir().unwrap_or(&ctx.repo));
	for (k, v) in command.get_envs() {
		if let Some(v) = v {
			c.env(k, v);
		} else {
			c.env_remove(k);
		}
	}
	c
}
fn pass(stdout: &str, stderr: &str, selector: &str) -> Result<()> {
	for bad in [
		"Undefined Behavior",
		"unsupported operation",
		"unsupported foreign function",
	] {
		ensure!(!stderr.contains(bad), "Miri reported {bad}");
	}
	ensure!(
		common::parse_counts(stdout)?
			== common::TestCounts {
				passed: 1,
				failed: 0,
				ignored: 0
			},
		"Miri must pass exactly one test"
	);
	let marker = match selector {
        "experimental_core::tests::patterns_safety::vec_local_host_shapes_moved_values_and_reuse" => Some("C16a Vec local Miri rows 12"),
        "experimental_core::tests::patterns_safety::loop_binders_control_flow_failures_and_reuse" => Some("C16b loop Miri rows 24"), _ => None };
	if let Some(marker) = marker {
		ensure!(
			stdout.matches(marker).count() == 1
				&& regex::Regex::new(&format!(
					r"(?m)^test {} \.\.\. {}\nok$",
					regex::escape(selector),
					regex::escape(marker)
				))?
				.is_match(stdout),
			"missing exact Miri marker"
		);
	} else {
		ensure!(
			stdout
				.lines()
				.filter(|l| *l == format!("test {selector} ... ok"))
				.count() == 1,
			"missing exact helper pass"
		);
	}
	Ok(())
}
fn selection(listing: &str, selector: &str) -> Result<()> {
	ensure!(
		listing
			.lines()
			.filter(|l| *l == format!("{selector}: test"))
			.count() == 1,
		"missing or multiple Miri selection: {selector}"
	);
	Ok(())
}

fn helper(ctx: &Context, m: &Manifest, h: &Helper, identity: &Value) -> Result<Value> {
	let bound = h.first_bound as f64;
	let mut c = cargo(ctx, m);
	c.arg(&h.selector).arg("--");
	if h.ignored {
		c.arg("--ignored");
	}
	c.args(["--exact", "--nocapture", "--test-threads=1"]);
	let run = ctx.run_seconds(&h.name, &mut traced(ctx, &h.name, &c), bound)?;
	ensure!(run.code == 0, "Miri {} failed: {}", h.name, run.code);
	pass(
		&fs::read_to_string(&run.stdout)?,
		&fs::read_to_string(&run.stderr)?,
		&h.selector,
	)?;
	let intervals = trace::intervals(&ctx.out, &h.name)?;
	ensure!(
		intervals.len() == 1,
		"expected exactly one actual Miri interpreter for {}",
		h.name
	);
	let result = json!({"helper":h,"run":run,"bound_seconds":bound,"identity":identity,"interpreters":intervals});
	common::write_json(&ctx.out.join(format!("{}-result.json", h.name)), &result)?;
	Ok(result)
}
fn batch(
	ctx: &Context,
	m: &Manifest,
	helpers: &[Helper],
	identity: &Value,
	workers: usize,
) -> Result<Vec<Value>> {
	let queue = Mutex::new(helpers.iter());
	let stop = AtomicBool::new(false);
	let results = Mutex::new(Vec::new());
	std::thread::scope(|scope| {
		let mut threads = Vec::new();
		for _ in 0..workers {
			threads.push(scope.spawn(|| -> Result<()> {
				loop {
					if stop.load(Ordering::SeqCst) {
						break;
					}
					let next = queue
						.lock()
						.map_err(|_| anyhow::anyhow!("helper queue poisoned"))?
						.next()
						.cloned();
					let Some(h) = next else { break };
					match helper(ctx, m, &h, identity) {
						Ok(r) => results
							.lock()
							.map_err(|_| anyhow::anyhow!("result lock poisoned"))?
							.push(r),
						Err(e) => {
							stop.store(true, Ordering::SeqCst);
							return Err(e);
						}
					}
				}
				Ok(())
			}));
		}
		let mut error = None;
		for worker in threads {
			match worker.join() {
				Ok(Ok(())) => {}
				Ok(Err(e)) => {
					if error.is_none() {
						error = Some(e);
					}
				}
				Err(_) => {
					error = Some(anyhow::anyhow!("Miri worker panicked"));
				}
			}
		}
		if let Some(e) = error {
			return Err(e);
		}
		Ok(())
	})?;
	results
		.into_inner()
		.map_err(|_| anyhow::anyhow!("result lock poisoned"))
}
pub fn run(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	// One snapshot before starting: do not weaken V1a's native/Miri exclusion.
	let ps = ctx.checked(
		"preflight-processes",
		Command::new("ps").args(["-eo", "pid,args"]),
		30,
	)?;
	let processes = fs::read_to_string(&ps.stdout)?;
	for line in processes.lines() {
		let cmd = line
			.split_whitespace()
			.skip(1)
			.collect::<Vec<_>>()
			.join(" ");
		ensure!(
			!(cmd.starts_with("rustc ")
				|| cmd.contains("/bin/rustc ")
				|| cmd.contains("/bin/miri ")
				|| cmd.contains("/rune-tests ")),
			"native or Miri job already active: {line}"
		);
	}
	let mut rustc = Command::new("rustc");
	rustc.arg(format!("+{}", m.toolchain)).arg("-vV");
	let rustc = ctx.checked("rustc-identity", &mut rustc, 30)?;
	let mut version = ctx.cargo_toolchain(&m.toolchain);
	version.args(["miri", "--version"]);
	let version = ctx.checked("miri-identity", &mut version, 30)?;
	let identity = json!({"toolchain":m.toolchain,"features":m.features,"rustc":fs::read_to_string(rustc.stdout)?,"miri":fs::read_to_string(version.stdout)?});
	common::write_json(&ctx.out.join("IDENTITY.json"), &identity)?;
	let mut prepare = cargo(ctx, &m);
	prepare.args(["--", "--list"]);
	let prep = ctx.checked("prepare", &mut traced(ctx, "prepare", &prepare), 300)?;
	let listing = fs::read_to_string(&prep.stdout)?;
	for h in &m.helpers {
		selection(&listing, &h.selector)?;
	}
	ensure!(!listing.is_empty(), "empty Miri test listing");
	let prepared_harness = trace::harness_records(ctx)?;
	common::write_json(&ctx.out.join("PREPARED-HARNESS.json"), &prepared_harness)?;
	let workers = crate::hosts::available_cpus()?.min(2);
	let mut results = batch(ctx, &m, &m.helpers[..2], &identity, workers)?;
	let first = trace::proof(
		&ctx.out,
		&m.helpers[..2]
			.iter()
			.map(|h| h.name.as_str())
			.collect::<Vec<_>>(),
	)?;
	if workers == 2 {
		ensure!(
			first["maximum_actual_interpreters"] == 2,
			"first Miri pair did not prove actual overlap"
		);
	} else {
		ensure!(
			first["maximum_actual_interpreters"] == 1,
			"serial Miri fallback overlapped"
		);
	}
	common::write_json(&ctx.out.join("FIRST-PAIR-OVERLAP.json"), &first)?;
	results.extend(batch(ctx, &m, &m.helpers[2..], &identity, workers)?);
	let proof = trace::proof(
		&ctx.out,
		&m.helpers
			.iter()
			.map(|h| h.name.as_str())
			.collect::<Vec<_>>(),
	)?;
	common::write_json(&ctx.out.join("INTERPRETER-EVENTS.json"), &proof)?;
	let harness = prepared_harness;
	ensure!(results.len() == 73, "incomplete Miri results");
	manifest(ctx)?;
	Ok(
		json!({"status":"PASS","helpers":73,"results":results,"overlap":proof,"concurrency":if workers == 1 {"serial-fallback"} else {"two-workers"},"overlap_evidence":if workers == 1 {"not exercised"} else {"exercised"},"harness_records":harness,"identity":identity,"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes())}),
	)
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn special_markers_require_exact_adjacent_pass_lines() {
		for (selector, marker) in [
            ("experimental_core::tests::patterns_safety::vec_local_host_shapes_moved_values_and_reuse", "C16a Vec local Miri rows 12"),
            ("experimental_core::tests::patterns_safety::loop_binders_control_flow_failures_and_reuse", "C16b loop Miri rows 24"),
        ] {
            let summary = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
            assert!(pass(&format!("test {selector} ... {marker}\nok\n{summary}"), "", selector).is_ok());
            for wrong in [
                format!("test {selector} ... ok\n{marker}\n{summary}"),
                format!("test {selector} ... {marker}\n{summary}"),
                format!("test {selector}_suffix ... {marker}\nok\n{summary}"),
                format!("test {selector} ... {marker}\nok_suffix\n{summary}"),
                format!("prefix test {selector} ... {marker}\nok\n{summary}"),
            ] { assert!(pass(&wrong, "", selector).is_err(), "accepted malformed marker output: {wrong}"); }
        }
	}

	#[test]
	fn exact_listing_rejects_missing_and_duplicate_selectors() {
		assert!(selection("other: test\n", "wanted").is_err());
		assert!(selection("wanted: test\nwanted: test\n", "wanted").is_err());
		assert!(selection("wanted_suffix: test\n", "wanted").is_err());
		assert!(selection("other: test\nwanted: test\n", "wanted").is_ok());
	}

	#[test]
	fn rejects_zero_multiple_and_wrong_selection() {
		for text in ["test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n", "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n", "test other ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"] {assert!(pass(text,"","wanted").is_err());}
	}
	#[test]
	fn rejects_ub_despite_pass() {
		assert!(pass("test wanted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n","Undefined Behavior","wanted").is_err());
	}
	#[test]
	fn rejects_parent_paths() {
		assert!(relative(Path::new("/x"), "../old").is_err());
	}
}
