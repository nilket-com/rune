//! Local measurement entry point; remote dispatch is not supported.
use crate::hosts;
use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

struct Options {
	repo: PathBuf,
	protocol: String,
	spec: PathBuf,
	out: PathBuf,
	hosts: Option<PathBuf>,
}

fn usage() {
	println!("cargo xtask measure --protocol counter2-v1|k1-v1|callgrind-v1 --spec PATH --out NEW [--hosts PATH] [--repo PATH]\nLocal captures can be sealed and rechecked; remote execution is not supported.");
}

fn parse(args: &[String]) -> Result<Option<Options>> {
	if args == ["--help"] || args == ["-h"] {
		usage();
		return Ok(None);
	}
	let mut values = BTreeMap::new();
	let mut i = 0;
	while i < args.len() {
		let key = &args[i];
		ensure!(
			matches!(
				key.as_str(),
				"--repo" | "--protocol" | "--spec" | "--out" | "--hosts"
			),
			"unknown measure option {key}"
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
	let protocol = required("--protocol")?;
	ensure!(
		matches!(protocol, "counter2-v1" | "callgrind-v1" | "k1-v1"),
		"unknown measurement protocol"
	);
	Ok(Some(Options {
		repo: values.get("--repo").map_or_else(
			|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
			PathBuf::from,
		),
		protocol: protocol.to_owned(),
		spec: required("--spec")?.into(),
		out: required("--out")?.into(),
		hosts: values.get("--hosts").map(PathBuf::from),
	}))
}

pub fn run(args: &[String]) -> Result<()> {
	let Some(options) = parse(args)? else {
		return Ok(());
	};
	let worker = match options.protocol.as_str() {
		"counter2-v1" => "counter_controller.py",
		"k1-v1" => "k1_controller.py",
		"callgrind-v1" => "callgrind_controller.py",
		_ => bail!(
			"{} measurement controller is not yet available",
			options.protocol
		),
	};
	let loaded = hosts::load(options.hosts.as_deref())?
		.context("measurement host configuration is required")?;
	let host = loaded
		.config
		.measurement
		.as_ref()
		.context("measurement host is not configured")?;
	ensure!(
		host.endpoint.is_none(),
		"remote measurement execution is not yet supported"
	);
	host.measurement_cpu
		.context("measurement CPU must be declared")?;
	host.measurement_lock
		.as_ref()
		.context("measurement lock must be declared")?;
	host.measurement_max_load
		.context("measurement load limit must be declared")?;
	host.measurement_min_available_bytes
		.context("measurement available-memory limit must be declared")?;
	if options.protocol == "callgrind-v1" {
		host.valgrind_path
			.as_ref()
			.context("valgrind path must be declared")?;
		host.valgrind_root
			.as_ref()
			.context("valgrind installation root must be declared")?;
		host.valgrind_identity
			.as_ref()
			.context("valgrind identity must be declared")?;
	}
	let config_path = loaded.receipt["path"]
		.as_str()
		.context("host receipt path missing")?;
	let config_hash = loaded.receipt["sha256"]
		.as_str()
		.context("host receipt hash missing")?;
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("measure")
		.join(worker);
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
		.arg(options.repo)
		.arg("--spec")
		.arg(options.spec)
		.arg("--out")
		.arg(options.out)
		.arg("--hosts")
		.arg(config_path)
		.arg("--hosts-sha256")
		.arg(config_hash);
	let status = cmd.status().context("launch measurement worker")?;
	if !status.success() {
		bail!(
			"{} measurement failed or rejected: {status}",
			options.protocol
		);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn options_are_closed_and_require_values() {
		assert!(parse(&["--help".into()]).unwrap().is_none());
		assert!(parse(&[]).is_err());
		let args = [
			"--protocol",
			"counter2-v1",
			"--spec",
			"spec.json",
			"--out",
			"out",
		]
		.into_iter()
		.map(str::to_owned)
		.collect::<Vec<_>>();
		assert!(parse(&args).unwrap().is_some());
		let mut duplicate = args.clone();
		duplicate.extend(["--out".into(), "again".into()]);
		assert!(parse(&duplicate).is_err());
		let mut unknown = args;
		unknown.extend(["--other".into(), "value".into()]);
		assert!(parse(&unknown).is_err());
	}
}
