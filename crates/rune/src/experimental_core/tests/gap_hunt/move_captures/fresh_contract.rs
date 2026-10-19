// Physical VM windows are retained in raw receipts. D1e logical frames/context
// are the shared user-facing contract for both engines.
fn semantic_projection(p: &Json) -> Json {
	let mut p = p.clone();
	if p["outcome"]["status"] == "error" {
		let o = &p["outcome"];
		p["outcome"] = json!({"status":o["status"],"class":o["class"],"display":o["display"]});
	} else if p["outcome"]["status"] == "compile-error" {
		p["outcome"] = json!({"status":"compile-error"});
	}
	p
}
fn verify_fresh_record(actual: &Json, case: &Json, stock: &Json, disposition: &Json) {
	assert_eq!(actual["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["source_sha256"], case["source_sha256"]);
	let a = &actual["fresh_audit"];
	assert_eq!(a["cleanup_error"], false);
	for key in ["row_idle", "reuse_idle"] {
		assert_eq!(a[key], true, "{}: {key}", case["label"]);
	}
	for key in ["reuse_live_entries", "reuse_payload_bytes"] {
		assert_eq!(a[key], 0);
	}
	if case["origin"] == "C11b/020" {
		assert_eq!(a["row_live_entries"], 2);
		assert_eq!(a["row_payload_bytes"], 32);
		assert_eq!(a["reuse_same_store"], false);
	} else {
		assert_eq!(a["row_live_entries"], 0);
		assert_eq!(a["row_payload_bytes"], 0);
		assert_eq!(a["reuse_same_store"], true);
	}
	if actual["observation"]["outcome"]["status"] == "error" {
		// Actual core ErrorKind, never a test-side reclassification.
		assert_eq!(
			actual["observation"]["outcome"]["kind"],
			actual["observation"]["outcome"]["class"]
		);
	}
	match disposition["classification"].as_str().unwrap() {
		"exact" => assert_eq!(
			semantic_projection(&actual["projected"]),
			semantic_projection(&stock["projected"]),
			"{}",
			case["label"]
		),
		"inherited-refusal" => {
			let o = &actual["observation"]["outcome"];
			assert_eq!(o["status"], "refused");
			assert_eq!(o["kind"], "Unsupported");
			assert_eq!(o["refusal"], disposition["refusal"]);
			assert_eq!(o["location"], disposition["site"]);
			assert_eq!(
				disposition["protected_main_refusal"]["refusal"],
				o["refusal"]
			);
			assert_eq!(
				disposition["protected_main_refusal"]["location"],
				o["location"]
			);
			assert_eq!(a["invoked"], false);
		}
		"policy-presentation" => {
			assert_eq!(
				semantic_projection(&actual["projected"]),
				semantic_projection(&disposition["fresh"])
			);
			assert_eq!(
				semantic_projection(&stock["projected"]),
				semantic_projection(&disposition["stock"])
			);
			assert_eq!(
				actual["projected"]["outcome"]["status"],
				stock["projected"]["outcome"]["status"]
			);
			if stock["projected"]["outcome"]["status"] == "error" {
				assert_eq!(
					actual["projected"]["outcome"]["class"],
					stock["projected"]["outcome"]["class"]
				);
			}
		}
		_ => panic!("unreviewed C11c disposition"),
	}
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "C11c strict fresh71, bounded children and exclusive GAP_OUT"]
fn fresh_contract() {
	run_contract("fresh");
}

// The independent fresh child never reads these expected outcomes. Stock raw
// observations stay frozen; only immutable function-token moves differ.
fn verify_function_copy_record(actual: &Json, case: &Json, stock: &Json, disposition: &Json) {
	assert_eq!(actual["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["classification"], "Intentional");
	assert_eq!(disposition["authority"], 20725);
	assert_eq!(
		semantic_projection(&actual["projected"]),
		semantic_projection(&disposition["fresh"])
	);
	assert_eq!(
		semantic_projection(&stock["projected"]),
		semantic_projection(&disposition["stock"])
	);
	assert_eq!(actual["observation"]["outcome"]["status"], "value");
	let audit = &actual["fresh_audit"];
	assert_eq!(audit["cleanup_error"], false);
	for key in ["row_idle", "reuse_idle", "reuse_same_store"] {
		assert_eq!(audit[key], true);
	}
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"reuse_live_entries",
		"reuse_payload_bytes",
	] {
		assert_eq!(audit[key], 0);
	}
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "C11c Copy policy, bounded independent fresh8 children"]
fn function_copy_contract() {
	run_contract_cases("fresh", true);
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "C11c frozen supplemental stock8, bounded children"]
fn function_stock_contract() {
	run_contract_cases("stock", true);
}
