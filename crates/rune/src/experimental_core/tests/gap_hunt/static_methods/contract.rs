/// The comparable part of one observation. Stock VM frame encoding and raw
/// error dumps are audit data; the process-local `AnyTypeInfo::full_name`
/// pointer is masked wherever a typed kind or class names a type.
fn canonical_record(record: &Json) -> Json {
	let mut observation = record["observation"].clone();
	for field in ["outcome", "reuse"] {
		let outcome = &mut observation[field];
		if outcome["status"] != "error" {
			continue;
		}
		outcome.as_object_mut().unwrap().remove("raw");
		for site in outcome["sites"].as_array_mut().unwrap() {
			site.as_object_mut().unwrap().remove("frames");
		}
		for key in ["kind", "class"] {
			outcome[key] = json!(stable_debug(outcome[key].as_str().unwrap()));
		}
		for link in outcome["chain"].as_array_mut().unwrap() {
			link["kind"] = json!(stable_debug(link["kind"].as_str().unwrap()));
		}
	}
	json!({"observation":observation,"logical":record["logical"],"diagnostics":record["diagnostics"]})
}

/// `stock` is the live legacy run of the same case as `fresh`.
#[cfg(target_os = "linux")]
fn verify_case(
	index: usize,
	case: &Json,
	stock_run: &super::live_oracle::Run,
	fresh: &super::live_oracle::Run,
) {
	crate::experimental_core::tests::gap_hunt::assert_warning_multisets(
		&stock_run.observation,
		&fresh.observation,
	);
	let stock = &stock_run.observation;
	let (actual, stdout, stderr) = (&fresh.observation, &fresh.stdout[..], &fresh.stderr[..]);
	let dispositions: Json = serde_json::from_str(include_str!("dispositions.json")).unwrap();
	let rows = dispositions["rows"].as_array().unwrap();
	assert_eq!(rows.len(), ROWS);
	let mut counts = std::collections::BTreeMap::new();
	for row in rows {
		*counts
			.entry(row["classification"].as_str().unwrap())
			.or_insert(0u64) += 1;
	}
	assert_eq!(counts, std::collections::BTreeMap::from(EXPECTED_COUNTS));
	assert_eq!(
		dispositions["summary"]["classifications"],
		serde_json::to_value(&counts).unwrap()
	);
	// C18b1 admits rows through run-time dispatch; each entry pins the rule it replaces.
	let rule = &super::object_order::admitted(include_str!("c18b1_admissions.json"), &rows[index]);
	let rule = &super::object_order::admitted(include_str!("c18b3a_admissions.json"), rule);
	let rule = &super::object_order::admitted(include_str!("c18b3b_admissions.json"), rule);
	let rule = &super::object_order::admitted(include_str!("c18b3d_admissions.json"), rule);
	let rule = &super::object_order::admitted(include_str!("c17f_admissions.json"), rule);
	let rule = &super::object_order::admitted(include_str!("c18b3e_admissions.json"), rule);
	let rule = &super::object_order::admitted(include_str!("s6_progressions.json"), rule);
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	// Validation is an internal backstop: no row reaches it.
	assert!(!serde_json::to_string(&actual["observation"])
		.unwrap()
		.contains("InvalidCode"));
	let audit = &actual["fresh_audit"];
	for key in ["row_idle", "executor_idle"] {
		assert_eq!(audit[key], true);
	}
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"live_entries",
		"payload_bytes",
	] {
		assert_eq!(audit[key], 0, "{key}");
	}
	assert_eq!(audit["cleanup_error"], false);
	let classification = rule["classification"].as_str().unwrap();
	let refused = !matches!(
		classification,
		"Exact" | "Intentional-D1" | "Retained-fuel" | "Retained-frame-limit"
	);
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);

	if classification == "Exact" {
		assert_eq!(canonical_record(actual), canonical_record(stock));
	} else if classification == "Intentional-D1" {
		// A missing method: new names the method in its message. Everything
		// else, including the kind, class, sites and chain, is stock-exact.
		let (execution, expected) = (canonical_record(actual), canonical_record(stock));
		crate::experimental_core::tests::gap_hunt::compare_runtime_policy(
			&execution,
			&expected,
			&rule["fresh"],
		);
		let (f, s) = (&execution["observation"], &expected["observation"]);
		for key in ["status", "class", "kind", "sites", "chain"] {
			assert_eq!(f["outcome"][key], s["outcome"][key], "{key}");
		}
		assert_eq!(f["outcome"]["class"], "Type");
		assert!(f["outcome"]["kind"]
			.as_str()
			.unwrap()
			.starts_with("MissingInstanceFunction"));
		assert_ne!(f["outcome"]["display"], s["outcome"]["display"]);
		assert_eq!(f["events"], s["events"]);
		assert_eq!(f["reuse"], s["reuse"]);
		assert_eq!(execution["logical"], expected["logical"]);
	} else if classification == "Retained-fuel" {
		let (execution, expected) = (canonical_record(actual), canonical_record(stock));
		crate::experimental_core::tests::gap_hunt::compare_runtime_policy(
			&execution,
			&expected,
			&rule["fresh"],
		);
		let (f, s) = (&execution["observation"], &expected["observation"]);
		for record in [f, s] {
			assert_eq!(record["outcome"]["status"], "error");
			assert_eq!(record["outcome"]["class"], "FuelExhausted");
		}
		assert_eq!(f["events"], json!([]));
		assert_eq!(f["reuse"], s["reuse"]);
	} else if classification == "Retained-frame-limit" {
		// Unbounded recursion: new charges fuel once per call, so its
		// 1024-frame limit trips before the fuel runs out, where stock ends in
		// FuelExhausted. The tapes agree index by index up to the shorter one.
		let (execution, expected) = (canonical_record(actual), canonical_record(stock));
		crate::experimental_core::tests::gap_hunt::compare_runtime_policy(
			&execution,
			&expected,
			&rule["fresh"],
		);
		let (f, s) = (&execution["observation"], &expected["observation"]);
		assert_eq!(f["outcome"]["status"], "error");
		assert_eq!(f["outcome"]["class"], "FrameCapacity");
		assert_eq!(f["outcome"]["depth"], 1024);
		assert_eq!(s["outcome"]["status"], "error");
		assert_eq!(s["outcome"]["class"], "FuelExhausted");
		for (a, b) in f["events"]
			.as_array()
			.unwrap()
			.iter()
			.zip(s["events"].as_array().unwrap())
		{
			assert_eq!(a, b);
		}
		assert_eq!(f["reuse"], s["reuse"]);
	} else {
		// A located, classified refusal: the program runs on stock instead.
		let outcome = &actual["observation"]["outcome"];
		assert_eq!(outcome, &rule["fresh"]);
		assert_eq!(outcome["status"], "refused");
		assert_eq!(outcome["kind"], "Unsupported");
		assert!(outcome["location"].is_object());
		let refusal = outcome["refusal"].as_str().unwrap();
		match classification {
			"Refused-arity" => assert_eq!(refusal, "Arity"),
			// A script method shares a native fast-path name: never the native form.
			"Refused-shadow" => assert_eq!(refusal, "Method"),
			"Inherited-refusal" => assert_ne!(refusal, "CodeLimit"),
			_ => panic!("unknown C18a disposition"),
		}
		assert_eq!(actual["diagnostics"], rule["diagnostics"]);
		assert_eq!(actual["observation"]["events"], json!([]));
		assert_eq!(actual["logical"], Json::Null);
		if stock["observation"]["outcome"]["status"] == "compile-error" {
			// Stock builds no VM, so it records no reuse; new reuses a
			// fallback program and must still return 7.
			assert_eq!(stock["observation"]["reuse"], Json::Null);
			assert_eq!(actual["observation"]["reuse"]["graph"]["nodes"][0][2], "7");
		} else {
			assert_eq!(
				actual["observation"]["reuse"],
				stock["observation"]["reuse"]
			);
		}
	}
	if refused {
		// A refused program prints nothing: only the test harness header.
		assert_eq!(stdout, b"\nrunning 1 test\n");
	} else {
		assert_eq!(hex(stdout), hex(&stock_run.stdout), "stdout");
	}
	assert_eq!(hex(stderr), hex(&stock_run.stderr), "stderr");
}

const EXPECTED_COUNTS: [(&str, u64); 5] = [
	("Exact", 128),
	("Inherited-refusal", 263),
	("Intentional-D1", 4),
	("Refused-arity", 54),
	("Refused-shadow", 14),
];
