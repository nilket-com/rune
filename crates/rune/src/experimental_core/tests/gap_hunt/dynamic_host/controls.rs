#[test]
fn observer_graph_preserves_aliases_and_breaks_cycles_without_protocols() {
	cleanup();
	let shared = make("dag-vec");
	let distinct = make("distinct-equal");
	assert_eq!(graph(&shared)["nodes"].as_array().unwrap().len(), 3);
	assert_eq!(graph(&distinct)["nodes"].as_array().unwrap().len(), 5);
	let alias = graph(&shared);
	assert_eq!(alias["nodes"][0][2][0], alias["nodes"][0][2][1]);
	for name in [
		"cycle-vec",
		"cycle-object",
		"cycle-two",
		"cycle-tuple",
		"cycle-option",
		"cycle-result",
	] {
		let value = make(name);
		assert!(graph(&value)["nodes"].as_array().unwrap().len() <= 3);
		cleanup();
	}
	let custom = make("custom");
	let _ = graph(&custom);
	assert!(!events()
		.iter()
		.any(|s| s.starts_with("protocol/") || s.starts_with("forbidden/")));
	cleanup();
}
#[test]
fn observer_scalar_bits_and_empty_tuple_are_not_coerced() {
	assert_eq!(graph(&Value::unit())["nodes"][0][0], "Unit");
	assert_eq!(graph(&tuple([]))["nodes"][0][0], "Tuple");
	for bits in [
		0x8000000000000000u64,
		0x7ff8000000000042,
		0xfff800000000002a,
		0x7ff0000000000001,
	] {
		let value = Value::from(f64::from_bits(bits));
		assert_eq!(graph(&value)["nodes"][0][2], format!("{bits:016x}"));
	}
}
