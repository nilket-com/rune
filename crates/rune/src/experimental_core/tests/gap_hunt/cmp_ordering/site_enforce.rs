// Frozen C9 per-row C5 sites and C7/C8 frame representations; no discover mode.
const C9_DIAGNOSTIC_PINS: &str = r########"{"schema":1,"rows":[{"index":120,"label":"format/display/Less","retained":{"fresh_sites":[{"end":52,"frames":"[]","source":0,"start":14}],"invocation_hex":"666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a4c65737329"},"source_hex":"70756220666e206d61696e28297b666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a4c657373297d0a70756220666e2063395f726575736528297b377d","category":"Retained-site","source_sha256":"a335297156466dd086cfa6418d55554aac98fd507c89215442586ab8b49e6b53"},{"index":122,"label":"format/display/Equal","retained":{"fresh_sites":[{"end":53,"frames":"[]","source":0,"start":14}],"invocation_hex":"666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a457175616c29"},"source_hex":"70756220666e206d61696e28297b666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a457175616c297d0a70756220666e2063395f726575736528297b377d","category":"Retained-site","source_sha256":"16f74a8939062209a5a4ed51018678a103d58dff83b7d0ccb1919b7020f898e6"},{"index":124,"label":"format/display/Greater","retained":{"fresh_sites":[{"end":55,"frames":"[]","source":0,"start":14}],"invocation_hex":"666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a4772656174657229"},"source_hex":"70756220666e206d61696e28297b666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a47726561746572297d0a70756220666e2063395f726575736528297b377d","category":"Retained-site","source_sha256":"34b812c22aae5e484a46e853af2d1aaa6f8ae66eb2a1dbd9fe158b62fe5f586c"},{"index":127,"label":"format/display-nested","retained":{"fresh_sites":[{"end":55,"frames":"[]","source":0,"start":14}],"invocation_hex":"666f726d61742128227b7d222c287374643a3a636d703a3a4f72646572696e673a3a4c6573732c2929"},"source_hex":"70756220666e206d61696e28297b666f726d61742128227b7d222c287374643a3a636d703a3a4f72646572696e673a3a4c6573732c29297d0a70756220666e2063395f726575736528297b377d","category":"Retained-site","source_sha256":"acf332962f2c7618ea7851b805efdad6b434c3d6e9ec36ee472aae1f7820d32a"},{"index":173,"label":"frames/missing","retained":{"call_chain":[{"end":41,"source":0,"start":34},{"end":63,"source":0,"start":56}],"caller_hex":["696e6e65722829","6f757465722829"],"fresh_sites":[{"end":22,"frames":"[{\"end\":41,\"source\":0,\"start\":34},{\"end\":63,\"source\":0,\"start\":56}]","source":0,"start":11}]},"source_hex":"666e20696e6e657228297b747275652e636d702831297d666e206f7574657228297b696e6e657228297d70756220666e206d61696e28297b6f7574657228297d0a70756220666e2063395f726575736528297b377d","category":"Retained-frame","source_sha256":"cbb65703d94e95efe5f33c566ddf3fba430c9fadfa54b9125c39fc2e0eacc676"},{"index":174,"label":"frames/invalid-rhs","retained":{"call_chain":[{"end":42,"source":0,"start":35},{"end":64,"source":0,"start":57}],"caller_hex":["696e6e65722829","6f757465722829"],"fresh_sites":[{"end":23,"frames":"[{\"end\":42,\"source\":0,\"start\":35},{\"end\":64,\"source\":0,\"start\":57}]","source":0,"start":11}]},"source_hex":"666e20696e6e657228297b312e636d702866616c7365297d666e206f7574657228297b696e6e657228297d70756220666e206d61696e28297b6f7574657228297d0a70756220666e2063395f726575736528297b377d","category":"Retained-frame","source_sha256":"514a55630f58f9343156cb4e192de5bc6e2f2e28aaf1d51e6dc6e7f856ad8641"}]}"########;

fn c9_retained_manifest() -> Result<Json, String> {
	let actual: Json = serde_json::from_str(include_str!("../../cmp_ordering_retained.json"))
		.map_err(|_| "retained manifest JSON")?;
	let lease: Json = serde_json::from_str(C9_RETAINED_PIN).unwrap();
	let diagnostics: Json = serde_json::from_str(C9_DIAGNOSTIC_PINS).unwrap();
	let expected = json!({"schema":2,"leased_parent":lease,"diagnostics":diagnostics});
	if actual != expected {
		return Err("C9 retained manifest drift".into());
	}
	Ok(actual)
}

fn compare_c9_diagnostic_retained(
	label: &str,
	source: &str,
	actual: &Json,
	stock: &Json,
	counts: &Json,
	call_chain: &Json,
) -> Result<(), String> {
	let manifest = c9_retained_manifest()?;
	let rows = manifest["diagnostics"]["rows"]
		.as_array()
		.ok_or("retained rows")?;
	let row = rows
		.iter()
		.find(|row| row["label"] == label)
		.ok_or("unlisted retained diagnostic")?;
	if row["source_hex"] != hex(source.as_bytes()) {
		return Err("retained source drift".into());
	}
	let pin = &row["retained"];
	if actual["sites"] != pin["fresh_sites"] { return Err("retained fresh sites drift".into()); }
    let stock_sites = stock["sites"].as_array().ok_or("stock sites")?;
    let fresh_sites = actual["sites"].as_array().ok_or("fresh sites")?;
    if stock_sites.len() != 1 || fresh_sites.len() != 1 { return Err("diagnostic site count".into()); }
    let old = &stock_sites[0]; let new = &fresh_sites[0];
    if old["source"] != 0 || new["source"] != 0 { return Err("diagnostic source".into()); }
	match row["category"].as_str() {
		Some("Retained-site") => {
            if old["start"] != 0 || old["end"].as_u64().and_then(|v| v.checked_add(1)) != new["end"].as_u64() || old["frames"] != "[]" { return Err("live stock site drift".into()); }
			if *call_chain != json!([]) {
				return Err("retained site caller drift".into());
			}
			let site = &actual["sites"][0];
			let start = usize::try_from(site["start"].as_u64().ok_or("site start")?)
				.map_err(|_| "start overflow")?;
			let end = usize::try_from(site["end"].as_u64().ok_or("site end")?)
				.map_err(|_| "end overflow")?;
			let bytes = source.get(start..end).ok_or("site bytes")?;
			if pin["invocation_hex"] != hex(bytes.as_bytes()) {
				return Err("retained invocation bytes drift".into());
			}
		}
		Some("Retained-frame") => {
            let legacy = old["frames"].as_str().ok_or("stock frames")?;
            if old["start"] != new["start"] || old["end"] != new["end"] || !legacy.starts_with("[CallFrame {") || !legacy.ends_with(" }]") || legacy.matches("CallFrame {").count() != call_chain.as_array().ok_or("callers")?.len() { return Err("live stock frame drift".into()); }
			if *call_chain != pin["call_chain"] {
				return Err("retained actual caller chain drift".into());
			}
			let callers = call_chain.as_array().ok_or("callers")?;
			let mut bytes = Vec::new();
			for at in callers {
				let start = usize::try_from(at["start"].as_u64().ok_or("caller start")?)
					.map_err(|_| "start overflow")?;
				let end = usize::try_from(at["end"].as_u64().ok_or("caller end")?)
					.map_err(|_| "end overflow")?;
				bytes.push(hex(source
					.get(start..end)
					.ok_or("caller bytes")?
					.as_bytes()));
			}
			if json!(bytes) != pin["caller_hex"] {
				return Err("retained caller bytes drift".into());
			}
		}
		_ => return Err("unlisted retained category".into()),
	}
	let mut expected = stock.clone();
	expected["sites"] = pin["fresh_sites"].clone();
	compare_p1(actual, &expected, counts)
}

#[test]
#[cfg(not(miri))]
fn c9_frozen_diagnostics_reject_unlisted_sites_frames_and_callers() {
	let manifest = c9_retained_manifest().unwrap();
	assert_eq!(manifest["diagnostics"]["rows"].as_array().unwrap().len(), 6);
	let counts: Json = serde_json::from_str(C9_PINNED_COUNTS).unwrap();
	for row in manifest["diagnostics"]["rows"].as_array().unwrap() {
		let label = row["label"].as_str().unwrap();
		let source = pins_decode(row["source_hex"].as_str().unwrap());
		let stock = &diagnostic_stock(label, &source, &counts[label]);
		let mut actual = raw_fixture(stock);
		actual["sites"] = row["retained"]["fresh_sites"].clone();
		let callers = if row["category"] == "Retained-frame" {
			row["retained"]["call_chain"].clone()
		} else {
			json!([])
		};
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&actual,
			stock,
			&counts[label],
			&callers
		)
		.is_ok());
		assert!(compare_c9_diagnostic_retained(
			"unlisted",
			&source,
			&actual,
			stock,
			&counts[label],
			&callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"][0]["start"] = json!(bad["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&bad,
			stock,
			&counts[label],
			&callers
		)
		.is_err());
		let mut bad_stock = stock.clone();
		bad_stock["sites"][0]["start"] = json!(1);
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&actual,
			&bad_stock,
			&counts[label],
			&callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"]
			.as_array_mut()
			.unwrap()
			.push(actual["sites"][0].clone());
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&bad,
			stock,
			&counts[label],
			&callers
		)
		.is_err());
		for field in ["kind", "typed", "display", "chain", "status"] {
			let mut bad = actual.clone();
			bad[field] = json!("drift");
			assert!(compare_c9_diagnostic_retained(
				label,
				&source,
				&bad,
				stock,
				&counts[label],
				&callers
			)
			.is_err());
		}
		let mut bad_callers = callers.clone();
		if row["category"] == "Retained-frame" {
			bad_callers[0]["start"] = json!(bad_callers[0]["start"].as_u64().unwrap() + 1);
			let mut bad = actual.clone();
			bad["sites"][0]["frames"] = json!(serde_json::to_string(&json!([callers[0]])).unwrap());
			assert!(compare_c9_diagnostic_retained(
				label,
				&source,
				&bad,
				stock,
				&counts[label],
				&callers
			)
			.is_err());
			let mut bad_source = source.clone().into_bytes();
			let at = callers[0]["start"].as_u64().unwrap() as usize;
			bad_source[at] = b'x';
			assert!(compare_c9_diagnostic_retained(
				label,
				std::str::from_utf8(&bad_source).unwrap(),
				&actual,
				stock,
				&counts[label],
				&callers
			)
			.is_err());
		} else {
			bad_callers = json!([{"source":0,"start":1,"end":2}]);
		}
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&actual,
			stock,
			&counts[label],
			&bad_callers
		)
		.is_err());
	}
}

#[test]
#[cfg(miri)]
fn miri_c9_diagnostic_site() {
	let manifest = c9_retained_manifest().unwrap();
	let selected = ["format/display/Less"];
	let mut checked = std::vec::Vec::new();
	for label in selected {
		let row = manifest["diagnostics"]["rows"]
			.as_array()
			.unwrap()
			.iter()
			.find(|row| row["label"] == label)
			.unwrap();
		let fixture =
			crate::experimental_core::tests::miri_fixtures::fixture("C9-diagnostics", label);
		let counts = &fixture["counts"];
		checked.push(label);
		let label = row["label"].as_str().unwrap();
		let source = pins_decode(row["source_hex"].as_str().unwrap());
		let stock = &diagnostic_stock(label, &source, counts);
		let mut actual = raw_fixture(stock);
		actual["sites"] = row["retained"]["fresh_sites"].clone();
		let callers = if row["category"] == "Retained-frame" {
			row["retained"]["call_chain"].clone()
		} else {
			json!([])
		};
		assert!(
			compare_c9_diagnostic_retained(label, &source, &actual, stock, counts, &callers)
				.is_ok()
		);
		assert!(compare_c9_diagnostic_retained(
			"unlisted", &source, &actual, stock, counts, &callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"][0]["start"] = json!(bad["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(
			compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers).is_err()
		);
		let mut bad_stock = stock.clone();
		bad_stock["sites"][0]["start"] = json!(1);
		assert!(compare_c9_diagnostic_retained(
			label, &source, &actual, &bad_stock, counts, &callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"]
			.as_array_mut()
			.unwrap()
			.push(actual["sites"][0].clone());
		assert!(
			compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers).is_err()
		);
		for field in ["kind", "typed", "display", "chain", "status"] {
			let mut bad = actual.clone();
			bad[field] = json!("drift");
			assert!(
				compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers)
					.is_err()
			);
		}
		let mut bad_callers = callers.clone();
		if row["category"] == "Retained-frame" {
			bad_callers[0]["start"] = json!(bad_callers[0]["start"].as_u64().unwrap() + 1);
			let mut bad = actual.clone();
			bad["sites"][0]["frames"] = json!(serde_json::to_string(&json!([callers[0]])).unwrap());
			assert!(
				compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers)
					.is_err()
			);
			let mut bad_source = source.clone().into_bytes();
			let at = callers[0]["start"].as_u64().unwrap() as usize;
			bad_source[at] = b'x';
			assert!(compare_c9_diagnostic_retained(
				label,
				std::str::from_utf8(&bad_source).unwrap(),
				&actual,
				stock,
				counts,
				&callers
			)
			.is_err());
		} else {
			bad_callers = json!([{"source":0,"start":1,"end":2}]);
		}
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&actual,
			stock,
			counts,
			&bad_callers
		)
		.is_err());
	}
	assert_eq!(checked, selected);
}

#[test]
#[cfg(miri)]
fn miri_c9_diagnostic_frame() {
	let manifest = c9_retained_manifest().unwrap();
	let selected = ["frames/missing"];
	let mut checked = std::vec::Vec::new();
	for label in selected {
		let row = manifest["diagnostics"]["rows"]
			.as_array()
			.unwrap()
			.iter()
			.find(|row| row["label"] == label)
			.unwrap();
		let fixture =
			crate::experimental_core::tests::miri_fixtures::fixture("C9-diagnostics", label);
		let counts = &fixture["counts"];
		checked.push(label);
		let label = row["label"].as_str().unwrap();
		let source = pins_decode(row["source_hex"].as_str().unwrap());
		let stock = &diagnostic_stock(label, &source, counts);
		let mut actual = raw_fixture(stock);
		actual["sites"] = row["retained"]["fresh_sites"].clone();
		let callers = if row["category"] == "Retained-frame" {
			row["retained"]["call_chain"].clone()
		} else {
			json!([])
		};
		assert!(
			compare_c9_diagnostic_retained(label, &source, &actual, stock, counts, &callers)
				.is_ok()
		);
		assert!(compare_c9_diagnostic_retained(
			"unlisted", &source, &actual, stock, counts, &callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"][0]["start"] = json!(bad["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(
			compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers).is_err()
		);
		let mut bad_stock = stock.clone();
		bad_stock["sites"][0]["start"] = json!(1);
		assert!(compare_c9_diagnostic_retained(
			label, &source, &actual, &bad_stock, counts, &callers
		)
		.is_err());
		let mut bad = actual.clone();
		bad["sites"]
			.as_array_mut()
			.unwrap()
			.push(actual["sites"][0].clone());
		assert!(
			compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers).is_err()
		);
		for field in ["kind", "typed", "display", "chain", "status"] {
			let mut bad = actual.clone();
			bad[field] = json!("drift");
			assert!(
				compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers)
					.is_err()
			);
		}
		let mut bad_callers = callers.clone();
		if row["category"] == "Retained-frame" {
			bad_callers[0]["start"] = json!(bad_callers[0]["start"].as_u64().unwrap() + 1);
			let mut bad = actual.clone();
			bad["sites"][0]["frames"] = json!(serde_json::to_string(&json!([callers[0]])).unwrap());
			assert!(
				compare_c9_diagnostic_retained(label, &source, &bad, stock, counts, &callers)
					.is_err()
			);
			let mut bad_source = source.clone().into_bytes();
			let at = callers[0]["start"].as_u64().unwrap() as usize;
			bad_source[at] = b'x';
			assert!(compare_c9_diagnostic_retained(
				label,
				std::str::from_utf8(&bad_source).unwrap(),
				&actual,
				stock,
				counts,
				&callers
			)
			.is_err());
		} else {
			bad_callers = json!([{"source":0,"start":1,"end":2}]);
		}
		assert!(compare_c9_diagnostic_retained(
			label,
			&source,
			&actual,
			stock,
			counts,
			&bad_callers
		)
		.is_err());
	}
	assert_eq!(checked, selected);
}
