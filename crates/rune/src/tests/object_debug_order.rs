//! Debug presentation is sorted without changing the map or its iteration order.
prelude!();

#[test]
fn debug_sorts_all_insertion_permutations_by_key_bytes() {
	let keys = ["z", "y", "x", "w"];
	let mut checked = 0;
	for a in 0..4 {
		for b in 0..4 {
			for c in 0..4 {
				for d in 0..4 {
					let order = [a, b, c, d];
					if (0..4).any(|i| (i + 1..4).any(|j| order[i] == order[j])) {
						continue;
					}
					let mut object = Object::new();
					for i in order {
						object
							.insert(
								alloc::String::try_from(keys[i]).unwrap(),
								Value::from(i as i64 + 1),
							)
							.unwrap();
					}
					let before: Vec<_> = object.keys().map(|key| key.to_string()).collect();
					assert_eq!(
						format!("{object:?}"),
						"{\"w\": 4, \"x\": 3, \"y\": 2, \"z\": 1}"
					);
					assert_eq!(
						object.keys().map(|key| key.to_string()).collect::<Vec<_>>(),
						before
					);
					checked += 1;
				}
			}
		}
	}
	assert_eq!(checked, 24);
	let mut object = Object::new();
	for key in ["é", "a", "B"] {
		object
			.insert(alloc::String::try_from(key).unwrap(), Value::from(1i64))
			.unwrap();
	}
	assert_eq!(format!("{object:?}"), "{\"B\": 1, \"a\": 1, \"é\": 1}");
	assert_eq!(format!("{:?}", Object::new()), "{}");
}

#[test]
fn script_debug_sorts_nested_objects() {
	let text: String = rune! { format!("{:?}", #{z: [#{y: 2, x: 1}], a: 0}) };
	assert_eq!(text, "{\"a\": 0, \"z\": [{\"x\": 1, \"y\": 2}]}");
}
