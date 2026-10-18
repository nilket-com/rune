use serde_json::Value as Json;
use std::{borrow::ToOwned, string::String, vec::Vec};

/// Validate D1b's exact display and source sites, then project them to the
/// immutable pre-D1b observation used by each contract.
pub(super) fn intentional_json_before(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
	actual: &Json,
) -> Json {
	let document: Json = serde_json::from_str(include_str!("d1b_diagnostics.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 45, "D1b source-pinned change rows");
	let Some(row) = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)
	else {
		return actual.clone();
	};

	assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
	let expected_after = &row["after"][engine];
	assert_eq!(
		actual["display"], expected_after["display"],
		"D1b text {label}/{engine}"
	);
	assert_eq!(
		actual["sites"], expected_after["sites"],
		"D1b sites {label}/{engine}"
	);

	let before = &row["before"][engine];
	let mut actual = actual.clone();
	actual["display"] = before["display"].clone();
	actual["sites"] = before["sites"].clone();
	actual
}

/// Return the pre-D1b source sites recorded for an intentional row.
pub(super) fn intentional_sites_before(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
) -> Option<Json> {
	let document: Json = serde_json::from_str(include_str!("d1b_diagnostics.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 45, "D1b source-pinned change rows");
	let row = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)?;

	assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
	Some(row["before"][engine]["sites"].clone())
}

/// Validate a serialized len/clone record against D1b, then restore its old display text.
pub(super) fn intentional_record_before(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
	actual: &str,
) -> String {
	let document: Json = serde_json::from_str(include_str!("d1b_diagnostics.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 45, "D1b source-pinned change rows");
	let Some(row) = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)
	else {
		return actual.to_owned();
	};

	assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
	let mut fields: Vec<_> = actual.split('\t').map(str::to_owned).collect();
	assert_eq!(fields.first().map(String::as_str), Some("error"));
	assert!(fields.len() >= 4);
	let bytes = fields[3].as_bytes();
	assert_eq!(bytes.len() % 2, 0);
	let mut display = Vec::with_capacity(bytes.len() / 2);
	let (pairs, remainder) = bytes.as_chunks::<2>();
	assert!(remainder.is_empty());
	for pair in pairs {
		let digit = |byte: u8| match byte {
			b'0'..=b'9' => byte - b'0',
			b'a'..=b'f' => byte - b'a' + 10,
			b'A'..=b'F' => byte - b'A' + 10,
			_ => panic!("invalid display hex"),
		};
		display.push(digit(pair[0]) * 16 + digit(pair[1]));
	}
	let display = std::str::from_utf8(&display).unwrap();
	assert_eq!(display, row["after"][engine]["display"]);
	assert_eq!(row["after"][engine]["sites"], serde_json::json!([]));
	fields[3] = super::hex(
		row["before"][engine]["display"]
			.as_str()
			.unwrap()
			.as_bytes(),
	);
	fields.join("\t")
}

/// Validate an intentional assertion-site update and return its frozen site.
pub(in crate::experimental_core::tests) fn intentional_site_before(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
	actual: &str,
	frozen: &str,
) -> Option<()> {
	let document: Json = serde_json::from_str(include_str!("d1b_diagnostics.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 45, "D1b source-pinned change rows");
	let row = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)?;

	assert_eq!(row["source_hex"], super::hex(source.as_bytes()));
	let after = site_json(actual);
	assert_eq!(row["after"][engine]["sites"], after);
	assert_eq!(row["before"][engine]["sites"], site_json(frozen));
	Some(())
}

/// Check an intentional site without panicking on negative-control inputs.
pub(super) fn matches_intentional_site(
	family: &str,
	label: &str,
	source: &str,
	engine: &str,
	actual: &str,
	frozen: &str,
) -> bool {
	let document: Json = serde_json::from_str(include_str!("d1b_diagnostics.json")).unwrap();
	let rows = document["rows"].as_array().unwrap();
	let Some(row) = rows
		.iter()
		.find(|row| row["family"] == family && row["label"] == label)
	else {
		return false;
	};
	row["source_hex"] == super::hex(source.as_bytes())
		&& row["after"][engine]["sites"] == site_json(actual)
		&& row["before"][engine]["sites"] == site_json(frozen)
}

fn site_json(site: &str) -> Json {
	let mut parts = site.split(':');
	let source = parts.next().unwrap().parse::<u32>().unwrap();
	let start = parts.next().unwrap().parse::<u32>().unwrap();
	let end = parts.next().unwrap().parse::<u32>().unwrap();
	assert!(parts.next().is_none());
	serde_json::json!([{"source": source, "start": start, "end": end, "frames": "[]"}])
}
