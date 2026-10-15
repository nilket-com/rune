// C8 frozen per-row enforcement. Actual observations are never rewritten.
fn retained_expected(
	manifest: &Json,
	label: &str,
	source: &str,
	stock: &Json,
	fresh: &Json,
	audit: &Json,
	outcome: &Json,
) -> Json {
	let pinned: Json = serde_json::from_str(PINNED_MANIFEST).unwrap();
	assert_eq!(*manifest, pinned);
	let Some(row) = manifest["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|r| r["label"] == label)
	else {
		return fresh.clone();
	};
	assert_eq!(source, policy_decode(row["source_hex"].as_str().unwrap()));
	assert_eq!(*stock, row["stock"]);
	assert_eq!(*fresh, *stock);
	assert_eq!(audit["returned_root"], false);
	match row["policy"].as_str().unwrap() {
		"Retained-depth" => {
			assert_eq!(audit["invoked"], false);
			assert!(audit["typed_failure_kind"].is_null());
			assert_eq!(audit["call_chain"], row["call_chain"]);
			assert_eq!(outcome["status"], "refused");
			assert_eq!(outcome["kind"], "Unsupported");
			assert_eq!(outcome["refusal"], "DepthLimit");
			assert_eq!(outcome["operation"], row["operation"]);
			let at = &outcome["location"];
			assert_eq!(
				*at,
				json!({"source":row["fresh"]["source"],"start":row["fresh"]["start"],"end":row["fresh"]["end"]})
			);
			let start = at["start"].as_u64().unwrap() as usize;
			let end = at["end"].as_u64().unwrap() as usize;
			assert_eq!(
				source.get(start..end).unwrap(),
				row["refusal_bytes"].as_str().unwrap()
			);
		}
		"Retained-display" | "Retained-frame" => {
			assert_eq!(audit["invoked"], true);
			assert_eq!(audit["typed_failure_kind"], row["fresh"]["class"]);
			assert_eq!(outcome["status"], "error");
			assert_eq!(outcome["kind"], row["fresh"]["class"]);
			if row["policy"] == "Retained-display" {
				assert_eq!(audit["call_chain"], row["fresh_call_chain"]);
				assert_eq!(
					source.get(52..59).unwrap(),
					row["site_bytes"].as_str().unwrap()
				);
			} else {
				assert_eq!(audit["call_chain"], row["call_chain"]);
				for (at, bytes) in row["call_chain"]
					.as_array()
					.unwrap()
					.iter()
					.zip(row["caller_bytes"].as_array().unwrap())
				{
					let start = at["start"].as_u64().unwrap() as usize;
					let end = at["end"].as_u64().unwrap() as usize;
					assert_eq!(source.get(start..end).unwrap(), bytes.as_str().unwrap());
				}
			}
		}
		_ => panic!("unlisted retained category"),
	}
	row["fresh"].clone()
}

const PINNED_MANIFEST: &str = r###"{"schema":1,"rows":[{"policy":"Retained-display","label":"effects/leased-child-drop","source_sha256":"9cb74d376e9f6afe2e81830c744ca763d2fa19ee86f0aba8544515c1fbf55006","source_hex":"70756220666e206d61696e28297b6c657420763d5b312c325d3b666f72207820696e20767b6c657428612c62293d28762c37293b64726f702861293b7d307d","stock":{"chain_displays":[],"class":"AccessConflict","display":"Cannot take, the value is being read from","sites":[{"end":59,"frames":"[]","source":0,"start":52}],"status":"error"},"fresh":{"chain_displays":[],"class":"AccessConflict","display":"Cannot write, the value is being read from","sites":[{"end":59,"frames":"[]","source":0,"start":52}],"status":"error"},"site_bytes":"drop(a)","fresh_call_chain":[],"ruling":"01a13d8c-e7f3-73a4-bb12-86b9fa174bfb","witness_source_sha256":"4bedbe4c07533f095748cdc15ccedf1a06ff677d4f9cea3e67afac662445f74c"},{"label":"effects/rhs-panic","source_hex":"666e2072687328297b7072696e746c6e21282272687322293b70616e6963212822726873206661696c75726522297d70756220666e206d61696e28297b6c657428612c62293d72687328293b377d","source_sha256":"dc68924281830357c081dd37222cde49abba5132c776af5b4b36bb30d34c6402","stock":{"chain_displays":[],"class":"Panic","display":"Panicked: rhs failure","sites":[{"end":46,"frames":"[CallFrame { ip: 2, top: 0, isolated: None, out: keep(2) }]","source":0,"start":25}],"status":"error"},"fresh":{"chain_displays":[],"class":"Panic","display":"Panicked: rhs failure","sites":[{"end":46,"frames":"[{\"end\":75,\"source\":0,\"start\":70}]","source":0,"start":25}],"status":"error"},"call_chain":[{"end":75,"source":0,"start":70}],"caller_bytes":["rhs()"],"policy":"Retained-frame","ruling":"01a13d94-198a-7420-b890-c315239492ba"},{"label":"frames/nested-failure","source_hex":"666e20696e6e657228297b6c657428612c62293d313b377d666e206f7574657228297b696e6e657228297d70756220666e206d61696e28297b6f7574657228297d","source_sha256":"6d991c019c7dd1263593452826952ced10a2cbde82282494373d4bd0b129998b","stock":{"chain_displays":[],"class":"Panic","display":"Panicked: pattern did not match","sites":[{"end":19,"frames":"[CallFrame { ip: 2, top: 0, isolated: None, out: keep(0) }, CallFrame { ip: 5, top: 1, isolated: None, out: keep(0) }]","source":0,"start":14}],"status":"error"},"fresh":{"chain_displays":[],"class":"Panic","display":"Panicked: pattern did not match","sites":[{"end":19,"frames":"[{\"end\":42,\"source\":0,\"start\":35},{\"end\":64,\"source\":0,\"start\":57}]","source":0,"start":14}],"status":"error"},"call_chain":[{"end":42,"source":0,"start":35},{"end":64,"source":0,"start":57}],"caller_bytes":["inner()","outer()"],"policy":"Retained-frame","ruling":"01a13d94-198a-7420-b890-c315239492ba"},{"label":"depth/128","source_hex":"70756220666e206d61696e28297b6c6574202828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828612c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c293d2828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828372c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c293b377d","source_sha256":"abf9ecf466fcc7f537ef63dd6e35bda445abb97861269697bdb641f79bce6138","stock":{"status":"value","type":"I64","value":"7"},"fresh":{"end":533,"invoked":false,"kind":"Unsupported","refusal":"DepthLimit","source":0,"span_hex":"37","start":532,"status":"refused"},"call_chain":[],"caller_bytes":[],"policy":"Retained-depth","ruling":"01a13d96-ef2d-75c6-895a-34ffa7db38db","operation":"DepthLimit","refusal_bytes":"7","default_switch_disclosure":"Retained-depth resource limit; not stock-defect divergence"},{"label":"depth/129","source_hex":"70756220666e206d61696e28297b6c657420282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828612c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c293d282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828282828372c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c292c293b377d","source_sha256":"8235590bdd92d8751b1b714e393d7f34c78ae0ff7bafe4cb521eccd077494e92","stock":{"status":"value","type":"I64","value":"7"},"fresh":{"end":539,"invoked":false,"kind":"Unsupported","refusal":"DepthLimit","source":0,"span_hex":"28372c29","start":535,"status":"refused"},"call_chain":[],"caller_bytes":[],"policy":"Retained-depth","ruling":"01a13d96-ef2d-75c6-895a-34ffa7db38db","operation":"DepthLimit","refusal_bytes":"(7,)","default_switch_disclosure":"Retained-depth resource limit; not stock-defect divergence"}]}"###;

#[test]
fn retained_pins_reject_drift() {
	let manifest: Json =
		serde_json::from_str(include_str!("../../tuple_let_retained.json")).unwrap();
	assert_eq!(manifest["rows"].as_array().unwrap().len(), 5);
	for row in manifest["rows"].as_array().unwrap() {
		let label = row["label"].as_str().unwrap();
		let source = policy_decode(row["source_hex"].as_str().unwrap());
		let depth = row["policy"] == "Retained-depth";
		let callers = if row["policy"] == "Retained-display" {
			&row["fresh_call_chain"]
		} else {
			&row["call_chain"]
		};
		let audit = json!({"invoked":!depth,"returned_root":false,"typed_failure_kind":if depth {Json::Null} else {row["fresh"]["class"].clone()},"call_chain":callers});
		let outcome = if depth {
			json!({"status":"refused","kind":"Unsupported","refusal":"DepthLimit","operation":"DepthLimit","location":{"source":0,"start":row["fresh"]["start"],"end":row["fresh"]["end"]}})
		} else {
			json!({"status":"error","kind":row["fresh"]["class"]})
		};
		let check = |m: &Json, name: &str, src: &str, a: &Json, o: &Json, got: &Json| {
			let expected = retained_expected(m, name, src, &row["stock"], &row["stock"], a, o);
			assert_eq!(*got, expected);
		};
		check(&manifest, label, &source, &audit, &outcome, &row["fresh"]);
		let rejects = |m: &Json, name: &str, src: &str, a: &Json, o: &Json, got: &Json| {
			assert!(std::panic::catch_unwind(|| check(m, name, src, a, o, got)).is_err());
		};
		rejects(
			&manifest,
			"unlisted",
			&source,
			&audit,
			&outcome,
			&row["fresh"],
		);
		rejects(
			&manifest,
			label,
			&(source.clone() + " "),
			&audit,
			&outcome,
			&row["fresh"],
		);
		for field in ["label", "source_sha256", "source_hex", "policy", "ruling"] {
			let mut bad = manifest.clone();
			bad["rows"][0][field] = json!("changed");
			rejects(&bad, label, &source, &audit, &outcome, &row["fresh"]);
		}
		let mut bad = audit.clone();
		bad["invoked"] = json!(depth);
		rejects(&manifest, label, &source, &bad, &outcome, &row["fresh"]);
		for calls in [json!([{"source":0,"start":0,"end":1}]), Json::Null] {
			let mut bad = audit.clone();
			bad["call_chain"] = calls;
			rejects(&manifest, label, &source, &bad, &outcome, &row["fresh"]);
		}
		if depth {
			for field in ["refusal", "operation", "kind"] {
				let mut bad = outcome.clone();
				bad[field] = json!("Pattern");
				rejects(&manifest, label, &source, &audit, &bad, &row["fresh"]);
			}
			let mut bad = outcome.clone();
			bad["location"]["start"] = json!(row["fresh"]["start"].as_u64().unwrap() + 1);
			rejects(&manifest, label, &source, &audit, &bad, &row["fresh"]);
			for field in ["start", "end", "span_hex", "refusal", "invoked"] {
				let mut bad = row["fresh"].clone();
				bad[field] = json!("changed");
				rejects(&manifest, label, &source, &audit, &outcome, &bad);
			}
		} else {
			for chain in [
				json!(["Bad argument #1"]),
				json!(["Bad argument #2"]),
				Json::Null,
			] {
				let mut bad = row["fresh"].clone();
				bad["chain_displays"] = chain;
				rejects(&manifest, label, &source, &audit, &outcome, &bad);
			}
			for field in ["display", "class"] {
				let mut bad = row["fresh"].clone();
				bad[field] = json!("changed");
				rejects(&manifest, label, &source, &audit, &outcome, &bad);
			}
			let mut bad = row["fresh"].clone();
			bad["sites"][0]["start"] = json!(0);
			rejects(&manifest, label, &source, &audit, &outcome, &bad);
			let mut bad = row["fresh"].clone();
			bad["sites"][0]["frames"] = json!("[] changed");
			rejects(&manifest, label, &source, &audit, &outcome, &bad);
		}
		if row["policy"] == "Retained-frame" {
			let mut bad = audit.clone();
			bad["call_chain"] = json!([]);
			rejects(&manifest, label, &source, &bad, &outcome, &row["fresh"]);
			let mut extra = callers.as_array().unwrap().clone();
			extra.push(extra[0].clone());
			bad["call_chain"] = json!(extra);
			rejects(&manifest, label, &source, &bad, &outcome, &row["fresh"]);
			if callers.as_array().unwrap().len() == 2 {
				bad["call_chain"] = json!([callers[1], callers[0]]);
				rejects(&manifest, label, &source, &bad, &outcome, &row["fresh"]);
			}
		}
	}
}
