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
	let expected = &stock[id];
	let d1_observed = super::method_diagnostics::intentional_json_before(
		"text_methods",
		case["label"].as_str().unwrap(),
		case["source"].as_str().unwrap(),
		"fresh",
		&actual["observation"],
	);
	let observed = &d1_observed;
	assert_eq!(observed["reuse"], expected["reuse"]);
	let receivers: Json = serde_json::from_str(include_str!("receiver_policy.json")).unwrap();
	let admissions: Json = serde_json::from_str(include_str!("c14b_admissions.json")).unwrap();
	if let Some(rule) = admissions.get(id) {
		assert_eq!(rule["source_sha256"], case["source_sha256"]);
		assert_eq!(rule["label"], case["label"]);
		assert_eq!(rule["policy"], "Exact-stock-admission");
	}
	let refused = admissions.get(id).is_none()
		&& frozen["disposition"]
			.as_str()
			.unwrap()
			.starts_with("Refused-")
		|| receivers.get(id).is_some();
	if refused {
		if let Some(rule) = receivers.get(id) {
			assert_eq!(rule["source_sha256"], case["source_sha256"]);
		}
		assert_eq!(audit["invoked"], false);
		assert_eq!(audit["reuse_used_fallback"], true);
		assert_eq!(observed["outcome"]["status"], "refused");
		let refusal = observed["outcome"]["refusal"].as_str().unwrap();
		assert!(
			refusal == "Method" || (frozen["disposition"] == "Refused-arity" && refusal == "Arity")
		);
		assert_eq!(observed["events"], json!([]));
	} else {
		assert_eq!(audit["invoked"], true);
		assert_eq!(audit["reuse_used_fallback"], false);
		if id == "184" {
			let policy: Json = serde_json::from_str(include_str!("resource_policy.json")).unwrap();
			assert_eq!(policy[id]["source_sha256"], case["source_sha256"]);
			assert_eq!(policy[id]["fresh_fuel"], case["fuel"]);
			assert_eq!(*observed, policy[id]["observation"]);
		} else if id == "185" {
			assert_eq!(observed["outcome"]["status"], "error");
			assert_eq!(observed["outcome"]["class"], "FuelExhausted");
			assert_eq!(observed["events"], expected["events"]);
		} else {
			assert_eq!(
				project_observation(observed, expected, ""),
				*expected,
				"C14 row {id}"
			);
		}
	}
}

#[test]
fn contract_controls_reject_output_tape_and_lifecycle_drift() {
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	for index in [0usize, 184, 185] {
		TAPE.with(|t| t.borrow_mut().clear());
		let actual = observe_fresh(&cases[index]);
		let id = cases[index]["id"].as_str().unwrap();
		compare_fresh(&cases[index], &frozen[id], &actual);
		for path in [
			"/observation/reuse/graph/nodes/0/2",
			"/fresh_audit/live_entries",
			"/observation/events",
		] {
			let mut bad = actual.clone();
			*bad.pointer_mut(path).unwrap() = json!("drift");
			assert!(
				std::panic::catch_unwind(|| compare_fresh(&cases[index], &frozen[id], &bad))
					.is_err()
			);
		}
	}
}
