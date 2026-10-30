//! Independent local verification jobs, with one shared build and CPU budget.
use crate::common::{self, Context, Result};
use anyhow::{bail, ensure};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
	collections::{BTreeMap, BTreeSet},
	sync::OnceLock,
	time::Instant,
};

#[derive(Clone, Debug, Serialize)]
pub struct Task {
	pub id: String,
	pub suite: String,
	pub parent: Option<String>,
	pub estimate_seconds: f64,
	pub action: String,
	pub key: String,
}
impl Task {
	pub fn new(suite: &str, action: &str, key: &str, estimate_seconds: f64) -> Self {
		Self {
			id: format!("{suite}/{action}/{key}"),
			suite: suite.into(),
			parent: None,
			estimate_seconds,
			action: action.into(),
			key: key.into(),
		}
	}
}
#[derive(Default)]
struct State {
	miri: OnceLock<std::result::Result<crate::miri::Prepared, String>>,
}
pub fn catalog(ctx: &Context, suites: &[String]) -> Result<Vec<Task>> {
	let mut tasks = Vec::new();
	for suite in suites {
		match suite.as_str() {
			"contracts" | "native" => tasks.extend(crate::native::task_specs(suite)?),
			"awfy" => tasks.extend(crate::awfy::task_specs(ctx)?),
			"miri" => tasks.extend(crate::miri::task_specs(ctx)?),
			"l1" | "clippy" | "layouts" | "measure-checks" => {
				tasks.push(Task::new(suite, "suite", "all", 300.0))
			}
			_ => bail!("unknown verification suite {suite}"),
		}
	}
	ensure!(
		tasks
			.iter()
			.map(|task| &task.id)
			.collect::<BTreeSet<_>>()
			.len() == tasks.len(),
		"duplicate verification job"
	);
	tasks.sort_by(|a, b| {
		b.estimate_seconds
			.total_cmp(&a.estimate_seconds)
			.then_with(|| a.id.cmp(&b.id))
	});
	Ok(tasks)
}
fn run(ctx: &Context, state: &State, task: &Task) -> Result<Value> {
	if task.suite == "miri" {
		let prepared = state
			.miri
			.get_or_init(|| crate::miri::prepare(ctx).map_err(|error| format!("{error:#}")));
		return crate::miri::run_prepared(
			prepared
				.as_ref()
				.map_err(|error| anyhow::anyhow!(error.clone()))?,
			&task.key,
		);
	}
	let child = ctx.suite(&format!("task-{}", common::hash_bytes(task.id.as_bytes())))?;
	match task.suite.as_str() {
		"contracts" | "native" => crate::native::run_task(&child, task),
		"awfy" => crate::awfy::run_task(&child, task),
		"l1" => crate::preservation::run(&child),
		"clippy" | "layouts" => crate::native::run(&child, &task.suite),
		"measure-checks" => crate::measure_checks::run(&child),
		_ => bail!("unknown verification job"),
	}
}
fn execute(ctx: &Context, state: &State, task: &Task, epoch: Instant) -> Result<Value> {
	let start = epoch.elapsed().as_secs_f64();
	let result = run(ctx, state, task);
	let row = json!({"task":task,"status":if result.is_ok(){"PASS"}else{"FAIL"},"start_seconds":start,"end_seconds":epoch.elapsed().as_secs_f64(),"result":result.as_ref().ok(),"error":result.as_ref().err().map(|error|format!("{error:#}"))});
	common::write_json(
		&ctx.out.join(format!(
			"task-{}-RESULT.json",
			common::hash_bytes(task.id.as_bytes())
		)),
		&row,
	)?;
	Ok(row)
}
pub fn run_all(ctx: &Context, suites: &[String], workers: usize) -> Result<Vec<Value>> {
	ctx.verify_source()?;
	let tasks = catalog(ctx, suites)?;
	common::write_json(&ctx.out.join("TASKS.json"), &tasks)?;
	let (miri, native): (Vec<_>, Vec<_>) = tasks.iter().partition(|task| task.suite == "miri");
	let miri_workers = crate::miri::workers()?
		.min(ctx.budget.capacity())
		.min(workers);
	let state = State::default();
	let epoch = Instant::now();
	let rows = std::thread::scope(|scope| -> Result<_> {
		let native = scope.spawn(|| {
			crate::scheduler::map(&native, workers.min(ctx.jobs), |task| {
				execute(ctx, &state, task, epoch)
			})
		});
		let miri = crate::scheduler::map(&miri, miri_workers, |task| {
			execute(ctx, &state, task, epoch)
		});
		let mut rows = native
			.join()
			.map_err(|_| anyhow::anyhow!("native job worker panicked"))??;
		rows.extend(miri?);
		Ok(rows)
	})?;
	common::write_json(&ctx.out.join("TASK-RESULTS.json"), &rows)?;
	let grouped = group(&tasks, &rows)?;
	let mut results = Vec::new();
	for (suite, rows) in grouped {
		let outcome = (|| -> Result<Value> {
			ensure!(
				rows.iter().all(|row| row["status"] == "PASS"),
				"verification job failed"
			);
			match suite.as_str() {
				"awfy" => crate::awfy::aggregate_tasks(ctx, &rows),
				"miri" => crate::miri::finish(
					state
						.miri
						.get()
						.ok_or_else(|| anyhow::anyhow!("Miri was not prepared"))?
						.as_ref()
						.map_err(|error| anyhow::anyhow!(error.clone()))?,
					&rows,
					miri_workers,
				),
				"native" => Ok(
					json!({"status":"PASS","libraries":rows.iter().filter(|row|row["task"]["action"]=="library").map(|row|&row["result"]).collect::<Vec<_>>(),"embedding_borrow_docs":rows.iter().filter(|row|row["task"]["action"]=="doc").map(|row|&row["result"]).collect::<Vec<_>>()}),
				),
				"contracts" => Ok(
					json!({"status":"PASS","cases":rows.iter().filter(|row|row["task"]["action"]=="case").map(|row|&row["result"]).collect::<Vec<_>>(),"contract_bundle":rows.iter().find(|row|row["task"]["action"]=="bundle").map(|row|&row["result"]),"diagnostic_witnesses":rows.iter().find(|row|row["task"]["action"]=="registry").map(|row|&row["result"])}),
				),
				_ => {
					ensure!(rows.len() == 1, "duplicate suite job");
					Ok(rows[0]["result"].clone())
				}
			}
		})();
		let start = rows
			.iter()
			.filter_map(|row| row["start_seconds"].as_f64())
			.fold(f64::INFINITY, f64::min);
		let end = rows
			.iter()
			.filter_map(|row| row["end_seconds"].as_f64())
			.fold(0.0, f64::max);
		let value = json!({"suite":suite,"status":if outcome.is_ok(){"PASS"}else{"FAIL"},"seconds":end-start,"result":outcome.as_ref().ok(),"error":outcome.as_ref().err().map(|error|format!("{error:#}"))});
		std::fs::create_dir_all(ctx.out.join(&suite))?;
		common::write_json(&ctx.out.join(&suite).join("RESULT.json"), &value)?;
		results.push(value);
	}
	Ok(results)
}
fn group(tasks: &[Task], rows: &[Value]) -> Result<BTreeMap<String, Vec<Value>>> {
	let expected: BTreeMap<_, _> = tasks.iter().map(|task| (&task.id, task)).collect();
	let mut seen = BTreeSet::new();
	let mut groups = BTreeMap::<String, Vec<Value>>::new();
	for row in rows {
		let id = row["task"]["id"]
			.as_str()
			.ok_or_else(|| anyhow::anyhow!("missing job id"))?;
		let task = expected
			.get(&id.to_owned())
			.ok_or_else(|| anyhow::anyhow!("foreign verification job"))?;
		ensure!(
			row["task"] == serde_json::to_value(task)? && seen.insert(id),
			"foreign or duplicate verification job"
		);
		groups
			.entry(task.suite.clone())
			.or_default()
			.push(row.clone());
	}
	ensure!(seen.len() == tasks.len(), "incomplete verification jobs");
	Ok(groups)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn grouping_rejects_missing_duplicate_and_foreign_jobs() {
		let task = Task::new("native", "library", "native", 600.0);
		let tasks = vec![task];
		let row = json!({"task":tasks[0],"status":"PASS"});
		assert!(group(&tasks, std::slice::from_ref(&row)).is_ok());
		assert!(group(&tasks, &[]).is_err());
		assert!(group(&tasks, &[row.clone(), row.clone()]).is_err());
		let mut foreign = row;
		foreign["task"]["key"] = json!("foreign");
		assert!(group(&tasks, &[foreign]).is_err());
	}
}
