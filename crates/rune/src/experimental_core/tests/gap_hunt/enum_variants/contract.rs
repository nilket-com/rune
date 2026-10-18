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

fn d1_enum_expectation(case: &Json, expected: &Json, engine: &str) -> Json {
	let rows: Vec<Json> = serde_json::from_str(include_str!("d1_diagnostics.json")).unwrap();
	let mut expected = expected.clone();
	if let Some(row) = rows.iter().find(|row| row["id"] == case["id"]) {
		assert_eq!(row["source_sha256"], case["source_sha256"]);
		assert_eq!(
			expected["outcome"]["display"],
			row["changes"][engine]["before"]
		);
		expected["outcome"]["display"] = row["changes"][engine]["after"].clone();
	}
	expected
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
	assert!(audit["fixture_cleanup"].is_null());
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
		super::nominal_clone::intentional_json("enum_variants", case, &stock[id]["observation"]);
	let expected = intentional.as_ref().unwrap_or(&stock[id]["observation"]);
	let observed = &actual["observation"];
	if frozen["disposition"] == "Refused-frontend" {
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
				| "TypeExpr" | "DepthLimit"
				| "AsyncBlock"
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
			let intentional: Json = serde_json::from_str(include_str!("intentional.json")).unwrap();
			let expected = intentional
				.get(id)
				.map_or(expected, |rule| &rule["expected_observation"]);
			let mut d1_expected = d1_enum_expectation(case, expected, "fresh");
			let mut observed = observed.clone();
			let d1b_outcome = super::d1b_diagnostics::intentional_json_before(
				"enum_variants",
				case["label"].as_str().unwrap(),
				case["source"].as_str().unwrap(),
				"fresh",
				&observed["outcome"],
			);
			observed["outcome"] = d1b_outcome;
			if let Some(sites) = super::d1b_diagnostics::intentional_sites_before(
				"enum_variants",
				case["label"].as_str().unwrap(),
				case["source"].as_str().unwrap(),
				"fresh",
			) {
				assert_eq!(observed["outcome"]["sites"], sites);
				d1_expected["outcome"]["sites"] = sites;
			}
			let expected = &d1_expected;
			assert_eq!(
				project_observation(&observed, expected, ""),
				*expected,
				"C15b row {id}"
			);
		}
	}
}

fn compare_stock(case: &Json, actual: &Json) {
	let stock: Json = serde_json::from_str(STOCK).unwrap();
	let rules: Json = serde_json::from_str(include_str!("intentional.json")).unwrap();
	let id = case["id"].as_str().unwrap();
	for key in [
		"label",
		"source_sha256",
		"fixture",
		"fuel",
		"diagnostics",
		"source_hex",
	] {
		assert_eq!(actual[key], stock[id][key], "{id} {key}");
	}
	let intentional =
		super::nominal_clone::intentional_json("enum_variants", case, &stock[id]["observation"]);
	let expected = rules.get(id).map_or_else(
		|| intentional.as_ref().unwrap_or(&stock[id]["observation"]),
		|rule| &rule["expected_observation"],
	);
	let d1_expected = d1_enum_expectation(case, expected, "stock");
	let expected = &d1_expected;
	let mut observed = actual["observation"].clone();
	let d1b_outcome = super::d1b_diagnostics::intentional_json_before(
		"enum_variants",
		case["label"].as_str().unwrap(),
		case["source"].as_str().unwrap(),
		"stock",
		&observed["outcome"],
	);
	observed["outcome"] = d1b_outcome;
	assert_eq!(
		project_observation(&observed, expected, ""),
		*expected,
		"C15b stock row {id}"
	);
}
