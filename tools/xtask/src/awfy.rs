//! Live AWFY comparisons and explicit resource-control assertions.
use crate::common::{self, Context, Result};
use anyhow::{ensure, Context as _};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
	collections::{BTreeMap, BTreeSet},
	fs,
	path::{Component, Path, PathBuf},
	process::Command,
};

const MANIFEST: &str = include_str!("../manifests/awfy-live.json");
const HISTORICAL: &str =
	"inactive environment control: S7A_* names unread by the S6_* child; ran under ordinary limits";
const SELECTOR: &str = "tests::verification_awfy::child";
const FEATURES: &str = "fmt,workspace,native-code,capture-io,disable-io,sha2";
const OBSERVER_STACK_BYTES: usize = 8usize << 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
	schema: u8,
	cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
	name: String,
	family: String,
	num: i64,
	inner: i64,
	inputs: Vec<String>,
	environment: BTreeMap<String, String>,
	policy: String,
}
fn source(ctx: &Context, name: &str) -> Result<PathBuf> {
	let path = Path::new(name);
	ensure!(
		!path.is_absolute() && path.components().all(|c| matches!(c, Component::Normal(_))),
		"invalid AWFY input path"
	);
	let path = ctx.repo.join(path);
	ensure!(path.is_file(), "missing AWFY input: {}", path.display());
	Ok(path)
}
fn manifest(ctx: &Context) -> Result<Manifest> {
	let m: Manifest = serde_json::from_str(MANIFEST)?;
	ensure!(
		m.schema == 1 && m.cases.len() == 36,
		"invalid AWFY manifest"
	);
	let mut names = BTreeSet::new();
	let mut families = BTreeMap::new();
	for c in &m.cases {
		ensure!(
			!c.name.is_empty()
				&& c.name
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b == b'-')
				&& names.insert(&c.name),
			"invalid or duplicate AWFY case"
		);
		ensure!(
			matches!(
				c.family.as_str(),
				"candidate" | "historical-inactive-control" | "active-limit-control"
			),
			"invalid AWFY family"
		);
		ensure!(
			matches!(c.policy.as_str(), "match" | "refuse-method" | "frame-limit"),
			"invalid AWFY policy"
		);
		ensure!(
			c.num > 0 && c.inner > 0 && !c.inputs.is_empty(),
			"invalid AWFY input"
		);
		for input in &c.inputs {
			source(ctx, input)?;
		}
		*families.entry(c.family.as_str()).or_insert(0usize) += 1;
	}
	ensure!(
		families.get("candidate") == Some(&24)
			&& families.get("historical-inactive-control") == Some(&6)
			&& families.get("active-limit-control") == Some(&6),
		"AWFY case inventory changed"
	);
	Ok(m)
}
pub fn check_manifest(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	Ok(json!({"cases":m.cases.len(),"manifest_sha256":common::hash_bytes(MANIFEST.as_bytes())}))
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

fn observe(ctx: &Context, binary: &Path, c: &Case, engine: &str) -> Result<Value> {
	let label = format!("{}-{engine}", c.name);
	let config = ctx.out.join(format!("{label}-config.json"));
	let output = ctx.out.join(format!("{label}-wire.json"));
	let stdout = ctx.out.join(format!("{label}-wire.stdout"));
	let inputs = c
		.inputs
		.iter()
		.map(|i| Ok(json!({"path":source(ctx,i)?,"origin":i})))
		.collect::<Result<Vec<_>>>()?;
	let env = if engine == "stock" {
		BTreeMap::new()
	} else {
		c.environment.clone()
	};
	let control = if engine == "stock" || c.family == "candidate" {
		"ordinary"
	} else if c.family == "active-limit-control" {
		"active"
	} else {
		HISTORICAL
	};
	common::write_json(
		&config,
		&json!({"engine":engine,"num":c.num,"inner":c.inner,"inputs":inputs,"stdout":stdout,"control_mode":control,"control_environment":env}),
	)?;
	let mut child = Command::new(binary);
	child
		.args([
			SELECTOR,
			"--ignored",
			"--exact",
			"--nocapture",
			"--test-threads=1",
		])
		.env("RUST_MIN_STACK", OBSERVER_STACK_BYTES.to_string())
		.env("RUNE_VERIFY_AWFY_CONFIG", &config)
		.env("RUNE_VERIFY_AWFY_OUTPUT", &output)
		.envs(&env);
	let run = ctx.checked(&label, &mut child, 600)?;
	ensure!(
		fs::read(&run.stderr)?.is_empty(),
		"unexpected observer process stderr: {label}"
	);
	common::require_one_test(&run.stdout)?;
	let wire: Value = common::read_json(&output)?;
	let text = wire["stdout"].as_str().context("missing wire stdout")?;
	ensure!(
		fs::read(&stdout)? == text.as_bytes(),
		"wire capture mismatch: {label}"
	);
	ensure!(
		wire["code"].is_i64() && wire["stderr"].is_string(),
		"invalid wire result: {label}"
	);
	Ok(wire)
}

pub fn run(ctx: &Context) -> Result<Value> {
	let m = manifest(ctx)?;
	let binary = ctx.build_rune("awfy-native", FEATURES)?;
	let mut list = Command::new(&binary);
	list.args([SELECTOR, "--ignored", "--exact", "--list"]);
	let selected = ctx.checked("select", &mut list, 30)?;
	common::require_one_selection(&selected.stdout, SELECTOR)?;
	let negatives = negative_controls(ctx, &binary, SELECTOR)?;
	let mut baselines = BTreeMap::<i64, Value>::new();
	let mut counts = BTreeMap::<String, usize>::new();
	for c in &m.cases {
		let fresh = observe(ctx, &binary, c, "fresh")?;
		if c.family == "candidate" {
			let stock = observe(ctx, &binary, c, "stock")?;
			ensure!(
				stock["code"] == 0 && stock["stderr"] == "",
				"stock AWFY failed: {}",
				c.name
			);
			if c.policy == "refuse-method" {
				ensure!(
					fresh["code"] == 3
						&& fresh["stdout"].as_str().is_some_and(|s| s.starts_with(
							"REFUSED kind=Method operation=Method error=Unsupported site="
						)),
					"unexpected refusal: {}",
					c.name
				);
			} else {
				ensure!(
					fresh["code"] == stock["code"] && fresh["stdout"] == stock["stdout"],
					"live AWFY mismatch: {}",
					c.name
				);
				ensure!(
					fresh["stderr"].as_str().is_some_and(|s| s
						.strip_prefix("fresh live_entries=")
						.and_then(|n| n.trim_end().parse::<usize>().ok())
						.is_some()),
					"invalid fresh resource report: {}",
					c.name
				);
			}
			if c.name.starts_with("havlak-") {
				baselines.insert(c.num, fresh.clone());
			}
		} else if c.policy == "frame-limit" {
			let error = fresh["stderr"].as_str().context("frame limit stderr")?;
			ensure!(
				fresh["code"] == 7
					&& error.contains("kind: FrameCapacity")
					&& error.contains("idle=true cleanup_error=false"),
				"frame limit control failed: {}",
				c.name
			);
		} else {
			let baseline = baselines
				.get(&c.num)
				.context("missing live Havlak baseline")?;
			ensure!(
				fresh == *baseline,
				"limit control differs from live baseline: {}",
				c.name
			);
		}
		*counts.entry(c.policy.clone()).or_default() += 1;
	}
	ctx.verify_source()?;
	Ok(
		json!({"status":"PASS","cases":m.cases.len(),"policies":counts,"negative_controls":negatives}),
	)
}
