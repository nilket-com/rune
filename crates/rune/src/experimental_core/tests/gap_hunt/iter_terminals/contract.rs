#[test]
#[cfg(target_os = "linux")]
#[ignore = "C17d frozen fresh replay; bounded children and exclusive GAP_OUT"]
fn fresh_contract() {
	run_contract("fresh");
}

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

/// `frozen` is the pinned stock row; `stock` is its S2 replacement, or the same
/// row where no Object key was moved.
fn verify_fresh_case(
	index: usize,
	case: &Json,
	frozen: &Json,
	stock: &Json,
	actual: &Json,
	stdout: &[u8],
	stderr: &[u8],
) {
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
	let rule = &super::object_order::admitted(include_str!("c18a_admissions.json"), &rows[index]);
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	assert_eq!(
		hex(Sha256::digest(case["source"].as_str().unwrap().as_bytes()).as_ref()),
		rule["source_sha256"]
	);
	assert_eq!(
		hex(Sha256::digest(serde_json::to_vec(frozen).unwrap()).as_ref()),
		rule["stock_row_sha256"]
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
	let mut fresh = canonical_record(actual);
	let mut expected = canonical_record(stock);
	if let Some(pin) = rule.get("warnings") {
		// Stock warns that a `for (a, b)` pattern might panic; the fresh frontend
		// does not report that warning.
		assert_eq!(fresh["diagnostics"], pin["fresh"]);
		assert_eq!(expected["diagnostics"], pin["stock"]);
		fresh["diagnostics"] = Json::Null;
		expected["diagnostics"] = Json::Null;
	}
	let (f, s) = (&fresh["observation"], &expected["observation"]);
	match classification {
		"Exact" => assert_eq!(fresh, expected),
		"Intentional-D1" => {
			// A wrong receiver: fresh names the missing method (D1). Kind, class,
			// tape, sites, logical frames and reuse stay stock-exact.
			assert_eq!(fresh, rule["fresh"]);
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
			assert_eq!(fresh["logical"], expected["logical"]);
			assert_eq!(fresh["diagnostics"], expected["diagnostics"]);
		}
		"Retained-fuel" => {
			// Fuel is charged per expanded instruction: fresh halts on its own
			// count, with the stock class, an empty tape and stock reuse.
			assert_eq!(fresh, rule["fresh"]);
			for record in [f, s] {
				assert_eq!(record["outcome"]["status"], "error");
				assert_eq!(record["outcome"]["class"], "FuelExhausted");
			}
			assert_eq!(f["events"], json!([]));
			assert_eq!(f["reuse"], s["reuse"]);
			assert_eq!(fresh["diagnostics"], expected["diagnostics"]);
		}
		"Inherited-refusal" => {
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
			assert_eq!(actual["diagnostics"], rule["diagnostics"]);
			assert_eq!(actual["observation"]["events"], json!([]));
			assert_eq!(actual["logical"], Json::Null);
			if stock["observation"]["outcome"]["status"] == "compile-error" {
				// Stock builds no VM, so it records no reuse; fresh reuses a
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
		_ => panic!("unknown C17d disposition"),
	}
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}

const EXPECTED_COUNTS: [(&str, u64); 4] = [
	("Exact", 488),
	("Inherited-refusal", 77),
	("Intentional-D1", 13),
	("Retained-fuel", 2),
];
