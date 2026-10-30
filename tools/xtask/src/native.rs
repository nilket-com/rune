//! The native acceptance matrix uses committed selectors and live observations.
use crate::common::{hash_file, read_json, Context};
use anyhow::{bail, ensure, Context as _, Result};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
#[path = "native_layout.rs"]
mod layout;
#[path = "native_witness.rs"]
mod witness;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
	version: u32,
	reference: String,
	libraries: Vec<Library>,
	clippy_features: Vec<String>,
	fixtures: Vec<Fixture>,
	contract_bundle: ContractBundle,
	excluded_legacy: Vec<String>,
	cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
	name: String,
	features: String,
	passed: usize,
	#[serde(default)]
	selector: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
	path: String,
	sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractBundle {
	selector: String,
	contracts: usize,
	external: String,
	pins: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
	name: String,
	selector: String,
	rows: Option<usize>,
	ignored: bool,
	exact: bool,
	sandbox: bool,
	#[serde(default)]
	exclusive: bool,
	empty_stderr: bool,
	timeout: u64,
	env: BTreeMap<String, String>,
}

pub fn manifest() -> Result<Manifest> {
	let m: Manifest = serde_json::from_str(include_str!("../manifests/native.json"))?;
	ensure!(m.version == 1, "unsupported native manifest version");
	let mut names = std::collections::BTreeSet::new();
	for c in &m.cases {
		ensure!(names.insert(&c.name), "duplicate case {}", c.name);
		ensure!(!c.selector.is_empty(), "empty selector");
	}
	Ok(m)
}

/// Validate selectors and pinned external data without compiling any subject.
pub fn check_manifest(ctx: &Context) -> Result<Value> {
	let m = manifest()?;
	fixtures(ctx, &m)?;
	Ok(
		json!({"version":m.version,"reference":m.reference,"cases":m.cases.len(),"contracts":m.contract_bundle.contracts,"fixtures":m.fixtures.len()}),
	)
}

fn fixtures(ctx: &Context, m: &Manifest) -> Result<()> {
	verify_fixtures(&ctx.repo, &m.fixtures)
}
fn verify_fixtures(root: &Path, fixtures: &[Fixture]) -> Result<()> {
	for f in fixtures {
		ensure!(
			hash_file(&root.join(&f.path))? == f.sha256,
			"fixture identity mismatch: {}",
			f.path
		);
	}
	Ok(())
}

/// The final summary belongs to this process; preceding summaries may be its children.
pub fn counts(text: &str) -> Result<(usize, usize)> {
	let re = Regex::new(r"(?m)^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;")?;
	let c = re
		.captures_iter(text)
		.last()
		.context("missing test summary")?;
	Ok((c[1].parse()?, c[2].parse()?))
}

fn exact_counts(text: &str, expected: usize) -> Result<()> {
	let re = Regex::new(r"(?m)^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;")?;
	ensure!(
		re.captures_iter(text).count() == 1,
		"expected one test-process summary"
	);
	ensure!(
		counts(text)? == (expected, 0),
		"selected/passed count mismatch"
	);
	Ok(())
}

fn contract_bundle_output(text: &str, selector: &str) -> Result<()> {
	exact_counts(text, 1)?;
	let marker = format!("test {selector} ... ");
	ensure!(
		text.lines()
			.filter(|line| line.starts_with(&marker))
			.count() == 1,
		"contract_bundle must contain one outer test start"
	);
	Ok(())
}

fn listed(text: &str) -> Vec<String> {
	text.lines()
		.filter_map(|l| l.strip_suffix(": test").map(str::to_owned))
		.collect()
}

fn test_args(cmd: &mut Command, selector: &str, exact: bool, ignored: bool) {
	cmd.arg(selector);
	if exact {
		cmd.arg("--exact");
	}
	if ignored {
		cmd.arg("--ignored");
	}
	cmd.args(["--nocapture", "--test-threads=1"]);
}

fn selected(
	ctx: &Context,
	binary: &Path,
	label: &str,
	selector: &str,
	exact: bool,
	ignored: bool,
) -> Result<usize> {
	let mut cmd = Command::new(binary);
	cmd.arg(selector).arg("--list");
	if exact {
		cmd.arg("--exact");
	}
	if ignored {
		cmd.arg("--ignored");
	}
	let r = ctx.checked(&format!("{label}-selection"), &mut cmd, 60)?;
	let names = listed(&fs::read_to_string(r.stdout)?);
	ensure!(!names.is_empty(), "zero selected tests for {selector}");
	if exact {
		ensure!(
			names == [selector],
			"expected exactly {selector}, selected {names:?}"
		);
	}
	Ok(names.len())
}

fn base_env(ctx: &Context, cmd: &mut Command, dir: &Path, receipt: &Path) -> Result<()> {
	cmd.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("LANG", "C.UTF-8");
	for (key, value) in [
		("GAP_OUT", "gap-out"),
		("C10_REPLAY_OUT", "c10-out"),
		("C11_REPLAY_OUT", "c11-out"),
		("C15B_STOCK_OUT", "c15b-out"),
		("C15C_STOCK_OUT", "c15c-out"),
		("C13_OUT", "c13-out"),
		("C15_OUT", "c15-out"),
		("C15B_OUT", "c15b-fresh-out"),
		("C15C_OUT", "c15c-fresh-out"),
		("C12_OUT", "c12-out"),
		("C14_OUT", "c14-out"),
		("C14B_OUT", "c14b-out"),
	] {
		cmd.env(key, dir.join(value));
	}
	cmd.env("C15C_NATIVE_RECEIPT", receipt)
		.env("C15C_NATIVE_RECEIPT_SHA256", hash_file(receipt)?)
		.env("C15C_NATIVE_SOURCE", &ctx.repo);
	Ok(())
}

fn sandbox(binary: &Path, out: &Path) -> Command {
	let mut cmd = Command::new("/usr/bin/bwrap");
	cmd.args(["--ro-bind", "/", "/", "--bind"])
		.arg(out)
		.arg(out)
		.args([
			"--tmpfs",
			"/tmp",
			"--dev-bind",
			"/dev",
			"/dev",
			"--proc",
			"/proc",
			"--",
		])
		.arg(binary)
		.current_dir("/tmp");
	cmd
}

fn contracts(ctx: &Context, m: &Manifest) -> Result<Value> {
	let binary = ctx.test_binary()?;
	let receipt = ctx.test_binary_receipt()?;
	contract_registry(ctx, m, &binary)?;
	// Check the process-exit child protocol before the expensive contract matrix.
	let witness = witness::run(ctx, &binary)?;
	let (rows, full) = std::thread::scope(|scope| -> Result<_> {
		let bundle = scope.spawn(|| contract_bundle(ctx, m, &binary, &receipt));
		let mut cases: Vec<_> = m.cases.iter().collect();
		cases.sort_by_key(|case| std::cmp::Reverse(case.timeout));
		let rows = crate::scheduler::map(&cases, ctx.jobs, |c| {
			contract_case(ctx, &binary, &receipt, c)
		});
		let full = bundle
			.join()
			.map_err(|_| anyhow::anyhow!("contract bundle worker panicked"))?;
		Ok((rows?, full?))
	})?;
	Ok(json!({"status":"PASS","cases":rows,"contract_bundle":full,"diagnostic_witnesses":witness}))
}

fn contract_registry(ctx: &Context, m: &Manifest, binary: &Path) -> Result<Value> {
	// Compare discovered legacy contracts to the registry so new or missing contracts
	// cannot silently change acceptance coverage.
	let mut cmd = Command::new(binary);
	cmd.args(["--list", "--ignored"]);
	let listing = ctx.checked("contract-registry", &mut cmd, 60)?;
	let mut actual: Vec<_> = listed(&fs::read_to_string(listing.stdout)?)
		.into_iter()
		.filter(|n| n.ends_with("::legacy_contract") && !m.excluded_legacy.contains(n))
		.collect();
	let mut expected: Vec<_> = m
		.cases
		.iter()
		.filter(|c| c.name.starts_with("stock-"))
		.map(|c| c.selector.clone())
		.collect();
	actual.sort();
	expected.sort();
	ensure!(
		actual == expected,
		"legacy registry differs from test binary"
	);
	Ok(json!({"contracts":expected.len()}))
}

fn contract_case(ctx: &Context, binary: &Path, receipt: &Path, c: &Case) -> Result<Value> {
	let n = selected(ctx, binary, &c.name, &c.selector, c.exact, c.ignored)?;
	let dir = ctx.out.join(&c.name);
	fs::create_dir(&dir)?;
	let mut cmd = if c.sandbox {
		sandbox(binary, &ctx.out)
	} else {
		let mut cmd = Command::new(binary);
		cmd.current_dir(&dir);
		cmd
	};
	base_env(ctx, &mut cmd, &dir, receipt)?;
	cmd.envs(&c.env);
	test_args(&mut cmd, &c.selector, c.exact, c.ignored);
	// The existing short compiler-limit child bound is sensitive to
	// contention. Preserve its bound and run this observed case alone.
	let r = if c.exclusive {
		let run = ctx.run_exclusive(&c.name, &mut cmd, c.timeout)?;
		ensure!(
			run.code == 0,
			"{} failed with code {}; raw logs: {}, {}",
			c.name,
			run.code,
			run.stdout.display(),
			run.stderr.display()
		);
		run
	} else {
		ctx.checked(&c.name, &mut cmd, c.timeout)?
	};
	let text = fs::read_to_string(&r.stdout)?;
	exact_counts(&text, n).with_context(|| c.name.clone())?;
	if let Some(count) = c.rows {
		ensure!(
			text.contains(&format!("CONTRACT ROWS {count}\n")),
			"{} contract rows mismatch",
			c.name
		);
	}
	if c.empty_stderr {
		ensure!(
			fs::metadata(&r.stderr)?.len() == 0,
			"{} nonempty stderr",
			c.name
		);
	}
	Ok(
		json!({"name":c.name,"selector":c.selector,"passed":n,"contract_rows":c.rows,"seconds":r.seconds,"receipt":r.receipt}),
	)
}

fn contract_bundle(ctx: &Context, m: &Manifest, binary: &Path, receipt: &Path) -> Result<Value> {
	let export: Value = read_json(&ctx.repo.join(&m.contract_bundle.pins))?;
	let external = ctx.fixtures.join(&m.contract_bundle.external);
	let pins = export["files"]
		.as_object()
		.context("external files missing")?;
	for (p, h) in pins {
		ensure!(
			hash_file(&external.join(p))? == h.as_str().context("invalid external hash")?,
			"external fixture changed: {p}"
		);
	}
	selected(
		ctx,
		binary,
		"contract_bundle",
		&m.contract_bundle.selector,
		true,
		true,
	)?;
	let dir = ctx.out.join("contract_bundle");
	fs::create_dir(&dir)?;
	let raw = dir.join("raw");
	let mut cmd = sandbox(binary, &ctx.out);
	base_env(ctx, &mut cmd, &dir, receipt)?;
	cmd.env("GAP_OUT", &raw).env("GAP_EXTERNAL", &external);
	test_args(&mut cmd, &m.contract_bundle.selector, true, true);
	let r = ctx.checked("contract_bundle", &mut cmd, 1800)?;
	contract_bundle_output(&fs::read_to_string(&r.stdout)?, &m.contract_bundle.selector)?;
	for suffix in ["stdout", "stderr"] {
		let n = fs::read_dir(&raw)?
			.filter_map(|e| e.ok())
			.filter(|e| e.path().extension().is_some_and(|x| x == suffix))
			.count();
		ensure!(
			n == m.contract_bundle.contracts,
			"contract_bundle {suffix}: expected {} files, got {n}",
			m.contract_bundle.contracts
		);
	}
	for (p, h) in pins {
		ensure!(
			hash_file(&external.join(p))? == h.as_str().unwrap(),
			"external fixture drift: {p}"
		);
	}
	Ok(
		json!({"contracts":m.contract_bundle.contracts,"outputs":m.contract_bundle.contracts*2,"seconds":r.seconds,"external_head":export["head"],"receipt":r.receipt}),
	)
}

fn native(ctx: &Context, m: &Manifest) -> Result<Value> {
	let rows = crate::scheduler::map(&m.libraries, ctx.jobs, |l| library(ctx, l))?;
	let mut docs = Vec::new();
	for (label, selector) in [
		(
			"prepared-borrows",
			"experimental_core::execute::PreparedInvocation",
		),
		("view-borrows", "experimental_core::heap::view::RootView"),
	] {
		docs.push(borrow_doc(ctx, label, selector)?);
	}
	Ok(json!({"status":"PASS","libraries":rows,"embedding_borrow_docs":docs}))
}

fn borrow_doc(ctx: &Context, label: &str, selector: &str) -> Result<Value> {
	let mut command = ctx.cargo();
	command.env("CARGO_TARGET_DIR", ctx.root_out.join("borrow-doc-target"));
	command.args([
		"test",
		"--locked",
		"-p",
		"rune",
		"--doc",
		"--features",
		"fmt,workspace,execution",
		selector,
	]);
	let run = ctx.checked(label, &mut command, 600)?;
	exact_counts(&fs::read_to_string(&run.stdout)?, 2)?;
	Ok(json!({"selector":selector,"receipt":run.receipt,"passed":2}))
}

fn library(ctx: &Context, l: &Library) -> Result<Value> {
	let failure_names = Regex::new(r"(?m)^    (\S+)$")?;
	let binary = ctx.build_rune(&l.name, &l.features)?;
	let mut cmd = Command::new(&binary);
	cmd.current_dir(&ctx.repo);
	let edge_output = ctx.out.join(format!("{}-budget-edges", l.name));
	let has_edges = matches!(l.name.as_str(), "execution-serde" | "native");
	if has_edges {
		cmd.env("E1_EDGE_OUTPUT", &edge_output);
	}
	if let Some(selector) = &l.selector {
		cmd.arg(selector);
	}
	cmd.arg(format!("--test-threads={}", ctx.test_threads));
	let r = ctx.run(&format!("{}-library", l.name), &mut cmd, 2400)?;
	let text = fs::read_to_string(&r.stdout)?;
	let (passed, failed) = counts(&text)?;
	let mut retry = Value::Null;
	if r.code != 0 || failed != 0 {
		let flaky = "experimental_core::nominal::tests::limits_refuse_before_publishing";
		let failures = text.rsplit("\nfailures:\n").next().unwrap_or("");
		let names: std::collections::BTreeSet<_> = failure_names
			.captures_iter(failures)
			.map(|c| c[1].to_owned())
			.collect();
		ensure!(
			failed == 1 && names == [flaky.to_owned()].into_iter().collect(),
			"{} library failed",
			l.name
		);
		let mut cmd = Command::new(&binary);
		cmd.current_dir(&ctx.repo).args(["--exact", flaky]);
		let rerun = ctx.checked(&format!("{}-known-flaky-rerun", l.name), &mut cmd, 600)?;
		exact_counts(&fs::read_to_string(&rerun.stdout)?, 1)?;
		retry = json!({"receipt":rerun.receipt,"passed":1});
	}
	ensure!(
		passed + failed == l.passed,
		"{} library count changed: {} vs {}",
		l.name,
		passed + failed,
		l.passed
	);
	let budget_classes = if has_edges {
		read_json::<Value>(&edge_output.join("COUNTS.json"))?
	} else {
		Value::Null
	};
	Ok(
		json!({"name":l.name,"features":l.features,"passed":passed,"failed":failed,"flaky_rerun":retry,"expected":l.passed,"receipt":r.receipt,"seconds":r.seconds,"budget_classes":budget_classes,"budget_observations":if has_edges {Some(edge_output)}else{None}}),
	)
}

fn clippy(ctx: &Context, m: &Manifest) -> Result<Value> {
	let mut rows = vec![];
	let mut cmd = ctx.cargo();
	cmd.args(["fmt", "--check"]);
	let r = ctx.checked("fmt", &mut cmd, 900)?;
	rows.push(json!({"name":"fmt","receipt":r.receipt,"seconds":r.seconds}));
	for (i, f) in m.clippy_features.iter().enumerate() {
		let mut cmd = ctx.cargo();
		cmd.args([
			"clippy",
			"--locked",
			"-p",
			"rune",
			"--all-targets",
			"--features",
			f,
			"--",
			"-D",
			"warnings",
		]);
		let r = ctx.checked(&format!("clippy-{i}"), &mut cmd, 2400)?;
		rows.push(json!({"features":f,"receipt":r.receipt,"seconds":r.seconds}));
	}
	Ok(json!({"status":"PASS","checks":rows}))
}

pub fn run(ctx: &Context, suite: &str) -> Result<Value> {
	let m = manifest()?;
	let uses_fixtures = matches!(suite, "contracts" | "layouts");
	if uses_fixtures {
		fixtures(ctx, &m)?;
	}
	let result = match suite {
		"native" => native(ctx, &m),
		"contracts" => contracts(ctx, &m),
		"clippy" => clippy(ctx, &m),
		"layouts" => layout::run(ctx),
		_ => bail!("unknown native suite {suite}"),
	}?;
	if uses_fixtures {
		fixtures(ctx, &m).context("native input changed during verification")?;
	}
	Ok(result)
}

pub fn task_specs(suite: &str) -> Result<Vec<crate::tasks::Task>> {
	use crate::tasks::Task;
	let m = manifest()?;
	let mut tasks = Vec::new();
	if suite == "contracts" {
		tasks.push(Task::new(suite, "registry", "all", 30.0));
		tasks.push(Task::new(suite, "bundle", "all", 1200.0));
		tasks.extend(
			m.cases
				.iter()
				.map(|c| Task::new(suite, "case", &c.name, c.timeout.min(900) as f64)),
		);
	} else {
		tasks.extend(
			m.libraries
				.iter()
				.map(|l| Task::new(suite, "library", &l.name, 700.0)),
		);
		for name in ["prepared-borrows", "view-borrows"] {
			tasks.push(Task::new(suite, "doc", name, 60.0));
		}
	}
	Ok(tasks)
}
pub fn run_task(ctx: &Context, task: &crate::tasks::Task) -> Result<Value> {
	let m = manifest()?;
	match task.action.as_str() {
		"registry" => {
			fixtures(ctx, &m)?;
			let binary = ctx.test_binary()?;
			Ok(
				json!({"registry":contract_registry(ctx,&m,&binary)?,"witness":witness::run(ctx,&binary)?}),
			)
		}
		"bundle" => contract_bundle(ctx, &m, &ctx.test_binary()?, &ctx.test_binary_receipt()?),
		"case" => {
			let c = m
				.cases
				.iter()
				.find(|c| c.name == task.key)
				.context("unknown contract task")?;
			contract_case(ctx, &ctx.test_binary()?, &ctx.test_binary_receipt()?, c)
		}
		"library" => library(
			ctx,
			m.libraries
				.iter()
				.find(|l| l.name == task.key)
				.context("unknown library task")?,
		),
		"doc" => {
			let selector = match task.key.as_str() {
				"prepared-borrows" => "experimental_core::execute::PreparedInvocation",
				"view-borrows" => "experimental_core::heap::view::RootView",
				_ => bail!("unknown borrow-doc task"),
			};
			borrow_doc(ctx, &task.key, selector)
		}
		_ => bail!("unknown native task"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn summary_uses_final_child_boundary() {
		assert_eq!(
			counts(
				"test result: ok. 1 passed; 0 failed;\ntest result: ok. 612 passed; 0 failed;\n"
			)
			.unwrap(),
			(612, 0)
		);
		assert!(counts("no tests").is_err());
	}
	#[test]
	fn exact_rejects_zero_and_duplicate_summaries() {
		assert!(exact_counts("test result: ok. 0 passed; 0 failed;\n", 1).is_err());
		assert!(exact_counts(
			"test result: ok. 1 passed; 0 failed;\ntest result: ok. 1 passed; 0 failed;\n",
			1
		)
		.is_err());
	}
	#[test]
	fn fixture_mutation_is_rejected() {
		let root = std::env::temp_dir().join(format!("rune-native-fixture-{}", std::process::id()));
		fs::create_dir(&root).unwrap();
		let path = root.join("input.json");
		fs::write(&path, b"frozen").unwrap();
		let fixtures = [Fixture {
			path: "input.json".to_owned(),
			sha256: hash_file(&path).unwrap(),
		}];
		assert!(verify_fixtures(&root, &fixtures).is_ok());
		fs::write(&path, b"mutated").unwrap();
		assert!(verify_fixtures(&root, &fixtures).is_err());
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn contract_bundle_summary_and_outer_selection_are_preserved() {
		let selector = "experimental_core::tests::gap_hunt::contracts::all_frozen_contracts";
		let text = format!("running 1 test\ntest {selector} ... PASS sample\nok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n");
		assert!(contract_bundle_output(&text, selector).is_ok());
		assert!(contract_bundle_output(
			&text.replace("test result: ok. 1 passed;", "test result: ok. 0 passed;"),
			selector
		)
		.is_err());
		assert!(
			contract_bundle_output(&format!("test {selector} ... \n{text}"), selector).is_err()
		);
		assert!(contract_bundle_output(
			&text.replace(&format!("test {selector} ... "), ""),
			selector
		)
		.is_err());
	}

	#[test]
	fn registry_is_nonempty_and_unique() {
		let m = manifest().unwrap();
		assert_eq!(m.cases.len(), 50);
		assert_eq!(
			m.cases
				.iter()
				.filter(|c| c.exclusive)
				.map(|c| c.name.as_str())
				.collect::<Vec<_>>(),
			vec!["wrong_arity-fresh"]
		);
		assert_eq!(m.contract_bundle.contracts, 19);
	}
}
