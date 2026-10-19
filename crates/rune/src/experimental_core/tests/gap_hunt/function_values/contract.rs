#[test]
#[cfg(target_os = "linux")]
#[ignore = "C16f frozen fresh replay; bounded children and exclusive GAP_OUT"]
fn fresh_contract() {
	run_contract("fresh");
}

fn canonical_record(index: usize, record: &Json) -> Json {
	let mut observation = record["observation"].clone();
	for field in ["outcome", "reuse"] {
		let outcome = &mut observation[field];
		if outcome["status"] != "error" {
			continue;
		}
		outcome.as_object_mut().unwrap().remove("raw");
		for site in outcome["sites"].as_array_mut().unwrap() {
			// Stock VM addresses/frame encoding are audit data. Exact origin
			// spans and the entire typed logical call chain are checked below.
			site.as_object_mut().unwrap().remove("frames");
		}
		if index == 95 || index == 96 {
			let kind = outcome["kind"].as_str().unwrap();
			if kind.contains("full_name:") {
				let prefix = "UnsupportedCallFn { actual: Any(AnyTypeInfo { full_name: 0x";
				let suffix = if index == 95 {
					", hash: 0x1cad9186c9641c4f }) }"
				} else {
					", hash: 0xbd7ccf79d00ee140 }) }"
				};
				let address = kind.strip_prefix(prefix).unwrap().strip_suffix(suffix).unwrap();
				assert!((9..=16).contains(&address.len()));
				assert!(address.bytes().all(|b| b.is_ascii_hexdigit()));
				outcome["kind"] = json!(format!("{prefix}ADDRESS{suffix}"));
			}
		}
	}
	json!({"observation":observation,"logical":record["logical"],"diagnostics":record["diagnostics"]})
}

fn verify_fresh_case(index: usize, case: &Json, stock: &Json, actual: &Json, stdout: &[u8], stderr: &[u8]) {
	use sha2::{Digest, Sha256};
	let dispositions: Json = serde_json::from_str(include_str!("dispositions.json")).unwrap();
	assert_eq!(dispositions["authority"], 21337);
	assert_eq!(dispositions["rows"].as_array().unwrap().len(), 101);
	let rule = &dispositions["rows"][index];
	assert_eq!(rule["index"], index);
	assert_eq!(rule["label"], case["label"]);
	assert_eq!(rule["source_sha256"], case["source_sha256"]);
	assert_eq!(hex(Sha256::digest(case["source"].as_str().unwrap().as_bytes()).as_ref()), rule["source_sha256"]);
	// The stock source and whole raw row are frozen independently in stock.json.
	assert_eq!(stock["source_sha256"], case["source_sha256"]);
	let mut raw_stock = stock.clone();
	for key in ["stdout_hex", "stderr_hex"] { raw_stock.as_object_mut().unwrap().remove(key); }
	assert_eq!(hex(Sha256::digest(serde_json::to_vec(&raw_stock).unwrap()).as_ref()), rule["stock_row_sha256"]);
	let audit = &actual["fresh_audit"];
	for key in ["row_idle", "executor_idle"] { assert_eq!(audit[key], true); }
	for key in ["row_live_entries", "row_payload_bytes", "live_entries", "payload_bytes"] { assert_eq!(audit[key], 0); }
	assert_eq!(audit["cleanup_error"], false);
	let refused = rule["classification"] == "Inherited-refusal";
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);
	match rule["classification"].as_str().unwrap() {
		"Exact" => assert_eq!(canonical_record(index, actual), canonical_record(index, stock)),
		"Intentional-C11c-Copy" => {
			assert!(matches!(index, 93 | 94));
			assert_eq!(case["policy"], "C11c-Copy-candidate");
			assert_eq!(stock["observation"]["outcome"]["class"], "AccessConflict");
			assert_eq!(canonical_record(index, actual), canonical_record(index, &rule["fresh"]));
		},
		"Inherited-diagnostic" => {
			assert_eq!(index, 96);
			assert_eq!(case["label"], "control/named-type-call");
			assert_eq!(rule["before_full_equals_after_except_type_name_pointer"], true);
			assert_eq!(canonical_record(index, actual), canonical_record(index, &rule["fresh"]));
		},
		"Inherited-refusal" => {
			assert_eq!(actual["diagnostics"], rule["before_diagnostics"]);
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
			assert_eq!(actual["observation"]["events"], json!([]));
			assert_eq!(actual["observation"]["reuse"], stock["observation"]["reuse"]);
			assert_eq!(actual["logical"], Json::Null);
		},
		_ => panic!("unknown C16f disposition"),
	}
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}
