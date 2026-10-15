// One pre-declared C9 borrow diagnostic pin; all remaining fields stay exact.
const C9_RETAINED_PIN: &str = r########"{"follow_up":"D-family: leased-parent write borrow primary Snapshot/readers and native wrapper, beside leased-drop take","fresh":{"chain":[],"kind":"WriteBorrowed","typed":{"raw_kind":"WriteBorrowed"}},"index":170,"label":"access/leased-parent-mutate","p1_fresh_counts":{"/kind":0,"/raw":0,"/typed/raw_kind":0},"p1_stock_counts":{"/chain/0/kind":0,"/kind":0,"/raw":0,"/typed/raw_kind":0},"paths":["kind","typed.raw_kind","chain"],"retained":["Retained-kind","Retained-chain"],"ruling_message":"01a13e28-562d-75dd-b7a6-87c1473e4244","ruling_seq":14575,"schema":1,"source_hex":"70756220666e206d61696e28297b6c657420763d5b2261225d3b666f72207820696e20767b782e636d70287b762e7075736828226322293b2262227d293b7d377d0a70756220666e2063395f726575736528297b377d","source_sha256":"ed04c37527950d21b66bfea500cac197003fc09b5c6ee69d008c32ef432a8519","stock":{"chain":[{"display":"Bad argument #0","kind":"BadArgument { arg: 0 }","typed":{"arg":0,"tag":"BadArgument"}}],"kind":"AccessError { error: AccessError { kind: NotAccessibleMut(Snapshot(--000001)) } }","typed":{"raw_kind":"AccessError { error: AccessError { kind: NotAccessibleMut(Snapshot(--000001)) } }"}},"witness":{"base":"620ae245c66ef0e9491eab33388a362c8a12e60d","binary_sha256":"20dff3d71223dfb06f765b0c906c0c5ecae2ce165e5ca466368cf51f6d65fd24","observations_sha256":"d0228f749815a5edc3e75e09e00d07a9d479dcb49f1f01b7ff0bb1d5b9855b44","source":"70756220666e206d61696e28297b6c657420763d5b2261225d3b666f72207820696e20767b762e7075736828226322293b7d377d"}}"########;

fn compare_c9_retained(
	label: &str,
	source: &str,
	actual: &Json,
	expected: &Json,
	counts: &Json,
) -> Result<(), String> {
	let pin: Json = serde_json::from_str(C9_RETAINED_PIN).unwrap();
	let manifest = c9_retained_manifest()?;
	if pin != manifest["leased_parent"] {
		return Err("C9 retained manifest drift".into());
	}
	if label != pin["label"].as_str().unwrap() || hex(source.as_bytes()) != pin["source_hex"] {
		return Err("unlisted C9 retained source".into());
	}
	// Independently pinned stock fields must still be exactly those in the frozen oracle.
	for side in ["kind", "typed", "chain"] {
		if expected[side] != pin["stock"][side] || actual[side] != pin["fresh"][side] {
			return Err("C9 retained primary/chain drift".into());
		}
	}
	let mut declared = expected.clone();
	for side in ["kind", "typed", "chain"] {
		declared[side] = pin["fresh"][side].clone();
	}
	if *counts != pin["p1_stock_counts"] {
		return Err("C9 retained count manifest drift".into());
	}
	let mut fresh_counts = counts.clone();
	// The sole retained native wrapper is absent; its zero-substitution kind field is absent too.
	if fresh_counts
		.as_object_mut()
		.ok_or("counts object")?
		.remove("/chain/0/kind")
		!= Some(json!(0))
	{
		return Err("C9 retained wrapper count drift".into());
	}
	if fresh_counts != pin["p1_fresh_counts"] {
		return Err("C9 retained fresh count drift".into());
	}
	compare_p1(actual, &declared, &fresh_counts)
}

#[test]
#[cfg(not(miri))]
fn c9_retained_borrow_pin_rejects_every_other_semantic_drift() {
	let pin: Json = serde_json::from_str(C9_RETAINED_PIN).unwrap();
	let dispositions: Json = serde_json::from_str(C9_PINNED_DISPOSITIONS).unwrap();
	let counts: Json = serde_json::from_str(C9_PINNED_COUNTS).unwrap();
	let label = pin["label"].as_str().unwrap();
	let source_bytes = (0..pin["source_hex"].as_str().unwrap().len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&pin["source_hex"].as_str().unwrap()[i..i + 2], 16).unwrap())
		.collect::<Vec<_>>();
	let source = std::str::from_utf8(&source_bytes).unwrap();
	let stock = &dispositions[label]["fresh_expectation"];
	let mut fresh = stock.clone();
	for field in ["kind", "typed", "chain"] {
		fresh[field] = pin["fresh"][field].clone();
	}
	assert!(compare_c9_retained(label, source, &fresh, stock, &counts[label]).is_ok());
	assert!(compare_c9_retained("other", source, &fresh, stock, &counts[label]).is_err());
	assert!(compare_c9_retained(label, "different source", &fresh, stock, &counts[label]).is_err());
	for (path, value) in [
		("kind", json!("Other")),
		("typed", json!({"raw_kind":"Other"})),
		("chain", json!([{"kind":"BadArgument { arg: 0 }"}])),
		("class", json!("Other")),
		("display", json!("Other")),
		("sites", json!([])),
		("status", json!("value")),
	] {
		let mut bad = fresh.clone();
		bad[path] = value;
		assert!(
			compare_c9_retained(label, source, &bad, stock, &counts[label]).is_err(),
			"{path}"
		);
	}
	let mut bad_stock = stock.clone();
	bad_stock["kind"] = json!("Other");
	assert!(compare_c9_retained(label, source, &fresh, &bad_stock, &counts[label]).is_err());
	// The envelope's existing lifecycle/site/frame/host checks remain outside this helper.
}

#[test]
#[cfg(miri)]
fn miri_c9_retained_borrow_pin_rejects_every_other_semantic_drift() {
	let pin: Json = serde_json::from_str(C9_RETAINED_PIN).unwrap();
	let fixture = crate::experimental_core::tests::miri_fixtures::fixture(
		"C9-leased",
		"access/leased-parent-mutate",
	);
	let counts = &fixture["counts"];
	let label = pin["label"].as_str().unwrap();
	let source_bytes = (0..pin["source_hex"].as_str().unwrap().len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&pin["source_hex"].as_str().unwrap()[i..i + 2], 16).unwrap())
		.collect::<Vec<_>>();
	let source = std::str::from_utf8(&source_bytes).unwrap();
	assert_eq!(label, "access/leased-parent-mutate");
	let stock = &fixture["disposition"]["fresh_expectation"];
	let mut fresh = stock.clone();
	for field in ["kind", "typed", "chain"] {
		fresh[field] = pin["fresh"][field].clone();
	}
	assert!(compare_c9_retained(label, source, &fresh, stock, counts).is_ok());
	assert!(compare_c9_retained("other", source, &fresh, stock, counts).is_err());
	assert!(compare_c9_retained(label, "different source", &fresh, stock, counts).is_err());
	for (path, value) in [
		("kind", json!("Other")),
		("typed", json!({"raw_kind":"Other"})),
		("chain", json!([{"kind":"BadArgument { arg: 0 }"}])),
		("class", json!("Other")),
		("display", json!("Other")),
		("sites", json!([])),
		("status", json!("value")),
	] {
		let mut bad = fresh.clone();
		bad[path] = value;
		assert!(
			compare_c9_retained(label, source, &bad, stock, counts).is_err(),
			"{path}"
		);
	}
	let mut bad_stock = stock.clone();
	bad_stock["kind"] = json!("Other");
	assert!(compare_c9_retained(label, source, &fresh, &bad_stock, counts).is_err());
	// The envelope's existing lifecycle/site/frame/host checks remain outside this helper.
}
