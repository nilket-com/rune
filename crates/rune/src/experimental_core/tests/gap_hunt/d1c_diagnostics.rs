use serde_json::Value as Json;

/// Check the exact source-bound D1c display before comparing immutable oracles.
/// All non-display fields remain available to the existing strict comparators.
pub(super) fn before(family: &str, label: &str, source: &str, engine: &str, actual: &Json) -> Json {
	let document: Json = serde_json::from_str(include_str!("d1c_diagnostics.json")).unwrap();
	assert_eq!(document["schema"], 1);
	assert!(matches!(engine, "stock" | "fresh"));
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 68);
	let Some(row) = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)
	else {
		return actual.clone();
	};
	assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
	assert_eq!(actual["status"], "error");
	assert!(
		actual["class"] == "AccessConflict"
			|| actual["kind"]
				.as_str()
				.is_some_and(|kind| kind.starts_with("AccessError { error:"))
	);
	assert_eq!(
		actual["display"], row["changes"][engine]["after"],
		"D1c {family}/{label}/{engine}"
	);
	let mut before = actual.clone();
	before["display"] = row["changes"][engine]["before"].clone();
	before
}

pub(super) fn stock_display(error: &crate::runtime::VmError) -> std::string::String {
	let kind = error.error().kind();
	if matches!(kind, crate::runtime::VmErrorKind::AccessError { .. }) {
		std::string::ToString::to_string(error)
	} else {
		std::string::ToString::to_string(kind)
	}
}

/// Validate exact source-bound display changes in the five-field record schema.
pub(super) fn record_before(source: &str, engine: &str, actual: &str) -> std::string::String {
	use std::string::ToString;
	let mut fields = actual
		.split('\t')
		.map(str::to_string)
		.collect::<std::vec::Vec<_>>();
	if fields.len() == 5 && fields[0] == "observation-error" && fields[1] == "Debug" {
		let rows: std::vec::Vec<Json> =
			serde_json::from_str(include_str!("d1c_debug_diagnostics.json")).unwrap();
		assert_eq!(rows.len(), 17);
		let source_hex = super::hex(source.as_bytes());
		if let Some(row) = rows.iter().find(|row| row["source_hex"] == source_hex) {
			assert_eq!(
				fields[3],
				super::hex(row["after"].as_str().unwrap().as_bytes()),
				"D1c Debug {}",
				row["label"]
			);
			fields[3] = super::hex(row["before"].as_str().unwrap().as_bytes());
			return fields.join("\t");
		}
	}
	if fields.len() != 5 || fields[0] != "error" || fields[1] != "AccessConflict" {
		return actual.to_string();
	}
	let rows: std::vec::Vec<Json> =
		serde_json::from_str(include_str!("d1c_record_diagnostics.json")).unwrap();
	assert_eq!(rows.len(), 91);
	let source_hex = super::hex(source.as_bytes());
	let matching = rows
		.iter()
		.filter(|row| row["source_hex"] == source_hex)
		.collect::<std::vec::Vec<_>>();
	let Some(row) = matching.first() else {
		return actual.to_string();
	};
	let rule = &row["changes"][engine];
	assert!(
		matching.iter().all(|row| row["changes"][engine] == *rule),
		"ambiguous source pin"
	);
	assert_eq!(
		fields[3],
		super::hex(rule["after"].as_str().unwrap().as_bytes()),
		"D1c record {}/{engine}",
		row["label"]
	);
	fields[3] = super::hex(rule["before"].as_str().unwrap().as_bytes());
	fields.join("\t")
}

#[test]
fn record_pins_reject_wrong_display_and_preserve_operation_and_value() {
	let rows: std::vec::Vec<Json> =
		serde_json::from_str(include_str!("d1c_record_diagnostics.json")).unwrap();
	for row in &rows {
		let hex = row["source_hex"].as_str().unwrap();
		let bytes = (0..hex.len())
			.step_by(2)
			.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
			.collect::<std::vec::Vec<_>>();
		let source = std::str::from_utf8(&bytes).unwrap();
		for engine in ["stock", "fresh"] {
			let after = super::hex(row["changes"][engine]["after"].as_str().unwrap().as_bytes());
			let actual =
				std::format!("error\tAccessConflict\toperation-sentinel\t{after}\tvalue-sentinel");
			let projected = record_before(source, engine, &actual);
			let fields = projected.split('\t').collect::<std::vec::Vec<_>>();
			assert_eq!(fields[2], "operation-sentinel");
			assert_eq!(fields[4], "value-sentinel");
			assert_eq!(
				fields[3],
				super::hex(
					row["changes"][engine]["before"]
						.as_str()
						.unwrap()
						.as_bytes()
				)
			);
			assert_eq!(record_before("changed source", engine, &actual), actual);
			let wrong = "error\tAccessConflict\toperation-sentinel\twrong-display\tvalue-sentinel";
			assert!(std::panic::catch_unwind(|| record_before(source, engine, wrong)).is_err());
		}
	}
}

#[test]
#[cfg(not(miri))]
fn display_pins_reject_wrong_state_source_and_position() {
	let document: Json = serde_json::from_str(include_str!("d1c_diagnostics.json")).unwrap();
	for row in document["rows"].as_array().unwrap() {
		let family = row["family"].as_str().unwrap();
		let label = row["label"].as_str().unwrap();
		let hex = row["source_hex"].as_str().unwrap();
		let bytes = (0..hex.len())
			.step_by(2)
			.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
			.collect::<std::vec::Vec<_>>();
		let source = std::str::from_utf8(&bytes).unwrap();
		for engine in ["stock", "fresh"] {
			let actual = serde_json::json!({"status":"error","class":"AccessConflict","display":row["changes"][engine]["after"],"sites":[{"start":1,"end":2}],"chain":[{"kind":"sentinel"}]});
			let projected = before(family, label, source, engine, &actual);
			assert_eq!(projected["display"], row["changes"][engine]["before"]);
			assert_eq!(projected["sites"], actual["sites"]);
			assert_eq!(projected["chain"], actual["chain"]);
			for wrong in [
				"Cannot read the value: it is not available",
				"Cannot write argument 9: it is being read",
			] {
				let mut bad = actual.clone();
				bad["display"] = serde_json::json!(wrong);
				assert!(
					std::panic::catch_unwind(|| before(family, label, source, engine, &bad))
						.is_err()
				);
			}
			assert!(std::panic::catch_unwind(|| before(
				family,
				label,
				"changed source",
				engine,
				&actual
			))
			.is_err());
		}
	}
}
