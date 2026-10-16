// Frozen P1: canonical AnyTypeInfo function-name addresses only.
fn p1_path(path: &str) -> bool {
	let rest = path
		.strip_prefix("/observation/outcome/")
		.or_else(|| path.strip_prefix("/observation/reuse/"));
	let Some(rest) = rest else {
		return false;
	};
	if matches!(rest, "class" | "kind" | "raw") {
		return true;
	}
	let Some(rest) = rest.strip_prefix("chain/") else {
		return false;
	};
	let Some((index, field)) = rest.split_once('/') else {
		return false;
	};
	!index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) && field == "kind"
}
fn p1_text(text: &str, expected: usize) -> Result<String, String> {
	let mut result = String::new();
	let mut cursor = 0;
	let mut substitutions = 0;
	while let Some(relative) = text[cursor..].find("full_name:") {
		let start = cursor + relative;
		if !text[..start].ends_with("AnyTypeInfo { ") || !text[start..].starts_with("full_name: 0x")
		{
			return Err("noncanonical pointer context".into());
		}
		let digits = start + "full_name: 0x".len();
		let length = text.as_bytes()[digits..]
			.iter()
			.take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
			.count();
		if length == 0 {
			return Err("empty pointer".into());
		}
		let end = digits + length;
		let Some(tail) = text[end..].strip_prefix(", hash: 0x") else {
			return Err("missing type hash".into());
		};
		let hash_len = tail
			.bytes()
			.take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
			.count();
		if hash_len == 0 || !tail[hash_len..].starts_with(" }") {
			return Err("invalid type hash context".into());
		}
		result.push_str(&text[cursor..start]);
		result.push_str("full_name: <fn>");
		cursor = end;
		substitutions += 1;
	}
	result.push_str(&text[cursor..]);
	if substitutions != expected {
		return Err("pointer substitution count drift".into());
	}
	Ok(result)
}
fn project_p1(value: &Json, counts: &Json) -> Result<Json, String> {
	fn walk(
		value: &Json,
		path: &str,
		counts: &Json,
		seen: &mut std::collections::BTreeMap<String, usize>,
	) -> Result<Json, String> {
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
					.collect::<Result<Vec<_>, _>>()?,
			),
			Json::String(text) if text.contains("full_name:") => {
				if !p1_path(path) {
					return Err("pointer outside approved Debug path".into());
				}
				let expected = counts
					.get(path)
					.and_then(Json::as_u64)
					.ok_or("unlisted pointer field")?;
				let expected = usize::try_from(expected).map_err(|_| "count overflow")?;
				seen.insert(path.into(), expected);
				Json::String(p1_text(text, expected)?)
			}
			_ => value.clone(),
		})
	}
	let mut seen = std::collections::BTreeMap::new();
	let result = walk(value, "", counts, &mut seen)?;
	if serde_json::to_value(seen).map_err(|_| "count serialization")? != *counts {
		return Err("pointer field set drift".into());
	}
	Ok(result)
}
#[test]
fn p1_controls() {
	let a = json!({"observation":{"outcome":{"kind":"ExpectedType { actual: Any(AnyTypeInfo { full_name: 0xabc, hash: 0x55 }) }"}}});
	let counts = json!({"/observation/outcome/kind":1});
	let mut b = a.clone();
	b["observation"]["outcome"]["kind"] = Json::String(
		a["observation"]["outcome"]["kind"]
			.as_str()
			.unwrap()
			.replace("0xabc", "0xdef"),
	);
	assert_eq!(
		project_p1(&a, &counts).unwrap(),
		project_p1(&b, &counts).unwrap()
	);
	for (old, new) in [("hash: 0x55", "hash: 0x56"), ("ExpectedType", "OtherType")] {
		let mut changed = a.clone();
		changed["observation"]["outcome"]["kind"] = Json::String(
			a["observation"]["outcome"]["kind"]
				.as_str()
				.unwrap()
				.replace(old, new),
		);
		assert_ne!(
			project_p1(&a, &counts).unwrap(),
			project_p1(&changed, &counts).unwrap()
		);
	}
	assert!(project_p1(&a, &json!({})).is_err());
	assert!(project_p1(&a, &json!({"/observation/outcome/kind":2})).is_err());
	assert!(p1_text("user full_name: 0xabc", 1).is_err());
	assert!(p1_text("AnyTypeInfo { full_name: 0x, hash: 0x55 }", 1).is_err());
	assert!(project_p1(
		&json!({"events":[a["observation"]["outcome"]["kind"]]}),
		&counts
	)
	.is_err());
	let hashes = json!({"hash":"0xabc","sites":[1,2],"value":"0x55"});
	assert_eq!(project_p1(&hashes, &json!({})).unwrap(), hashes);
}
