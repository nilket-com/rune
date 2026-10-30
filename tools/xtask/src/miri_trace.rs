//! Actual interpreter lifetimes from kernel process events, never status polling.
use crate::common::{self, Context, Result};
use anyhow::{ensure, Context as _};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
	fs,
	path::{Path, PathBuf},
};
#[derive(Clone, Serialize)]
pub struct Interval {
	pid: u32,
	start_ns: u64,
	end_ns: u64,
	trace: PathBuf,
	sha256: String,
}
fn timestamp(line: &str) -> Result<u64> {
	let stamp = line
		.split_whitespace()
		.next()
		.context("missing event timestamp")?;
	let (seconds, fraction) = stamp.split_once('.').context("invalid event timestamp")?;
	ensure!(
		fraction.len() <= 9 && fraction.bytes().all(|b| b.is_ascii_digit()),
		"invalid timestamp fraction"
	);
	Ok(seconds
		.parse::<u64>()?
		.checked_mul(1_000_000_000)
		.context("timestamp overflow")?
		+ fraction.parse::<u64>()? * 10u64.pow((9 - fraction.len()) as u32))
}
pub fn intervals(dir: &Path, label: &str) -> Result<Vec<Interval>> {
	execution_intervals(dir, &format!("{label}.trace."), true)
}
pub fn native_intervals(dir: &Path, label: &str) -> Result<Vec<serde_json::Value>> {
	execution_intervals(dir, &format!("{label}.native.trace."), false)?
		.into_iter()
		.map(|i| Ok(serde_json::to_value(i)?))
		.collect()
}
fn execution_intervals(dir: &Path, prefix: &str, miri: bool) -> Result<Vec<Interval>> {
	let mut out = Vec::new();
	for entry in fs::read_dir(dir)? {
		let path = entry?.path();
		let name = path.file_name().context("trace name")?.to_string_lossy();
		let Some(pid) = name.strip_prefix(prefix) else {
			continue;
		};
		let pid = pid.parse()?;
		let text = fs::read_to_string(&path)?;
		let mut start = None;
		let mut end = None;
		for line in text.lines() {
			let executable = line
				.split_once("execve(\"")
				.and_then(|(_, rest)| rest.split_once('"').map(|(path, _)| path));
			let selected = executable.is_some_and(|path| {
				if miri {
					path.ends_with("/miri")
						&& line.contains("\"--test\"")
						&& line.contains("\"--crate-name\"")
				} else {
					path.ends_with("/rune-tests")
				}
			});
			if line.contains("execve(") && selected && line.ends_with("= 0") {
				ensure!(
					start.is_none(),
					"multiple interpreter execs in {}",
					path.display()
				);
				start = Some(timestamp(line)?);
			}
			if start.is_some()
				&& (line.contains("exit_group(")
					|| line.contains("+++ exited with")
					|| line.contains("+++ killed by"))
			{
				end = Some(timestamp(line)?);
			}
		}
		if let Some(start_ns) = start {
			let end_ns = end.context("interpreter trace lacks terminal event")?;
			ensure!(end_ns >= start_ns, "negative interpreter interval");
			out.push(Interval {
				pid,
				start_ns,
				end_ns,
				sha256: common::hash_file(&path)?,
				trace: path,
			});
		}
	}
	Ok(out)
}
fn maximum(intervals: &[Interval]) -> Result<usize> {
	let mut events = Vec::new();
	for i in intervals {
		events.push((i.start_ns, 1i32));
		events.push((i.end_ns, -1));
	}
	events.sort_unstable();
	let mut active = 0;
	let mut maximum = 0;
	for (_, change) in events {
		active += change;
		ensure!(active >= 0, "malformed event ordering");
		maximum = maximum.max(active);
	}
	ensure!(active == 0, "unterminated interpreter");
	Ok(maximum as usize)
}
pub fn proof(dir: &Path, labels: &[&str], capacity: usize) -> Result<Value> {
	let mut all = Vec::new();
	for label in labels {
		let selected = intervals(dir, label)?;
		ensure!(
			selected.len() == 1,
			"{label}: expected exactly one interpreter"
		);
		all.extend(selected);
	}
	let max = maximum(&all)?;
	ensure!(
		capacity > 0 && max <= capacity,
		"actual Miri interpreters exceed declared CPU budget"
	);
	Ok(
		json!({"maximum_actual_interpreters":max,"intervals":all,"method":"strace execve/exit_group terminal process events; ends sort before simultaneous starts"}),
	)
}
fn files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
	for entry in fs::read_dir(root)? {
		let path = entry?.path();
		if path.is_dir() {
			files(&path, out)?;
		} else {
			out.push(path);
		}
	}
	Ok(())
}
pub fn harness_records(ctx: &Context) -> Result<Value> {
	let root = ctx.out.join("target-miri");
	let mut paths = Vec::new();
	files(&root, &mut paths)?;
	let dest = ctx.out.join("harness-records");
	fs::create_dir(&dest)?;
	let mut records = Vec::new();
	for path in paths {
		let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
			continue;
		};
		if !name.starts_with("rune-") || !path.components().any(|c| c.as_os_str() == "out") {
			continue;
		}
		let Ok(record) = common::read_json::<Value>(&path) else {
			continue;
		};
		let Some(args) = record["args"].as_array() else {
			continue;
		};
		if !args.iter().any(|v| v == "--test") {
			continue;
		}
		let cwd = if let Some(s) = record["current_dir"].as_str() {
			PathBuf::from(s)
		} else {
			use std::os::unix::ffi::OsStringExt;
			let Some(bytes) = record["current_dir"]["Unix"].as_array() else {
				continue;
			};
			let bytes = bytes
				.iter()
				.map(|v| {
					v.as_u64()
						.and_then(|v| u8::try_from(v).ok())
						.context("bad harness cwd byte")
				})
				.collect::<Result<Vec<_>>>()?;
			PathBuf::from(std::ffi::OsString::from_vec(bytes))
		};
		ensure!(
			fs::canonicalize(&cwd)? == ctx.repo,
			"Miri harness belongs to different source: {}",
			cwd.display()
		);
		let copy = dest.join(format!("{:03}.json", records.len()));
		common::write_json(&copy, &record)?;
		records.push(
			json!({"original":path,"copy":copy,"sha256":common::hash_file(&copy)?,"recorded_cwd":cwd}),
		);
	}
	ensure!(
		!records.is_empty(),
		"missing Cargo Miri test harness source record"
	);
	Ok(json!(records))
}
#[cfg(test)]
mod tests {
	use super::*;
	fn i(start_ns: u64, end_ns: u64) -> Interval {
		Interval {
			pid: 1,
			start_ns,
			end_ns,
			trace: PathBuf::new(),
			sha256: String::new(),
		}
	}
	#[test]
	fn detects_three_actual_interpreters() {
		assert_eq!(maximum(&[i(1, 9), i(2, 8), i(3, 7)]).unwrap(), 3);
	}
	#[test]
	fn simultaneous_end_precedes_start() {
		assert_eq!(maximum(&[i(1, 2), i(2, 3)]).unwrap(), 1);
	}
	#[test]
	fn timestamp_is_integer_exact() {
		assert_eq!(timestamp("123.000001 execve(").unwrap(), 123000001000);
	}
	#[test]
	fn wrapper_arguments_are_not_executable_events() {
		let root = std::env::temp_dir().join(format!("rune-native-trace-{}", std::process::id()));
		fs::create_dir(&root).unwrap();
		let path = root.join("wrapped.native.trace.123");
		fs::write(
			&path,
			concat!(
            "1.000001 execve(\"/usr/bin/prlimit\", [\"prlimit\", \"/tmp/rune-tests\"], 0x0) = 0\n",
            "1.000002 execve(\"/usr/bin/bwrap\", [\"bwrap\", \"/tmp/rune-tests\"], 0x0) = 0\n",
            "1.000003 execve(\"/tmp/rune-tests\", [\"rune-tests\"], 0x0) = 0\n",
            "2.000001 exit_group(0) = ?\n"
        ),
		)
		.unwrap();
		let rows = execution_intervals(&root, "wrapped.native.trace.", false).unwrap();
		assert_eq!(rows.len(), 1);
		assert_eq!(rows[0].start_ns, 1_000_003_000);
		assert_eq!(rows[0].end_ns, 2_000_001_000);
		fs::remove_dir_all(root).unwrap();
	}
}
