//! Frozen stock observations before named-field receiver proofs.
use super::*;

fn contract() -> Json {
	serde_json::from_str(include_str!("field_receiver_contract.json")).unwrap()
}

#[test]
#[ignore = "operator-only pre-implementation field freeze; requires new C18F_ORACLE_OUT"]
fn field_receiver_freeze_oracle() {
	use std::io::Write;
	let mut data = contract();
	for row in data["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
	}
	let mut out = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18F_ORACLE_OUT").unwrap())
		.unwrap();
	out.write_all(serde_json::to_string_pretty(&data).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn field_receiver_stock_is_frozen() {
	let data = contract();
	for original in data["rows"].as_array().unwrap() {
		assert_eq!(
			digest(original["source"].as_str().unwrap().as_bytes()),
			original["source_sha256"]
		);
		let mut row = original.clone();
		let actual = stock(&mut row);
		let mut expected = original["stock"].clone();
		if !cfg!(feature = "serde") {
			expected["unit_sha256"] = Json::Null;
		}
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

#[test]
fn field_receiver_existing_admissions_are_frozen() {
	for row in contract()["rows"].as_array().unwrap() {
		if row["before"]["admitted"] == true {
			assert_eq!(baseline(row), row["before"], "{}", row["name"]);
		}
	}
}
