#[test]
#[cfg(target_os = "linux")]
#[ignore = "C16h frozen fresh replay; bounded children and exclusive GAP_OUT"]
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
	assert_eq!(dispositions["rows"].as_array().unwrap().len(), 120);
	let rule = &dispositions["rows"][index];
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	assert_eq!(
		hex(Sha256::digest(case["source"].as_str().unwrap().as_bytes()).as_ref()),
		rule["source_sha256"]
	);
	let mut raw_stock = stock.clone();
	for key in ["stdout_hex", "stderr_hex"] {
		raw_stock.as_object_mut().unwrap().remove(key);
	}
	assert_eq!(
		hex(Sha256::digest(serde_json::to_vec(&raw_stock).unwrap()).as_ref()),
		rule["stock_row_sha256"]
	);
	let stock = &super::object_order::order_fixed("type_values", index, stock);
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
		assert_eq!(audit[key], 0);
	}
	assert_eq!(audit["cleanup_error"], false);
	let classification = rule["classification"].as_str().unwrap();
	let refused = matches!(classification, "Inherited-refusal" | "Frontend-refusal");
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);
	let fresh = canonical_record(actual);
	let expected = canonical_record(stock);
	match classification {
		"Exact" => assert_eq!(fresh, expected),
		"Inherited-diagnostic" | "Intentional-D1" => {
			assert_eq!(fresh, rule["fresh"]);
			// Only the primary diagnostic differs: tape, sites, logical
			// frames and reuse stay stock-exact.
			let (f, s) = (&fresh["observation"], &expected["observation"]);
			assert_eq!(f["events"], s["events"]);
			assert_eq!(f["reuse"], s["reuse"]);
			assert_eq!(f["outcome"]["sites"], s["outcome"]["sites"]);
			assert_eq!(f["outcome"]["chain"], s["outcome"]["chain"]);
			assert_eq!(fresh["logical"]["frames"], expected["logical"]["frames"]);
			assert_eq!(fresh["diagnostics"], expected["diagnostics"]);
			if classification == "Intentional-D1" {
				assert_eq!(f["outcome"]["kind"], s["outcome"]["kind"]);
				assert_eq!(f["outcome"]["class"], s["outcome"]["class"]);
			} else {
				assert!(s["outcome"]["kind"]
					.as_str()
					.unwrap()
					.starts_with("MissingFunction"));
			}
		}
		"Inherited-refusal" | "Frontend-refusal" => {
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
			assert_eq!(actual["diagnostics"], rule["diagnostics"]);
			assert_eq!(actual["observation"]["events"], json!([]));
			assert_eq!(actual["logical"], Json::Null);
			if classification == "Frontend-refusal" {
				assert_eq!(stock["observation"]["outcome"]["status"], "compile-error");
				assert_eq!(actual["diagnostics"], stock["diagnostics"]);
			}
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
		_ => panic!("unknown C16h disposition"),
	}
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}
