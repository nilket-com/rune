fn c10_p1_debug(text: &str, expected: usize) -> std::result::Result<String, String> {
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

fn c10_project(value: &Json, index: usize) -> Result<Json, String> {
	fn walk(
		value: &Json,
		path: &str,
		index: usize,
		seen: &mut Vec<String>,
	) -> Result<Json, String> {
		Ok(match value {
			Json::Object(map) => {
				let mut out = serde_json::Map::new();
				for (k, v) in map {
					out.insert(k.clone(), walk(v, &format!("{path}/{k}"), index, seen)?);
				}
				Json::Object(out)
			}
			Json::Array(values) => Json::Array(
				values
					.iter()
					.enumerate()
					.map(|(i, v)| walk(v, &format!("{path}/{i}"), index, seen))
					.collect::<Result<Vec<_>, _>>()?,
			),
			Json::String(text) => {
				if index == 123 && matches!(path, "/outcome/kind" | "/outcome/raw") {
					seen.push(path.into());
					Json::String(c10_p1_debug(text, 2)?)
				} else {
					if text.contains("full_name:") {
						return Err(format!("unapproved full_name row/path {index} {path}"));
					}
					value.clone()
				}
			}
			_ => value.clone(),
		})
	}
	let mut seen = Vec::new();
	let result = walk(value, "", index, &mut seen)?;
	seen.sort();
	let expected = if index == 123 {
		vec!["/outcome/kind".to_owned(), "/outcome/raw".to_owned()]
	} else {
		vec![]
	};
	if seen != expected {
		return Err("P1 Debug field set drift".into());
	}
	Ok(result)
}
fn c10_common(portable: &Json) -> Result<Json, String> {
	let keys = portable
		.as_object()
		.ok_or("observation not object")?
		.keys()
		.map(String::as_str)
		.collect::<std::collections::BTreeSet<_>>();
	let expected = [
		"cleanup",
		"diagnostics",
		"events",
		"events_after_reuse",
		"host_calls",
		"host_sites",
		"invoked",
		"outcome",
		"protocol_calls",
		"reuse",
	]
	.into_iter()
	.collect::<std::collections::BTreeSet<_>>();
	if keys != expected {
		return Err("unknown/missing observation field".into());
	}
	let mut out = portable.clone();
	let map = out.as_object_mut().unwrap();
	map.remove("host_sites");
	map.remove("cleanup");
	if map["outcome"]["status"] == "error" {
		map.get_mut("outcome")
			.unwrap()
			.as_object_mut()
			.unwrap()
			.remove("raw")
			.ok_or("missing error raw")?;
	}
	Ok(out)
}
