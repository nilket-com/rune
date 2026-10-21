#[test]
#[cfg(target_os = "linux")]
#[ignore = "S4 frozen fresh replay; bounded children and exclusive GAP_OUT"]
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

/// Stock outcomes of a wrong-count call that `CallOffset` let run: stock
/// checks the count only on calls it does not rewrite to an offset.
const STOCK_DEFECTS: [&str; 3] = ["too-many-runs", "too-few-leaks-empty", "unbounded-recursion"];

/// `stock` is the pinned stock row.
fn verify_fresh_case(
	index: usize,
	case: &Json,
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
	assert_eq!(counts, std::collections::BTreeMap::from(EXPECTED_COUNTS));
	assert_eq!(
		dispositions["summary"]["classifications"],
		serde_json::to_value(&counts).unwrap()
	);
	let rule = &super::object_order::admitted(include_str!("c18a_admissions.json"), &rows[index]);
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	assert_eq!(
		hex(Sha256::digest(serde_json::to_vec(stock).unwrap()).as_ref()),
		rule["stock_row_sha256"]
	);
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
	let refused = !matches!(classification, "Exact" | "Intentional-D1");
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);
	match rule.get("stock_defect") {
		None => (),
		Some(defect) => {
			assert_eq!(classification, "Refused-arity");
			assert!(STOCK_DEFECTS.contains(&defect.as_str().unwrap()));
		}
	}
	if classification == "Exact" {
		assert_eq!(canonical_record(actual), canonical_record(stock));
	} else if classification == "Intentional-D1" {
		// A capturing closure called with the wrong count: stock counts its
		// environment as one more argument on both sides, fresh reports the
		// counts the caller wrote. Everything else is stock-exact.
		let (fresh, expected) = (canonical_record(actual), canonical_record(stock));
		assert_eq!(fresh, rule["fresh"]);
		let (f, s) = (&fresh["observation"], &expected["observation"]);
		for key in ["status", "class", "sites", "chain"] {
			assert_eq!(f["outcome"][key], s["outcome"][key], "{key}");
		}
		assert_eq!(f["outcome"]["class"], "ArgumentCount");
		let counts = |kind: &Json| {
			let kind = kind.as_str().unwrap();
			let kind = kind.strip_prefix("BadArgumentCount { actual: ").unwrap();
			let (actual, expected) = kind.strip_suffix(" }").unwrap().split_once(", expected: ").unwrap();
			(actual.parse::<u32>().unwrap(), expected.parse::<u32>().unwrap())
		};
		let (given, wanted) = counts(&f["outcome"]["kind"]);
		assert_eq!(counts(&s["outcome"]["kind"]), (given + 1, wanted + 1));
		assert_eq!(f["events"], s["events"]);
		assert_eq!(f["reuse"], s["reuse"]);
		assert_eq!(fresh["diagnostics"], expected["diagnostics"]);
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
			"Refused-limit" => assert_eq!(refusal, "CodeLimit"),
			"Inherited-refusal" => assert_ne!(refusal, "CodeLimit"),
			_ => panic!("unknown S4 disposition"),
		}
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
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}

const EXPECTED_COUNTS: [(&str, u64); 5] = [
	("Exact", 85),
	("Inherited-refusal", 217),
	("Intentional-D1", 2),
	("Refused-arity", 77),
	("Refused-limit", 4),
];
