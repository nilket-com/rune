use crate::experimental_core::value::{Type, Value as FreshValue};
struct FreshGraph {
	memo: std::collections::HashMap<(u32, u32), usize>,
	pending: Vec<FreshValue>,
}
impl FreshGraph {
	fn intern(&mut self, value: FreshValue) -> usize {
		let key = match value {
			FreshValue::Heap(h) => Some((h.slot, h.generation)),
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
	fn node(&mut self, store: &mut Store, value: FreshValue) -> Json {
		use crate::TypeHash;
		let scalar = match value {
			FreshValue::Unit => Some(Value::unit()),
			FreshValue::Bool(v) => Some(Value::from(v)),
			FreshValue::I64(v) => Some(Value::from(v)),
			FreshValue::U64(v) => Some(Value::from(v)),
			FreshValue::F64(v) => Some(Value::from(v)),
			FreshValue::Char(v) => Some(Value::from(v)),
			FreshValue::Ordering(v) => Some(Value::from(v)),
			_ => None,
		};
		if let Some(scalar) = scalar {
			return graph(&scalar)["nodes"][0].clone();
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
					json!(["Option", Option::<Value>::HASH.to_string(), child])
				} else {
					json!([
						"Result",
						Result::<Value, Value>::HASH.to_string(),
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
			other => panic!("unmapped fresh graph node: {other:?}"),
		}
	}
}
fn fresh_graph(store: &mut Store, root: FreshValue) -> Json {
	let mut graph = FreshGraph {
		memo: std::collections::HashMap::new(),
		pending: Vec::new(),
	};
	let root = graph.intern(root);
	let mut nodes = Vec::new();
	let mut at = 0;
	while at < graph.pending.len() {
		let value = graph.pending[at];
		nodes.push(graph.node(store, value));
		at += 1;
	}
	json!({"root":root,"nodes":nodes})
}
