#[test]
fn observer_graph_preserves_aliases_and_breaks_cycles_without_protocols() {
	cleanup();
	let shared = make("dag-vec");
	let distinct = make("distinct-equal");
	assert_eq!(runtime_graph(&shared)["nodes"].as_array().unwrap().len(), 3);
	assert_eq!(runtime_graph(&distinct)["nodes"].as_array().unwrap().len(), 5);
	let alias = runtime_graph(&shared);
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
		assert!(runtime_graph(&value)["nodes"].as_array().unwrap().len() <= 3);
		cleanup();
	}
	let custom = make("custom");
	let _ = runtime_graph(&custom);
	assert!(!events()
		.iter()
		.any(|s| s.starts_with("protocol/") || s.starts_with("forbidden/")));
	cleanup();
}
#[test]
fn observer_scalar_bits_and_empty_tuple_are_not_coerced() {
	assert_eq!(runtime_graph(&RuntimeValue::unit())["nodes"][0][0], "Unit");
	assert_eq!(runtime_graph(&tuple([]))["nodes"][0][0], "Tuple");
	for bits in [
		0x8000000000000000u64,
		0x7ff8000000000042,
		0xfff800000000002a,
		0x7ff0000000000001,
	] {
		let value = RuntimeValue::from(f64::from_bits(bits));
		assert_eq!(runtime_graph(&value)["nodes"][0][2], format!("{bits:016x}"));
	}
}

#[test]
fn certified_json_repeated_equal_strings_keep_distinct_allocations() {
	let source = r#"pub fn main(){c10::parse("[\"same\",\"same\",{\"s\":\"same\"}]").unwrap()}"#;
	let context = context();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			FUEL,
		)
		.unwrap();
	let first = store.get(root.value, 0).unwrap();
	let second = store.get(root.value, 1).unwrap();
	assert_ne!(first, second);
	assert_eq!(store.text(first).unwrap(), "same");
	assert_eq!(store.text(second).unwrap(), "same");
	let object = store.get(root.value, 2).unwrap();
	let third = store.field(object, "s").unwrap();
	assert_ne!(first, third);
	assert_ne!(second, third);
	assert_eq!(store.text(third).unwrap(), "same");
	for value in [first, second, object, third] {
		store.release(value).unwrap();
	}
	store.root_release(root).unwrap();
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}
