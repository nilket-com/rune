//! C10 stock dynamic-return oracle, source-only until peer approval.
use super::*;
use serde_json::{json, Value as Json};
include!("dynamic_host/cases.rs");
include!("dynamic_host/fixture.rs");

const OBSERVER_NODES: usize = 70_000;
struct RuntimeGraph {
	memo: std::collections::HashMap<(u8, usize), usize>,
	pending: Vec<crate::runtime::Value>,
}
fn identity(value: &crate::runtime::Value) -> Option<(u8, usize)> {
	macro_rules! key {
		($tag:expr,$ty:ty) => {
			if let Ok(v) = value.borrow_ref::<$ty>() {
				return Some(($tag, (&*v as *const $ty) as usize));
			}
		};
	}
	key!(0, crate::alloc::String);
	key!(1, crate::runtime::Vec);
	key!(2, crate::runtime::OwnedTuple);
	key!(3, crate::runtime::Object);
	key!(4, Option<crate::runtime::Value>);
	key!(5,Result<crate::runtime::Value,crate::runtime::Value>);
	None
}
impl RuntimeGraph {
	fn intern(&mut self, value: crate::runtime::Value) -> usize {
		let key = identity(&value);
		if let Some(id) = key.and_then(|k| self.memo.get(&k).copied()) {
			return id;
		}
		assert!(
			self.pending.len() < OBSERVER_NODES,
			"C10 observation graph bound"
		);
		let id = self.pending.len();
		if let Some(key) = key {
			self.memo.insert(key, id);
		}
		self.pending.push(value);
		id
	}
	fn node(&mut self, value: &crate::runtime::Value) -> Json {
		use crate::runtime::{Inline, Repr};
		let hash = value.type_hash().to_string();
		if let Some(inline) = value.as_inline() {
			return match inline {
				Inline::Empty => json!(["Empty", hash]),
				Inline::Unit => json!(["Unit", hash]),
				Inline::Bool(v) => json!(["Bool", hash, v]),
				Inline::Char(v) => json!(["Char", hash, *v as u32]),
				Inline::Signed(v) => json!(["I64", hash, v.to_string()]),
				Inline::Unsigned(v) => json!(["U64", hash, v.to_string()]),
				Inline::Float(v) => json!(["F64", hash, format!("{:016x}", v.to_bits())]),
				Inline::Type(v) => json!(["Type", hash, v.into_hash().to_string()]),
				Inline::Hash(v) => json!(["Hash", hash, v.to_string()]),
				Inline::Ordering(v) => json!([
					"Ordering",
					hash,
					match v {
						core::cmp::Ordering::Less => "Less",
						core::cmp::Ordering::Equal => "Equal",
						core::cmp::Ordering::Greater => "Greater",
					}
				]),
			};
		}
		if let Ok(v) = value.borrow_ref::<crate::alloc::String>() {
			return json!(["String", hash, hex(v.as_bytes())]);
		}
		if let Ok(v) = value.borrow_ref::<crate::runtime::Vec>() {
			return json!([
				"Vec",
				hash,
				v.iter().map(|v| self.intern(v.clone())).collect::<Vec<_>>()
			]);
		}
		if let Ok(v) = value.borrow_ref::<crate::runtime::OwnedTuple>() {
			return json!([
				"Tuple",
				hash,
				v.iter().map(|v| self.intern(v.clone())).collect::<Vec<_>>()
			]);
		}
		if let Ok(v) = value.borrow_ref::<crate::runtime::Object>() {
			return json!([
				"Object",
				hash,
				v.iter()
					.map(|(k, v)| (hex(k.as_bytes()), self.intern(v.clone())))
					.collect::<Vec<_>>()
			]);
		}
		if let Ok(v) = value.borrow_ref::<Option<crate::runtime::Value>>() {
			return json!([
				"Option",
				hash,
				(*v).as_ref().map(|v| self.intern(v.clone()))
			]);
		}
		if let Ok(v) = value.borrow_ref::<Result<crate::runtime::Value, crate::runtime::Value>>() {
			return match &*v {
				Ok(v) => json!(["Result", hash, "Ok", self.intern(v.clone())]),
				Err(v) => json!(["Result", hash, "Err", self.intern(v.clone())]),
			};
		}
		match value.as_ref() {
			Repr::Dynamic(v) => json!([
				"Dynamic",
				hash,
				format!("{:?}", v.rtti().kind),
				value.is_readable(),
				value.is_writable()
			]),
			Repr::Any(_) => json!(["OpaqueAny", hash, value.is_readable(), value.is_writable()]),
			Repr::Inline(_) => unreachable!(),
		}
	}
}
fn runtime_graph(value: &crate::runtime::Value) -> Json {
	let mut g = RuntimeGraph {
		memo: std::collections::HashMap::new(),
		pending: Vec::new(),
	};
	let root = g.intern(value.clone());
	let mut nodes = Vec::new();
	let mut at = 0;
	while at < g.pending.len() {
		let value = g.pending[at].clone();
		nodes.push(g.node(&value));
		at += 1;
	}
	json!({"root":root,"nodes":nodes})
}
fn error_observation(error: &crate::runtime::VmError) -> Json {
	let kind = error.error().kind();
	let chain = error
		.chain()
		.iter()
		.map(|at| json!({"kind":format!("{:?}",at.kind()),"display":at.to_string()}))
		.collect::<Vec<_>>();
	let sites=error.stacktrace().iter().map(|at|{
		let inst=at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();let span=inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":span.start,"end":span.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	json!({"status":"error","class":runtime_error_class(kind),"kind":format!("{kind:?}"),"display":kind.to_string(),"chain":chain,"sites":sites,"raw":format!("{error:?}")})
}
fn legacy_observe(source: &str) -> Json {
	let context = legacy_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c10.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			cleanup();
			return json!({"outcome":{"status":"compile-error","error":format!("{e:?}")},"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"events":events(),"invoked":false});
		}
	};
	// Actual compiler call sites, never guessed source ranges.
	let host_sites=vm.unit().iter_instructions().filter_map(|(ip,inst)|{
		let crate::runtime::inst::Kind::Call{hash,..}=&inst.kind else{return None;};
		let at=vm.unit().debug_info().unwrap().instruction_at(ip).unwrap();let span=at.span.range();
		Some(json!({"hash":hash.to_string(),"ip":ip,"instruction":format!("{inst:?}"),"start":span.start,"end":span.end,"source":at.source_id.into_index(),"span_hex":hex(source[span].as_bytes())}))
	}).collect::<Vec<_>>();
	let result = crate::runtime::budget::with(FUEL, || vm.call(["main"], ())).call();
	let outcome = match result {
		Ok(value) => json!({"status":"value","graph":runtime_graph(&value)}),
		Err(e) => error_observation(&e),
	};
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
	cleanup(); // explicitly breaks stock cycles and releases held guards before reuse/drop.
	let reuse = match crate::runtime::budget::with(FUEL, || vm.call(["c10_reuse"], ())).call() {
		Ok(value) => json!({"status":"value","graph":runtime_graph(&value)}),
		Err(e) => error_observation(&e),
	};
	json!({"outcome":outcome,"host_calls":host_calls,"protocol_calls":protocol_calls,"events":before_cleanup,"events_after_reuse":events(),"reuse":reuse,"host_sites":host_sites,"invoked":true,"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"cleanup":"fixture cycles cleared and borrow guards dropped"})
}
#[test]
#[ignore = "C10 source-only stock oracle; bounded external controller only"]
fn child() {
	assert_eq!(CASES.len(), COUNT);
	let index: usize = std::env::var("C10_CASE").unwrap().parse().unwrap();
	let (label, source, policy) = CASES[index];
	let engine = std::env::var("C10_ENGINE").unwrap();
	let (observation, audit) = match engine.as_str() {
		"stock" => (legacy_observe(source), Json::Null),
		#[cfg(target_os = "linux")]
		"fresh" => observe_dynamic(source),
		_ => panic!("unknown C10 engine"),
	};
	let envelope = json!({"schema":1,"engine":engine,"audit":audit,"index":index,"label":label,"source_hex":hex(source.as_bytes()),"policy":policy,"fuel":FUEL,"observation":observation});
	fs::write(
		std::env::var_os("C10_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("dynamic_host/controls.rs");

#[cfg(target_os = "linux")]
include!("dynamic_host/observe.rs");

#[cfg(target_os = "linux")]
include!("dynamic_host/projection.rs");

#[cfg(target_os = "linux")]
include!("dynamic_host/digest.rs");

#[cfg(target_os = "linux")]
include!("dynamic_host/contract.rs");

#[cfg(target_os = "linux")]
include!("dynamic_host/contract_controls.rs");
