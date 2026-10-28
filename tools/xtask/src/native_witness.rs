//! Compare diagnostic witnesses from live stock and fresh executions.
use crate::common::{read_json, Context};
use anyhow::{ensure, Context as _, Result};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

pub fn run(ctx: &Context, binary: &Path) -> Result<Value> {
	let cases = [
		("callback_sort", 44),
		("callback_sort", 89),
		("callback_sort", 98),
		("tuple_let", 55),
		("tuple_let", 56),
		("deferred_index", 151),
		("deferred_index", 154),
		("callback_sort", 40),
		("callback_sort", 41),
		("callback_sort", 42),
		("callback_sort", 75),
		("callback_sort", 76),
		("callback_sort", 77),
		("callback_sort", 78),
		("callback_sort", 79),
		("callback_sort", 80),
		("callback_sort", 81),
		("callback_sort", 82),
		("callback_sort", 83),
		("callback_sort", 85),
		("callback_sort", 96),
		("callback_sort", 97),
		("callback_sort", 90),
		("format_specs", 254),
	];
	let mut results = vec![];
	for (number, (family, index)) in cases.into_iter().enumerate() {
		let prefix = match family {
			"callback_sort" => "C11",
			"tuple_let" => "C8",
			"deferred_index" => "C7",
			"format_specs" => "C5",
			_ => unreachable!(),
		};
		let mut source_pair = vec![];
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
			// observation/trace comparisons establish the live acceptance.
			exiting_child_started(&fs::read_to_string(&r.stdout)?, &selector)?;
			let obs: Value = read_json(&observation)?;
			let actual: Value = read_json(&trace)?;
			let source = decode_hex(obs["source_hex"].as_str().context("source hex absent")?)?;
			ensure!(
				!source.is_empty() && obs["index"] == index,
				"witness input differs"
			);
			let frames = actual["frames"]
				.as_array()
				.context("witness frames absent")?;
			ensure!(
				!frames.is_empty() && actual["rendered"].as_str().is_some_and(|s| !s.is_empty()),
				"empty witness diagnostic"
			);
			for (i, frame) in frames.iter().enumerate() {
				let expected_role = if i == 0 { "Origin" } else { "Caller" };
				let location = &frame["location"];
				let start = location["start"].as_u64().context("witness start absent")? as usize;
				let end = location["end"].as_u64().context("witness end absent")? as usize;
				ensure!(
					frame["role"] == expected_role
						&& location["source"] == 0
						&& source.get(start..end).is_some(),
					"invalid witness frame"
				);
			}
			source_pair.push(source);
			traces.push(actual);
		}
		ensure!(
			source_pair[0] == source_pair[1],
			"paired witness source differs"
		);
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
		let text = "\nrunning 1 test\ntest experimental_core::tests::gap_hunt::tuple_let::child ... example\n";
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
