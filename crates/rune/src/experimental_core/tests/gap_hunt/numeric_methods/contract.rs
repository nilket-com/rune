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
	use sha2::{Digest, Sha256};
	let dispositions: Json = serde_json::from_str(include_str!("dispositions.json")).unwrap();
	let rows = dispositions["rows"].as_array().unwrap();
	assert_eq!(rows.len(), ROWS);
	let mut counts = std::collections::BTreeMap::new();
	for row in rows {
		*counts
			.entry(row["classification"].as_str().unwrap())
			.or_insert(0u64) += 1;
	}
	let expected_counts = std::collections::BTreeMap::from(EXPECTED_COUNTS);
	assert_eq!(counts, expected_counts);
	assert_eq!(
		dispositions["summary"]["classifications"],
		serde_json::to_value(&counts).unwrap()
	);
	let rule = &super::object_order::admitted(
		include_str!("c18a_admissions.json"),
		&super::object_order::admitted(include_str!("c17e_admissions.json"), &rows[index]),
	);
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	assert_eq!(
		hex(Sha256::digest(case["source"].as_str().unwrap().as_bytes()).as_ref()),
		rule["source_sha256"]
	);
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
	let refused = classification == "Inherited-refusal";
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);
	let execution = canonical_record(actual);
	let expected = canonical_record(stock);
	match classification {
		"Exact" => assert_eq!(execution, expected),
		"Retained-frame (callback)" => {
			// C17e: an error inside a direct chain's function value. Stock reports
			// one site per nested VM execution; new the origin with its callers.
			assert!(super::iter_adapters::callback_frames(actual, stock));
			let mut execution = execution.clone();
			execution["observation"]["outcome"]["sites"] =
				expected["observation"]["outcome"]["sites"].clone();
			assert_eq!(execution, expected);
		}
		"Intentional-D1" => {
			// A wrong receiver: new names the missing method (D1). Kind, class,
			// tape, sites, logical frames and reuse stay stock-exact.
			crate::experimental_core::tests::gap_hunt::compare_runtime_policy(
				&execution,
				&expected,
				&rule["fresh"],
			);
			let (f, s) = (&execution["observation"], &expected["observation"]);
			for key in ["status", "class", "kind", "sites", "chain"] {
				assert_eq!(f["outcome"][key], s["outcome"][key], "{key}");
			}
			assert!(f["outcome"]["display"]
				.as_str()
				.unwrap()
				.contains("has no method named"));
			assert!(s["outcome"]["display"]
				.as_str()
				.unwrap()
				.ends_with("has no matching method"));
			assert_eq!(f["events"], s["events"]);
			assert_eq!(f["reuse"], s["reuse"]);
			assert_eq!(execution["logical"], expected["logical"]);
			assert_eq!(execution["diagnostics"], expected["diagnostics"]);
		}
		"Inherited-refusal" => {
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
			assert_eq!(actual["diagnostics"], rule["diagnostics"]);
			assert_eq!(actual["observation"]["events"], json!([]));
			assert_eq!(actual["logical"], Json::Null);
			assert_eq!(
				actual["observation"]["reuse"],
				stock["observation"]["reuse"]
			);
		}
		_ => panic!("unknown C17c disposition"),
	}
	assert_eq!(hex(stdout), hex(&stock_run.stdout), "stdout");
	assert_eq!(hex(stderr), hex(&stock_run.stderr), "stderr");
}

const EXPECTED_COUNTS: [(&str, u64); 3] = [
	("Exact", 238),
	("Inherited-refusal", 91),
	("Intentional-D1", 41),
];
