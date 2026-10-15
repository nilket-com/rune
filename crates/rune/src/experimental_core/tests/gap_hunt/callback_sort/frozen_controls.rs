const FROZEN: &str = include_str!("../../callback_sort_stock.tsv");
fn read_json(text: &str) -> Json {
	serde_json::from_str(text).unwrap()
}
fn synthetic_raw(common: &Json) -> Json {
	fn inflate(value: &Json) -> Json {
		match value {
			Json::Object(map) => {
				Json::Object(map.iter().map(|(k, v)| (k.clone(), inflate(v))).collect())
			}
			Json::Array(values) => Json::Array(values.iter().map(inflate).collect()),
			Json::String(s) => json!(s.replace("full_name: <fn>", "full_name: 0xabcdef012345")),
			_ => value.clone(),
		}
	}
	let mut value = inflate(common);
	// Pure reconstruction for comparator controls only; never an observer input.
	if value["outcome"]["status"] == "error" {
		value["outcome"]["raw"] = value["outcome"]["kind"].clone();
	}
	value
}
fn stock_cell(cells: &[&str], dispositions: &Json) -> Json {
	let stored: Json = serde_json::from_slice(&decode(cells[3])).unwrap();
	if stored.get("storage").is_some() {
		assert_eq!(cells[6], "Exact");
		dispositions[cells[0]]["fresh_expectation"].clone()
	} else {
		stored
	}
}
#[test]
fn frozen_cells_and_python_canonical_crosscheck() {
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let cross = read_json(include_str!("../../callback_sort_canonical.json"));
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let mut rows = 0;
	let mut digests = 0;
	let mut pointers = 0;
	for line in FROZEN.lines().skip(1) {
		let c = line.split('\t').collect::<Vec<_>>();
		assert_eq!(c.len(), 9);
		assert_eq!(c[0], CASES[rows].0);
		assert_eq!(decode(c[2]), CASES[rows].1.as_bytes());
		assert_eq!(c[6], dispositions[c[0]]["policy"].as_str().unwrap());
		let stock = stock_cell(&c, &dispositions);
		let fresh = &dispositions[c[0]]["fresh_expectation"];
		for (side, cell, value) in [("stock", c[3], &stock), ("fresh", c[7], fresh)] {
			let stored: Json = serde_json::from_slice(&decode(cell)).unwrap();
			check_cell(&stored, value).unwrap();
			digests += usize::from(stored.get("storage").is_some());
			let bytes = canonical(value);
			assert_eq!(cross[c[0]][side]["bytes"], bytes.len());
			assert_eq!(cross[c[0]][side]["sha256"], digest(&bytes));
		}
		let raw = synthetic_raw(&stock);
		assert_eq!(common(&project_p1(&raw, &counts[c[0]]).unwrap()), stock);
		if !counts[c[0]].as_object().unwrap().is_empty() {
			pointers += 1;
		}
		rows += 1;
	}
	assert_eq!((rows, digests, pointers), (131, 18, 12));
	let depth = read_json(include_str!("../../callback_sort_depth.json"));
	assert_eq!(depth.as_object().unwrap().len(), 7);
	for row in depth.as_object().unwrap().values() {
		assert!(row["fresh_constraint"]["literal_fresh_expectation"].is_null());
	}
}
#[test]
fn p1_digest_and_semantic_drift_are_rejected() {
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let mut checked = 0;
	for line in FROZEN.lines().skip(1) {
		let c = line.split('\t').collect::<Vec<_>>();
		let stock = stock_cell(&c, &dispositions);
		let raw = synthetic_raw(&stock);
		for field in [
			"before",
			"post",
			"events",
			"events_after_reuse",
			"invoked",
			"reuse",
		] {
			let mut bad = raw.clone();
			bad[field] = Json::Null;
			assert!(
				project_p1(&bad, &counts[c[0]]).is_err()
					|| common(&project_p1(&bad, &counts[c[0]]).unwrap()) != stock
			);
			checked += 1;
		}
		if stock["outcome"]["status"] == "error" {
			for field in ["kind", "class", "display", "typed", "chain", "sites"] {
				let mut bad = raw.clone();
				bad["outcome"][field] = json!("drift");
				assert!(
					project_p1(&bad, &counts[c[0]]).is_err()
						|| common(&project_p1(&bad, &counts[c[0]]).unwrap()) != stock
				);
				checked += 1;
			}
		}
		if !counts[c[0]].as_object().unwrap().is_empty() {
			let mut bad_counts = counts[c[0]].clone();
			let key = bad_counts
				.as_object()
				.unwrap()
				.keys()
				.next()
				.unwrap()
				.clone();
			bad_counts[&key] = json!(99);
			assert!(project_p1(&raw, &bad_counts).is_err());
			let mut bad = raw.clone();
			bad["outcome"]["kind"] = json!(bad["outcome"]["kind"].as_str().unwrap().replacen(
				"full_name:",
				"pointer:",
				1
			));
			assert!(project_p1(&bad, &counts[c[0]]).is_err());
			let mut bad = raw.clone();
			bad["outcome"]["display"] = json!("AnyTypeInfo { full_name: 0xabc, hash: 0x123 }");
			assert!(project_p1(&bad, &counts[c[0]]).is_err());
			checked += 3;
		}
		let stored: Json = serde_json::from_slice(&decode(c[3])).unwrap();
		if stored.get("storage").is_some() {
			let mut bad = stock.clone();
			let middle = bad["events"].as_array().unwrap().len() / 2;
			bad["events"][middle]["a"] = json!("999999999");
			assert!(check_cell(&stored, &bad).is_err());
			let mut bad_stored = stored.clone();
			bad_stored["bytes"] = json!(0);
			assert!(check_cell(&bad_stored, &stock).is_err());
			bad_stored = stored.clone();
			bad_stored["extra"] = true.into();
			assert!(check_cell(&bad_stored, &stock).is_err());
			checked += 3;
		}
	}
	assert_eq!(checked, 1137);
	let text = "AnyTypeInfo { full_name: 0xabc, hash: 0x123 }";
	assert_eq!(p1_debug(text, 1).unwrap(), text.replace("0xabc", "<fn>"));
	for bad in [
		text.replace("AnyTypeInfo", "Other"),
		text.replace("0xabc", "0XABC"),
		text.replace("full_name: ", "full_name:  "),
		format!("{text} ptr: 0xabc"),
		text.replace("hash: 0x123", "hash: 0x"),
	] {
		assert!(p1_debug(&bad, 1).is_err());
	}
	assert!(p1_debug(text, 2).is_err());
}
#[test]
fn resource_and_refusal_constraints_reject_drift() {
	let resources = read_json(include_str!("../../callback_sort_resources.json"));
	let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
	let mut resource_rows = 0;
	for (label, row) in resources.as_object().unwrap() {
		let stock = &row["stock_common"];
		let kind = if label.starts_with("cancel/") {
			"Cancelled"
		} else {
			"FuelExhausted"
		};
		let audit = json!({"typed_failure_kind":kind,"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
		let mut fresh = stock.clone();
		fresh["outcome"] = json!({"status":"error","class":kind});
		fresh["post"] = stock["before"].clone();
		fresh["events"] = json!([]);
		fresh["events_after_reuse"] = json!([{"event":"mark","hex":"7265757365"}]);
		assert!(resource_valid(stock, &fresh, &audit, kind));
		// Empty and full prefixes are both allowed; after-last-callback exhaustion is valid.
		fresh["events"] = stock["events"].clone();
		fresh["events_after_reuse"] = stock["events_after_reuse"].clone();
		assert!(resource_valid(stock, &fresh, &audit, kind));
		for field in [
			"typed_failure_kind",
			"cleanup_error",
			"executor_idle",
			"live_entries",
			"payload_bytes",
			"reuse_i64",
		] {
			let mut bad = audit.clone();
			bad[field] = json!("drift");
			assert!(!resource_valid(stock, &fresh, &bad, kind));
		}
		let mut bad = fresh.clone();
		bad["post"] = Json::Null;
		assert!(!resource_valid(stock, &bad, &audit, kind));
		bad = fresh.clone();
		bad["events"] = json!([{"event":"invented"}]);
		assert!(!resource_valid(stock, &bad, &audit, kind));
		if stock["outcome"]["status"] == "value" {
			let success_audit = json!({"typed_failure_kind":null,"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
			assert!(resource_valid(stock, stock, &success_audit, kind));
		}
		assert_eq!(row["fresh_constraint"]["tape_rule"], "prefix");
		resource_rows += 1;
	}
	assert_eq!(resource_rows, 17);
	for (label, row) in refusals.as_object().unwrap() {
		let source = CASES.iter().find(|c| c.0 == label).unwrap().1;
		let mut fresh = row["fresh_constraint"].clone();
		fresh["refusal"] = if fresh["invoked"] == false {
			fresh["operation"].clone()
		} else {
			Json::Null
		};
		let audit = json!({"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
		assert!(refusal_valid(label, source, &fresh, &audit));
		assert!(!refusal_valid("unlisted", source, &fresh, &audit));
		assert!(!refusal_valid(label, &format!("{source} "), &fresh, &audit));
		for field in [
			"status",
			"kind",
			"operation",
			"refusal",
			"source_sha256",
			"invoked",
		] {
			let mut bad = fresh.clone();
			bad[field] = json!("drift");
			assert!(!refusal_valid(label, source, &bad, &audit));
		}
		let mut bad = audit.clone();
		bad["live_entries"] = 1.into();
		assert!(!refusal_valid(label, source, &fresh, &bad));
	}
	assert_eq!(refusals.as_object().unwrap().len(), 8);
}
