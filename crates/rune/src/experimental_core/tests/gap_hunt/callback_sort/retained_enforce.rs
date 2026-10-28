// The retained callback diagnostics are checked against the live stock run.
// These are reviewed source spans for intentional diagnostic presentation.
fn c11_diagnostic_span(label: &str) -> Option<(usize, usize)> {
    Some(match label {
        "failure/first-call-failure" | "failure/later-failure" => (45, 52),
        "failure/all-failing" | "parent/read-len" | "parent/take-drop" => (31, 38),
        "failure/panic" => (31, 55), "parent/read-clone" => (37, 46),
        "parent/read-index" => (37, 41), "parent/read-format" => (37, 54),
        "parent/read-iterate" => (40, 41), "parent/write-push" => (31, 40),
        "parent/write-index" => (31, 37), "parent/nested-same" => (31, 49),
        "parent/noncapturing-closure-read" => (93, 100), "frames/caller-chain" => (31, 54),
        "failure/child-mutation" | "failure/shared-child-mutation" => (50, 66),
        "nested/different-error" => (148, 164), _ => return None,
    })
}

fn c11_enforce_diagnostic(label: &str, source: &str, actual: &Json, stock: &Json, chain: &Json) -> Result<(), String> {
    let (start, end) = c11_diagnostic_span(label).ok_or("unlisted diagnostic label")?;
    if !CASES.iter().any(|case| case.0 == label && case.1 == source) || source.get(start..end).is_none() {
        return Err("diagnostic source drift".into());
    }
    let site = json!({"source": 0, "start": start, "end": end});
    let shape = c11_retained_frame_shape_d1b(label, source, actual, stock, &site, chain)?;
    let expected = if label == "parent/read-format" {
        json!(["Retained-site", "Retained-frame (callback)"])
    } else { json!(["Retained-frame (callback)"]) };
    if shape["categories"] != expected { return Err("diagnostic category drift".into()); }
    Ok(())
}

fn c11_retained_frame_shape_d1b(
	label: &str,
	source: &str,
	actual: &Json,
	stock: &Json,
	frozen_site: &Json,
	chain: &Json,
) -> Result<Json, String> {
	let mut projected = actual.clone();
	let site = &actual["outcome"]["sites"][0];
	let actual_site = format!(
		"{}:{}:{}",
		site["source"].as_u64().ok_or("execution site source")?,
		site["start"].as_u64().ok_or("execution site start")?,
		site["end"].as_u64().ok_or("execution site end")?
	);
	let frozen = format!(
		"{}:{}:{}",
		frozen_site["source"].as_u64().ok_or("stock site source")?,
		frozen_site["start"].as_u64().ok_or("stock site start")?,
		frozen_site["end"].as_u64().ok_or("stock site end")?
	);
	if actual_site != frozen {
		if super::d1b_diagnostics::matches_intentional_site(
			"callback_sort",
			label,
			source,
			"fresh",
			&actual_site,
			&frozen,
		) {
			let mut site = frozen_site.clone();
			site["frames"] = actual["outcome"]["sites"][0]["frames"].clone();
			projected["outcome"]["sites"][0] = site;
		} else if label == "parent/read-format" {
			return Err("D1b source-site drift".into());
		}
	}
	c11_retained_frame_shape(source, &projected, stock, chain)
}
#[test]
#[cfg_attr(miri, ignore = "Miri: live stock observations use bounded child processes")]
fn stale_closure_refusals_are_preserved_and_reclassified() {
	let overlay: Json = serde_json::from_str(C11_STALE_REFUSAL_ADMISSIONS).unwrap();
	assert_eq!(overlay["schema"], 1);
	let rows = overlay["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 6);
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
	let spans = read_json(include_str!("../../callback_sort_refusal_spans.json"));
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let out = std::env::temp_dir().join(format!("rune-c11-stale-controls-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	fs::create_dir(&out).unwrap();
	for (row, expected_index, expected_policy, expected_operation) in [
		(&rows[0], 90, "Retained-frame", "CallClosure"),
		(&rows[1], 91, "Exact", "CallClosure"),
		(&rows[2], 92, "Retained-ownership", "CallClosure"),
		(&rows[3], 93, "Exact", "CallClosure"),
		(&rows[4], 94, "Exact", "Method"),
		(&rows[5], 130, "Exact", "Method"),
	] {
		let label = row["label"].as_str().unwrap();
		assert_eq!(row["index"], expected_index);
		assert_eq!(row["disposition"], expected_policy);
		assert_eq!(row["legacy_policy"], "Refused-surface");
		assert_eq!(dispositions[label]["policy"], "Refused-surface");
		assert_eq!(row["legacy_operation"], expected_operation);
		assert_eq!(
			refusals[label]["fresh_constraint"]["operation"],
			expected_operation
		);
		assert_eq!(row["legacy_refusal_span"], spans[label]);
		assert_eq!(row["source_sha256"], dispositions[label]["source_sha256"]);
		let _stock = live_stock_case(expected_index, &out.join(format!("row-{expected_index}")), &counts).1;
		if expected_policy == "Exact" {
			assert!(row.get("fresh_expectation").is_none());
		} else if expected_policy == "Retained-ownership" {
			assert!(row.get("fresh_expectation").is_none());
			assert_eq!(row["only_allowed_difference"], "/post/nodes/2");
		} else {
			let frame = &row["retained_frame"];
			assert_eq!(frame["categories"], json!(["Retained-frame (callback)"]));
		}
	}
	fs::remove_dir_all(out).unwrap();
}
