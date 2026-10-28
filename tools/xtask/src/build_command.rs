//! Host-configured entry point for the isolated commit-based build worker.
use crate::hosts;
use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeSet, path::PathBuf, process::Command};

struct Options {
	repo: PathBuf,
	revision: String,
	profile: String,
	role: String,
	native: bool,
	out: PathBuf,
	hosts: Option<PathBuf>,
	toolchain: PathBuf,
	cargo_cache: PathBuf,
	jobs: Option<usize>,
	timeout: u64,
}

fn usage() {
	println!("cargo xtask build --rev REV --profile cgu1|cgu16 --role candidate|tip|base|milestone --out NEW --toolchain-root PATH --cargo-cache PATH [--native] [--hosts PATH] [--repo PATH] [--jobs N] [--timeout SECONDS]");
}

fn parse(args: &[String]) -> Result<Option<Options>> {
	if args == ["--help"] || args == ["-h"] {
		usage();
		return Ok(None);
	}
	let mut values = std::collections::BTreeMap::new();
	let mut native = false;
	let mut i = 0;
	while i < args.len() {
		let key = &args[i];
		if key == "--native" {
			ensure!(!native, "duplicate --native");
			native = true;
			i += 1;
			continue;
		}
		ensure!(
			matches!(
				key.as_str(),
				"--repo"
					| "--rev" | "--profile"
					| "--role" | "--out"
					| "--hosts" | "--toolchain-root"
					| "--cargo-cache"
					| "--jobs" | "--timeout"
			),
			"unknown build option {key}"
		);
		i += 1;
		let value = args
			.get(i)
			.with_context(|| format!("missing value for {key}"))?;
		ensure!(
			!value.is_empty() && !value.starts_with("--"),
			"invalid value for {key}"
		);
		ensure!(
			values.insert(key.as_str(), value.as_str()).is_none(),
			"duplicate {key}"
		);
		i += 1;
	}
	let required = |name| {
		values
			.get(name)
			.copied()
			.with_context(|| format!("missing {name}"))
	};
	let profile = required("--profile")?;
	ensure!(matches!(profile, "cgu1" | "cgu16"), "invalid build profile");
	let role = required("--role")?;
	ensure!(
		matches!(role, "candidate" | "tip" | "base" | "milestone"),
		"invalid source role"
	);
	let jobs = values
		.get("--jobs")
		.map(|value| value.parse::<usize>())
		.transpose()?;
	ensure!(jobs != Some(0), "jobs must be positive");
	let timeout = values
		.get("--timeout")
		.map_or(Ok(1500), |value| value.parse::<u64>())?;
	ensure!((1..=7200).contains(&timeout), "invalid build timeout");
	Ok(Some(Options {
		repo: values.get("--repo").map_or_else(
			|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
			PathBuf::from,
		),
		revision: required("--rev")?.to_owned(),
		profile: profile.to_owned(),
		role: role.to_owned(),
		native,
		out: required("--out")?.into(),
		hosts: values.get("--hosts").map(PathBuf::from),
		toolchain: required("--toolchain-root")?.into(),
		cargo_cache: required("--cargo-cache")?.into(),
		jobs,
		timeout,
	}))
}

pub fn run(args: &[String]) -> Result<()> {
	let Some(options) = parse(args)? else {
		return Ok(());
	};
	let loaded = hosts::load(options.hosts.as_deref())?
		.context("measurement host configuration is required for builds")?;
	let host = loaded
		.config
		.measurement
		.as_ref()
		.context("measurement host is not configured")?;
	ensure!(
		host.endpoint.is_none(),
		"remote measurement builds are not yet supported by this local worker"
	);
	let measurement_cpu = host
		.measurement_cpu
		.context("measurement CPU must be declared")?;
	let cpus = host.cpus.clone().unwrap_or_else(|| vec![measurement_cpu]);
	ensure!(
		cpus.contains(&measurement_cpu)
			&& cpus.iter().copied().collect::<BTreeSet<_>>().len() == cpus.len(),
		"invalid measurement CPU allocation"
	);
	let lock = host
		.measurement_lock
		.as_ref()
		.context("measurement lock must be declared")?;
	let rustc = host
		.rustc_identity
		.as_deref()
		.context("compiler identity must be declared")?;
	let linker = host
		.linker_identity
		.as_deref()
		.context("linker identity must be declared")?;
	let jobs = options.jobs.unwrap_or(cpus.len());
	ensure!(
		jobs <= cpus.len(),
		"jobs exceed the declared CPU allocation"
	);
	let config_path = loaded.receipt["path"]
		.as_str()
		.context("host receipt path missing")?;
	let config_hash = loaded.receipt["sha256"]
		.as_str()
		.context("host receipt hash missing")?;
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("measure/build_worker.py");
	let mut cmd = Command::new("/usr/bin/python3");
	cmd.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("LANG", "C.UTF-8")
		.env("TZ", "UTC");
	if let Some(home) = std::env::var_os("HOME") {
		cmd.env("HOME", home);
	}
	cmd.args(["-I", "-B", "-S"])
		.arg(script)
		.arg("--repo")
		.arg(&options.repo)
		.arg("--revision")
		.arg(&options.revision)
		.arg("--profile")
		.arg(&options.profile)
		.arg("--role")
		.arg(&options.role)
		.arg("--out")
		.arg(&options.out)
		.arg("--toolchain")
		.arg(&options.toolchain)
		.arg("--cargo-cache")
		.arg(&options.cargo_cache)
		.arg("--lock")
		.arg(lock)
		.arg("--jobs")
		.arg(jobs.to_string())
		.arg("--timeout")
		.arg(options.timeout.to_string())
		.arg("--rustc-identity")
		.arg(rustc)
		.arg("--linker-identity")
		.arg(linker)
		.arg("--host-config")
		.arg(config_path)
		.arg("--host-sha256")
		.arg(config_hash);
	for cpu in cpus {
		cmd.arg("--cpu").arg(cpu.to_string());
	}
	if options.native {
		cmd.arg("--native");
	}
	if let Some(memory) = host.memory_bytes {
		cmd.arg("--memory-bytes").arg(memory.to_string());
	}
	let status = cmd.status().context("launch build worker")?;
	if !status.success() {
		bail!("measurement build failed: {status}");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn build_options_fail_closed() {
		assert!(parse(&["--help".into()]).unwrap().is_none());
		assert!(parse(&[]).is_err());
		let args = [
			"--rev",
			"HEAD",
			"--profile",
			"cgu1",
			"--role",
			"candidate",
			"--out",
			"out",
			"--toolchain-root",
			"tools",
			"--cargo-cache",
			"cache",
		]
		.into_iter()
		.map(str::to_owned)
		.collect::<Vec<_>>();
		assert!(parse(&args).unwrap().is_some());
		let mut bad = args.clone();
		bad.extend(["--jobs".into(), "0".into()]);
		assert!(parse(&bad).is_err());
		let mut duplicate = args;
		duplicate.extend(["--role".into(), "tip".into()]);
		assert!(parse(&duplicate).is_err());
	}
}
