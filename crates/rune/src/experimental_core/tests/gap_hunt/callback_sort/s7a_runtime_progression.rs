// One source-bound runtime-refusal progression; immutable C11 inputs stay intact.
fn c11_s7a_runtime_progression(
	label: &str,
	source: &str,
	input_fixture: &str,
	policy: &str,
	observation: &Json,
	disposition: &Json,
	live_stock: &Json,
) -> Result<bool, String> {
	if label != "type/non-vector" {
		return Ok(false);
	}
	let document = read_json(include_str!("s7a_runtime_progression.json"));
	let rows = document["rows"].as_array().ok_or("S7a progression rows")?;
	if document["schema"] != 1 || rows.len() != 1 {
		return Err("S7a progression schema drift".into());
	}
	let row = &rows[0];
	if row["index"] != 99 || row["label"] != label || row["fixture"] != input_fixture
		|| row["source_sha256"] != digest(source.as_bytes())
		|| policy != "Refused-runtime" || disposition["policy"] != policy
		|| disposition["index"] != 99 || CASES[99].0 != label || CASES[99].1 != source
	{
		return Err("S7a progression identity drift".into());
	}
	let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
	let spans = read_json(include_str!("../../callback_sort_refusal_spans.json"));
	if digest(&canonical(&refusals[label])) != row["legacy_refusal_fields_sha256"]
		|| spans[label] != row["legacy_span"]
	{
		return Err("S7a progression historical pin drift".into());
	}
	// Use precisely the existing P1 fn-pointer projection on the common view.
	// The diagnostic APIs differ: stock's archived observer renders the kind,
	// while new renders the source-aware public failure. Validate both texts
	// before translating this one row into the archived API view.
	let mut actual = project_p1(&common(observation), &row["p1_counts"])?;
	if actual["outcome"]["display"] != row["public_display"]
		|| row["public_display"] != "Type `std::i64` has no method named `sort_by`"
		|| row["kind_display"] != "Type `std::i64` has no matching method"
	{
		return Err("S7a progression public display drift".into());
	}
	actual["outcome"]["display"] = row["kind_display"].clone();
	if actual != *live_stock {
		return Err("S7a progression typed error, effect, span or lifecycle drift".into());
	}
	Ok(true)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: live stock observations use bounded child processes")]
fn s7a_non_vector_progression_rejects_identity_effect_and_error_drift() {
	let document = read_json(include_str!("s7a_runtime_progression.json"));
	let row = &document["rows"][0];
	let disposition = read_json(include_str!("../../callback_sort_dispositions.json"));
	let label = "type/non-vector";
	let source = CASES[99].1;
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let out = std::env::temp_dir().join(format!("rune-c11-s7a-control-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	let stock = live_stock_case(99, &out, &counts).1;
	let mut observed = stock.clone();
	for path in ["/outcome/kind", "/outcome/typed/raw_kind"] {
		let value = observed.pointer_mut(path).unwrap();
		*value = json!(value.as_str().unwrap().replace("full_name: <fn>", "full_name: 0x1"));
	}
	observed["outcome"]["display"] = row["public_display"].clone();
	let check = |value: &Json| c11_s7a_runtime_progression(label, source, "non-vector", "Refused-runtime", value, &disposition[label], &stock);
	assert!(check(&observed).unwrap());
	for path in ["/outcome/status", "/outcome/kind", "/outcome/typed/raw_kind", "/outcome/display", "/outcome/sites/0/start", "/before", "/post", "/events", "/events_after_reuse", "/reuse", "/invoked"] {
		let mut bad = observed.clone();
		*bad.pointer_mut(path).unwrap() = json!("drift");
		assert!(check(&bad).is_err(), "{path}");
	}
	assert!(c11_s7a_runtime_progression(label, "different", "non-vector", "Refused-runtime", &observed, &disposition[label], &stock).is_err());
	assert!(c11_s7a_runtime_progression(label, source, "ints", "Refused-runtime", &observed, &disposition[label], &stock).is_err());
	assert!(!c11_s7a_runtime_progression("another-row", source, "non-vector", "Refused-runtime", &observed, &disposition[label], &stock).unwrap());
	fs::remove_dir_all(out).unwrap();
}
