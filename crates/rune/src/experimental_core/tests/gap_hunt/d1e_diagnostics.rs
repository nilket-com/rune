//! Mandatory source-bound D1e API checks; optional independent evidence output.
use super::*;
use serde_json::{json, Value as Json};
fn write<'a>(
	frames: impl Iterator<Item = crate::runtime::LogicalFrame<'a>>,
	context: impl Iterator<Item = crate::runtime::InvocationContext>,
	rendered: impl core::fmt::Display,
	sources: &Sources,
) {
	let path = std::env::var_os("D1E_TRACE_OUT");
	let pins: Json =
		serde_json::from_str(include_str!("d1e_intentional_diagnostics.json")).unwrap();
	let source = sources
		.get(crate::SourceId::new(0))
		.map(|s| hex(s.as_str().as_bytes()));
	let row = pins["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|row| source.as_deref() == row["source_hex"].as_str());
	if path.is_none() && row.is_none() {
		return;
	}
	let frames = frames.map(|frame| json!({"role":format!("{:?}",frame.role), "location":frame.location.map(|at| {
        let range=at.span.range(); json!({"source":at.source_id.into_index(),"start":range.start,"end":range.end})
    })})).collect::<Vec<Json>>();
	let context = context
		.map(|entry| match entry {
			crate::runtime::InvocationContext::BadArgument { arg } => {
				json!({"tag":"BadArgument","arg":arg})
			}
		})
		.collect::<Vec<_>>();
	let actual = json!({"frames":frames,"context":context,"rendered":rendered.to_string()});
	if let Some(row) = row {
		assert!(
			matches_pin(row, &actual),
			"D1e exact API drift: {}: {actual}",
			row["label"]
		);
	}
	if let Some(path) = path {
		fs::write(path, serde_json::to_vec_pretty(&actual).unwrap()).unwrap();
	}
}
pub(super) fn stock(error: &crate::runtime::VmError, sources: &Sources) {
	write(
		error.logical_frames(),
		error.invocation_context(),
		error.diagnostic(sources),
		sources,
	);
}
pub(super) fn execution(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
	sources: &Sources,
) {
	write(
		failure.logical_frames(),
		failure.invocation_context(),
		failure.diagnostic(program, sources),
		sources,
	);
}

fn matches_pin(row: &Json, actual: &Json) -> bool {
	actual["frames"] == row["frames"]
		&& actual["rendered"] == row["rendered"]
		&& row
			.get("context")
			.is_none_or(|context| actual["context"] == *context)
}

#[test]
fn d1e_pins_reject_missing_repeated_frames_rendering_and_argument_drift() {
	let pins: Json =
		serde_json::from_str(include_str!("d1e_intentional_diagnostics.json")).unwrap();
	let rows = pins["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 24);
	for row in rows {
		let actual = json!({"frames":row["frames"],"rendered":row["rendered"],"context":row.get("context").cloned().unwrap_or_else(||json!([]))});
		assert!(matches_pin(row, &actual));
		let mut changed = actual.clone();
		changed["frames"].as_array_mut().unwrap().pop();
		assert!(!matches_pin(row, &changed));
		let mut changed = actual.clone();
		changed["frames"]
			.as_array_mut()
			.unwrap()
			.push(actual["frames"][0].clone());
		assert!(!matches_pin(row, &changed));
		let mut changed = actual.clone();
		changed["rendered"] = json!("unproven chain");
		assert!(!matches_pin(row, &changed));
		if row.get("context").is_some() {
			let mut changed = actual.clone();
			changed["context"] = json!([]);
			assert!(!matches_pin(row, &changed));
			changed["context"] = json!([{"tag":"BadArgument","arg":1}]);
			assert!(!matches_pin(row, &changed));
		}
	}
}
