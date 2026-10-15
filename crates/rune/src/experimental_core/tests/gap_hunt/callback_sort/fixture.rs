use crate::runtime::{self as stock, Value};
use std::cell::RefCell;

#[derive(Default)]
struct Tape {
	comparisons: i64,
	events: Vec<Json>,
}
thread_local! {
	static TAPE: RefCell<Tape> = RefCell::new(Tape::default());
}
fn reset_tape() {
	TAPE.with(|t| *t.borrow_mut() = Tape::default());
}
fn tape() -> Vec<Json> {
	TAPE.with(|t| t.borrow().events.clone())
}
fn tick(a: i64, b: i64) -> i64 {
	TAPE.with(|t| {
		let mut t = t.borrow_mut();
		t.comparisons += 1;
		let n = t.comparisons;
		t.events
			.push(json!({"event":"compare","n":n,"a":a.to_string(),"b":b.to_string()}));
		n
	})
}
fn mark(name: &str) -> i64 {
	TAPE.with(|t| {
		t.borrow_mut()
			.events
			.push(json!({"event":"mark","hex":hex(name.as_bytes())}))
	});
	7
}
fn context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c11"]).unwrap();
	module.function("tick", tick).build().unwrap();
	module.function("mark", mark).build().unwrap();
	context.install(module).unwrap();
	context
}
fn vector(items: impl IntoIterator<Item = Value>) -> Value {
	let mut values = crate::alloc::Vec::new();
	for value in items {
		values.try_push(value).unwrap();
	}
	Value::vec(values).unwrap()
}
fn fixture(name: &str) -> (Value, Value) {
	if name == "non-vector" {
		return (Value::from(7i64), vector([]));
	}
	if name == "self-cycle" {
		let v = vector([]);
		{
			let mut values = v.borrow_mut::<stock::Vec>().unwrap();
			values.push(v.clone()).unwrap();
			values.push(v.clone()).unwrap();
		}
		return (v, vector([]));
	}
	if name == "tagged" {
		return (
			vector((0..17i64).map(|id| vector([Value::from(id % 3), Value::from(id)]))),
			vector([]),
		);
	}
	if matches!(name, "children-shared" | "children-distinct") {
		let shared = vector([Value::from(0i64)]);
		let v = vector((0..5i64).map(|i| {
			vector([
				Value::from(4 - i),
				if name == "children-shared" {
					shared.clone()
				} else {
					vector([Value::from(0i64)])
				},
			])
		}));
		return (v, vector([shared]));
	}
	if let Some(depth) = name.strip_prefix("depth/") {
		let n: i64 = depth.parse().unwrap();
		return (vector([Value::from(n), Value::from(n)]), vector([]));
	}
	let name = if name == "ints" {
		"ints/5/descending"
	} else {
		name
	};
	let mut parts = name.split('/');
	assert_eq!(parts.next(), Some("ints"));
	let n: i64 = parts.next().unwrap().parse().unwrap();
	let order = parts.next().unwrap();
	assert!(parts.next().is_none() && (0..=64).contains(&n));
	let values = (0..n).map(|i| {
		Value::from(match order {
			"ascending" => i,
			"descending" => n - 1 - i,
			"duplicates" => (n - 1 - i) % 3,
			_ => panic!("unlisted fixture order"),
		})
	});
	(vector(values), vector([]))
}
fn roots_graph(v: &Value, state: &Value) -> Json {
	graph(
		&Value::tuple({
			let mut values = crate::alloc::Vec::new();
			values.try_push(v.clone()).unwrap();
			values.try_push(state.clone()).unwrap();
			values
		})
		.unwrap(),
	)
}
fn cleanup_fixture(name: &str, v: &Value) -> Json {
	if name == "self-cycle" {
		match v.borrow_mut::<stock::Vec>() {
			Ok(mut values) => {
				values.clear();
				json!({"cycle_cleared":true})
			}
			Err(e) => json!({"cycle_cleared":false,"error":format!("{e:?}")}),
		}
	} else {
		json!({"cycle_cleared":null})
	}
}
