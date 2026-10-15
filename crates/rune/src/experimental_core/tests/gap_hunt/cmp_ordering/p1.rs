// P1, seq14523: only full_name: 0x[0-9a-f]+ -> full_name: <fn>.
// Full VM raw is audit-only (C8 boundary); common Debug counts stay exact.
fn debug_path(path: &str) -> bool {
	if matches!(path, "/kind" | "/raw" | "/typed/raw_kind") {
		return true;
	}
	let Some(rest) = path.strip_prefix("/chain/") else {
		return false;
	};
	let Some((index, tail)) = rest.split_once('/') else {
		return false;
	};
	!index.is_empty()
		&& index.bytes().all(|b| b.is_ascii_digit())
		&& matches!(tail, "kind" | "typed/raw_kind")
}

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

fn project_p1(value: &Json, counts: &Json) -> std::result::Result<Json, String> {
	fn walk(
		value: &Json,
		path: &str,
		counts: &Json,
		seen: &mut Vec<String>,
	) -> std::result::Result<Json, String> {
		Ok(match value {
			Json::Object(map) => {
				let mut out = serde_json::Map::new();
				for (key, value) in map {
					out.insert(
						key.clone(),
						walk(value, &format!("{path}/{key}"), counts, seen)?,
					);
				}
				Json::Object(out)
			}
			Json::Array(array) => Json::Array(
				array
					.iter()
					.enumerate()
					.map(|(i, value)| walk(value, &format!("{path}/{i}"), counts, seen))
					.collect::<std::result::Result<Vec<_>, _>>()?,
			),
			Json::String(text) if debug_path(path) => {
				let count = counts
					.get(path)
					.and_then(Json::as_u64)
					.ok_or("unexpected Debug field")?;
				seen.push(path.into());
				Json::String(p1_debug(
					text,
					usize::try_from(count).map_err(|_| "count overflow")?,
				)?)
			}
			Json::String(text) => {
				if text.contains("full_name: 0x") {
					return Err("P1 token outside Debug".into());
				}
				value.clone()
			}
			_ => value.clone(),
		})
	}
	let mut seen = Vec::new();
	let result = walk(value, "", counts, &mut seen)?;
	let fields = counts.as_object().ok_or("count manifest not object")?;
	if fields.len() != seen.len() || !seen.iter().all(|name| fields.contains_key(name)) {
		return Err("Debug field set drift".into());
	}
	Ok(result)
}

fn semantic_projection(value: &Json) -> std::result::Result<Json, String> {
	let mut value = value.clone();
	match value["status"].as_str() {
		Some("value") => {
			if value.as_object().unwrap().len() != 2 || value.get("value").is_none() {
				return Err("value schema".into());
			}
		}
		Some("error") => {
			let object = value.as_object_mut().unwrap();
			object.remove("raw");
			let names = [
				"status", "class", "kind", "typed", "display", "chain", "sites",
			];
			if object.len() != names.len() || !names.iter().all(|k| object.contains_key(*k)) {
				return Err("error schema".into());
			}
			if object["class"]
				.as_str()
				.is_none_or(|s| s == "UNMAPPED" || s.starts_with("unmapped"))
			{
				return Err("unmapped error".into());
			}
		}
		_ => return Err("unmapped or unexpected semantic status".into()),
	}
	Ok(value)
}

fn compare_p1(raw: &Json, expected: &Json, counts: &Json) -> std::result::Result<(), String> {
	let mut common = raw.clone();
	let mut counts = counts.clone();
	common
		.as_object_mut()
		.ok_or("outcome object")?
		.remove("raw");
	counts
		.as_object_mut()
		.ok_or("counts object")?
		.remove("/raw");
	let actual = semantic_projection(&project_p1(&common, &counts)?)?;
	if actual != *expected {
		return Err("exact semantic projection drift".into());
	}
	Ok(())
}
