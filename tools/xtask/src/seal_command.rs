//! Create and recheck fresh measurement evidence seals.
use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

struct Options {
	operation: String,
	spec: PathBuf,
	out: PathBuf,
	seal: Option<PathBuf>,
}

fn usage() {
	println!("cargo xtask seal create --spec PATH --out NEW\ncargo xtask seal check --spec PATH --seal PATH --out NEW\nSeals reanalyze fresh captures; remote receipts are not supported.");
}

fn parse(args: &[String]) -> Result<Option<Options>> {
	if args == ["--help"] || args == ["-h"] {
		usage();
		return Ok(None);
	}
	let operation = args.first().context("seal requires create or check")?;
	ensure!(
		matches!(operation.as_str(), "create" | "check"),
		"invalid seal operation"
	);
	let mut values = BTreeMap::new();
	let mut i = 1;
	while i < args.len() {
		let key = &args[i];
		ensure!(
			matches!(key.as_str(), "--spec" | "--out" | "--seal"),
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
	let seal = values.get("--seal").map(PathBuf::from);
	ensure!(
		(operation == "check") == seal.is_some(),
		"--seal is required only for check"
	);
	Ok(Some(Options {
		operation: operation.to_owned(),
		spec: required("--spec")?.into(),
		out: required("--out")?.into(),
		seal,
	}))
}

pub fn run(args: &[String]) -> Result<()> {
	let Some(options) = parse(args)? else {
		return Ok(());
	};
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("measure/seal_store.py");
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
	if let Some(seal) = options.seal {
		cmd.arg("--seal").arg(seal);
	}
	let status = cmd.status().context("launch seal worker")?;
	if !status.success() {
		bail!("seal {} failed: {status}", options.operation);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn options_require_matching_operation_and_seal() {
		assert!(parse(&["--help".into()]).unwrap().is_none());
		let create = ["create", "--spec", "spec.json", "--out", "out"]
			.into_iter()
			.map(str::to_owned)
			.collect::<Vec<_>>();
		assert!(parse(&create).unwrap().is_some());
		let mut invalid = create.clone();
		invalid.extend(["--seal".into(), "seal.json".into()]);
		assert!(parse(&invalid).is_err());
		let mut check = create;
		check[0] = "check".into();
		assert!(parse(&check).is_err());
		check.extend(["--seal".into(), "seal.json".into()]);
		assert!(parse(&check).unwrap().is_some());
	}
}
