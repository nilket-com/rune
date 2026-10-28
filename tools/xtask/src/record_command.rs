//! Check and atomically update the measurement ledger.
use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

struct Options {
	operation: String,
	spec: PathBuf,
	out: PathBuf,
	hosts: PathBuf,
}

fn usage() {
	println!("cargo xtask record check|summary|append --spec PATH --hosts PATH --out NEW");
}

fn parse(args: &[String]) -> Result<Option<Options>> {
	if args == ["--help"] || args == ["-h"] {
		usage();
		return Ok(None);
	}
	let operation = args
		.first()
		.context("record requires check, summary or append")?;
	ensure!(
		matches!(operation.as_str(), "check" | "summary" | "append"),
		"invalid record operation"
	);
	let mut values = BTreeMap::new();
	let mut i = 1;
	while i < args.len() {
		let key = &args[i];
		ensure!(
			matches!(key.as_str(), "--spec" | "--out" | "--hosts"),
			"unknown seal option {key}"
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

	Ok(Some(Options {
		operation: operation.to_owned(),
		spec: required("--spec")?.into(),
		out: required("--out")?.into(),
		hosts: required("--hosts")?.into(),
	}))
}

pub fn run(args: &[String]) -> Result<()> {
	let Some(options) = parse(args)? else {
		return Ok(());
	};
	let loaded =
		crate::hosts::load(Some(&options.hosts))?.context("host configuration required")?;
	ensure!(
		loaded
			.config
			.measurement
			.as_ref()
			.and_then(|h| h.measurement_lock.as_ref())
			.is_some(),
		"measurement lock required"
	);
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("measure/record_store.py");
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
		.arg(&options.operation)
		.arg("--spec")
		.arg(options.spec)
		.arg("--out")
		.arg(options.out);
	cmd.arg("--hosts").arg(options.hosts);

	let status = cmd.status().context("launch record worker")?;
	if !status.success() {
		bail!("record {} failed: {status}", options.operation);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn options_require_host_configuration() {
		let args = [
			"check",
			"--spec",
			"spec.json",
			"--out",
			"out",
			"--hosts",
			"hosts.json",
		]
		.into_iter()
		.map(str::to_owned)
		.collect::<Vec<_>>();
		assert!(parse(&args).unwrap().is_some());
		assert!(parse(&args[..5]).is_err());
	}
}
