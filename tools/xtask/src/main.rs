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
mod seal_command;

use anyhow::{bail, ensure, Context as _};
use common::{write_json, Context, Result};
use serde_json::{json, Value};
use std::{
	collections::{BTreeSet, VecDeque},
	path::PathBuf,
	sync::Mutex,
	time::Instant,
};

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
               --check-manifests --hosts PATH\n\
      cargo xtask list\n\
      cargo xtask build --help\n\
      cargo xtask measure --help\n\
      cargo xtask seal --help\n\
      cargo xtask record --help\n\
      Output must be new; default scratch is under target/xtask-verify. Miri runs after native work.",
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
	let default_parallelism = hosts::available_cpus()?.min(4);
	let mut jobs = default_parallelism;
	let mut test_threads = default_parallelism;
	let mut workers = 1;
	let mut suites = Vec::new();
	let mut check = false;
	let mut i = 1;
	while i < args.len() {
		let flag = &args[i];
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
	}))
}
fn dispatch(ctx: &Context, suite: &str) -> Result<Value> {
	match suite {
		"native" | "contracts" | "clippy" | "layouts" => native::run(ctx, suite),
		"l1" => preservation::run(ctx),
		"miri" => miri::run(ctx),
		"awfy" => awfy::run(ctx),
		"measure-checks" => measure_checks::run(ctx),
		_ => bail!("unknown suite {suite}"),
	}
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
fn one(ctx: &Context, suite: &str) -> Result<Value> {
	let child = ctx.suite(suite)?;
	child.verify_source()?;
	let start = Instant::now();
	let result = dispatch(&child, suite);
	let source = child.verify_source();
	let ok = result.is_ok() && source.is_ok();
	let value = json!({"schema":1,"suite":suite,"status":if ok {"PASS"}else{"FAIL"},
        "seconds":start.elapsed().as_secs_f64(),"result":result.as_ref().ok(),
        "external_fixtures":external_requirements(&child, suite),
        "error":result.as_ref().err().map(|e|format!("{e:#}")),
        "source_error":source.as_ref().err().map(|e|format!("{e:#}"))});
	write_json(&child.out.join("RESULT.json"), &value)?;
	source?;
	result?;
	Ok(value)
}
fn validate_budget(workers: usize, jobs: usize, test_threads: usize, cpus: usize) -> Result<()> {
	let allocation = workers
		.checked_mul(jobs.max(test_threads))
		.context("worker allocation overflow")?;
	ensure!(
		allocation <= cpus,
		"worker allocation {allocation} exceeds {cpus} available CPUs"
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
        "fixtures":ctx.fixtures,"repo":ctx.repo,"cpu_budget":cpu_budget,"check_manifests":o.check,
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

	// V1c owns the cross-suite resource DAG. Here callers explicitly bound native
	// workers, and Miri remains a separate final phase with at most two interpreters.
	let queue = Mutex::new(VecDeque::from_iter(
		o.suites.iter().filter(|s| *s != "miri").cloned(),
	));
	let results = Mutex::new(Vec::new());
	let errors = Mutex::new(Vec::new());
	std::thread::scope(|scope| {
		for _ in 0..o.workers {
			scope.spawn(|| loop {
				if !errors.lock().unwrap().is_empty() {
					break;
				}
				let Some(suite) = queue.lock().unwrap().pop_front() else {
					break;
				};
				match one(&ctx, &suite) {
					Ok(value) => results.lock().unwrap().push(value),
					Err(error) => errors.lock().unwrap().push(format!("{suite}: {error:#}")),
				}
			});
		}
	});
	if errors.lock().unwrap().is_empty() && o.suites.iter().any(|s| s == "miri") {
		match one(&ctx, "miri") {
			Ok(value) => results.lock().unwrap().push(value),
			Err(error) => errors.lock().unwrap().push(format!("miri: {error:#}")),
		}
	}
	let errors = errors.into_inner().unwrap();
	let status = if errors.is_empty() { "PASS" } else { "FAIL" };
	write_json(
		&ctx.root_out.join("RESULT.json"),
		&json!({"schema":1,"status":status,
        "seconds":started.elapsed().as_secs_f64(),"results":results.into_inner().unwrap(),"errors":errors}),
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
	fn worker_allocation_respects_affinity_and_overflow() {
		assert!(validate_budget(3, 8, 4, 30).is_ok());
		assert!(validate_budget(4, 8, 4, 30).is_err());
		assert!(validate_budget(2, 1, 16, 30).is_err());
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
