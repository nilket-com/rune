#[test]
fn observer_graph_alias_cycle_cleanup() {
	let (value, _) = input_fixture("children-shared");
	let observed = runtime_graph(&value);
	let rows = observed["nodes"][0][2].as_array().unwrap();
	let children = rows
		.iter()
		.map(|id| {
			observed["nodes"][id.as_u64().unwrap() as usize][2][1]
				.as_u64()
				.unwrap()
		})
		.collect::<Vec<_>>();
	assert!(children.iter().all(|id| *id == children[0]));
	let (value, _) = input_fixture("self-cycle");
	let observed = runtime_graph(&value);
	assert_eq!(observed["nodes"].as_array().unwrap().len(), 1);
	assert_eq!(observed["nodes"][0][2], json!([0, 0]));
	assert_eq!(cleanup_fixture("self-cycle", &value)["cycle_cleared"], true);
	assert_eq!(runtime_graph(&value)["nodes"][0][2], json!([]));
}
#[test]
fn observer_before_and_post_are_independent_snapshots() {
	let (v, state) = input_fixture("ints/3/descending");
	let before = roots_graph(&v, &state);
	v.borrow_mut::<stock::Vec>()
		.unwrap()
		.push(RuntimeValue::from(9i64))
		.unwrap();
	let post = roots_graph(&v, &state);
	assert_ne!(before, post);
	let before_vec = before["nodes"][0][2][0].as_u64().unwrap() as usize;
	let post_vec = post["nodes"][0][2][0].as_u64().unwrap() as usize;
	assert_eq!(before["nodes"][before_vec][2].as_array().unwrap().len(), 3);
	assert_eq!(post["nodes"][post_vec][2].as_array().unwrap().len(), 4);
}
#[test]
fn observer_event_tape_is_ordered_and_resettable() {
	reset_tape();
	assert_eq!(tick(2, 1), 1);
	assert_eq!(mark("x"), 7);
	assert_eq!(
		tape(),
		vec![
			json!({"event":"compare","n":1,"a":"2","b":"1"}),
			json!({"event":"mark","hex":"78"})
		]
	);
	reset_tape();
	assert!(tape().is_empty());
	TAPE.with(|t| assert_eq!(t.borrow().comparisons, 0));
}
