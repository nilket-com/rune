fn c10_frozen_stock(label: &str) -> Json {
	let rows = c10_rows();
	let row = rows.iter().find(|r| r[0] == label).unwrap();
	c10_decode(row[3])
}
fn c10_sample_common(name: &str) -> Json {
	cleanup();
	STATE.with(|s| *s.borrow_mut() = State::default());
	let value = make(name);
	let tape = events();
	json!({"diagnostics":"[]","events":tape,"events_after_reuse":events(),"host_calls":1,"invoked":true,"outcome":{"status":"value","graph":graph(&value)},"protocol_calls":0,"reuse":{"status":"value","graph":graph(&Value::from(7i64))}})
}
#[test]
fn frozen_schema_and_digest_full_projection_controls() {
	c10_data_seals();
	for (label, name) in [
		("visits/plain/65534", "visits/65534"),
		("visits/plain/65535", "visits/65535"),
		("visits/plain/65536", "visits/65536"),
		("large/key-1MiB", "key/1048576"),
	] {
		let actual = c10_sample_common(name);
		let expected = c10_frozen_stock(label);
		c10_compare(&actual, &expected, label).unwrap();
	}
	// Result-wrapped fixtures have identical recorder events, plus one graph node.
	for (label, name) in [
		("visits/result/65533", "visits/65533"),
		("visits/result/65534", "visits/65534"),
		("visits/result/65535", "visits/65535"),
	] {
		cleanup();
		STATE.with(|s| *s.borrow_mut() = State::default());
		let value = crate::to_value(fallible(name)).unwrap();
		let tape = events();
		let actual = json!({"diagnostics":"[]","events":tape,"events_after_reuse":events(),"host_calls":1,"invoked":true,"outcome":{"status":"value","graph":graph(&value)},"protocol_calls":0,"reuse":{"status":"value","graph":graph(&Value::from(7i64))}});
		c10_compare(&actual, &c10_frozen_stock(label), label).unwrap();
	}
}
#[test]
fn digest_rejects_middle_byte_length_node_count_and_key_drift() {
	let label = "visits/plain/65534";
	let original = c10_sample_common("visits/65534");
	let expected = c10_frozen_stock(label);
	c10_compare(&original, &expected, label).unwrap();
	let mut changed = original.clone();
	let mut bytes = changed["outcome"]["graph"]["nodes"][32768][1]
		.as_str()
		.unwrap()
		.as_bytes()
		.to_vec();
	let end = bytes.len() - 1;
	bytes[end] = if bytes[end] == b'0' { b'1' } else { b'0' };
	changed["outcome"]["graph"]["nodes"][32768][1] =
		Json::String(String::from_utf8(bytes).unwrap());
	assert_eq!(
		c10_summary(&changed).unwrap(),
		expected["summary"],
		"middle-byte drift is outside every summary sample"
	);
	assert!(c10_compare(&changed, &expected, label).is_err());
	let mut changed = original.clone();
	changed["diagnostics"] = Json::String("[] ".into());
	assert_eq!(
		c10_canonical(&changed).len(),
		expected["bytes"].as_u64().unwrap() as usize + 1
	);
	assert!(c10_compare(&changed, &expected, label).is_err());
	let mut changed = original.clone();
	let last = changed["outcome"]["graph"]["nodes"]
		.as_array()
		.unwrap()
		.last()
		.unwrap()
		.clone();
	changed["outcome"]["graph"]["nodes"]
		.as_array_mut()
		.unwrap()
		.push(last);
	assert!(c10_compare(&changed, &expected, label).is_err());
	let label = "large/key-1MiB";
	let mut changed = c10_sample_common("key/1048576");
	let expected = c10_frozen_stock(label);
	let key = changed["outcome"]["graph"]["nodes"][0][2][0][0]
		.as_str()
		.unwrap();
	let mut bytes = c10_hex_bytes(key).unwrap();
	let middle = bytes.len() / 2;
	bytes[middle] ^= 1;
	changed["outcome"]["graph"]["nodes"][0][2][0][0] = Json::String(hex(&bytes));
	assert_eq!(
		c10_canonical(&changed).len(),
		expected["bytes"].as_u64().unwrap() as usize
	);
	assert!(c10_compare(&changed, &expected, label).is_err());
}
fn c10_pointer_control() -> Json {
	let mut raw = c10_frozen_stock("argument/wrong-type");
	for field in ["kind", "raw"] {
		raw["outcome"][field] = Json::String(
			raw["outcome"][field]
				.as_str()
				.unwrap()
				.replace("full_name: <fn>", "full_name: 0x1234"),
		);
	}
	raw
}
#[test]
fn p1_rejects_wrong_row_path_count_context_and_nonpointer_drift() {
	let raw = c10_pointer_control();
	let portable = c10_project(&raw, 123).unwrap();
	let expected = c10_common(&c10_frozen_stock("argument/wrong-type")).unwrap();
	assert_eq!(c10_common(&portable).unwrap(), expected);
	assert!(c10_project(&raw, 122).is_err());
	let mut changed = raw.clone();
	changed["outcome"]["extra"] = Json::String("AnyTypeInfo { full_name: 0x1, hash: 0x2 }".into());
	assert!(c10_project(&changed, 123).is_err());
	let mut changed = raw.clone();
	changed["outcome"].as_object_mut().unwrap().remove("raw");
	assert!(c10_project(&changed, 123).is_err());
	let mut changed = raw.clone();
	changed["outcome"]["kind"] = Json::String(
		changed["outcome"]["kind"]
			.as_str()
			.unwrap()
			.replace("AnyTypeInfo { ", "Other { "),
	);
	assert!(c10_project(&changed, 123).is_err());
	let mut changed = raw.clone();
	changed["outcome"]["kind"] = Json::String(
		changed["outcome"]["kind"].as_str().unwrap().to_owned()
			+ " AnyTypeInfo { full_name: 0x1, hash: 0x2 }",
	);
	assert!(c10_project(&changed, 123).is_err());
	assert!(
		c10_project(&portable, 123).is_err(),
		"already projected raw is rejected"
	);
	let mut changed = raw.clone();
	changed["outcome"]["kind"] = Json::String(
		changed["outcome"]["kind"]
			.as_str()
			.unwrap()
			.replace("hash: 0x2f4720d36d2b70c", "hash: 0x2f4720d36d2b70d"),
	);
	assert_ne!(
		c10_common(&c10_project(&changed, 123).unwrap()).unwrap(),
		expected,
		"typed hash is never erased"
	);
	let mut changed = raw.clone();
	changed["outcome"]["chain"][0]["kind"] = Json::String("BadArgument { arg: 1 }".into());
	assert_ne!(
		c10_common(&c10_project(&changed, 123).unwrap()).unwrap(),
		expected
	);
	let mut changed = raw.clone();
	changed["outcome"]["sites"][0]["start"] = json!(15);
	assert_ne!(
		c10_common(&c10_project(&changed, 123).unwrap()).unwrap(),
		expected
	);
	let mut changed = raw;
	changed["new_unreviewed_field"] = Json::Bool(true);
	assert!(c10_common(&c10_project(&changed, 123).unwrap()).is_err());
}
#[test]
fn comparison_keeps_graph_identity_order_effects_and_private_raw_boundary() {
	let original = c10_frozen_stock("sharing/graph/dag-vec");
	let expected = c10_common(&original).unwrap();
	let mut changed = original;
	changed["outcome"]["graph"]["nodes"][0][2][1] = json!(2);
	assert_ne!(c10_common(&changed).unwrap(), expected);
	let original = c10_frozen_stock("plain/mixed");
	let expected = c10_common(&original).unwrap();
	let mut changed = original;
	changed["outcome"]["graph"]["nodes"][0][2]
		.as_array_mut()
		.unwrap()
		.reverse();
	assert_ne!(c10_common(&changed).unwrap(), expected);
	let raw = c10_pointer_control();
	let expected = c10_common(&c10_project(&raw, 123).unwrap()).unwrap();
	let mut changed = raw.clone();
	changed["host_calls"] = json!(1);
	assert_ne!(
		c10_common(&c10_project(&changed, 123).unwrap()).unwrap(),
		expected
	);
	let mut changed = raw;
	changed["outcome"]["raw"] =
		Json::String(changed["outcome"]["raw"].as_str().unwrap().to_owned() + " private VM audit");
	assert_eq!(
		c10_common(&c10_project(&changed, 123).unwrap()).unwrap(),
		expected
	);
}
