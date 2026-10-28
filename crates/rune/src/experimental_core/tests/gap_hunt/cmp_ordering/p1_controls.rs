fn pins_decode(text: &str) -> String {
	assert!(text.len().is_multiple_of(2));
	let bytes = (0..text.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
		.collect::<Vec<_>>();
	String::from_utf8(bytes).unwrap()
}

// Pure fixtures only. This reconstruction is never called by either observer.
fn raw_fixture(value: &Json) -> Json {
	match value {
		Json::Object(map) => Json::Object(
			map.iter()
				.map(|(k, v)| (k.clone(), raw_fixture(v)))
				.collect(),
		),
		Json::Array(values) => Json::Array(values.iter().map(raw_fixture).collect()),
		Json::String(text) => {
			Json::String(text.replace("full_name: <fn>", "full_name: 0xabcdef012345"))
		}
		_ => value.clone(),
	}
}

#[test]
#[cfg(not(miri))]
fn p1_frozen_rows_and_adversarial_controls() {
	let counts: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_p1_counts.json")).unwrap();
	let dispositions: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_dispositions.json")).unwrap();
	assert_eq!(
		counts,
		serde_json::from_str::<Json>(C9_PINNED_COUNTS).unwrap()
	);
	assert_eq!(
		dispositions,
		serde_json::from_str::<Json>(C9_PINNED_DISPOSITIONS).unwrap()
	);
	let pinned_cases: Json = serde_json::from_str(C9_PINNED_CASES).unwrap();
	let out = std::env::temp_dir().join(format!("rune-c9-p1-controls-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	fs::create_dir(&out).unwrap();
	let mut rows = 0;
	let mut rejected = 0;
	let mut policies = [0; 4];
	for line in include_str!("../../cmp_ordering_inputs.tsv").lines().skip(1) {
		let cells = line.split('\t').collect::<Vec<_>>();
		assert_eq!(cells.len(), 9);
		assert_eq!(cells[3], "-");
		assert_eq!(cells[8], "-");
		if cells[6] == "Exact" {
			assert_eq!(cells[7], "-");
		}
		let label = cells[0];
		assert_eq!(
			json!([cells[0], cells[1], cells[2], cells[6]]),
			pinned_cases[rows]
		);
		let source = pins_decode(cells[2]);
		let index = rows.to_string();
		let stock = super::live_oracle::run_child("experimental_core::tests::gap_hunt::cmp_ordering::child", label, &out.join(format!("row-{rows}")), "C9_RESULT", &[("C9_ENGINE", "stock".as_ref()), ("C9_CASE", index.as_ref())]);
		assert_eq!(stock.observation["source_hex"], cells[2]);
		let portable = project_p1(&stock.observation["observation"]["outcome"], &counts[label]).unwrap();
		let expected: Json = if cells[6] == "Exact" { semantic_projection(&portable).unwrap() } else { serde_json::from_str(&pins_decode(cells[7])).unwrap() };
		if cells[6] != "Exact" { assert_eq!(expected, dispositions[label]["fresh_expectation"]); }
		let raw = raw_fixture(&portable);
		assert_eq!(project_p1(&raw, &counts[label]).unwrap(), portable);
		let oracle_expected = semantic_projection(&portable).unwrap();
		compare_p1(&raw, &oracle_expected, &counts[label]).unwrap();
		if raw["status"] == "error" {
			for field in ["kind", "display"] {
				let mut drift = raw.clone();
				drift[field] = json!(format!("{}!", drift[field].as_str().unwrap()));
				assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
				rejected += 1;
			}
			let mut drift = raw.clone();
			drift["sites"][0]["start"] = json!(999999);
			assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
			rejected += 1;
			let mut drift = raw.clone();
			drift["sites"][0]["frames"] = json!("drift");
			assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
			rejected += 1;
			let mut drift = raw.clone();
			drift["chain"].as_array_mut().unwrap().push(json!({"kind":"BadArgument { arg: 9 }","typed":{"tag":"BadArgument","arg":9},"display":"Bad argument #9"}));
			assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
			rejected += 1;
			if !raw["chain"].as_array().unwrap().is_empty() {
				let mut drift = raw.clone();
				drift["chain"].as_array_mut().unwrap().clear();
				assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
				rejected += 1;
			}
			if counts[label]["/kind"].as_u64().unwrap() > 0 {
				let kind = raw["kind"].as_str().unwrap();
				for changed in [
					kind.replacen("full_name:", "pointer:", 1),
					kind.replacen("full_name: 0xabcdef012345", "full_name: <fn>", 1),
					format!("{kind} AnyTypeInfo {{ full_name: 0xabc, hash: 0x123 }}"),
					kind.replacen("hash: 0x", "hash: 0x123", 1),
				] {
					let mut drift = raw.clone();
					drift["kind"] = json!(changed);
					assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
					rejected += 1;
				}
			}
			if raw["typed"]["tag"] == "IllegalFloatComparison" {
				let mut drift = raw.clone();
				drift["typed"]["lhs_bits"] = json!("0000000000000000");
				assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
				rejected += 1;
			}
		} else {
			let mut drift = raw.clone();
			drift["value"]["type_hash"] = json!("0x123");
			assert!(compare_p1(&drift, &oracle_expected, &counts[label]).is_err());
			rejected += 1;
		}
		match cells[6] {
			"Exact" => {
				policies[0] += 1;
				assert_eq!(expected, oracle_expected);
			}
			"Refused-surface" | "Retained-arity" | "Refused-runtime" => {
				policies[match cells[6] {
					"Refused-surface" => 1,
					"Refused-runtime" => 2,
					_ => 3,
				}] += 1;
				let start = expected["start"].as_u64().unwrap() as usize;
				let end = expected["end"].as_u64().unwrap() as usize;
				assert_eq!(
					&source.as_bytes()[start..end],
					pins_decode(expected["span_hex"].as_str().unwrap()).as_bytes()
				);
				assert_eq!(expected["kind"], "Unsupported");
				assert_eq!(expected["invoked"], cells[6] == "Refused-runtime");
				assert_eq!(
					expected["refusal"],
					if cells[6] == "Refused-runtime" {
						Json::Null
					} else {
						expected["operation"].clone()
					}
				);
			}
			_ => panic!("unlisted policy"),
		}
		rows += 1;
	}
	fs::remove_dir_all(out).unwrap();
	assert_eq!(rows, 210);
	assert_eq!(policies, [177, 16, 9, 8]);
	let text = "Expected { actual: Any(AnyTypeInfo { full_name: 0xabc, hash: 0x123 }) }";
	assert_eq!(
		p1_debug(text, 1).unwrap(),
		text.replace("full_name: 0xabc", "full_name: <fn>")
	);
	for (bad, count) in [
		(text.into(), 0),
		(text.into(), 2),
		(text.replace("full_name", "data_ptr"), 0),
		(text.replace("AnyTypeInfo", "OtherType"), 1),
		(text.replace("0xabc", "0XABC"), 0),
		(text.replace("full_name: ", "full_name:  "), 0),
		(format!("{text} ptr: 0xabc"), 1),
	] {
		assert!(p1_debug(&bad, count).is_err());
		rejected += 1;
	}
	assert!(project_p1(&json!({"status":"error","display":text}), &json!({})).is_err());
	rejected += 1;
	let raw = json!({"status":"error","class":"Type","kind":"X","typed":{},"display":"X","chain":[{"kind":"A"},{"kind":"B"}],"sites":[{"source":0,"start":1,"end":2,"frames":"[A,B]"}],"raw":"audit"});
	let manifest = json!({"/kind":0,"/raw":0,"/chain/0/kind":0,"/chain/1/kind":0});
	let expected = semantic_projection(&raw).unwrap();
	let mut drift = raw.clone();
	drift["chain"].as_array_mut().unwrap().reverse();
	assert!(compare_p1(&drift, &expected, &manifest).is_err());
	rejected += 1;
	let mut drift = raw.clone();
	drift["sites"][0]["frames"] = json!("[B,A]");
	assert!(compare_p1(&drift, &expected, &manifest).is_err());
	rejected += 1;
	assert_eq!(rejected, 696);
	assert!(semantic_projection(&json!({"status":"unmapped"})).is_err());
}

#[cfg(miri)]
fn miri_p1_rows(selected: &[&str], expected_policies: [usize; 4], expected_rejected: usize) {
	let mut policies = [0; 4];
	let mut rejected = 0;
	let mut checked = std::vec::Vec::new();
	for &label in selected {
		let fixture = crate::experimental_core::tests::miri_fixtures::fixture("C9-P1", label);
		let c = fixture
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v.as_str().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(c[0], label);
		let cells = &c;
		let counts = crate::experimental_core::tests::miri_fixtures::fixture("C9-counts", label);
		let source = pins_decode(cells[2]);
		let outcome = legacy_observe(&source, cells[1])["outcome"].clone();
		let portable = project_p1(&outcome, &counts).unwrap();
		let expected = if cells[6] == "Exact" {
			semantic_projection(&portable).unwrap()
		} else {
			serde_json::from_str(&pins_decode(cells[7])).unwrap()
		};
		let raw = raw_fixture(&portable);
		assert_eq!(project_p1(&raw, &counts).unwrap(), portable);
		let oracle_expected = semantic_projection(&portable).unwrap();
		compare_p1(&raw, &oracle_expected, &counts).unwrap();
		if raw["status"] == "error" {
			for field in ["kind", "display"] {
				let mut drift = raw.clone();
				drift[field] = json!(format!("{}!", drift[field].as_str().unwrap()));
				assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
				rejected += 1;
			}
			let mut drift = raw.clone();
			drift["sites"][0]["start"] = json!(999999);
			assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
			rejected += 1;
			let mut drift = raw.clone();
			drift["sites"][0]["frames"] = json!("drift");
			assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
			rejected += 1;
			let mut drift = raw.clone();
			drift["chain"].as_array_mut().unwrap().push(json!({"kind":"BadArgument { arg: 9 }","typed":{"tag":"BadArgument","arg":9},"display":"Bad argument #9"}));
			assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
			rejected += 1;
			if !raw["chain"].as_array().unwrap().is_empty() {
				let mut drift = raw.clone();
				drift["chain"].as_array_mut().unwrap().clear();
				assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
				rejected += 1;
			}
			if counts["/kind"].as_u64().unwrap() > 0 {
				let kind = raw["kind"].as_str().unwrap();
				for changed in [
					kind.replacen("full_name:", "pointer:", 1),
					kind.replacen("full_name: 0xabcdef012345", "full_name: <fn>", 1),
					format!("{kind} AnyTypeInfo {{ full_name: 0xabc, hash: 0x123 }}"),
					kind.replacen("hash: 0x", "hash: 0x123", 1),
				] {
					let mut drift = raw.clone();
					drift["kind"] = json!(changed);
					assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
					rejected += 1;
				}
			}
			if raw["typed"]["tag"] == "IllegalFloatComparison" {
				let mut drift = raw.clone();
				drift["typed"]["lhs_bits"] = json!("0000000000000000");
				assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
				rejected += 1;
			}
		} else {
			let mut drift = raw.clone();
			drift["value"]["type_hash"] = json!("0x123");
			assert!(compare_p1(&drift, &oracle_expected, &counts).is_err());
			rejected += 1;
		}
		match cells[6] {
			"Exact" => {
				policies[0] += 1;
				assert_eq!(expected, oracle_expected);
			}
			"Refused-surface" | "Retained-arity" | "Refused-runtime" => {
				policies[match cells[6] {
					"Refused-surface" => 1,
					"Refused-runtime" => 2,
					_ => 3,
				}] += 1;
				let start = expected["start"].as_u64().unwrap() as usize;
				let end = expected["end"].as_u64().unwrap() as usize;
				assert_eq!(
					&source.as_bytes()[start..end],
					pins_decode(expected["span_hex"].as_str().unwrap()).as_bytes()
				);
				assert_eq!(expected["kind"], "Unsupported");
				assert_eq!(expected["invoked"], cells[6] == "Refused-runtime");
				assert_eq!(
					expected["refusal"],
					if cells[6] == "Refused-runtime" {
						Json::Null
					} else {
						expected["operation"].clone()
					}
				);
			}
			_ => panic!("unlisted policy"),
		}

		checked.push(label);
	}
	assert_eq!(checked, selected);
	assert_eq!(policies, expected_policies);
	assert_eq!(rejected, expected_rejected);
}

#[test]
#[cfg(miri)]
fn miri_p1_value_and_refusals() {
	miri_p1_rows(
		&[
			"receiver/i64/less",
			"refusal/partial-cmp",
			"runtime-refusal/tuple",
			"arity/i64/none",
		],
		[1, 1, 1, 1],
		8,
	);
}

#[test]
#[cfg(miri)]
fn miri_p1_pointer_errors() {
	miri_p1_rows(
		&[
			"receiver/i64/bool",
			"receiver/f64/int",
			"format/display/Less",
		],
		[3, 0, 0, 0],
		29,
	);
}

#[test]
#[cfg(miri)]
fn miri_p1_float_bits() {
	miri_p1_rows(&["float-bits/nan-left"], [1, 0, 0, 0], 6);
}

#[test]
#[cfg(miri)]
fn miri_p1_standalone_controls() {
	let mut rejected = 0;
	let text = "Expected { actual: Any(AnyTypeInfo { full_name: 0xabc, hash: 0x123 }) }";
	assert_eq!(
		p1_debug(text, 1).unwrap(),
		text.replace("full_name: 0xabc", "full_name: <fn>")
	);
	for (bad, count) in [
		(text.into(), 0),
		(text.into(), 2),
		(text.replace("full_name", "data_ptr"), 0),
		(text.replace("AnyTypeInfo", "OtherType"), 1),
		(text.replace("0xabc", "0XABC"), 0),
		(text.replace("full_name: ", "full_name:  "), 0),
		(format!("{text} ptr: 0xabc"), 1),
	] {
		assert!(p1_debug(&bad, count).is_err());
		rejected += 1;
	}
	assert!(project_p1(&json!({"status":"error","display":text}), &json!({})).is_err());
	rejected += 1;
	let raw = json!({"status":"error","class":"Type","kind":"X","typed":{},"display":"X","chain":[{"kind":"A"},{"kind":"B"}],"sites":[{"source":0,"start":1,"end":2,"frames":"[A,B]"}],"raw":"audit"});
	let manifest = json!({"/kind":0,"/raw":0,"/chain/0/kind":0,"/chain/1/kind":0});
	let expected = semantic_projection(&raw).unwrap();
	let mut drift = raw.clone();
	drift["chain"].as_array_mut().unwrap().reverse();
	assert!(compare_p1(&drift, &expected, &manifest).is_err());
	rejected += 1;
	let mut drift = raw.clone();
	drift["sites"][0]["frames"] = json!("[B,A]");
	assert!(compare_p1(&drift, &expected, &manifest).is_err());
	rejected += 1;
	assert_eq!(rejected, 10);
	assert!(semantic_projection(&json!({"status":"unmapped"})).is_err());
	assert!(debug_path("/chain/0/typed/raw_kind"));
	for path in [
		"/chain//kind",
		"/chain/x/kind",
		"/chain/0",
		"/chain/0/display",
		"/display",
	] {
		assert!(!debug_path(path));
	}
}
