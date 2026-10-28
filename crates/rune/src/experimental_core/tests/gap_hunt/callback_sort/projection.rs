fn p1_debug(text: &str, expected: usize) -> std::result::Result<String, String> {
	let mut result = String::new();
	let mut cursor = 0;
	let mut substitutions = 0;
	while let Some(relative) = text[cursor..].find("full_name:") {
		let start = cursor + relative;
		let value = start + "full_name: ".len();
		if !text[..start].ends_with("AnyTypeInfo { ") || !text[start..].starts_with("full_name: 0x")
		{
			return Err("noncanonical full_name or wrong context".into());
		}
		let digits = value + 2;
		let length = text.as_bytes()[digits..]
			.iter()
			.take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
			.count();
		if length == 0 {
			return Err("empty pointer".into());
		}
		let end = digits + length;
		let Some(tail) = text[end..].strip_prefix(", hash: 0x") else {
			return Err("missing exact hash context".into());
		};
		let hash_length = tail
			.bytes()
			.take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
			.count();
		if hash_length == 0 || !tail[hash_length..].starts_with(" }") {
			return Err("invalid type hash context".into());
		}
		result.push_str(&text[cursor..start]);
		result.push_str("full_name: <fn>");
		cursor = end;
		substitutions += 1;
	}
	result.push_str(&text[cursor..]);
	if substitutions != expected {
		return Err("P1 substitution count".into());
	}
	// A field carrying hexadecimal data must be the approved fn pointer or a hash.
	// Unknown fields are rejected, never erased or guessed from address length.
	let bytes = text.as_bytes();
	for (colon, byte) in bytes.iter().enumerate() {
		if *byte != b':' {
			continue;
		}
		let mut value = colon + 1;
		while value < bytes.len() && bytes[value].is_ascii_whitespace() {
			value += 1;
		}
		if !(text[value..].starts_with("0x") || text[value..].starts_with("0X")) {
			continue;
		}
		let mut start = colon;
		while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
			start -= 1;
		}
		let name = &text[start..colon];
		if name
			.as_bytes()
			.first()
			.is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
			&& !matches!(name, "full_name" | "hash" | "variant_hash")
		{
			return Err("other pointer-like field".into());
		}
	}
	Ok(result)
}

fn project_p1(observation: &Json, counts: &Json) -> Result<Json, String> {
	fn walk(
		value: &Json,
		path: &str,
		counts: &serde_json::Map<String, Json>,
		seen: &mut Vec<String>,
	) -> Result<Json, String> {
		if let Some(count) = counts.get(path) {
			if !matches!(
				path,
				"/outcome/kind"
					| "/outcome/raw"
					| "/outcome/typed/actual_debug"
					| "/outcome/typed/expected_debug"
					| "/outcome/typed/raw_kind"
			) {
				return Err("unlisted P1 path".into());
			}
			let count = count.as_u64().ok_or("P1 count")?;
			if count == 0 {
				return Err("zero P1 count".into());
			}
			seen.push(path.into());
			return Ok(json!(p1_debug(
				value.as_str().ok_or("P1 string")?,
				usize::try_from(count).map_err(|_| "P1 count overflow")?
			)?));
		}
		Ok(match value {
			Json::Object(map) => {
				let mut out = serde_json::Map::new();
				for (k, v) in map {
					out.insert(k.clone(), walk(v, &format!("{path}/{k}"), counts, seen)?);
				}
				Json::Object(out)
			}
			Json::Array(values) => Json::Array(
				values
					.iter()
					.enumerate()
					.map(|(i, v)| walk(v, &format!("{path}/{i}"), counts, seen))
					.collect::<Result<Vec<_>, _>>()?,
			),
			Json::String(s) => {
				if s.contains("full_name:") {
					return Err("P1 outside declared path".into());
				}
				value.clone()
			}
			_ => value.clone(),
		})
	}
	let counts = counts.as_object().ok_or("P1 counts object")?;
	let mut seen = Vec::new();
	let result = walk(observation, "", counts, &mut seen)?;
	if seen.len() != counts.len() || !seen.iter().all(|p| counts.contains_key(p)) {
		return Err("P1 field set".into());
	}
	Ok(result)
}
fn common(observation: &Json) -> Json {
	let mut out = serde_json::Map::new();
	for k in [
		"outcome",
		"before",
		"post",
		"events",
		"events_after_reuse",
		"reuse",
		"invoked",
	] {
		out.insert(k.into(), observation[k].clone());
	}
	if let Some(outcome) = out.get_mut("outcome").and_then(Json::as_object_mut) {
		outcome.remove("raw");
	}
	Json::Object(out)
}
fn canonical(value: &Json) -> Vec<u8> {
	fn sorted(value: &Json) -> Json {
		match value {
			Json::Object(map) => {
				let mut keys = map.keys().collect::<Vec<_>>();
				keys.sort();
				let mut out = serde_json::Map::new();
				for key in keys {
					out.insert(key.clone(), sorted(&map[key]));
				}
				Json::Object(out)
			}
			Json::Array(values) => Json::Array(values.iter().map(sorted).collect()),
			_ => value.clone(),
		}
	}
	serde_json::to_vec(&sorted(value)).unwrap()
}
fn digest(bytes: &[u8]) -> String {
	use sha2::Digest as _;
	hex(&sha2::Sha256::digest(bytes))
}
fn check_cell(stored: &Json, actual: &Json) -> Result<(), String> {
	if stored.get("storage") == Some(&json!("canonical-json-sha256")) {
		let object = stored.as_object().ok_or("storage object")?;
		if object.len() != 3
			|| !["storage", "bytes", "sha256"]
				.iter()
				.all(|k| object.contains_key(*k))
		{
			return Err("storage schema".into());
		}
		let bytes = canonical(actual);
		if stored["bytes"] != json!(bytes.len()) || stored["sha256"] != digest(&bytes) {
			return Err("canonical digest drift".into());
		}
	} else if stored != actual {
		return Err("portable semantic drift".into());
	}
	Ok(())
}
fn decode(text: &str) -> Vec<u8> {
	assert!(text.len().is_multiple_of(2));
	text.as_bytes()
		.as_chunks::<2>()
		.0
		.iter()
		.map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
		.collect()
}
fn resource_valid(stock: &Json, execution: &Json, audit: &Json, kind: &str) -> bool {
	if stock["outcome"]["status"] != "value" {
		return false;
	}
	if !matches!(kind, "FuelExhausted" | "Cancelled") {
		return false;
	}
	let clean = json!({"typed_failure_kind":kind,"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
	if execution == stock && stock["outcome"]["status"] == "value" {
		return audit
			== &json!({"typed_failure_kind":null,"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
	}
	let (Some(tape), Some(baseline)) = (execution["events"].as_array(), stock["events"].as_array())
	else {
		return false;
	};
	let mut after = tape.clone();
	after.push(json!({"event":"mark","hex":"7265757365"}));
	execution["outcome"]["status"] == "error"
		&& execution["outcome"]["class"] == kind
		&& audit == &clean
		&& execution["before"] == stock["before"]
		&& execution["post"] == stock["before"]
		&& tape.len() <= baseline.len()
		&& tape == &baseline[..tape.len()]
		&& execution["reuse"] == stock["reuse"]
		&& execution["invoked"] == true
		&& execution["events_after_reuse"] == json!(after)
}
fn refusal_valid(label: &str, source: &str, execution: &Json, audit: &Json) -> bool {
	let rows: Json =
		serde_json::from_str(include_str!("../../callback_sort_refusals.json")).unwrap();
	let Some(row) = rows.get(label) else {
		return false;
	};
	let dispositions: Json =
		serde_json::from_str(include_str!("../../callback_sort_dispositions.json")).unwrap();
	let expected = &row["fresh_constraint"];
	let Some((_, pinned, _, _, _, _, _)) = CASES.iter().find(|c| c.0 == label) else {
		return false;
	};
	if source != *pinned || digest(source.as_bytes()) != dispositions[label]["source_sha256"] {
		return false;
	}
	execution["refusal"]
		== if expected["invoked"] == false {
			expected["operation"].clone()
		} else {
			Json::Null
		} && execution["status"] == expected["status"]
		&& execution["kind"] == expected["kind"]
		&& execution["operation"] == expected["operation"]
		&& execution["source_sha256"] == expected["source_sha256"]
		&& execution["invoked"] == expected["invoked"]
		&& audit
			== &json!({"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7})
}

// Bind the live completion to the reviewed source and fixture, independent of fuel parity.
fn resource_completion_live(
	label: &str,
	source: &str,
	input_fixture: &str,
	current: &Json,
	stock: &Json,
) -> Result<(Json, Json), String> {
	let anchors = read_json(include_str!("../../callback_sort_resource_anchors.json"));
	let binding = anchors.get(label).ok_or("unbound resource row")?;
	let (index, case) = CASES
		.iter()
		.enumerate()
		.find(|(_, c)| c.0 == label)
		.ok_or("resource case")?;
	let anchor_label = if label.starts_with("fuel/") {
		"fuel/128"
	} else if label.starts_with("cancel/") {
		label
	} else {
		return Err("resource label".into());
	};
	let (anchor_index, anchor_case) = CASES
		.iter()
		.enumerate()
		.find(|(_, c)| c.0 == anchor_label)
		.ok_or("completion case")?;
	if source != case.1
		|| input_fixture != case.2
		|| source != anchor_case.1
		|| input_fixture != anchor_case.2
		|| case.4 != anchor_case.4
		|| stock["outcome"]["status"] != "value"
		|| stock["events"].as_array().ok_or("complete tape")?.len() != 12
		|| binding["index"] != json!(index)
		|| binding["source_sha256"] != digest(source.as_bytes())
		|| binding["fixture"] != input_fixture
		|| binding["anchor_label"] != anchor_label
		|| binding["anchor_index"] != json!(anchor_index)
		|| binding["anchor_source_sha256"] != digest(anchor_case.1.as_bytes())
		|| current["before"] != stock["before"]
	{
		return Err("resource completion identity drift".into());
	}
	Ok((stock.clone(), binding.clone()))
}
