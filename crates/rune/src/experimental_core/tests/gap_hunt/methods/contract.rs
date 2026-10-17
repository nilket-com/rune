const DISPOSITIONS: &str = include_str!("dispositions.json");
const RETAINED: &str = include_str!("retained.json");

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
	assert_eq!(audit["row_idle"], true);
	assert_eq!(audit["executor_idle"], true);
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"live_entries",
		"payload_bytes",
	] {
		assert_eq!(audit[key], 0, "{key}");
	}
	let observed = &actual["observation"];
	let c14: Json = serde_json::from_str(include_str!("c14_admissions.json")).unwrap();
	if let Some(rule) = c14.get(case["id"].as_str().unwrap()) {
		assert_eq!(rule["source_sha256"], case["source_sha256"]);
		assert_eq!(frozen["disposition"], "Refused-surface");
		assert_eq!(audit["invoked"], true);
		assert_eq!(audit["reuse_used_fallback"], false);
		let expected = &frozen["stock_observation"];
		assert_eq!(project_observation(observed, expected, ""), *expected);
		return;
	}
	match frozen["disposition"].as_str().unwrap() {
		"Refused-surface" => {
			assert_eq!(audit["invoked"], false);
			assert_eq!(observed["outcome"]["status"], "refused");
			assert_eq!(observed["outcome"]["refusal"], frozen["fresh_refusal"]);
			let at = &observed["outcome"]["location"];
			let expected = &frozen["fresh_refusal_sites"][0];
			for key in ["source", "start", "end"] {
				assert_eq!(at[key], expected[key]);
			}
			assert_eq!(observed["events"], json!([]));
			assert_eq!(observed["reuse"], frozen["stock_observation"]["reuse"]);
		}
		"Exact" => {
			assert_eq!(audit["invoked"], true);
			let expected = &frozen["fresh_expectation"];
			let mut effective = project_observation(observed, expected, "");
			let retained: Json = serde_json::from_str(RETAINED).unwrap();
			if let Some(rule) = retained.get(case["id"].as_str().unwrap()) {
				assert_eq!(rule["source_sha256"], case["source_sha256"]);
				let frames: Vec<Json> = serde_json::from_str(
					effective["outcome"]["sites"][0]["frames"].as_str().unwrap(),
				)
				.unwrap();
				assert_eq!(frames.len(), rule["stock_count"].as_u64().unwrap() as usize);
				assert_eq!(
					frames.len(),
					expected["outcome"]["sites"][0]["frames"]
						.as_str()
						.unwrap()
						.matches("CallFrame {")
						.count()
				);
				assert_eq!(frames.len(), rule["calls"].as_array().unwrap().len());
				let source = case["source"].as_str().unwrap();
				for (frame, call) in frames.iter().zip(rule["calls"].as_array().unwrap()) {
					assert_eq!(frame["source"], 0);
					let start = frame["start"].as_u64().unwrap() as usize;
					let end = frame["end"].as_u64().unwrap() as usize;
					assert!(start < end && source.get(start..end).is_some());
					assert!(
						start <= call["start"].as_u64().unwrap() as usize
							&& end >= call["end"].as_u64().unwrap() as usize
					);
					assert_eq!(
						source
							.get(
								call["start"].as_u64().unwrap() as usize
									..call["end"].as_u64().unwrap() as usize
							)
							.unwrap(),
						call["expression"].as_str().unwrap()
					);
				}
				effective["outcome"]["sites"][0]["frames"] =
					expected["outcome"]["sites"][0]["frames"].clone();
			}
			assert_eq!(effective, *expected, "C13 row {}", case["id"]);
		}
		other => panic!("unreviewed policy {other}"),
	}
}

#[test]
fn retained_frame_controls() {
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let actual = observe_fresh(&cases[0]);
	compare_fresh(&cases[0], &frozen["000"], &actual);
	for frames in [
		json!([]),
		json!([{"source":0,"start":0,"end":1}]),
		json!([{"source":0,"start":40,"end":999999}]),
	] {
		let mut bad = actual.clone();
		bad["observation"]["outcome"]["sites"][0]["frames"] =
			json!(serde_json::to_string(&frames).unwrap());
		assert!(
			std::panic::catch_unwind(|| compare_fresh(&cases[0], &frozen["000"], &bad)).is_err()
		);
	}
}
