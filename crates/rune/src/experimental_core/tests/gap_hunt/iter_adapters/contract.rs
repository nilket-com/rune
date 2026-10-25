#[test]
#[cfg(target_os = "linux")]
#[ignore = "C17e frozen execution replay; bounded children and exclusive GAP_OUT"]
fn contract() {
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

/// Whether a new error raised inside a function value a chain called
/// carries stock's frames. Stock reports one site per nested VM execution:
/// the origin, then the native call site of each callback, each with the
/// script frames of its window. Execution reports the origin with every caller
/// in one chain; read in order, the windows' sites and frame counts must
/// consume that chain exactly.
pub(super) fn callback_frames(actual: &Json, stock: &Json) -> bool {
	let (Some(execution), Some(windows)) = (
		actual["observation"]["outcome"]["sites"].as_array(),
		stock["observation"]["outcome"]["sites"].as_array(),
	) else {
		return false;
	};
	let [execution] = execution.as_slice() else {
		return false;
	};
	let Some((origin, callers)) = windows.split_first() else {
		return false;
	};
	if callers.is_empty() {
		return false;
	}
	let chain: Vec<Json> = serde_json::from_str(execution["frames"].as_str().unwrap()).unwrap();
	let site = |at: &Json| json!({"source":at["source"],"start":at["start"],"end":at["end"]});
	if site(execution) != site(origin) {
		return false;
	}
	let frames = |window: &Json| window["frames"].as_str().unwrap().matches("CallFrame {").count();
	let mut used = frames(origin);
	for window in callers {
		if chain.get(used) != Some(&site(window)) {
			return false;
		}
		used += 1 + frames(window);
	}
	used == chain.len()
}

/// `stock` is the pinned stock row.
fn verify_case(
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
		hex(Sha256::digest(serde_json::to_vec(stock).unwrap()).as_ref()),
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
	let mut execution = canonical_record(actual);
	let mut expected = canonical_record(stock);
	if let Some(pin) = rule.get("warnings") {
		// Stock warns that a `for (a, b)` pattern might panic; the new frontend
		// does not report that warning.
		assert_eq!(execution["diagnostics"], pin["fresh"]);
		assert_eq!(expected["diagnostics"], pin["stock"]);
		execution["diagnostics"] = Json::Null;
		expected["diagnostics"] = Json::Null;
	}
	let (f, s) = (&execution["observation"], &expected["observation"]);
	match classification {
		"Exact" => assert_eq!(execution, expected),
		"Intentional-D1" => {
			// A wrong receiver: new names the missing method (D1). Kind, class,
			// tape, sites, logical frames and reuse stay stock-exact.
			assert_eq!(execution, rule["fresh"]);
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
		"Retained-fuel" => {
			// Fuel is charged per expanded instruction: new halts on its own
			// count, with the stock class, an empty tape and stock reuse.
			assert_eq!(execution, rule["fresh"]);
			for record in [f, s] {
				assert_eq!(record["outcome"]["status"], "error");
				assert_eq!(record["outcome"]["class"], "FuelExhausted");
			}
			assert_eq!(f["events"], json!([]));
			assert_eq!(f["reuse"], s["reuse"]);
			assert_eq!(execution["diagnostics"], expected["diagnostics"]);
		}
		"Retained-frame (callback)" => {
			// Sites only: everything else, the logical frames included, is exact.
			assert!(callback_frames(actual, stock));
			let mut execution = execution.clone();
			execution["observation"]["outcome"]["sites"] = s["outcome"]["sites"].clone();
			assert_eq!(execution, expected);
		}
		"Inherited-divergence" => {
			// A new behavior outside C17e that the chain reaches; pinned.
			assert_eq!(execution, rule["fresh"]);
			assert!(rule["note"].as_str().is_some_and(|n| !n.is_empty()));
			assert_eq!(f["outcome"]["status"], s["outcome"]["status"]);
			assert_eq!(f["outcome"]["class"], s["outcome"]["class"]);
			assert_eq!(f["events"], s["events"]);
			assert_eq!(f["reuse"], s["reuse"]);
		}
		"Intentional-function-copy" => {
			// Stock moves a function value passed to an adapter or consumer, so a
			// later use fails; new function values are copies.
			assert_eq!(case["row_class"], "function-move");
			assert_eq!(execution, rule["fresh"]);
			assert_eq!(s["outcome"]["status"], "error");
			assert_eq!(s["outcome"]["class"], "AccessConflict");
			assert!(s["outcome"]["display"]
				.as_str()
				.unwrap()
				.ends_with("it has been moved"));
			assert_ne!(f["outcome"], s["outcome"]);
			assert_eq!(f["reuse"], s["reuse"]);
			assert_eq!(execution["diagnostics"], expected["diagnostics"]);
		}
		"Inherited-refusal" => {
			assert_eq!(actual["observation"]["outcome"], rule["fresh"]);
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
		_ => panic!("unknown C17e disposition"),
	}
	assert_eq!(hex(stdout), stock["stdout_hex"]);
	assert_eq!(hex(stderr), stock["stderr_hex"]);
}

const EXPECTED_COUNTS: [(&str, u64); 7] = [
	("Exact", 861),
	("Inherited-divergence", 3),
	("Inherited-refusal", 184),
	("Intentional-D1", 14),
	("Intentional-function-copy", 31),
	("Retained-frame (callback)", 30),
	("Retained-fuel", 11),
];
