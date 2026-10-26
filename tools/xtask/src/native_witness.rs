//! The 24 frozen diagnostic witnesses, compared structurally without normalizing output.
use crate::common::{read_json, Context};
use anyhow::{ensure, Context as _, Result};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

pub fn run(ctx: &Context, binary: &Path) -> Result<Value> {
	let table: Value = read_json(&ctx.repo.join("tools/xtask/fixtures/native/TABLE.json"))?;
	let cases = table["rows"].as_array().context("witness rows absent")?;
	ensure!(cases.len() == 24, "expected 24 diagnostic witnesses");
	let mut results = vec![];
	for (number, row) in cases.iter().enumerate() {
		let family = row["family"].as_str().context("witness family absent")?;
		let prefix = match family {
			"callback_sort" => "C11",
			"tuple_let" => "C8",
			"deferred_index" => "C7",
			"format_specs" => "C5",
			_ => anyhow::bail!("unknown witness family {family}"),
		};
		let before = Path::new(
			row["before_provenance"]["fresh"]["path"]
				.as_str()
				.context("witness provenance absent")?,
		);
		let index = before
			.parent()
			.and_then(Path::file_name)
			.and_then(|n| n.to_str())
			.context("witness index absent")?
			.parse::<usize>()?;
		let mut traces = vec![];
		for engine in ["stock", "fresh"] {
			let label = format!("witness-{number}-{engine}");
			let dir = ctx.out.join(&label);
			fs::create_dir(&dir)?;
			let observation = dir.join("observation.json");
			let trace = dir.join("logical.json");
			let selector = format!("experimental_core::tests::gap_hunt::{family}::child");
			super::selected(ctx, binary, &label, &selector, true, true)?;
			let mut cmd = Command::new("/usr/bin/prlimit");
			cmd.arg("--core=0")
				.arg("--")
				.arg(binary)
				.current_dir(&ctx.repo)
				.args([
					"--exact",
					&selector,
					"--ignored",
					"--nocapture",
					"--test-threads=1",
				])
				.env(format!("{prefix}_CASE"), index.to_string())
				.env(format!("{prefix}_ENGINE"), engine)
				.env(format!("{prefix}_RESULT"), &observation)
				.env("D1E_TRACE_OUT", &trace);
			let r = ctx.checked(&label, &mut cmd, 30)?;
			// These four pinned child entry points deliberately process::exit(0)
			// after writing their observation, bypassing libtest's summary.
			// Exact selection, the started test, rc0 and the complete fresh
			// observation/trace comparisons establish the frozen acceptance.
			exiting_child_started(&fs::read_to_string(&r.stdout)?, &selector)?;
			let obs: Value = read_json(&observation)?;
			let actual: Value = read_json(&trace)?;
			let source = decode_hex(obs["source_hex"].as_str().context("source hex absent")?)?;
			ensure!(
				source
					== row["source"]
						.as_str()
						.context("expected source absent")?
						.as_bytes(),
				"witness source differs"
			);
			let mut frames = actual["frames"].clone();
			for frame in frames.as_array_mut().context("trace frames absent")? {
				frame["role"] = Value::String(
					frame["role"]
						.as_str()
						.context("role absent")?
						.to_ascii_lowercase(),
				);
			}
			ensure!(
				frames == row["proposed_after"][format!("{engine}_logical_frames")],
				"witness frame mismatch {family}/{index}/{engine}"
			);
			let rendered = actual["rendered"].as_str().context("rendered absent")?;
			ensure!(
				rendered.lines().next()
					== row["proposed_after"]["primary_message"][engine].as_str(),
				"witness primary message differs"
			);
			traces.push(actual);
		}
		ensure!(
			traces[0]["frames"] == traces[1]["frames"]
				&& traces[0]["rendered"] == traces[1]["rendered"],
			"paired witness differs"
		);
		if family == "format_specs" {
			let expected = json!([{"tag":"BadArgument","arg":0}]);
			ensure!(
				traces.iter().all(|t| t["context"] == expected),
				"format witness context differs"
			);
		}
		results.push(json!({"family":family,"index":index,"traces":traces}));
	}
	Ok(json!({"status":"PASS","witnesses":24,"paired_observations":48,"rows":results}))
}
fn decode_hex(s: &str) -> Result<Vec<u8>> {
	ensure!(s.len().is_multiple_of(2), "odd source hex length");
	s.as_bytes()
		.as_chunks::<2>()
		.0
		.iter()
		.map(|b| Ok(u8::from_str_radix(std::str::from_utf8(b)?, 16)?))
		.collect()
}

fn exiting_child_started(text: &str, selector: &str) -> Result<()> {
	let starts: Vec<_> = text
		.lines()
		.filter(|line| line.starts_with("test "))
		.collect();
	ensure!(
		starts.len() == 1 && starts[0].starts_with(&format!("test {selector} ... ")),
		"expected one exact process-exit witness test start"
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn process_exit_body_may_follow_test_start_on_same_line() {
		let text = include_str!("../fixtures/native/tuple-let-process-exit.stdout");
		assert!(exiting_child_started(
			text,
			"experimental_core::tests::gap_hunt::tuple_let::child"
		)
		.is_ok());
		assert!(exiting_child_started(
			text,
			"experimental_core::tests::gap_hunt::tuple_let::child_suffix"
		)
		.is_err());
	}

	#[test]
	fn process_exit_witness_does_not_require_libtest_summary() {
		let text = "\nrunning 1 test\ntest wanted ... ";
		assert!(exiting_child_started(text, "wanted").is_ok());
		assert!(super::super::counts(text).is_err());
		for bad in [
			"",
			"test other ... ",
			"test wanted_suffix ... ",
			"test wanted ... \ntest wanted ... ",
		] {
			assert!(exiting_child_started(bad, "wanted").is_err());
		}
	}
}
