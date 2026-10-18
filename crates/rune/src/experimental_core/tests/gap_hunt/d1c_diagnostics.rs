use serde_json::Value as Json;

/// Check the exact source-bound D1c display before comparing immutable oracles.
/// All non-display fields remain available to the existing strict comparators.
pub(super) fn before(family: &str, label: &str, source: &str, engine: &str, actual: &Json) -> Json {
	let actual = d1d_before(family, label, source, engine, actual);
	let actual = &actual;
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
	if matches!(
		kind,
		crate::runtime::VmErrorKind::AccessError { .. }
			| crate::runtime::VmErrorKind::Overflow
			| crate::runtime::VmErrorKind::Underflow
			| crate::runtime::VmErrorKind::DivideByZero
			| crate::runtime::VmErrorKind::ValueToIntegerCoercionError { .. }
	) {
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
	if fields.len() == 5 && fields[0] == "error" {
		let doc: Json = serde_json::from_str(include_str!("d1d_numeric_diagnostics.json")).unwrap();
		let source_hex = super::hex(source.as_bytes());
		let matching = doc["records"]
			.as_array()
			.unwrap()
			.iter()
			.filter(|row| row["source_hex"] == source_hex)
			.collect::<std::vec::Vec<_>>();
		if let Some(row) = matching.first() {
			let rule = &row["changes"][engine];
			assert!(matching.iter().all(|r| r["changes"][engine] == *rule));
			assert_eq!(
				super::unhex(&fields[3]),
				rule["after"].as_str().unwrap(),
				"D1d record {}",
				row["label"]
			);
			fields[3] = super::hex(rule["before"].as_str().unwrap().as_bytes());
			return fields.join("\t");
		}
		let message = super::unhex(&fields[3]);
		if let Some(previous) = previous_operator_display(&message) {
			fields[3] = super::hex(previous.as_bytes());
			return fields.join("\t");
		}
	}
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

/// Reversible presentation migration for the existing typed operator payloads.
/// The frozen comparisons still check the complete operator/type payload and sites.
fn previous_operator_display(current: &str) -> Option<std::string::String> {
	let tail = current.strip_prefix("Cannot apply `")?;
	let (op, operands) = tail.split_once("` to `")?;
	let operands = operands.strip_suffix('`')?;
	fn previous_type(name: &str) -> &str {
		match name {
			"i64" => "std::i64",
			"u64" => "std::u64",
			"f64" => "std::f64",
			"bool" => "std::bool",
			"char" => "std::char",
			"String" => "std::string::String",
			"Vec" => "std::vec::Vec",
			other => other,
		}
	}
	Some(if let Some((lhs, rhs)) = operands.split_once("` and `") {
		std::format!(
			"Unsupported binary operation `{op}` on `{}` and `{}`",
			previous_type(lhs),
			previous_type(rhs)
		)
	} else {
		assert!(matches!(op, "!" | "-"));
		std::format!(
			"Unsupported unary operation `{op}` on `{}`",
			previous_type(operands)
		)
	})
}

pub(super) fn d1d_before(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
	actual: &Json,
) -> Json {
	let mut before = actual.clone();
	if let Some(current) = actual["display"].as_str() {
		if let Some(previous) = previous_operator_display(current) {
			before["display"] = Json::String(previous);
			return before;
		}
	}
	let doc: Json = serde_json::from_str(include_str!("d1d_numeric_diagnostics.json")).unwrap();
	assert_eq!(doc["schema"], 1);
	if let Some(row) = doc["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)
	{
		assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
		assert_eq!(
			actual["display"], row["changes"][engine]["after"],
			"D1d {family}/{label}/{engine}"
		);
		before["display"] = row["changes"][engine]["before"].clone();
	}
	before
}
