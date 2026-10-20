#[test]
#[cfg(target_os = "linux")]
#[ignore = "O1 frozen fresh replay; bounded children and exclusive GAP_OUT"]
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

/// `frozen` is the pinned pre-O1 stock row; `stock` is its O1 replacement, or
/// the same row where key order was never observable.
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
	assert_eq!(rows.len(), 74);
	let mut counts = std::collections::BTreeMap::new();
	for row in rows {
		*counts
			.entry(row["classification"].as_str().unwrap())
			.or_insert(0u64) += 1;
	}
	assert_eq!(
		dispositions["summary"]["classifications"],
		serde_json::to_value(&counts).unwrap()
	);
	let rule = &admitted(
		include_str!("c17e_admissions.json"),
		&admitted(
			include_str!("c17d_admissions.json"),
		&admitted(
			include_str!("c17b_admissions.json"),
			&admitted(include_str!("o1b_admissions.json"), &rows[index]),
			),
		),
	);
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
		assert_eq!(audit[key], 0);
	}
	assert_eq!(audit["cleanup_error"], false);
	let classification = rule["classification"].as_str().unwrap();
	let refused = classification == "Inherited-refusal";
	assert_eq!(audit["invoked"], !refused);
	assert_eq!(audit["reuse_used_fallback"], refused);
	let (mut f, mut s) = (canonical_record(actual), canonical_record(stock));
	if let Some(pin) = rule.get("warnings") {
		// Stock warns that the `ks` helper's `for (k, _)` pattern might panic;
		// the fresh frontend does not report that warning.
		assert_eq!(f["diagnostics"], pin["fresh"]);
		assert_eq!(s["diagnostics"], pin["stock"]);
		f["diagnostics"] = Json::Null;
		s["diagnostics"] = Json::Null;
	}
	match classification {
		"Exact" => assert_eq!(f, s),
		"Inherited-kind" => {
			// Field writes during iteration: stock's typed kind names a borrow
			// snapshot, which has no fresh counterpart. Everything else is exact.
			assert_eq!(f["observation"]["outcome"]["kind"], rule["fresh_kind"]);
			assert_eq!(s["observation"]["outcome"]["kind"], rule["stock_kind"]);
			f["observation"]["outcome"]["kind"] = Json::Null;
			s["observation"]["outcome"]["kind"] = Json::Null;
			assert_eq!(f, s);
		}
		"Inherited-runtime-refusal" => {
			// Pre-O1 fresh behavior, unchanged here: admitted, then refused while
			// running. Pinned so any change is visible.
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
			assert_eq!(
				actual["observation"]["reuse"],
				stock["observation"]["reuse"]
			);
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
		_ => panic!("unknown O1 disposition"),
	}
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}

/// A later record's admission of a pinned row: the whole disposition it
/// replaces must match exactly before its new rule applies.
pub(super) fn admitted(doc: &str, rule: &Json) -> Json {
	let doc: Json = serde_json::from_str(doc).unwrap();
	let Some(row) = doc["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|row| row["index"] == rule["index"])
	else {
		return rule.clone();
	};
	assert_eq!(&row["legacy"], rule);
	let mut admitted = row["admitted"].clone();
	for key in ["index", "label", "source_sha256", "stock_row_sha256"] {
		assert!(admitted.get(key).is_none());
		admitted[key] = rule[key].clone();
	}
	admitted
}
