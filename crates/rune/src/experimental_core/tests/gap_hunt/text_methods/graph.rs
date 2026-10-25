use crate::experimental_core::value::{Type, Value};
struct Graph {
	memo: std::collections::HashMap<(u32, u32), usize>,
	pending: Vec<Value>,
}
impl Graph {
	fn intern(&mut self, value: Value) -> usize {
		let key = match value {
			Value::Heap(h) => Some((h.slot, h.generation)),
			_ => None,
		};
		if let Some(id) = key.and_then(|key| self.memo.get(&key).copied()) {
			return id;
		}
		assert!(self.pending.len() < OBSERVER_NODES);
		let id = self.pending.len();
		if let Some(key) = key {
			self.memo.insert(key, id);
		}
		self.pending.push(value);
		id
	}
	fn node(&mut self, store: &mut Store, value: Value) -> Json {
		use crate::TypeHash;
		let scalar = match value {
			Value::Unit => Some(RuntimeValue::unit()),
			Value::Bool(v) => Some(RuntimeValue::from(v)),
			Value::I64(v) => Some(RuntimeValue::from(v)),
			Value::U64(v) => Some(RuntimeValue::from(v)),
			Value::F64(v) => Some(RuntimeValue::from(v)),
			Value::Char(v) => Some(RuntimeValue::from(v)),
			Value::Ordering(v) => Some(RuntimeValue::from(v)),
			_ => None,
		};
		if let Some(scalar) = scalar {
			return runtime_graph(&scalar)["nodes"][0].clone();
		}
		match store.tag(value).unwrap() {
			Type::String => json!([
				"String",
				crate::alloc::String::HASH.to_string(),
				hex(store.text(value).unwrap().as_bytes())
			]),
			Type::Vec | Type::Tuple => {
				let tag = store.tag(value).unwrap();
				let mut children = Vec::new();
				for at in 0..store.len(value).unwrap() {
					let child = store.get(value, at).unwrap();
					children.push(self.intern(child));
					store.release(child).unwrap();
				}
				let (name, hash) = if tag == Type::Vec {
					("Vec", stock::Vec::HASH)
				} else {
					("Tuple", stock::OwnedTuple::HASH)
				};
				json!([name, hash.to_string(), children])
			}
			Type::Object => {
				let mut fields = Vec::new();
				for (key, child) in store.host_observe_fields(value).unwrap() {
					fields.push((hex(key.as_bytes()), self.intern(*child)));
				}
				json!(["Object", stock::Object::HASH.to_string(), fields])
			}
			Type::Option | Type::Result => {
				let (tag, success, child) = store.variant_parts(value).unwrap();
				let child = child.map(|child| self.intern(child));
				if tag == Type::Option {
					json!(["Option", Option::<RuntimeValue>::HASH.to_string(), child])
				} else {
					json!([
						"Result",
						Result::<RuntimeValue, RuntimeValue>::HASH.to_string(),
						if success { "Ok" } else { "Err" },
						child.unwrap()
					])
				}
			}
			Type::Range => {
				let start = store.field(value, "start").unwrap();
				let end = store.field(value, "end").unwrap();
				let ids = [self.intern(start), self.intern(end)];
				store.release(start).unwrap();
				store.release(end).unwrap();
				json!(["Range", stock::Range::HASH.to_string(), ids])
			}
			Type::RangeFrom
			| Type::RangeFull
			| Type::RangeInclusive
			| Type::RangeToInclusive
			| Type::RangeTo => {
				let (tag, start, end) = store.host_observe_other_range(value).unwrap();
				let (name, hash) = match tag {
					Type::RangeFrom => ("RangeFrom", stock::RangeFrom::HASH),
					Type::RangeFull => ("RangeFull", stock::RangeFull::HASH),
					Type::RangeInclusive => ("RangeInclusive", stock::RangeInclusive::HASH),
					Type::RangeToInclusive => ("RangeToInclusive", stock::RangeToInclusive::HASH),
					Type::RangeTo => ("RangeTo", stock::RangeTo::HASH),
					_ => unreachable!(),
				};
				let ids = start
					.into_iter()
					.chain(end)
					.map(|v| self.intern(v))
					.collect::<Vec<_>>();
				json!([name, hash.to_string(), ids])
			}
			other => panic!("unmapped execution graph node: {other:?}"),
		}
	}
}
fn graph(store: &mut Store, root: Value) -> Json {
	let mut runtime_graph = Graph {
		memo: std::collections::HashMap::new(),
		pending: Vec::new(),
	};
	let root = runtime_graph.intern(root);
	let mut nodes = Vec::new();
	let mut at = 0;
	while at < runtime_graph.pending.len() {
		let value = runtime_graph.pending[at];
		nodes.push(runtime_graph.node(store, value));
		at += 1;
	}
	json!({"root":root,"nodes":nodes})
}
