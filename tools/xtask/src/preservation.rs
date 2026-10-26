//! Historical compiler and serialized-Unit preservation, without historical controllers.
mod emissions;
mod records;

use crate::common::{self, Context, Result};
use anyhow::{ensure, Context as _};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
	collections::BTreeMap,
	fs,
	path::{Path, PathBuf},
	process::Command,
};

#[derive(Deserialize)]
struct Manifest {
	counts: BTreeMap<String, usize>,
	fixture_pins: BTreeMap<String, String>,
	repository_pins: BTreeMap<String, String>,
	l1_records: String,
	historical_anchor_component: String,
	modules: Vec<Module>,
	unit_records: String,
	l1_classification: String,
}
#[derive(Deserialize)]
struct Case {
	#[serde(rename = "case")]
	name: String,
	source: String,
	source_sha256: String,
	sha256: String,
}
#[derive(Deserialize)]
struct Module {
	name: String,
	prefix: String,
	root: String,
	stocks: Vec<String>,
	overlays: Vec<String>,
	baseline_progress: String,
}

fn load() -> Result<Manifest> {
	Ok(serde_json::from_str(include_str!(
		"../manifests/preservation.json"
	))?)
}
fn read(path: &Path) -> Result<Value> {
	Ok(serde_json::from_slice(
		&fs::read(path).with_context(|| format!("read {}", path.display()))?,
	)?)
}
fn number(v: &Value, key: &str) -> Result<usize> {
	Ok(v[key]
		.as_u64()
		.with_context(|| format!("missing integer {key}"))? as usize)
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
	v[key]
		.as_str()
		.with_context(|| format!("missing string {key}"))
}
fn array(v: &Value) -> Result<&Vec<Value>> {
	v.as_array().context("expected array")
}
fn require_count(actual: usize, expected: usize, what: &str) -> Result<()> {
	ensure!(
		actual == expected,
		"{what}: expected {expected} records, got {actual}"
	);
	Ok(())
}
fn exact(before: &Path, after: &Path) -> Result<()> {
	ensure!(
		fs::read(before)? == fs::read(after)?,
		"frozen byte mismatch: {} versus {}",
		before.display(),
		after.display()
	);
	Ok(())
}
fn verify_pins(ctx: &Context, m: &Manifest) -> Result<()> {
	for (root, pins) in [
		(&ctx.fixtures, &m.fixture_pins),
		(&ctx.repo, &m.repository_pins),
	] {
		for (path, expected) in pins {
			ensure!(
				common::hash_file(&root.join(path))? == *expected,
				"preservation input digest mismatch: {path}"
			);
		}
	}
	Ok(())
}
fn child(
	ctx: &Context,
	binary: &Path,
	dir: &Path,
	selector: &str,
	env: &[(&str, String)],
	timeout: u64,
	limited: bool,
) -> Result<()> {
	fs::create_dir(dir)?;
	let mut command = if limited {
		let mut c = Command::new("/usr/bin/prlimit");
		c.args([
			"--cpu=15",
			"--as=2147483648",
			"--fsize=2097152",
			"--core=0",
			"--",
		])
		.arg(binary);
		c
	} else {
		Command::new(binary)
	};
	command
		.current_dir(dir)
		.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("LANG", "C.UTF-8")
		.env("TZ", "UTC")
		.args(["--exact", selector, "--ignored", "--nocapture", "--quiet"]);
	for (key, value) in env {
		command.env(key, value);
	}
	let label = dir
		.strip_prefix(&ctx.out)?
		.to_string_lossy()
		.replace('/', "-");
	let run = ctx.checked(&label, &mut command, timeout)?;
	let output = fs::read_to_string(&run.stdout)?;
	ensure!(
		output
			.lines()
			.filter(|line| line.trim() == "running 1 test")
			.count() == 1,
		"observer must select exactly one test"
	);
	ensure!(
		!output.contains("running 0 tests") && !output.contains("FAILED"),
		"observer selection/failure output"
	);
	fs::copy(&run.stdout, dir.join("stdout"))?;
	fs::copy(&run.stderr, dir.join("stderr"))?;
	Ok(())
}
fn verify_indexed_artifacts(ctx: &Context, m: &Manifest) -> Result<()> {
	let l1_root = ctx
		.fixtures
		.join(&m.l1_records)
		.parent()
		.context("L1 baseline parent")?
		.to_path_buf();
	for case in records::cases(ctx, m)? {
		records::source(ctx, m, &case)?;
		ensure!(
			common::hash_file(&l1_root.join(format!("{}.code", case.name)))? == case.sha256,
			"L1 baseline digest changed"
		);
	}
	let rows = read(&ctx.fixtures.join(&m.unit_records))?;
	require_count(array(&rows)?.len(), m.counts["legacy_units"], "Unit index")?;
	let base = ctx
		.fixtures
		.join(&m.unit_records)
		.parent()
		.context("Unit baseline parent")?
		.join("records");
	for row in array(&rows)? {
		let name = string(row, "case")?;
		let path = base.join(format!("{name}.json"));
		ensure!(
			common::hash_file(&path)? == string(row, "record_sha256")?,
			"Unit metadata digest changed"
		);
		let record = read(&path)?;
		if record["status"] == "compiled" {
			ensure!(
				common::hash_file(&base.join(format!("{name}.unit")))?
					== string(&record, "unit_sha256")?,
				"Unit byte digest changed"
			);
		}
	}
	let mut all = 0;
	let mut admitted = 0;
	for module in &m.modules {
		let (cases, rules, stocks) = emissions::rules(ctx, module)?;
		for (i, ((case, rule), stock)) in cases.iter().zip(&rules).zip(&stocks).enumerate() {
			ensure!(
				number(rule, "index")? == i && case["source_sha256"] == rule["source_sha256"],
				"policy case identity mismatch"
			);
			ensure!(
				common::hash_bytes(&serde_json::to_vec(stock)?)
					== string(rule, "stock_row_sha256")?,
				"policy stock row digest mismatch"
			);
		}
		let path = ctx.fixtures.join(&module.baseline_progress);
		let rows = read(&path)?;
		for (i, row) in array(&rows)?.iter().enumerate() {
			ensure!(
				number(row, "index")? == i,
				"emission baseline index changed"
			);
			all += 1;
			if row["invoked"] == true {
				let file = path
					.parent()
					.context("emission baseline parent")?
					.join(format!("{i:03}/emission.code"));
				ensure!(
					common::hash_file(&file)? == string(row, "emission_sha256")?,
					"emission byte digest changed"
				);
				admitted += 1;
			}
		}
	}
	require_count(all, m.counts["emissions"], "emission index")?;
	require_count(
		admitted,
		m.counts["admitted_emissions"],
		"admitted emission index",
	)?;
	Ok(())
}
pub fn check_manifest(ctx: &Context) -> Result<Value> {
	let manifest = load()?;
	verify_pins(ctx, &manifest)?;
	verify_indexed_artifacts(ctx, &manifest)?;
	Ok(
		json!({"fixture_indices":manifest.fixture_pins.len(),"repository_inputs":manifest.repository_pins.len(),"counts":manifest.counts}),
	)
}
pub fn run(ctx: &Context, suite: &str) -> Result<Value> {
	let manifest = load()?;
	verify_pins(ctx, &manifest)?;
	verify_indexed_artifacts(ctx, &manifest)?;
	let binary: PathBuf = if suite == "legacy-units" {
		ctx.build_rune(
			"legacy-units",
			"fmt,workspace,execution,capture-io,disable-io,sha2,serde,musli,musli/storage",
		)?
	} else {
		ctx.test_binary()?
	};
	let digest = common::hash_file(&binary)?;
	let selectors: Vec<String> = match suite {
		"emissions" => manifest
			.modules
			.iter()
			.map(|m| format!("experimental_core::tests::gap_hunt::{}::child", m.name))
			.collect(),
		"l1" => vec!["experimental_core::tests::gap_hunt::emission_child".to_owned()],
		"legacy-units" => vec![
			"experimental_core::tests::gap_hunt::legacy_unit_receipts::legacy_unit_child"
				.to_owned(),
		],
		_ => anyhow::bail!("unknown preservation suite {suite}"),
	};
	for (i, selector) in selectors.iter().enumerate() {
		let mut cmd = Command::new(&binary);
		cmd.args(["--exact", selector, "--ignored", "--list"]);
		let proof = ctx.checked(&format!("selector-{i}"), &mut cmd, 30)?;
		common::require_one_selection(&proof.stdout, selector)?;
	}
	if suite == "legacy-units" {
		let selector="experimental_core::tests::gap_hunt::legacy_unit_receipts::relocated_modules_preserve_logical_identity_and_never_fall_back";
		let mut list = Command::new(&binary);
		list.args(["--exact", selector, "--list"]);
		let registration = ctx.checked("relocation-regression-selection", &mut list, 30)?;
		common::require_one_selection(&registration.stdout, selector)?;
		let mut test = Command::new(&binary);
		test.args(["--exact", selector, "--test-threads=1"]);
		let regression = ctx.checked("relocation-regression", &mut test, 30)?;
		common::require_one_test(&regression.stdout)?;
	}
	let result = match suite {
		"emissions" => emissions::run(ctx, &manifest, &binary),
		"l1" => records::l1(ctx, &manifest, &binary),
		"legacy-units" => records::units(ctx, &manifest, &binary),
		_ => anyhow::bail!("unknown preservation suite {suite}"),
	}?;
	verify_pins(ctx, &manifest)?;
	verify_indexed_artifacts(ctx, &manifest)?;
	ensure!(
		common::hash_file(&binary)? == digest,
		"test binary changed during preservation suite"
	);
	Ok(json!({"status":"PASS","suite":suite,"binary_sha256":digest,"result":result}))
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn missing_record_fails_expected_corpus_count() {
		assert!(require_count(2940, 2941, "L1").is_err());
		assert!(require_count(2942, 2941, "L1").is_err());
		assert!(require_count(2941, 2941, "L1").is_ok());
	}
	#[test]
	fn missing_policy_field_is_error() {
		assert!(number(&json!({}), "index").is_err());
	}
	#[test]
	fn changed_or_missing_frozen_bytes_fail() {
		let root = std::env::temp_dir().join(format!("rune-preservation-{}", std::process::id()));
		fs::create_dir(&root).unwrap();
		let a = root.join("a");
		let b = root.join("b");
		fs::write(&a, b"float bits\0\xff").unwrap();
		fs::write(&b, b"float bits\0\xfe").unwrap();
		assert!(exact(&a, &b).is_err());
		fs::remove_file(&b).unwrap();
		assert!(exact(&a, &b).is_err());
		fs::remove_dir_all(root).unwrap();
	}
}
