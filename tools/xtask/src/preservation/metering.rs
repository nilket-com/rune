//! The execution core may count less work, but must never count more than
//! legacy execution for a program that completes within the same allowance.
use super::{inputs, live};
use crate::common::{self, Context, Result};
use anyhow::ensure;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};

const CHILD: &str = "experimental_core::tests::embedding_boundary::metering::child";

fn classify(legacy: &Value, execution: &Value) -> Result<&'static str> {
	fn phase(v: &Value) -> &str {
		v["phase"].as_str().unwrap_or("invalid")
	}
	let (old, new) = (phase(legacy), phase(execution));
	if new == "refused" {
		return Ok("safe-refusal");
	}
	if old == "compile-error" {
		return Ok("legacy-compile-error");
	}
	if old == "compiled" && new == "compiled" {
		return Ok("compile-only");
	}
	if old == "complete" {
		ensure!(
			new == "complete",
			"legacy completed but execution stopped: {execution}"
		);
		let old = legacy["used"]
			.as_u64()
			.ok_or_else(|| anyhow::anyhow!("missing legacy work"))?;
		let new = execution["used"]
			.as_u64()
			.ok_or_else(|| anyhow::anyhow!("missing execution work"))?;
		ensure!(new <= old, "execution charged more work: {new} > {old}");
		return Ok("complete-no-more-work");
	}
	if old == "budget" && new == "complete" {
		return Ok("reaches-further/success");
	}
	if old == "runtime-error" && new == "entry-error" {
		return Ok("nonexecution-entry-check-semantic-pass-required");
	}
	if old == "runtime-error" {
		ensure!(
			new == "runtime-error",
			"legacy reached an error but execution stopped differently: {execution}"
		);
		let old = legacy["used"]
			.as_u64()
			.ok_or_else(|| anyhow::anyhow!("missing legacy error work"))?;
		let new = execution["used"]
			.as_u64()
			.ok_or_else(|| anyhow::anyhow!("missing execution error work"))?;
		ensure!(
			new <= old,
			"execution charged more work before error: {new} > {old}"
		);
		return Ok("runtime-error-no-more-work-semantic-pass-required");
	}
	ensure!(
		["budget", "runtime-error"].contains(&old),
		"unknown legacy phase: {legacy}"
	);
	ensure!(new == "budget", "unknown execution phase: {execution}");
	Ok("noncompletion-semantic-pass-required")
}

pub(super) fn run(ctx: &Context) -> Result<Value> {
	let corpus = ctx.repo.join("tools/xtask/corpus").canonicalize()?;
	let index = fs::read_to_string(corpus.join("l1-cases.tsv"))?;
	let cases = inputs::rows(&index)?;
	let mut profiles = Vec::new();
	for native in [false, true] {
		let features = if native {
			std::format!("{},native-code", common::EXECUTION_FEATURES)
		} else {
			common::EXECUTION_FEATURES.to_owned()
		};
		let binary = if native {
			ctx.build_rune("metering-native", &features)?
		} else {
			ctx.test_binary()?
		};
		let mut run = ctx.clone();
		run.out = ctx.out.join(if native {
			"metering-native"
		} else {
			"metering-interpreted"
		});
		fs::create_dir(&run.out)?;
		let root = run.out.join("cases");
		fs::create_dir(&root)?;
		let next = std::sync::atomic::AtomicUsize::new(0);
		let outcomes = std::sync::Mutex::new(Vec::new());
		std::thread::scope(|scope| {
			for _ in 0..ctx.jobs {
				let run = &run;
				let corpus = &corpus;
				let root = &root;
				let binary = &binary;
				let cases = &cases;
				let next = &next;
				let outcomes = &outcomes;
				scope.spawn(move || loop {
					let position = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
					let Some(&[id, source, kind, entry, _, _]) = cases.get(position) else {
						break;
					};
					let mode = if kind == "test" {
						format!("test:{entry}")
					} else {
						kind.to_owned()
					};
					let directory = root.join(id);
					let observed = (|| -> Result<_> {
						fs::create_dir(&directory)?;
						let a = live::invoke_child(
							run, binary, corpus, &directory, id, source, &mode, "legacy", CHILD,
						)?;
						let b = live::invoke_child(
							run,
							binary,
							corpus,
							&directory,
							id,
							source,
							&mode,
							"execution",
							CHILD,
						)?;
						let streams_equal = a.1 == b.1 && a.2 == b.2;
						let a: Value = serde_json::from_str(&a.0)?;
						let b: Value = serde_json::from_str(&b.0)?;
						let class = classify(&a, &b)?;
						if class == "complete-no-more-work" {
							ensure!(streams_equal, "completed program streams differ");
						}
						Ok(json!({"id":id,"mode":mode,"class":class,"legacy":a,"execution":b}))
					})();
					outcomes.lock().unwrap().push((
						position,
						match observed {
							Ok(row) => row,
							Err(error) => {
								json!({"id":id,"mode":mode,"failure":format!("{error:#}")})
							}
						},
					));
				});
			}
		});
		let mut outcomes = outcomes
			.into_inner()
			.map_err(|_| anyhow::anyhow!("metering lock poisoned"))?;
		ensure!(outcomes.len() == cases.len(), "incomplete metering corpus");
		outcomes.sort_by_key(|(position, _)| *position);
		let rows: Vec<_> = outcomes.into_iter().map(|(_, row)| row).collect();
		let mut counts = BTreeMap::<String, usize>::new();
		for row in &rows {
			*counts
				.entry(row["class"].as_str().unwrap_or("failure").to_owned())
				.or_default() += 1;
		}
		let report = json!({"native":native,"cases":rows,"classes":counts,"binary_sha256":common::hash_file(&binary)?});
		common::write_json(&run.out.join("METERING.json"), &report)?;
		profiles.push(report);
	}
	ctx.verify_source()?;
	let report = json!({"profiles":profiles});
	common::write_json(&ctx.out.join("METERING.json"), &report)?;
	ensure!(
		report["profiles"]
			.as_array()
			.unwrap()
			.iter()
			.all(|profile| profile["cases"]
				.as_array()
				.unwrap()
				.iter()
				.all(|row| row.get("failure").is_none())),
		"L1 metering failed; inspect METERING.json"
	);
	Ok(report)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn work_audit_refuses_more_work_and_unproved_error_advances() {
		assert_eq!(
			classify(
				&json!({"phase":"complete","used":7}),
				&json!({"phase":"complete","used":1})
			)
			.unwrap(),
			"complete-no-more-work"
		);
		assert!(classify(
			&json!({"phase":"complete","used":7}),
			&json!({"phase":"complete","used":8})
		)
		.is_err());
		assert!(classify(
			&json!({"phase":"complete","used":7}),
			&json!({"phase":"budget"})
		)
		.is_err());
		assert_eq!(
			classify(
				&json!({"phase":"budget"}),
				&json!({"phase":"complete","used":1})
			)
			.unwrap(),
			"reaches-further/success"
		);
		assert!(classify(
			&json!({"phase":"runtime-error"}),
			&json!({"phase":"complete","used":1})
		)
		.is_err());
	}
}
