//! Frozen AWFY semantic observations, never throughput measurements.
use crate::common::{self, Context, Result};
use anyhow::{ensure, Context as _};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
	collections::{BTreeMap, BTreeSet},
	fs,
	path::{Path, PathBuf},
	process::Command,
};
const HISTORICAL: &str =
	"inactive environment control: S7A_* names unread by the S6_* child; ran under ordinary limits";
const MANIFEST: &str = include_str!("../manifests/awfy.json");
// Match the historical Linux child main-thread stack with a libtest worker.
// This configures only the test observer, without changing release code.
const OBSERVER_STACK_BYTES: usize = 8usize << 20;
#[derive(Deserialize)]
struct Manifest {
	features: String,
	selector: String,
	pins: BTreeMap<String, Pin>,
	ports: BTreeMap<String, String>,
	cases: Vec<Case>,
	historical_defect: String,
	active_reference: String,
}
#[derive(Deserialize)]
struct Pin {
	kind: String,
	sha256: String,
}
#[derive(Deserialize)]
struct Input {
	kind: String,
	fixture: String,
	origin: String,
}
#[derive(Deserialize)]
struct Case {
	name: String,
	family: String,
	classification: String,
	num: i64,
	inner: i64,
	inputs: Vec<Input>,
	expected: Value,
	environment: BTreeMap<String, String>,
	active_limits: bool,
}
fn fixture(ctx: &Context, name: &str) -> Result<PathBuf> {
	let path = Path::new(name);
	ensure!(
		!path.is_absolute()
			&& path
				.components()
				.all(|c| matches!(c, std::path::Component::Normal(_))),
		"unsafe fixture path"
	);
	Ok(ctx.repo.join(path))
}
fn historical_origin(path: &str) -> Result<String> {
	if let Some(name) = path.strip_prefix("benches/awfy/") {
		ensure!(
			Path::new(name).components().count() == 1,
			"invalid benchmark path"
		);
		return Ok(format!(
			"/home/me/work/.reviews/0185-c17f-awfy1/ports/{name}"
		));
	}
	let relative = path
		.strip_prefix("tools/xtask/fixtures/awfy/")
		.context("unknown repository AWFY fixture")?;
	Ok(format!("/home/me/work/.reviews/{relative}"))
}

fn wire_hashes(case: &Case, row: &Value) -> Result<()> {
	for stream in ["stdout", "stderr"] {
		let bytes = case.expected[stream]
			.as_str()
			.context("expected wire stream")?
			.as_bytes();
		ensure!(
			row[format!("{stream}_sha256")] == common::hash_bytes(bytes),
			"frozen {stream} digest disagreement: {}",
			case.name
		);
	}
	Ok(())
}

fn active_wire(case: &Case, reference: &Value) -> Result<()> {
	ensure!(
		reference["status"] == "PENDING_EXPECTATION_FREEZE"
			&& reference["product_baseline"] == "f0baf933382039f2b8c44b84a04532ba5703b459",
		"unexpected active first-observation provenance"
	);
	let rows = reference["observations"]
		.as_array()
		.context("missing active baseline observations")?;
	ensure!(
		rows.len() == 6,
		"active baseline must contain exactly six observations"
	);
	let selected = rows
		.iter()
		.filter(|row| row["name"] == case.name)
		.collect::<Vec<_>>();
	ensure!(
		selected.len() == 1,
		"missing or duplicated active baseline name"
	);
	let row = selected[0];
	ensure!(
		row["environment"] == serde_json::to_value(&case.environment)?,
		"active baseline environment mismatch"
	);
	compare(&case.expected, &row["wire"])
}

fn manifest(ctx: &Context) -> Result<Manifest> {
	let m: Manifest = serde_json::from_str(MANIFEST)?;
	ensure!(
		m.cases.len() == 38,
		"AWFY requires24 candidate+6 inactive+6 active+2 diagnostic cases"
	);
	for (name, pin) in &m.pins {
		ensure!(pin.kind == "repo", "AWFY inputs must be repo-relative");
		ensure!(
			common::hash_file(&fixture(ctx, name)?)? == pin.sha256,
			"AWFY fixture mismatch: {name}"
		);
	}
	ensure!(
		m.ports.len() == 15,
		"AWFY requires12 benchmark ports plus3 support modules"
	);
	for (name, sha) in &m.ports {
		ensure!(
			Path::new(name).components().count() == 1,
			"invalid port name"
		);
		ensure!(
			common::hash_file(&ctx.repo.join("benches/awfy").join(name))? == *sha,
			"repository port changed: {name}"
		);
	}
	let frozen: Value = common::read_json(&fixture(
		ctx,
		"tools/xtask/fixtures/awfy/0185-s7b2-final35-awfy/OBSERVATIONS.json",
	)?)?;
	let controls: Value = common::read_json(&fixture(
		ctx,
		"tools/xtask/fixtures/awfy/0185-s7b2-final35-awfy/HAVLAK-LIMIT-CONTROLS.json",
	)?)?;
	let diagnostics: Value = common::read_json(&fixture(
		ctx,
		"tools/xtask/fixtures/awfy/0185-s7b2-final35-awfy/L1-DIAGNOSTIC-CHECK.json",
	)?)?;
	ensure!(
		m.pins.contains_key(&m.active_reference),
		"unpinned active first observations"
	);
	let active_reference: Value = common::read_json(&fixture(ctx, &m.active_reference)?)?;
	let mut names = BTreeSet::new();
	let mut families = BTreeMap::new();
	let mut programs = BTreeSet::new();
	for c in &m.cases {
		ensure!(
			names.insert(&c.name)
				&& !c.name.is_empty()
				&& c.name
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
			"duplicate or unsafe AWFY case"
		);
		*families.entry(c.family.as_str()).or_insert(0usize) += 1;
		match c.family.as_str() {
			"candidate" => {
				let (program, _) = c.name.rsplit_once('-').context("candidate name")?;
				let rows = frozen.as_array().context("frozen candidate rows")?;
				let selected = rows
					.iter()
					.filter(|r| r["program"] == program && r["outer"] == c.num)
					.collect::<Vec<_>>();
				ensure!(
					selected.len() == 1,
					"candidate missing from frozen reference"
				);
				let r = selected[0];
				ensure!(
					r["classification"] == c.classification,
					"classification changed"
				);
				let expected = &r["observations"]["candidate"];
				compare(
					&json!({"code":expected["rc"],"stdout":expected["stdout"],"stderr":expected["stderr"]}),
					&c.expected,
				)?;
			}
			"historical-inactive-control" => {
				let kind = c.name.rsplit_once('-').context("control name")?.1;
				let rows = controls.as_array().context("frozen controls")?;
				let selected = rows
					.iter()
					.filter(|r| r["outer"] == c.num && r["kind"] == kind)
					.collect::<Vec<_>>();
				ensure!(selected.len() == 1, "control missing from frozen reference");
				let r = selected[0];
				ensure!(
					r["rc"] == c.expected["code"]
						&& r["env"] == serde_json::to_value(&c.environment)?,
					"historical control changed"
				);
				wire_hashes(c, r)?;
			}
			"fatal-diagnostic" => {
				let case = c.name.strip_prefix("l1-").context("diagnostic case name")?;
				let rows = diagnostics["checks"]
					.as_array()
					.context("frozen diagnostic rows")?;
				let selected = rows
					.iter()
					.filter(|r| r["case"] == case)
					.collect::<Vec<_>>();
				ensure!(
					selected.len() == 1 && c.expected["code"] == 3,
					"diagnostic missing from frozen reference"
				);
				let r = selected[0];
				ensure!(
					r["old_span"] == json!([1057, 1072])
						&& r["new_span"] == json!([1189, 1198])
						&& r["new_fatal_diagnostic"] == true,
					"diagnostic progression changed"
				);
				wire_hashes(c, r)?;
			}
			"active-limit-control" => {
				active_wire(c, &active_reference)?;
				ensure!(
					c.active_limits
						&& !c.environment.is_empty()
						&& c.environment
							.keys()
							.all(|k| k == "S6_FRAME_LIMIT" || k == "S6_SLOT_LIMIT"),
					"invalid active controls"
				);
			}
			_ => anyhow::bail!("unknown AWFY family"),
		}

		ensure!(
			c.active_limits == (c.family == "active-limit-control"),
			"active control family/flag mismatch"
		);
		if c.family == "candidate" && c.classification == "Exact" {
			programs.insert(c.name.rsplit_once('-').context("case outer suffix")?.0);
		}
		if c.family == "historical-inactive-control" {
			ensure!(
				c.classification == HISTORICAL,
				"historical control qualification changed"
			);
			ensure!(
				c.environment
					.keys()
					.all(|k| k == "S7A_FRAME_LIMIT" || k == "S7A_SLOT_LIMIT"),
				"historical controls must remain inactive"
			);
		}
		for input in &c.inputs {
			ensure!(
				input.kind == "repo" && m.pins.contains_key(&input.fixture),
				"unpinned or non-repo AWFY input"
			);
			ensure!(
				input.origin == historical_origin(&input.fixture)?,
				"historical logical source origin changed"
			);
		}
		ensure!(
			(c.expected["code"].is_i64()
				&& c.expected["stdout"].is_string()
				&& c.expected["stderr"].is_string()),
			"invalid expected wire"
		);
	}
	ensure!(
		families.get("candidate") == Some(&24)
			&& families.get("historical-inactive-control") == Some(&6)
			&& families.get("fatal-diagnostic") == Some(&2)
			&& families.get("active-limit-control") == Some(&6)
			&& programs.len() == 10,
		"AWFY boundary/count changed"
	);
	Ok(m)
}
pub fn check_manifest(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	Ok(
		json!({"cases":m.cases.len(),"external_fixture_bytes":0,"candidate":24,"exact_programs":10,"programs":12,"historical_inactive_controls":6,"active_limit_controls":6,"active_expectations_frozen":m.cases.iter().filter(|c|c.active_limits).all(|c|!c.expected.is_null()),"fatal_diagnostics":2,"historical_defect":m.historical_defect,"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes())}),
	)
}
fn compare(expected: &Value, observed: &Value) -> Result<()> {
	ensure!(
		observed == expected,
		"AWFY exact stdout/stderr/status mismatch\nexpected: {expected}\nobserved: {observed}"
	);
	Ok(())
}
fn negative_controls(ctx: &Context, binary: &Path, selector: &str) -> Result<Value> {
	let known = json!({"S6_FRAME_LIMIT":"1024"});
	let cases = [
		(
			"unknown-active-knob",
			"active",
			json!({"S6_FRAME_LIMT":"1024"}),
			json!({"S6_FRAME_LIMT":"1024"}),
		),
		(
			"unused-ordinary-knob",
			"ordinary",
			known.clone(),
			known.clone(),
		),
		(
			"declared-actual-mismatch",
			"active",
			known.clone(),
			json!({}),
		),
		(
			"undeclared-unknown-knob",
			"ordinary",
			json!({}),
			json!({"S7B_FRAME_LIMIT":"1024"}),
		),
		(
			"active-under-historical-mode",
			HISTORICAL,
			known.clone(),
			known,
		),
	];
	let mut results = Vec::new();
	for (name, mode, declared, actual) in cases {
		let label = format!("negative-{name}");
		let config = ctx.out.join(format!("{label}-config.json"));
		let output = ctx.out.join(format!("{label}-wire.json"));
		let stdout = ctx.out.join(format!("{label}-wire.stdout"));
		common::write_json(
			&config,
			&json!({"num":1,"inner":1,"inputs":[],"stdout":stdout,"control_mode":mode,"control_environment":declared}),
		)?;
		let mut child = Command::new(binary);
		child
			.args([
				selector,
				"--ignored",
				"--exact",
				"--nocapture",
				"--test-threads=1",
			])
			.env("RUST_MIN_STACK", OBSERVER_STACK_BYTES.to_string())
			.env("RUNE_VERIFY_AWFY_CONFIG", &config)
			.env("RUNE_VERIFY_AWFY_OUTPUT", &output);
		for (key, value) in actual.as_object().context("negative environment")? {
			child.env(key, value.as_str().context("negative environment value")?);
		}
		let run = ctx.run(&label, &mut child, 30)?;
		let out = fs::read_to_string(&run.stdout)?;
		let err = fs::read_to_string(&run.stderr)?;
		ensure!(
			run.code == 101 && err.contains("AWFY_CONTROL_CONFIGURATION"),
			"negative control {name} did not reject its configuration"
		);
		ensure!(
			out.lines()
				.filter(
					|line| line.starts_with("test result: FAILED. 0 passed; 1 failed; 0 ignored;")
				)
				.count() == 1
				&& out.contains(&format!("test {selector} ... FAILED")),
			"negative control must select exactly one failed observer"
		);
		ensure!(
			!output.exists() && !stdout.exists(),
			"rejected control reached source execution/capture"
		);
		results.push(
			json!({"name":name,"expected":"configuration refusal before execution","run":run}),
		);
	}
	let report =
		json!({"status":"PASS","expected_configuration_refusals":5,"observations":results});
	common::write_json(&ctx.out.join("CONTROL-NEGATIVES.json"), &report)?;
	Ok(report)
}

pub fn run(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	let binary = ctx.build_rune("awfy-native", &m.features)?;
	let mut list = Command::new(&binary);
	list.args([&m.selector, "--ignored", "--exact", "--list"]);
	let selected = ctx.checked("select", &mut list, 30)?;
	common::require_one_selection(&selected.stdout, &m.selector)?;
	let negatives = negative_controls(ctx, &binary, &m.selector)?;
	let mut observations = Vec::new();
	for c in &m.cases {
		let config = ctx.out.join(format!("{}-config.json", c.name));
		let output = ctx.out.join(format!("{}-wire.json", c.name));
		let stdout = ctx.out.join(format!("{}-wire.stdout", c.name));
		let inputs = c
			.inputs
			.iter()
			.map(|i| Ok(json!({"path":fixture(ctx,&i.fixture)?,"origin":i.origin})))
			.collect::<Result<Vec<_>>>()?;
		common::write_json(
			&config,
			&json!({"num":c.num,"inner":c.inner,"inputs":inputs,"stdout":stdout,"control_mode":if c.active_limits {"active"} else if c.family == "historical-inactive-control" {HISTORICAL} else {"ordinary"},"control_environment":c.environment}),
		)?;
		let mut child = Command::new(&binary);
		child
			.args([
				&m.selector,
				"--ignored",
				"--exact",
				"--nocapture",
				"--test-threads=1",
			])
			.env("RUST_MIN_STACK", OBSERVER_STACK_BYTES.to_string())
			.env("RUNE_VERIFY_AWFY_CONFIG", &config)
			.env("RUNE_VERIFY_AWFY_OUTPUT", &output)
			.envs(&c.environment);
		let run = ctx.checked(&c.name, &mut child, 600)?;
		ensure!(
			fs::read(&run.stderr)?.is_empty(),
			"unexpected observer process stderr: {}",
			c.name
		);
		common::require_one_test(&run.stdout)?;
		let observed: Value = common::read_json(&output)?;
		compare(&c.expected, &observed)?;
		ensure!(
			fs::read(&stdout)?
				== observed["stdout"]
					.as_str()
					.context("wire stdout")?
					.as_bytes(),
			"wire capture disagrees with JSON"
		);
		observations.push(json!({"name":c.name,"family":c.family,"classification":c.classification,"active_limits":c.active_limits,"environment":c.environment,"wire":output,"wire_sha256":common::hash_file(&output)?,"run":run}));
	}
	manifest(ctx)?;
	Ok(
		json!({"status":"PASS","candidate_observations":24,"external_fixture_bytes":0,"configuration_refusals":negatives,"exact_programs":10,"programs":12,"historical_inactive_controls":6,"active_limit_controls":6,"active_expectations_frozen":m.cases.iter().filter(|c|c.active_limits).all(|c|!c.expected.is_null()),"fatal_diagnostics":2,"historical_defect":m.historical_defect,"binary_receipt":ctx.binary_receipt("awfy-native"),"observations":observations,"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes()),"source_origins":"Pinned historical logical filenames are supplied explicitly to Source::new; all fixture reads use repo-relative byte-pinned files without rewriting observed bytes."}),
	)
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn active_baseline_cannot_drift_or_be_partially_selected() {
		let m: Manifest = serde_json::from_str(MANIFEST).unwrap();
		let case = m.cases.into_iter().find(|c| c.active_limits).unwrap();
		let baseline: Value = serde_json::from_str(include_str!(
			"../fixtures/awfy/ACTIVE-FIRST-OBSERVATIONS.json"
		))
		.unwrap();
		assert!(active_wire(&case, &baseline).is_ok());
		for field in ["name", "environment", "wire"] {
			let mut changed = baseline.clone();
			changed["observations"][0][field] = json!("changed");
			assert!(
				active_wire(&case, &changed).is_err(),
				"accepted changed {field}"
			);
		}
		let mut missing = baseline.clone();
		missing["observations"].as_array_mut().unwrap().remove(0);
		assert!(active_wire(&case, &missing).is_err());
		let mut duplicate = baseline.clone();
		duplicate["observations"][1] = duplicate["observations"][0].clone();
		assert!(active_wire(&case, &duplicate).is_err());
		let mut wrong_source = baseline;
		wrong_source["product_baseline"] = json!("other");
		assert!(active_wire(&case, &wrong_source).is_err());
	}

	#[test]
	fn every_awfy_input_and_pin_is_repository_relative() {
		let m: Manifest = serde_json::from_str(MANIFEST).unwrap();
		for (path, pin) in &m.pins {
			assert_eq!(pin.kind, "repo");
			assert!(!Path::new(path).is_absolute());
			assert!(Path::new(path)
				.components()
				.all(|c| matches!(c, std::path::Component::Normal(_))));
		}
		for case in &m.cases {
			for input in &case.inputs {
				assert_eq!(input.kind, "repo");
				assert_eq!(historical_origin(&input.fixture).unwrap(), input.origin);
			}
		}
		assert_eq!(
			historical_origin("benches/awfy/havlak.rn").unwrap(),
			"/home/me/work/.reviews/0185-c17f-awfy1/ports/havlak.rn"
		);
		assert!(historical_origin("/home/me/work/.reviews/old.rn").is_err());
	}

	#[test]
	fn compares_all_wire_bytes_and_status() {
		let wire = json!({"code":3,"stdout":"x\n","stderr":"diagnostic\n"});
		assert!(compare(&wire, &wire).is_ok());
		for wrong in [
			json!({"code":0,"stdout":"x\n","stderr":"diagnostic\n"}),
			json!({"code":3,"stdout":"x","stderr":"diagnostic\n"}),
			json!({"code":3,"stdout":"x\n","stderr":""}),
		] {
			assert!(compare(&wire, &wrong).is_err());
		}
	}
}
