// The peer-frozen 20-row diagnostic allowlist. Full paired observations remain
// pinned; this matcher never rewrites a projection or invents metadata.
fn retained_pins() -> &'static Json {
	static PINS: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
	PINS.get_or_init(|| serde_json::from_str(include_str!("retained.json")).unwrap())
}

fn retained_case(label: &str) -> bool {
	retained_pins()
		.as_array()
		.unwrap()
		.iter()
		.any(|pin| pin["label"] == label)
}

fn retained_matches(
	label: &str,
	source: &str,
	stock: &Json,
	fresh: &Json,
	call_chain: &Json,
) -> Option<&'static str> {
	let pin = retained_pins()
		.as_array()?
		.iter()
		.find(|pin| pin["label"] == label)?;
	// Exact source bytes enforce the frozen SHA-256 identity without adding a
	// hash dependency. The digest/bytes pair is verified in the source package.
	if pin["source"] != source
		|| pin["source_hex"] != hex(source.as_bytes())
		|| stock != &pin["stock"]
		|| fresh != &pin["fresh"]
		|| call_chain != &pin["call_chain"]
	{
		return None;
	}
	let hash = pin["source_sha256"].as_str()?;
	if hash.len() != 64
		|| !hash
			.bytes()
			.all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
	{
		return None;
	}
	match pin["category"].as_str()? {
		"Retained-chain" => {
			let argument = u8::try_from(pin["argument"].as_u64()?).ok()?;
			(stock["chain_displays"] == json!([format!("Bad argument #{argument}")])
				&& fresh["chain_displays"] == json!([])
				&& call_chain == &json!([]))
				.then_some("Retained-chain")
		}
		"Retained-frame" => {
			let sites = pin["callsite_bytes"].as_array()?;
			let chain = call_chain.as_array()?;
			if sites.len() != chain.len() || sites.is_empty() {
				return None;
			}
			for (site, caller) in sites.iter().zip(chain) {
				let start = usize::try_from(site["start"].as_u64()?).ok()?;
				let end = usize::try_from(site["end"].as_u64()?).ok()?;
				if site["source"] != 0
					|| caller != &json!({"source":0,"start":start,"end":end})
					|| site["bytes_hex"] != hex(source.as_bytes().get(start..end)?)
				{
					return None;
				}
			}
			(fresh["sites"][0]["frames"] == serde_json::to_string(call_chain).ok()?)
				.then_some("Retained-frame")
		}
		"Retained-display" => (label == "effects/leased-drop"
			&& source.as_bytes().get(40..47) == Some(b"drop(v)".as_slice())
			&& stock["display"] == "Cannot take, the value is being read from"
			&& fresh["display"] == "Cannot write, the value is being read from")
			.then_some("Retained-display"),
		_ => None,
	}
}

#[test]
#[cfg(not(miri))]
fn retained_pins_reject_drift() {
	let pins = retained_pins().as_array().unwrap();
	assert_eq!(pins.len(), 20);
	let mut categories = std::collections::BTreeMap::new();
	for pin in pins {
		let label = pin["label"].as_str().unwrap();
		let source = pin["source"].as_str().unwrap();
		let stock = &pin["stock"];
		let fresh = &pin["fresh"];
		let chain = &pin["call_chain"];
		let category = pin["category"].as_str().unwrap();
		assert_eq!(
			retained_matches(label, source, stock, fresh, chain),
			Some(category)
		);
		*categories.entry(category).or_insert(0usize) += 1;
		// A known diagnostic pair can never grant a rule to a different row.
		assert!(retained_matches("unlisted/label", source, stock, fresh, chain).is_none());
		assert!(retained_matches(label, &format!("{source} "), stock, fresh, chain).is_none());
		let mut shifted = fresh.clone();
		shifted["sites"][0]["start"] = json!(fresh["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(retained_matches(label, source, stock, &shifted, chain).is_none());
		let mut extra = fresh.clone();
		extra["chain_displays"]
			.as_array_mut()
			.unwrap()
			.push(json!("Bad argument #0"));
		assert!(retained_matches(label, source, stock, &extra, chain).is_none());
		let mut extra_callers = chain.clone();
		extra_callers
			.as_array_mut()
			.unwrap()
			.push(json!({"source":0,"start":0,"end":1}));
		assert!(retained_matches(label, source, stock, fresh, &extra_callers).is_none());
		if category == "Retained-chain" {
			let mut missing = stock.clone();
			missing["chain_displays"] = json!([]);
			assert!(retained_matches(label, source, &missing, fresh, chain).is_none());
			let mut changed_n = stock.clone();
			let n = pin["argument"].as_u64().unwrap();
			changed_n["chain_displays"] =
				json!([format!("Bad argument #{}", if n == 0 { 1 } else { 0 })]);
			assert!(retained_matches(label, source, &changed_n, fresh, chain).is_none());
		}
		if category != "Retained-display" {
			let mut changed_display = fresh.clone();
			changed_display["display"] = json!("Cannot write, the value is being read from");
			if changed_display == *fresh {
				changed_display["display"] = json!("unexpected display");
			}
			assert!(retained_matches(label, source, stock, &changed_display, chain).is_none());
		}
		if category == "Retained-frame" {
			let mut removed = chain.clone();
			removed.as_array_mut().unwrap().pop();
			assert!(retained_matches(label, source, stock, fresh, &removed).is_none());
			let mut changed_frames = stock.clone();
			changed_frames["sites"][0]["frames"] = json!("[]");
			assert!(retained_matches(label, source, &changed_frames, fresh, chain).is_none());
		}
	}
	assert_eq!(
		categories,
		std::collections::BTreeMap::from([
			("Retained-chain", 17),
			("Retained-frame", 2),
			("Retained-display", 1),
		])
	);
}

#[test]
#[cfg(miri)]
fn miri_retained_chain() {
	let pins = retained_pins().as_array().unwrap();
	let selected = ["moved/string/normal", "matrix/vec/type-change/computed/set"];
	let mut checked = std::vec::Vec::new();
	let mut categories = std::collections::BTreeMap::new();
	for label in selected {
		let pin = pins.iter().find(|pin| pin["label"] == label).unwrap();
		checked.push(pin["label"].as_str().unwrap());
		let label = pin["label"].as_str().unwrap();
		let source = pin["source"].as_str().unwrap();
		let stock = &pin["stock"];
		let fresh = &pin["fresh"];
		let chain = &pin["call_chain"];
		let category = pin["category"].as_str().unwrap();
		assert_eq!(
			retained_matches(label, source, stock, fresh, chain),
			Some(category)
		);
		*categories.entry(category).or_insert(0usize) += 1;
		// A known diagnostic pair can never grant a rule to a different row.
		assert!(retained_matches("unlisted/label", source, stock, fresh, chain).is_none());
		assert!(retained_matches(label, &format!("{source} "), stock, fresh, chain).is_none());
		let mut shifted = fresh.clone();
		shifted["sites"][0]["start"] = json!(fresh["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(retained_matches(label, source, stock, &shifted, chain).is_none());
		let mut extra = fresh.clone();
		extra["chain_displays"]
			.as_array_mut()
			.unwrap()
			.push(json!("Bad argument #0"));
		assert!(retained_matches(label, source, stock, &extra, chain).is_none());
		let mut extra_callers = chain.clone();
		extra_callers
			.as_array_mut()
			.unwrap()
			.push(json!({"source":0,"start":0,"end":1}));
		assert!(retained_matches(label, source, stock, fresh, &extra_callers).is_none());
		if category == "Retained-chain" {
			let mut missing = stock.clone();
			missing["chain_displays"] = json!([]);
			assert!(retained_matches(label, source, &missing, fresh, chain).is_none());
			let mut changed_n = stock.clone();
			let n = pin["argument"].as_u64().unwrap();
			changed_n["chain_displays"] =
				json!([format!("Bad argument #{}", if n == 0 { 1 } else { 0 })]);
			assert!(retained_matches(label, source, &changed_n, fresh, chain).is_none());
		}
		if category != "Retained-display" {
			let mut changed_display = fresh.clone();
			changed_display["display"] = json!("Cannot write, the value is being read from");
			if changed_display == *fresh {
				changed_display["display"] = json!("unexpected display");
			}
			assert!(retained_matches(label, source, stock, &changed_display, chain).is_none());
		}
		if category == "Retained-frame" {
			let mut removed = chain.clone();
			removed.as_array_mut().unwrap().pop();
			assert!(retained_matches(label, source, stock, fresh, &removed).is_none());
			let mut changed_frames = stock.clone();
			changed_frames["sites"][0]["frames"] = json!("[]");
			assert!(retained_matches(label, source, &changed_frames, fresh, chain).is_none());
		}
	}
	assert_eq!(checked, selected);
	assert_eq!(categories.values().sum::<usize>(), selected.len());
}

#[test]
#[cfg(miri)]
fn miri_retained_frame() {
	let pins = retained_pins().as_array().unwrap();
	let selected = ["effects/receiver-panic", "nested/get-error"];
	let mut checked = std::vec::Vec::new();
	let mut categories = std::collections::BTreeMap::new();
	for label in selected {
		let pin = pins.iter().find(|pin| pin["label"] == label).unwrap();
		checked.push(pin["label"].as_str().unwrap());
		let label = pin["label"].as_str().unwrap();
		let source = pin["source"].as_str().unwrap();
		let stock = &pin["stock"];
		let fresh = &pin["fresh"];
		let chain = &pin["call_chain"];
		let category = pin["category"].as_str().unwrap();
		assert_eq!(
			retained_matches(label, source, stock, fresh, chain),
			Some(category)
		);
		*categories.entry(category).or_insert(0usize) += 1;
		// A known diagnostic pair can never grant a rule to a different row.
		assert!(retained_matches("unlisted/label", source, stock, fresh, chain).is_none());
		assert!(retained_matches(label, &format!("{source} "), stock, fresh, chain).is_none());
		let mut shifted = fresh.clone();
		shifted["sites"][0]["start"] = json!(fresh["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(retained_matches(label, source, stock, &shifted, chain).is_none());
		let mut extra = fresh.clone();
		extra["chain_displays"]
			.as_array_mut()
			.unwrap()
			.push(json!("Bad argument #0"));
		assert!(retained_matches(label, source, stock, &extra, chain).is_none());
		let mut extra_callers = chain.clone();
		extra_callers
			.as_array_mut()
			.unwrap()
			.push(json!({"source":0,"start":0,"end":1}));
		assert!(retained_matches(label, source, stock, fresh, &extra_callers).is_none());
		if category == "Retained-chain" {
			let mut missing = stock.clone();
			missing["chain_displays"] = json!([]);
			assert!(retained_matches(label, source, &missing, fresh, chain).is_none());
			let mut changed_n = stock.clone();
			let n = pin["argument"].as_u64().unwrap();
			changed_n["chain_displays"] =
				json!([format!("Bad argument #{}", if n == 0 { 1 } else { 0 })]);
			assert!(retained_matches(label, source, &changed_n, fresh, chain).is_none());
		}
		if category != "Retained-display" {
			let mut changed_display = fresh.clone();
			changed_display["display"] = json!("Cannot write, the value is being read from");
			if changed_display == *fresh {
				changed_display["display"] = json!("unexpected display");
			}
			assert!(retained_matches(label, source, stock, &changed_display, chain).is_none());
		}
		if category == "Retained-frame" {
			let mut removed = chain.clone();
			removed.as_array_mut().unwrap().pop();
			assert!(retained_matches(label, source, stock, fresh, &removed).is_none());
			let mut changed_frames = stock.clone();
			changed_frames["sites"][0]["frames"] = json!("[]");
			assert!(retained_matches(label, source, &changed_frames, fresh, chain).is_none());
		}
	}
	assert_eq!(checked, selected);
	assert_eq!(categories.values().sum::<usize>(), selected.len());
}

#[test]
#[cfg(miri)]
fn miri_retained_display() {
	let pins = retained_pins().as_array().unwrap();
	let selected = ["effects/leased-drop"];
	let mut checked = std::vec::Vec::new();
	let mut categories = std::collections::BTreeMap::new();
	for label in selected {
		let pin = pins.iter().find(|pin| pin["label"] == label).unwrap();
		checked.push(pin["label"].as_str().unwrap());
		let label = pin["label"].as_str().unwrap();
		let source = pin["source"].as_str().unwrap();
		let stock = &pin["stock"];
		let fresh = &pin["fresh"];
		let chain = &pin["call_chain"];
		let category = pin["category"].as_str().unwrap();
		assert_eq!(
			retained_matches(label, source, stock, fresh, chain),
			Some(category)
		);
		*categories.entry(category).or_insert(0usize) += 1;
		// A known diagnostic pair can never grant a rule to a different row.
		assert!(retained_matches("unlisted/label", source, stock, fresh, chain).is_none());
		assert!(retained_matches(label, &format!("{source} "), stock, fresh, chain).is_none());
		let mut shifted = fresh.clone();
		shifted["sites"][0]["start"] = json!(fresh["sites"][0]["start"].as_u64().unwrap() + 1);
		assert!(retained_matches(label, source, stock, &shifted, chain).is_none());
		let mut extra = fresh.clone();
		extra["chain_displays"]
			.as_array_mut()
			.unwrap()
			.push(json!("Bad argument #0"));
		assert!(retained_matches(label, source, stock, &extra, chain).is_none());
		let mut extra_callers = chain.clone();
		extra_callers
			.as_array_mut()
			.unwrap()
			.push(json!({"source":0,"start":0,"end":1}));
		assert!(retained_matches(label, source, stock, fresh, &extra_callers).is_none());
		if category == "Retained-chain" {
			let mut missing = stock.clone();
			missing["chain_displays"] = json!([]);
			assert!(retained_matches(label, source, &missing, fresh, chain).is_none());
			let mut changed_n = stock.clone();
			let n = pin["argument"].as_u64().unwrap();
			changed_n["chain_displays"] =
				json!([format!("Bad argument #{}", if n == 0 { 1 } else { 0 })]);
			assert!(retained_matches(label, source, &changed_n, fresh, chain).is_none());
		}
		if category != "Retained-display" {
			let mut changed_display = fresh.clone();
			changed_display["display"] = json!("Cannot write, the value is being read from");
			if changed_display == *fresh {
				changed_display["display"] = json!("unexpected display");
			}
			assert!(retained_matches(label, source, stock, &changed_display, chain).is_none());
		}
		if category == "Retained-frame" {
			let mut removed = chain.clone();
			removed.as_array_mut().unwrap().pop();
			assert!(retained_matches(label, source, stock, fresh, &removed).is_none());
			let mut changed_frames = stock.clone();
			changed_frames["sites"][0]["frames"] = json!("[]");
			assert!(retained_matches(label, source, &changed_frames, fresh, chain).is_none());
		}
	}
	assert_eq!(checked, selected);
	assert_eq!(categories.values().sum::<usize>(), selected.len());
}
