// C17e new observation graph: the C15b graph, with moved handles observable.
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
		// A moved handle is observed as stock's inaccessible `OpaqueAny`.
		{
			use crate::experimental_core::error::{Detail, ErrorKind, MovedAccess};
			if matches!(store.len(value), Err(error) if error.kind == ErrorKind::AccessConflict
				&& matches!(error.detail, Detail::Moved { access: MovedAccess::Read }))
			{
				let hash = store
					.diagnostic_type(value)
					.unwrap()
					.builtin_hash()
					.unwrap()
					.to_string();
				return json!(["OpaqueAny", hash, false, false]);
			}
		}
		if let Value::Type(tag) = value {
			let hash = match tag {
				Type::Struct(n) => {
					crate::Hash::new(crate::experimental_core::nominal::hash_copy(n).unwrap())
				}
				_ => crate::experimental_core::error::TypeName::from_type(tag)
					.builtin_hash()
					.unwrap(),
			};
			return json!(["Type", stock::Type::HASH.to_string(), hash.to_string()]);
		}
		match store.tag(value).unwrap() {
			Type::Struct(n) => {
				use crate::experimental_core::nominal::{self, Shape};
				let shape = nominal::shape_copy(n).unwrap();
				let mut buf = [0; 256];
				let len = nominal::name_copy(n, &mut buf).unwrap();
				let name = core::str::from_utf8(&buf[..len]).unwrap();
				let hash = crate::Hash::new(nominal::hash_copy(n).unwrap());
				let (kind, fields, children) = if shape == Shape::Named {
					let data = store.host_observe_fields(value).unwrap();
					(
						"Struct",
						data.iter()
							.enumerate()
							.map(|(i, (k, _))| (i, hex(k.as_bytes())))
							.collect::<Vec<_>>(),
						data.iter()
							.map(|(_, v)| self.intern(*v))
							.collect::<Vec<_>>(),
					)
				} else {
					(
						if shape == Shape::Empty {
							"Empty"
						} else {
							"Tuple"
						},
						Vec::new(),
						store
							.host_observe_values(value)
							.unwrap()
							.iter()
							.map(|v| self.intern(*v))
							.collect::<Vec<_>>(),
					)
				};
				json!([
					"Nominal",
					hash.to_string(),
					crate::Hash::new(nominal::variant_info(n).unwrap().0).to_string(),
					name,
					kind,
					fields,
					children
				])
			}
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
				let data = store.host_observe_fields(value).unwrap();
				let mut fields = Vec::new();
				for (key, child) in data {
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
pub(super) fn graph(store: &mut Store, root: Value) -> Json {
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
