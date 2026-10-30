//! Versioned correctness commands with checkout-owned inputs.
mod awfy;
mod build_command;
mod common;
#[path = "../../../crates/rune/src/experimental_core/d1_message.rs"]
mod d1_message;
mod hosts;
mod measure_checks;
mod measure_command;
mod miri;
mod native;
mod preservation;
mod record_command;
mod scheduler;
mod seal_command;
mod tasks;

use anyhow::{bail, ensure, Context as _};
use common::{write_json, Context, Result};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf, time::Instant};

const SUITES: &[&str] = &[
	"native",
	"contracts",
	"l1",
	"awfy",
	"clippy",
	"layouts",
	"measure-checks",
	"miri",
];
fn usage() {
	println!(
		"cargo xtask verify [--suite SUITE[,SUITE]] [--out NEW] [--fixtures-root PATH]\n\
	  suites: {}\n\
	  options: --repo PATH --toolchain VERSION --jobs N --test-threads N --workers N\n\
			   --check-manifests --cold --hosts PATH\n\
	  cargo xtask list\n\
	  cargo xtask build --help\n\
	  cargo xtask measure --help\n\
	  cargo xtask seal --help\n\
	  cargo xtask record --help\n\
	  Output must be new; default scratch is under target/xtask-verify. Native and Miri work share one local CPU budget.",
		SUITES.join(", ")
	);
}
struct Options {
	hosts: Option<PathBuf>,
	repo: PathBuf,
	fixtures: PathBuf,
	out: PathBuf,
	toolchain: String,
	jobs: usize,
	test_threads: usize,
	workers: usize,
	suites: Vec<String>,
	check: bool,
	cold: bool,
}
fn parse(args: &[String]) -> Result<Option<Options>> {
	if args == ["--help"] || args == ["-h"] {
		usage();
		return Ok(None);
	}
	if args == ["list"] {
		for suite in SUITES {
			println!("{suite}");
		}
		return Ok(None);
	}
	ensure!(
		args.first().is_some_and(|arg| arg == "verify"),
		"unknown command {}; expected verify or list",
		args.first().map_or("<empty>", String::as_str)
	);
	let mut repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
	let mut hosts = None;
	let mut fixtures = None;
	let mut out = None;
	let mut toolchain = "1.98.1".to_owned();
	let default_parallelism = hosts::available_cpus()?;
	let mut jobs = default_parallelism;
	let mut test_threads = default_parallelism;
	let mut workers = default_parallelism;
	let mut suites = Vec::new();
	let mut check = false;
	let mut cold = false;
	let mut i = 1;
	while i < args.len() {
		let flag = &args[i];
		if flag == "--cold" {
			cold = true;
			i += 1;
			continue;
		}
		if flag == "--check-manifests" {
			check = true;
			i += 1;
			continue;
		}
		if flag == "--help" {
			usage();
			return Ok(None);
		}
		i += 1;
		let value = args
			.get(i)
			.with_context(|| format!("missing value for {flag}"))?;
		match flag.as_str() {
			"--repo" => repo = value.into(),
			"--hosts" => hosts = Some(PathBuf::from(value)),
			"--fixtures-root" => fixtures = Some(PathBuf::from(value)),
			"--out" => out = Some(PathBuf::from(value)),
			"--toolchain" => toolchain = value.clone(),
			"--jobs" => jobs = value.parse()?,
			"--test-threads" => test_threads = value.parse()?,
			"--workers" => workers = value.parse()?,
			"--suite" => suites.extend(value.split(',').map(str::to_owned)),
			_ => bail!("unknown option {flag}"),
		}
		i += 1;
	}
	ensure!(
		jobs > 0 && test_threads > 0 && workers > 0,
		"worker counts must be positive"
	);
	if suites.is_empty() {
		suites.push("all".to_owned());
	}
	if suites == ["all"] {
		suites = SUITES.iter().map(|s| (*s).to_owned()).collect();
	}
	let mut unique = BTreeSet::new();
	for suite in &suites {
		ensure!(SUITES.contains(&suite.as_str()), "unknown suite {suite}");
		ensure!(unique.insert(suite), "duplicate suite {suite}");
	}
	let external = suites
		.iter()
		.any(|suite| matches!(suite.as_str(), "contracts" | "l1"));
	let fixtures = match fixtures {
		Some(root) => root,
		None if !external => repo.clone(),
		None => repo.join("tools/xtask/corpus"),
	};
	let out = match out {
		Some(path) => path,
		None => {
			let timestamp = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)?
				.as_nanos();
			repo.join("target/xtask-verify")
				.join(format!("{}-{timestamp}", std::process::id()))
		}
	};
	Ok(Some(Options {
		hosts,
		repo,
		fixtures,
		out,
		toolchain,
		jobs,
		test_threads,
		workers,
		suites,
		check,
		cold,
	}))
}
fn external_requirements(ctx: &Context, suite: &str) -> Value {
	// The historical inventory describes the earlier archive, not this checkout.
	// Current contracts and L1 inputs are checked in. An explicitly supplied
	// alternative root is external, but its size is not known until inspected.
	let uses_corpus = matches!(suite, "contracts" | "l1");
	let bundled = ctx.fixtures == ctx.repo.join("tools/xtask/corpus");
	if uses_corpus && !bundled {
		json!({"source":"provided", "external_files":null, "external_bytes":null})
	} else {
		json!({"source":if uses_corpus {"bundled"} else {"none"}, "external_files":0, "external_bytes":0})
	}
}
fn validate_budget(workers: usize, jobs: usize, test_threads: usize, cpus: usize) -> Result<()> {
	ensure!(
		cpus > 0 && workers > 0 && jobs > 0 && test_threads > 0,
		"worker capacities must be positive"
	);
	ensure!(
		workers <= cpus && jobs <= cpus && test_threads <= cpus,
		"requested capacity exceeds {cpus} available CPUs"
	);
	Ok(())
}

fn execute(o: Options) -> Result<()> {
	let started = Instant::now();
	let hosts = hosts::load(o.hosts.as_deref())?;
	let cpu_budget = hosts::available_cpus()?;
	validate_budget(o.workers, o.jobs, o.test_threads, cpu_budget)?;
	let ctx = Context::new(
		o.repo,
		o.fixtures,
		o.out,
		o.toolchain,
		o.jobs,
		o.test_threads,
	)?;
	write_json(
		&ctx.root_out.join("CONFIG.json"),
		&json!({"schema":1,"suites":o.suites,
		"jobs":o.jobs,"test_threads":o.test_threads,"workers":o.workers,"toolchain":ctx.toolchain,
		"fixtures":ctx.fixtures,"repo":ctx.repo,"cpu_budget":cpu_budget,"check_manifests":o.check,"cold":o.cold,
		"hosts":hosts.as_ref().map(|loaded| &loaded.receipt)}),
	)?;

	if o.check {
		let checked = (|| -> Result<Value> {
			let mut checks = serde_json::Map::new();
			for suite in &o.suites {
				let value = match suite.as_str() {
					"native" | "contracts" | "clippy" | "layouts" => native::check_manifest(&ctx)?,
					"l1" => preservation::check_l1_manifest(&ctx)?,
					"miri" => miri::check_manifest(&ctx)?,
					"awfy" => awfy::check_manifest(&ctx)?,
					"measure-checks" => measure_checks::check_manifest(&ctx)?,
					_ => unreachable!(),
				};
				checks.insert(
					suite.clone(),
					json!({"manifest":value,"external_fixtures":external_requirements(&ctx, suite)}),
				);
			}
			ctx.verify_source()?;
			Ok(Value::Object(checks))
		})();
		write_json(
			&ctx.root_out.join("RESULT.json"),
			&json!({"schema":1,
			"status":if checked.is_ok() {"PASS"} else {"FAIL"},
			"manifests":checked.as_ref().ok(),"error":checked.as_ref().err().map(|e|format!("{e:#}")),
			"seconds":started.elapsed().as_secs_f64()}),
		)?;
		checked?;
		return Ok(());
	}

	if o.suites.iter().any(|suite| suite == "miri") {
		miri::preflight(&ctx)?;
	}
	let graph = tasks::run_all(&ctx, &o.suites, o.workers);
	let mut results = Vec::new();
	let mut errors = Vec::new();
	match graph {
		Ok(rows) => {
			for row in rows {
				if row["status"] != "PASS" {
					errors.push(format!("{}: {}", row["suite"], row["error"]));
				}
				results.push(row);
			}
		}
		Err(error) => errors.push(format!("job graph: {error:#}")),
	}
	if let Err(error) = ctx.verify_source() {
		errors.push(format!("source: {error:#}"));
	}
	write_json(&ctx.root_out.join("SCHEDULER.json"), &ctx.budget.events()?)?;
	let native = ctx.native_intervals()?;
	let interpreters = results
		.iter()
		.find(|row| row["suite"] == "miri")
		.and_then(|row| row["result"]["overlap"]["intervals"].as_array())
		.cloned()
		.unwrap_or_default();
	let overlap = native.iter().any(|a| {
		interpreters.iter().any(|b| {
			a["start_ns"].as_u64() < b["end_ns"].as_u64()
				&& b["start_ns"].as_u64() < a["end_ns"].as_u64()
		})
	});
	write_json(
		&ctx.root_out.join("CORRECTNESS-EVENTS.json"),
		&json!({"native":native,"interpreters":interpreters,"native_miri_overlap":overlap,"cpu_budget":ctx.budget.capacity()}),
	)?;
	if errors.is_empty()
		&& ctx.budget.capacity() > 1
		&& o.suites.iter().any(|suite| suite == "miri")
		&& o.suites
			.iter()
			.any(|suite| suite == "native" || suite == "awfy")
		&& !overlap
	{
		errors.push("actual native/Miri overlap was not exercised".into());
	}
	write_json(
		&ctx.root_out.join("RESULT.json"),
		&json!({"schema":1,"status":if errors.is_empty(){"PASS"}else{"FAIL"},"seconds":started.elapsed().as_secs_f64(),"results":results,"errors":errors}),
	)?;
	ensure!(
		errors.is_empty(),
		"verification failed: {}",
		errors.join("; ")
	);
	println!("verification PASS: {}", ctx.root_out.display());
	Ok(())
}

fn main() {
	let mut args: Vec<_> = std::env::args().skip(1).collect();
	if args.is_empty() {
		args.push("verify".to_owned());
	}
	let result = if args.first().is_some_and(|arg| arg == "build") {
		build_command::run(&args[1..])
	} else if args.first().is_some_and(|arg| arg == "measure") {
		measure_command::run(&args[1..])
	} else if args.first().is_some_and(|arg| arg == "record") {
		record_command::run(&args[1..])
	} else if args.first().is_some_and(|arg| arg == "seal") {
		seal_command::run(&args[1..])
	} else {
		parse(&args).and_then(|options| match options {
			Some(o) => execute(o),
			None => Ok(()),
		})
	};
	if let Err(error) = result {
		eprintln!("xtask: {error:#}");
		std::process::exit(1);
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	fn options(tail: &[&str]) -> Vec<String> {
		["verify", "--fixtures-root", "/tmp", "--out", "/tmp/verify"]
			.into_iter()
			.chain(tail.iter().copied())
			.map(str::to_owned)
			.collect()
	}
	#[test]
	fn default_verification_uses_checkout_inputs_and_scratch() {
		let args = ["verify"].map(str::to_owned);
		let options = parse(&args).unwrap().unwrap();
		assert_eq!(options.suites.len(), SUITES.len());
		assert_eq!(options.fixtures, options.repo.join("tools/xtask/corpus"));
		assert!(options
			.out
			.starts_with(options.repo.join("target/xtask-verify")));
	}

	#[test]
	fn worker_capacities_share_the_same_cpu_budget() {
		assert!(validate_budget(3, 8, 4, 30).is_ok());
		assert!(validate_budget(4, 8, 4, 30).is_ok());
		assert!(validate_budget(30, 30, 30, 30).is_ok());
		assert!(validate_budget(31, 30, 30, 30).is_err());
		assert!(validate_budget(usize::MAX, 2, 1, 30).is_err());
	}

	#[test]
	fn unknown_duplicate_and_unbounded_selections_fail() {
		assert!(parse(&options(&["--suite", "bogus"])).is_err());
		assert!(parse(&options(&["--suite", "native,native"])).is_err());
		assert!(parse(&options(&["--suite", "native", "--workers", "0"])).is_err());
		assert!(parse(&options(&["--suite", "all,native"])).is_err());
	}
}
