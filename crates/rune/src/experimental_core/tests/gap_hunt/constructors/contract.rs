const DISPOSITIONS: &str = include_str!("dispositions.json");
const STOCK: &str = include_str!("stock.json");
fn project_observation(value: &Json, expected: &Json, path: &str) -> Json {
	match value {
		Json::Object(map) => Json::Object(
			map.iter()
				.filter(|(k, _)| k.as_str() != "raw")
				.map(|(k, v)| {
					(
						k.clone(),
						project_observation(v, &expected[k], &format!("{path}/{k}")),
					)
				})
				.collect(),
		),
		Json::Array(array) => Json::Array(
			array
				.iter()
				.enumerate()
				.map(|(i, v)| project_observation(v, &expected[i], &format!("{path}/{i}")))
				.collect(),
		),
		Json::String(text) if text.contains("full_name:") => {
			assert!(path.ends_with("/kind"), "P1 outside typed kind: {path}");
			let mut projected = String::new();
			let mut tail = text.as_str();
			let mut count = 0;
			while let Some(i) = tail.find("full_name:") {
				assert!(tail[..i].ends_with("AnyTypeInfo { "));
				projected.push_str(&tail[..i]);
				let digits = tail[i..].strip_prefix("full_name: 0x").unwrap();
				let len = digits
					.bytes()
					.take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
					.count();
				assert!(len > 0 && digits[len..].starts_with(", hash: 0x"));
				projected.push_str("full_name: <pointer>");
				tail = &digits[len..];
				count += 1;
			}
			projected.push_str(tail);
			assert_eq!(
				count,
				expected
					.as_str()
					.unwrap()
					.matches("full_name: <pointer>")
					.count()
			);
			json!(projected)
		}
		_ => value.clone(),
	}
}

fn compare_fresh(case: &Json, frozen: &Json, actual: &Json) {
	for key in ["label", "source_sha256", "fixture", "fuel"] {
		assert_eq!(actual[key], case[key]);
		assert_eq!(actual[key], frozen[key]);
	}
	assert_eq!(
		actual["source_hex"],
		hex(case["source"].as_str().unwrap().as_bytes())
	);
	let audit = &actual["fresh_audit"];
	if case["id"] == "064" {
		assert_eq!(audit["fixture_cleanup"], json!({"cycle_cleared":true}));
	} else {
		assert!(audit["fixture_cleanup"].is_null());
	}
	assert!(audit["cleanup_error"].is_null());
	for key in ["row_idle", "executor_idle"] {
		assert_eq!(audit[key], true);
	}
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"live_entries",
		"payload_bytes",
	] {
		assert_eq!(audit[key], 0, "{key}");
	}
	let stock: Json = serde_json::from_str(STOCK).unwrap();
	let id = case["id"].as_str().unwrap();
	let intentional =
		super::nominal_clone::intentional_json("constructors", case, &stock[id]["observation"]);
	let mut expected = intentional
		.as_ref()
		.unwrap_or(&stock[id]["observation"])
		.clone();
	let d1_observed = super::method_diagnostics::intentional_json_before(
		"constructors",
		case["label"].as_str().unwrap(),
		case["source"].as_str().unwrap(),
		"fresh",
		&actual["observation"],
	);
	let mut d1b_observed = d1_observed.clone();
	d1b_observed["outcome"] = super::d1b_diagnostics::intentional_json_before(
		"constructors",
		case["label"].as_str().unwrap(),
		case["source"].as_str().unwrap(),
		"fresh",
		&d1_observed["outcome"],
	);
	let observed = &d1b_observed;
	if let Some(sites) = super::d1b_diagnostics::intentional_sites_before(
		"constructors",
		case["label"].as_str().unwrap(),
		case["source"].as_str().unwrap(),
		"fresh",
	) {
		assert_eq!(observed["outcome"]["sites"], sites);
		expected["outcome"]["sites"] = sites;
	}
	let expected = &expected;
	let admissions: Json = serde_json::from_str(include_str!("c15b_admissions.json")).unwrap();
	if let Some(rule) = admissions.get(id) {
		assert_eq!(rule["source_sha256"], case["source_sha256"]);
		assert_eq!(frozen["disposition"], "Refused-surface");
		assert_eq!(audit["invoked"], true);
		assert_eq!(audit["reuse_used_fallback"], false);
		assert_eq!(project_observation(observed, expected, ""), *expected);
		return;
	}
	let c16f: Json = serde_json::from_str(include_str!("c16f_admissions.json")).unwrap();
	assert_eq!(c16f.as_object().unwrap().len(), 2);
	if let Some(rule) = c16f.get(id) {
		use sha2::{Digest, Sha256};
		assert!(matches!(id, "116" | "117"));
		assert_eq!(rule["authority"], 21337);
		assert_eq!(rule["source_sha256"], case["source_sha256"]);
		assert_eq!(hex(Sha256::digest(serde_json::to_vec(&stock[id]).unwrap()).as_ref()), rule["stock_row_sha256"]);
		assert_eq!(frozen["disposition"], "Refused-surface");
		assert_eq!(audit["invoked"], true);
		assert_eq!(audit["reuse_used_fallback"], false);
		assert_eq!(project_observation(observed, expected, ""), *expected);
		return;
	}
	if frozen["disposition"] == "Refused-frontend-arity" {
		assert_eq!(audit["invoked"], false);
		assert_eq!(actual["diagnostics"], stock[id]["diagnostics"]);
		assert_eq!(observed["events"], json!([]));
		assert_eq!(observed["reuse"]["graph"]["nodes"][0][2], "7");
		return;
	}
	assert_eq!(observed["reuse"], expected["reuse"]);
	let refused = frozen["disposition"]
		.as_str()
		.unwrap()
		.starts_with("Refused-");
	if refused {
		assert_eq!(audit["invoked"], false);
		assert_eq!(audit["reuse_used_fallback"], true);
		assert_eq!(observed["outcome"]["status"], "refused");
		let refusal = observed["outcome"]["refusal"].as_str().unwrap();
		assert!(matches!(
			refusal,
			"Method"
				| "UnresolvedFunction"
				| "ObjectConstructor"
				| "Pattern" | "Match"
				| "Call" | "Native"
		));
		assert_eq!(observed["events"], json!([]));
	} else {
		assert_eq!(audit["invoked"], true);
		assert_eq!(audit["reuse_used_fallback"], false);
		if frozen["disposition"] == "Retained-fuel" {
			let seen = observed["events"].as_array().unwrap();
			let complete: Json =
				serde_json::from_str(include_str!("resource_complete.json")).unwrap();
			let rule = complete
				.get(id)
				.expect("Authorized COMPLETE stock tape required");
			assert_eq!(rule["source_sha256"], case["source_sha256"]);
			assert_eq!(rule["stock_fuel"], 200000);
			assert_eq!(rule["fresh_fuel"], case["fuel"]);
			assert_eq!(rule["observation"]["outcome"]["status"], "value");
			let tape = rule["observation"]["events"].as_array().unwrap();
			assert!(seen.len() <= tape.len() && seen.iter().zip(tape).all(|(a, b)| a == b));
			if observed["outcome"]["class"] == "FuelExhausted" {
				assert_eq!(observed["outcome"]["status"], "error");
			} else {
				assert_eq!(
					project_observation(observed, &rule["observation"], ""),
					rule["observation"]
				);
			}
		} else {
			assert_eq!(
				project_observation(observed, expected, ""),
				*expected,
				"C15 row {id}"
			);
		}
	}
}

#[test]
fn contract_controls_reject_output_tape_and_lifecycle_drift() {
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	for i in [0usize, 131, 125] {
		TAPE.with(|t| t.borrow_mut().clear());
		let actual = observe_fresh(&cases[i]);
		let id = cases[i]["id"].as_str().unwrap();
		compare_fresh(&cases[i], &frozen[id], &actual);
		for path in [
			"/observation/reuse/graph/nodes/0/2",
			"/fresh_audit/live_entries",
			"/fresh_audit/row_payload_bytes",
			"/observation/events",
		] {
			let mut bad = actual.clone();
			*bad.pointer_mut(path).unwrap() = json!("drift");
			assert!(
				std::panic::catch_unwind(|| compare_fresh(&cases[i], &frozen[id], &bad)).is_err()
			);
		}
	}
}
