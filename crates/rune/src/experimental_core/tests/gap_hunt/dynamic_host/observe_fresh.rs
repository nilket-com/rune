// Independent fresh observation; no expected row or sidecar is read here.
use crate::experimental_core::{
	error::Detail,
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
fn fresh_location(at: crate::experimental_core::error::Location, source: &str) -> Json {
	json!({"source":at.source,"start":at.start,"end":at.end,"span_hex":hex(&source.as_bytes()[at.start as usize..at.end as usize])})
}
fn fresh_failure(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
	source: &str,
) -> Json {
	use crate::experimental_core::error::ErrorKind;
	if let Detail::HostDynamic { reason, type_hash } = failure.error.detail {
		assert_eq!(failure.error.kind, ErrorKind::Host);
		return json!({"status":"error","kind":"Host","detail":{"tag":"HostDynamic","reason":format!("{reason:?}"),"type_hash":type_hash.map(|h|h.to_string())},"display":failure.message(program).to_string(),"primary":failure.error.location.map(|at|fresh_location(at,source)),"call_chain":failure.call_chain.iter().map(|at|fresh_location(*at,source)).collect::<Vec<_>>(),"chain":[],"host_message":failure.host_message});
	}
	if let Detail::HostExpectedType {
		expected,
		actual,
		arg,
	} = failure.error.detail
	{
		// Reconstruct Debug exclusively from actual typed metadata. P1 folds
		// exactly the two full_name pointers. Raw Failure stays in the audit.
		fn info(hash: crate::Hash) -> stock::TypeInfo {
			use crate::TypeHash;
			if hash == crate::alloc::String::HASH {
				stock::TypeInfo::named::<crate::alloc::String>()
			} else if hash == i64::HASH {
				stock::TypeInfo::named::<i64>()
			} else {
				panic!("unmapped actual native type hash: {hash}");
			}
		}
		let kind = stock::VmErrorKind::ExpectedType {
			expected: info(expected),
			actual: info(actual),
		};
		let chain = arg.map(|arg|json!({"kind":format!("BadArgument {{ arg: {arg} }}"),"display":format!("Bad argument #{arg}")})).into_iter().collect::<Vec<_>>();
		let frames = if failure.call_chain.is_empty() {
			"[]".into()
		} else {
			serde_json::to_string(
				&failure
					.call_chain
					.iter()
					.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
					.collect::<Vec<_>>(),
			)
			.unwrap()
		};
		let sites = failure
			.error
			.location
			.iter()
			.map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}))
			.collect::<Vec<_>>();
		return json!({"status":"error","class":"Type","kind":c10_p1_debug(&format!("{kind:?}"),2).unwrap(),"display":failure.message(program).to_string(),"chain":chain,"sites":sites});
	}
	panic!("unmapped actual fresh error: {failure:?}")
}
fn observe_dynamic_fresh(source: &str) -> (Json, Json) {
	let context = fresh_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c10.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let compiled = frontend::compile(&mut sources, &context, &mut diagnostics, 1);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let trace = Executor::host_trace_begin();
	let mut invoked = false;
	let mut returned = false;
	let mut cleanup_error = None::<String>;
	let mut raw_failure = Json::Null;
	let mut actual_primary_call = false;
	let mut actual_callers = true;
	let outcome = match &compiled {
		Err(e) => {
			json!({"status":"refused","kind":format!("{:?}",e.kind),"operation":e.operation,"refusal":e.refusal.map(|r|r.name()),"primary":e.location.map(|at|fresh_location(at,source))})
		}
		Ok(program) => {
			invoked = true;
			match executor.invoke(
				program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				FUEL,
			) {
				Ok(root) => {
					returned = true;
					let graph = fresh_graph(&mut store, root.value);
					store.root_release(root).unwrap();
					json!({"status":"value","graph":graph})
				}
				Err(failure) => {
					cleanup_error = failure.cleanup_error.map(|e| format!("{e:?}"));
					raw_failure = json!({"raw":format!("{failure:?}"),"error":format!("{:?}",failure.error),"host_message":failure.host_message,"call_chain":format!("{:?}",failure.call_chain)});
					let is_call = |at| {
						(0..program.code.functions()).any(|id| {
							let function = program.code.function(id as u32).unwrap();
							function.locations.iter().enumerate().any(|(pc, location)| {
								*location == at
									&& matches!(
										function.operation(pc).unwrap(),
										crate::experimental_core::code::Op::Call { .. }
									)
							})
						})
					};
					actual_primary_call = failure.error.location.is_some_and(is_call);
					actual_callers = failure.call_chain.iter().all(|at| is_call(*at));
					fresh_failure(&failure, program, source)
				}
			}
		}
	};
	let boundaries = trace.finish();
	let before_cleanup = events();
	let host_calls = before_cleanup
		.iter()
		.filter(|s| {
			s.starts_with("make/")
				|| s.starts_with("fallible/")
				|| matches!(s.as_str(), "parse" | "mutate" | "inspect")
		})
		.count();
	let protocol_calls = before_cleanup
		.iter()
		.filter(|s| s.starts_with("protocol/") || s.starts_with("forbidden/"))
		.count();
	cleanup();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let row_idle = executor.is_idle();
	// The same compiled program/executor/store performs reuse after invocations.
	// Compile refusals use an independent clean program on that executor/store.
	let reuse = if let Ok(program) = &compiled {
		let root = executor
			.invoke(
				program,
				&mut store,
				program.function(Hash::type_hash(["c10_reuse"])).unwrap(),
				&[],
				FUEL,
			)
			.unwrap();
		let graph = fresh_graph(&mut store, root.value);
		store.root_release(root).unwrap();
		json!({"status":"value","graph":graph})
	} else {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory("pub fn main(){7}").unwrap())
			.unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 2).unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				FUEL,
			)
			.unwrap();
		let graph = fresh_graph(&mut store, root.value);
		store.root_release(root).unwrap();
		json!({"status":"value","graph":graph})
	};
	(
		json!({"outcome":outcome,"diagnostics":format!("{:?}",diagnostics.diagnostics()),"events":before_cleanup,"events_after_reuse":events(),"host_calls":host_calls,"protocol_calls":protocol_calls,"reuse":reuse,"invoked":invoked}),
		json!({"raw_failure":raw_failure,"boundaries":boundaries,"returned_root":returned,"cleanup_error":cleanup_error,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"row_idle":row_idle,"final_live_entries":store.live_entries(),"final_payload_bytes":store.payload_bytes(),"final_idle":executor.is_idle(),"actual_primary_call":actual_primary_call,"actual_callers":actual_callers,"same_program_reuse":compiled.is_ok()}),
	)
}
fn c10_fresh_audit(audit: &Json, policy: &str, label: &str) {
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"final_live_entries",
		"final_payload_bytes",
	] {
		assert_eq!(audit[key], 0, "{label} {key}");
	}
	for key in ["row_idle", "final_idle"] {
		assert_eq!(audit[key], true, "{label} {key}");
	}
	assert!(
		audit["cleanup_error"].is_null(),
		"{label} secondary cleanup"
	);
	assert_eq!(
		audit["same_program_reuse"],
		!matches!(policy, "Retained-arity" | "Refused-compile")
	);
	if policy == "Refused-runtime" {
		assert_eq!(audit["returned_root"], false);
		assert!(
			!audit["boundaries"]
				.as_array()
				.unwrap()
				.iter()
				.any(|s| s == "put"),
			"{label}: destination publication"
		);
		assert_eq!(
			audit["actual_primary_call"], true,
			"{label}: real primary Call"
		);
		assert_eq!(audit["actual_callers"], true, "{label}: real caller Calls");
	}
}
