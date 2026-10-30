//! Bounded, exact-selection Miri execution from committed inputs.
use crate::common::{self, Context, Result};
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
	collections::{BTreeMap, BTreeSet},
	fs,
	path::{Path, PathBuf},
	process::Command,
	sync::OnceLock,
	time::{Duration, Instant},
};
#[path = "miri_trace.rs"]
mod trace;
// Full-capacity and N/2 runs exhausted physical memory. Apply the reviewed
// general memory policy uniformly, without a per-host concurrency table.
const MEMORY_DIVISOR: usize = 4;

const MANIFEST: &str = include_str!("../manifests/miri.json");
#[derive(Deserialize)]
struct Manifest {
	toolchain: String,
	features: String,
	helpers: Vec<Helper>,
	retired: Vec<String>,
	coverage: BTreeMap<String, Coverage>,
}
#[derive(Deserialize, Serialize)]
struct Coverage {
	bound_seconds: u64,
	jobs: Vec<String>,
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
fn validate_coverage(m: &Manifest) -> Result<()> {
	ensure!(
		m.helpers.len() == 98 && m.coverage.len() == 78,
		"Miri witness inventory changed"
	);
	let mut names = BTreeSet::new();
	let mut selectors = BTreeSet::new();
	for h in &m.helpers {
		ensure!(
			names.insert(&h.name) && selectors.insert(&h.selector),
			"duplicate Miri job or selector"
		);
	}
	let mut covered = BTreeSet::new();
	for parent in m.coverage.values() {
		ensure!(
			parent.bound_seconds > 0 && !parent.jobs.is_empty(),
			"empty Miri witness group"
		);
		for name in &parent.jobs {
			ensure!(covered.insert(name), "duplicate witness row");
			let h = m
				.helpers
				.iter()
				.find(|h| &h.name == name)
				.ok_or_else(|| anyhow::anyhow!("missing witness row"))?;
			ensure!(
				h.first_bound <= parent.bound_seconds,
				"row bound exceeds parent bound"
			);
		}
	}
	ensure!(covered == names, "omitted witness row");
	Ok(())
}
fn manifest(ctx: &Context) -> Result<Manifest> {
	let m: Manifest = serde_json::from_str(MANIFEST)?;
	validate_coverage(&m)?;
	ensure!(
		m.helpers.len() == 98 && m.coverage.len() == 78,
		"expected 78 witness groups and 98 Miri row jobs"
	);
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
	let mut covered = BTreeSet::new();
	for group in m.coverage.values() {
		ensure!(
			group.bound_seconds > 0 && !group.jobs.is_empty(),
			"empty Miri witness group"
		);
		for name in &group.jobs {
			ensure!(
				names.contains(name) && covered.insert(name),
				"missing or duplicate Miri row coverage: {name}"
			);
		}
	}
	ensure!(covered == names, "Miri row omitted from witness coverage");
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
		json!({"helpers":m.helpers.len(),"witnesses":m.coverage.len(),"toolchain":m.toolchain,"features":m.features,"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes())}),
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
	.env_remove("MIRIFLAGS")
	.env_remove("MIRI_SYSROOT");
	c
}
fn traced(ctx: &Context, label: &str, command: &Command) -> Command {
	let mut c = Command::new("/usr/bin/strace");
	c.args([
		"--seccomp-bpf",
		"-ff",
		"-ttt",
		"-e",
		"trace=execve,exit_group",
		"-o",
	])
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

fn helper(
	ctx: &Context,
	m: &Manifest,
	h: &Helper,
	identity: &Value,
	started: &OnceLock<Instant>,
	parent_bound: u64,
) -> Result<Value> {
	let bound = h.first_bound as f64;
	let mut c = cargo(ctx, m);
	c.arg(&h.selector).arg("--");
	if h.ignored {
		c.arg("--ignored");
	}
	c.args(["--exact", "--nocapture", "--test-threads=1"]);
	let run = ctx.run_with_deadline(&h.name, &mut traced(ctx, &h.name, &c), bound, || {
		*started.get_or_init(Instant::now) + Duration::from_secs(parent_bound)
	})?;
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
pub fn native_intervals(dir: &Path, label: &str) -> Result<Vec<Value>> {
	trace::native_intervals(dir, label)
}
pub fn preflight(ctx: &Context) -> Result<()> {
	// One snapshot before managed correctness jobs are dispatched; no status watcher.
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
	Ok(())
}
pub fn workers() -> Result<usize> {
	Ok((crate::hosts::available_cpus()? / MEMORY_DIVISOR).max(1))
}

pub struct Prepared {
	ctx: Context,
	manifest: Manifest,
	identity: Value,
	harness: Value,
	starts: BTreeMap<String, OnceLock<Instant>>,
}
pub fn task_specs(ctx: &Context) -> Result<Vec<crate::tasks::Task>> {
	let m = manifest(ctx)?;
	let mut tasks = Vec::new();
	for (parent, coverage) in &m.coverage {
		for name in &coverage.jobs {
			let helper = m
				.helpers
				.iter()
				.find(|h| &h.name == name)
				.expect("validated coverage");
			let mut task = crate::tasks::Task::new(
				"miri",
				"row",
				name,
				(helper.first_bound as f64 / 4.0).max(30.0),
			);

			task.parent = Some(parent.clone());
			tasks.push(task);
		}
	}
	Ok(tasks)
}
pub fn prepare(root: &Context) -> Result<Prepared> {
	let ctx = root.suite("miri")?;
	let ctx = &ctx;

	let m = manifest(ctx)?;
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
	let starts = m
		.coverage
		.keys()
		.map(|name| (name.clone(), OnceLock::new()))
		.collect();
	Ok(Prepared {
		ctx: ctx.clone(),
		manifest: m,
		identity,
		harness: prepared_harness,
		starts,
	})
}
pub fn run_prepared(state: &Prepared, name: &str) -> Result<Value> {
	let m = &state.manifest;
	let h = m
		.helpers
		.iter()
		.find(|h| h.name == name)
		.ok_or_else(|| anyhow::anyhow!("unknown Miri row"))?;
	let (parent, coverage) = m
		.coverage
		.iter()
		.find(|(_, p)| p.jobs.iter().any(|job| job == name))
		.expect("validated coverage");
	helper(
		&state.ctx,
		m,
		h,
		&state.identity,
		&state.starts[parent],
		coverage.bound_seconds,
	)
}
pub fn finish(state: &Prepared, rows: &[Value], lanes: usize) -> Result<Value> {
	let names: Vec<_> = rows
		.iter()
		.map(|row| {
			row["task"]["key"]
				.as_str()
				.ok_or_else(|| anyhow::anyhow!("missing Miri task key"))
		})
		.collect::<Result<_>>()?;
	let capacity = (crate::hosts::available_cpus()? / MEMORY_DIVISOR)
		.max(1)
		.min(state.ctx.budget.capacity())
		.min(lanes);
	let proof = trace::proof(&state.ctx.out, &names, capacity)?;
	common::write_json(&state.ctx.out.join("INTERPRETER-EVENTS.json"), &proof)?;
	ensure!(
		rows.len() == state.manifest.helpers.len(),
		"incomplete Miri rows"
	);
	if capacity > 1 && names.len() > 1 {
		ensure!(
			proof["maximum_actual_interpreters"].as_u64().unwrap_or(0) > 1,
			"Miri interpreter overlap was not exercised"
		);
	}
	Ok(
		json!({"status":"PASS","helpers":names.len(),"witnesses":state.manifest.coverage.len(),"coverage":state.manifest.coverage,"results":rows.iter().map(|row|&row["result"]).collect::<Vec<_>>(),"workers":capacity,"memory_divisor":MEMORY_DIVISOR,"overlap":proof,"identity":state.identity,"harness_records":state.harness}),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn missing_duplicate_and_omitted_rows_fail_coverage() {
		let mut value: Value = serde_json::from_str(MANIFEST).unwrap();
		let m: Manifest = serde_json::from_value(value.clone()).unwrap();
		assert!(validate_coverage(&m).is_ok());
		let first = value["coverage"]
			.as_object()
			.unwrap()
			.keys()
			.next()
			.unwrap()
			.clone();
		value["coverage"][&first]["jobs"]
			.as_array_mut()
			.unwrap()
			.clear();
		assert!(validate_coverage(&serde_json::from_value(value).unwrap()).is_err());
		let mut value: Value = serde_json::from_str(MANIFEST).unwrap();
		let job = value["coverage"][&first]["jobs"][0].clone();
		value["coverage"][&first]["jobs"]
			.as_array_mut()
			.unwrap()
			.push(job);
		assert!(validate_coverage(&serde_json::from_value(value).unwrap()).is_err());
		let mut value: Value = serde_json::from_str(MANIFEST).unwrap();
		value["helpers"].as_array_mut().unwrap().pop();
		assert!(validate_coverage(&serde_json::from_value(value).unwrap()).is_err());
	}

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
