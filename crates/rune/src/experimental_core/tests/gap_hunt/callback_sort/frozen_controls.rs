const INPUTS: &str = include_str!("../../callback_sort_inputs.tsv");
fn read_json(text: &str) -> Json {
	serde_json::from_str(text).unwrap()
}
#[test]
#[cfg_attr(miri, ignore = "Miri: live stock observations use bounded child processes")]
fn p1_digest_and_semantic_drift_are_rejected() {
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let out = std::env::temp_dir().join(format!("rune-c11-p1-controls-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	fs::create_dir(&out).unwrap();
	let mut checked = 0;
	for (index, line) in INPUTS.lines().skip(1).enumerate() {
		let c = line.split('\t').collect::<Vec<_>>();
		let run = live_stock_case(index, &out.join(format!("row-{index}")), &counts).0;
		let raw = run.observation["observation"].clone();
		let stock = common(&project_p1(&raw, &counts[c[0]]).unwrap());
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
		let bytes = canonical(&stock);
		let stored = json!({"storage":"canonical-json-sha256", "bytes":bytes.len(), "sha256":digest(&bytes)});
		let mut bad = stock.clone();
		bad["invoked"] = Json::Null;
		assert!(check_cell(&stored, &bad).is_err());
		let mut bad_stored = stored.clone();
		bad_stored["bytes"] = json!(0);
		assert!(check_cell(&bad_stored, &stock).is_err());
		bad_stored = stored.clone();
		bad_stored["extra"] = true.into();
		assert!(check_cell(&bad_stored, &stock).is_err());
		checked += 3;
	}
	assert_eq!(checked, 1503);
	fs::remove_dir_all(out).unwrap();
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
#[cfg_attr(miri, ignore = "Miri: live stock observations use bounded child processes")]
fn resource_and_refusal_constraints_reject_drift() {
	let resources = read_json(include_str!("../../callback_sort_resources.json"));
	let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let out = std::env::temp_dir().join(format!("rune-c11-resource-controls-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	fs::create_dir(&out).unwrap();
	let fuel_anchor = CASES.iter().position(|c| c.0 == "fuel/128").unwrap();
	let complete_fuel = live_stock_case(fuel_anchor, &out.join("fuel-anchor"), &counts).1;
	let mut resource_rows = 0;
	for (label, row) in resources.as_object().unwrap() {
		let index = CASES.iter().position(|c| c.0 == label).unwrap();
		let (_, source, input_fixture, _, _, _, _) = CASES[index];
		let current = live_stock_case(index, &out.join(format!("row-{index}")), &counts).1;
		let anchor = if label.starts_with("fuel/") { &complete_fuel } else { &current };
		let (complete, _) = resource_completion_live(label, source, input_fixture, &current, anchor).unwrap();
		let stock = &complete;
		let kind = if label.starts_with("cancel/") {
			"Cancelled"
		} else {
			"FuelExhausted"
		};
		let audit = json!({"typed_failure_kind":kind,"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
		let mut execution = stock.clone();
		execution["outcome"] = json!({"status":"error","class":kind});
		execution["post"] = stock["before"].clone();
		execution["events"] = json!([]);
		execution["events_after_reuse"] = json!([{"event":"mark","hex":"7265757365"}]);
		assert!(resource_valid(stock, &execution, &audit, kind));
		// Empty and full prefixes are both allowed; after-last-callback exhaustion is valid.
		execution["events"] = stock["events"].clone();
		execution["events_after_reuse"] = stock["events_after_reuse"].clone();
		assert!(resource_valid(stock, &execution, &audit, kind));
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
			assert!(!resource_valid(stock, &execution, &bad, kind));
		}
		let mut bad = execution.clone();
		bad["post"] = Json::Null;
		assert!(!resource_valid(stock, &bad, &audit, kind));
		bad = execution.clone();
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
	let (_, source, input_fixture, _, _, _, _) = CASES[112];
	let current = live_stock_case(112, &out.join("fuel-16-control"), &counts).1;
	let (complete, _) = resource_completion_live("fuel/16", source, input_fixture, &current, &complete_fuel).unwrap();
	assert_eq!(
		current["events"]
			.as_array()
			.unwrap()
			.len(),
		2
	);
	let mut execution = complete.clone();
	execution["outcome"] = json!({"status":"error","class":"FuelExhausted"});
	execution["post"] = complete["before"].clone();
	let tape = &complete["events"].as_array().unwrap()[..3];
	execution["events"] = json!(tape);
	let mut after = tape.to_vec();
	after.push(json!({"event":"mark","hex":"7265757365"}));
	execution["events_after_reuse"] = json!(after);
	let audit = json!({"typed_failure_kind":"FuelExhausted","cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
	assert!(resource_valid(&complete, &execution, &audit, "FuelExhausted")); // 3 > same-budget stock 2.
	for events in [
		json!([complete["events"][1], complete["events"][2]]),
		json!([complete["events"][0], complete["events"][2]]),
		json!([complete["events"][0],{"event":"invented"}]),
	] {
		let mut bad = execution.clone();
		let mut after = events.as_array().unwrap().clone();
		after.push(json!({"event":"mark","hex":"7265757365"}));
		bad["events"] = events;
		bad["events_after_reuse"] = json!(after);
		assert!(!resource_valid(&complete, &bad, &audit, "FuelExhausted"));
	}
	assert!(resource_completion_live("fuel/16", &format!("{source} "), input_fixture, &current, &complete_fuel).is_err());
	assert!(resource_completion_live("fuel/16", source, "wrong-fixture", &current, &complete_fuel).is_err());
	fs::remove_dir_all(out).unwrap();

	for (label, row) in refusals.as_object().unwrap() {
		let source = CASES.iter().find(|c| c.0 == label).unwrap().1;
		let mut execution = row["fresh_constraint"].clone();
		execution["refusal"] = if execution["invoked"] == false {
			execution["operation"].clone()
		} else {
			Json::Null
		};
		let audit = json!({"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
		assert!(refusal_valid(label, source, &execution, &audit));
		assert!(!refusal_valid("unlisted", source, &execution, &audit));
		assert!(!refusal_valid(label, &format!("{source} "), &execution, &audit));
		for field in [
			"status",
			"kind",
			"operation",
			"refusal",
			"source_sha256",
			"invoked",
		] {
			let mut bad = execution.clone();
			bad[field] = json!("drift");
			assert!(!refusal_valid(label, source, &bad, &audit));
		}
		let mut bad = audit.clone();
		bad["live_entries"] = 1.into();
		assert!(!refusal_valid(label, source, &execution, &bad));
	}
	assert_eq!(refusals.as_object().unwrap().len(), 8);
}
