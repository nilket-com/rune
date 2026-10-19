// Independent fresh observation; no expected row or sidecar is read here.
use crate::experimental_core::{
	error::{Detail, ErrorKind, MovedAccess, TypeName},
	value::{Type, Value as FreshValue},
};

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

#[test]
fn c11c_graph_distinguishes_live_vec_from_moved_alias() {
	use crate::TypeHash;
	let mut store = Store::new(Limits::default()).unwrap();
	let value = store.list(Type::Vec, &[FreshValue::I64(7)]).unwrap();
	let live = json!({"root":0,"nodes":[["Vec",stock::Vec::HASH.to_string(),[1]],["I64",i64::HASH.to_string(),"7"]]});
	assert_eq!(fresh_graph(&mut store, value), live);
	let taken = store.take_capture(value).unwrap();
	assert_eq!(
		fresh_graph(&mut store, value),
		json!({"root":0,"nodes":[["OpaqueAny",stock::Vec::HASH.to_string(),false,false]]})
	);
	assert_eq!(fresh_graph(&mut store, taken), live);
	store.release(value).unwrap();
	store.release(taken).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}
