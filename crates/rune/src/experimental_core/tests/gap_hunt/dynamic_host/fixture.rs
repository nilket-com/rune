use crate as rune;
use crate::runtime::{self as stock, Value};
use std::cell::RefCell;

#[derive(Default)]
struct State {
	events: Vec<String>,
	aliases: Vec<Value>,
	cycles: Vec<Value>,
	read: Option<stock::Ref<stock::Vec>>,
	write: Option<stock::Mut<stock::Vec>>,
}
thread_local! {
	static STATE: RefCell<State> = RefCell::new(State::default());
}
fn event(name: &str) {
	STATE.with(|s| s.borrow_mut().events.push(name.into()));
}
fn events() -> Vec<String> {
	STATE.with(|s| s.borrow().events.clone())
}
fn cleanup() {
	STATE.with(|s| {
		let mut s = s.borrow_mut();
		s.read = None;
		s.write = None;
		for value in s.cycles.drain(..) {
			if let Ok(mut values) = value.borrow_mut::<stock::Vec>() {
				values.clear();
			} else if let Ok(mut object) = value.borrow_mut::<stock::Object>() {
				object.clear();
			}
		}
		s.aliases.clear();
	});
}
fn values(items: impl IntoIterator<Item = Value>) -> crate::alloc::Vec<Value> {
	let mut out = crate::alloc::Vec::new();
	for value in items {
		out.try_push(value).unwrap();
	}
	out
}
fn vector(items: impl IntoIterator<Item = Value>) -> Value {
	Value::vec(values(items)).unwrap()
}
fn tuple(items: impl IntoIterator<Item = Value>) -> Value {
	Value::tuple(values(items)).unwrap()
}
fn text(value: &str) -> Value {
	Value::try_from(crate::alloc::String::try_from(value).unwrap()).unwrap()
}
fn object(items: impl IntoIterator<Item = (String, Value)>) -> Value {
	let mut out = stock::Object::new();
	for (key, value) in items {
		out.insert(crate::alloc::String::try_from(key).unwrap(), value)
			.unwrap();
	}
	Value::new(out).unwrap()
}
fn nested(depth: usize) -> Value {
	let mut value = Value::unit();
	for _ in 0..depth {
		value = vector([value]);
	}
	value
}

#[derive(crate::Any)]
#[rune(item = ::c10)]
struct ProtocolMarker;
#[rune::function(instance, protocol = DISPLAY_FMT)]
fn marker_display(_: &ProtocolMarker, _: &mut stock::Formatter) -> core::fmt::Result {
	event("protocol/display");
	Ok(())
}
#[rune::function(instance, protocol = DEBUG_FMT)]
fn marker_debug(_: &ProtocolMarker, _: &mut stock::Formatter) -> core::fmt::Result {
	event("protocol/debug");
	Ok(())
}
#[rune::function(instance, protocol = CLONE)]
fn marker_clone(_: &ProtocolMarker) -> ProtocolMarker {
	event("protocol/clone");
	ProtocolMarker
}

// All factories are shared by the later stock/fresh fixtures. Only registration changes.
fn make(name: &str) -> Value {
	event(&format!("make/{name}"));
	match name {
		"unit" => Value::unit(),
		"bool" => Value::from(true),
		"i64-min" => Value::from(i64::MIN),
		"i64-max" => Value::from(i64::MAX),
		"u64-max" => Value::from(u64::MAX),
		"char" => Value::from('🦀'),
		"less" => Value::from(core::cmp::Ordering::Less),
		"equal" => Value::from(core::cmp::Ordering::Equal),
		"greater" => Value::from(core::cmp::Ordering::Greater),
		"string-empty" => text(""),
		"string" => text("α\0🦀"),
		"vec-empty" => vector([]),
		"tuple-empty" => tuple([]),
		"object-empty" => object([]),
		"none" => Value::new(Option::<Value>::None).unwrap(),
		"some" => Value::new(Some(text("α"))).unwrap(),
		"ok" => Value::new(Ok::<Value, Value>(vector([Value::from(7i64)]))).unwrap(),
		"err" => Value::new(Err::<Value, Value>(text("payload"))).unwrap(),
		"mixed" => object([
			("α\0".into(), vector([Value::from(u64::MAX), text("🦀")])),
			("tuple".into(), tuple([Value::unit(), Value::from(false)])),
			(
				"result".into(),
				Value::new(Ok::<Value, Value>(
					Value::new(Some(Value::from(core::cmp::Ordering::Less))).unwrap(),
				))
				.unwrap(),
			),
		]),
		"dag-vec" => {
			let v = vector([Value::from(7i64)]);
			vector([v.clone(), v])
		}
		"dag-object" => {
			let v = object([("n".into(), Value::from(7i64))]);
			vector([v.clone(), v])
		}
		"dag-string" => {
			let v = text("same");
			vector([v.clone(), v])
		}
		"diamond" => {
			let leaf = vector([Value::from(7i64)]);
			vector([vector([leaf.clone()]), vector([leaf])])
		}
		"distinct-equal" => vector([vector([Value::from(7i64)]), vector([Value::from(7i64)])]),
		"external" => STATE.with(|s| {
			let mut s = s.borrow_mut();
			if s.aliases.is_empty() {
				s.aliases.push(vector([Value::from(7i64)]));
			}
			s.aliases[0].clone()
		}),
		"cycle-vec" => {
			let v = vector([]);
			v.borrow_mut::<stock::Vec>()
				.unwrap()
				.push(v.clone())
				.unwrap();
			STATE.with(|s| s.borrow_mut().cycles.push(v.clone()));
			v
		}
		"cycle-object" => {
			let v = object([]);
			v.borrow_mut::<stock::Object>()
				.unwrap()
				.insert(crate::alloc::String::try_from("self").unwrap(), v.clone())
				.unwrap();
			STATE.with(|s| s.borrow_mut().cycles.push(v.clone()));
			v
		}
		"cycle-two" | "cycle-tuple" | "cycle-option" | "cycle-result" => {
			let v = vector([]);
			let next = match name {
				"cycle-two" => object([("back".into(), v.clone())]),
				"cycle-tuple" => tuple([v.clone()]),
				"cycle-option" => Value::new(Some(v.clone())).unwrap(),
				_ => Value::new(Ok::<Value, Value>(v.clone())).unwrap(),
			};
			v.borrow_mut::<stock::Vec>().unwrap().push(next).unwrap();
			STATE.with(|s| s.borrow_mut().cycles.push(v.clone()));
			v
		}
		"shared-read" | "exclusive" | "moved" | "deep-exclusive" | "deep-moved" => {
			let v = vector([Value::from(7i64)]);
			match name {
				"shared-read" => STATE.with(|s| {
					s.borrow_mut().read = Some(v.clone().into_ref::<stock::Vec>().unwrap())
				}),
				"exclusive" | "deep-exclusive" => STATE.with(|s| {
					s.borrow_mut().write = Some(v.clone().into_mut::<stock::Vec>().unwrap())
				}),
				_ => {
					drop(v.clone().downcast::<stock::Vec>().unwrap());
				}
			}
			if name.starts_with("deep-") {
				tuple([v])
			} else {
				v
			}
		}
		"empty" => Value::empty(),
		"type" => Value::from(stock::Type::new(crate::Hash::EMPTY)),
		"hash" => Value::from(crate::Hash::new(7)),
		"bytes" => Value::new(stock::Bytes::from_slice([0, 1, 255]).unwrap()).unwrap(),
		"bytes-empty" => Value::new(stock::Bytes::new()).unwrap(),
		"custom" => Value::new(ProtocolMarker).unwrap(),
		"nested-custom" => vector([Value::new(ProtocolMarker).unwrap()]),
		"function" => Value::new(
			stock::Function::new(|| {
				event("forbidden/function");
				7i64
			})
			.unwrap(),
		)
		.unwrap(),
		"future" => Value::new(
			stock::Future::new(async {
				event("forbidden/poll");
				Ok::<_, stock::VmError>(Value::unit())
			})
			.unwrap(),
		)
		.unwrap(),
		"range" => Value::new(stock::Range::new(Value::from(1i64), Value::from(3i64))).unwrap(),
		"nominal" => Value::empty_struct(
			crate::sync::Arc::try_new(stock::Rtti {
				kind: stock::RttiKind::Empty,
				hash: crate::Hash::type_hash(["c10", "Nominal"]),
				variant_hash: crate::Hash::EMPTY,
				item: crate::ItemBuf::with_item(["c10", "Nominal"]).unwrap(),
				fields: stock::new_field_map(),
			})
			.unwrap(),
		)
		.unwrap(),
		_ if name.starts_with("f64/") => {
			Value::from(f64::from_bits(u64::from_str_radix(&name[4..], 16).unwrap()))
		}
		_ if name.starts_with("depth/") => nested(name[6..].parse().unwrap()),
		_ if name.starts_with("visits/") => {
			vector((0..name[7..].parse::<usize>().unwrap()).map(|_| Value::unit()))
		}
		_ if name.starts_with("key/") => {
			object([("k".repeat(name[4..].parse().unwrap()), Value::unit())])
		}
		_ => panic!("unknown C10 factory name"),
	}
}
fn fallible(name: &str) -> Result<Value, String> {
	if name == "error" {
		event("fallible/error");
		Err("host error α\0".into())
	} else {
		Ok(make(name))
	}
}
fn document(value: serde_json::Value) -> Value {
	match value {
		serde_json::Value::Null => Value::unit(),
		serde_json::Value::Bool(v) => Value::from(v),
		serde_json::Value::Number(v) => {
			if let Some(n) = v.as_i64() {
				Value::from(n)
			} else if let Some(n) = v.as_u64() {
				Value::from(n)
			} else {
				Value::from(v.as_f64().unwrap())
			}
		}
		serde_json::Value::String(v) => text(&v),
		serde_json::Value::Array(v) => vector(v.into_iter().map(document)),
		serde_json::Value::Object(v) => object(v.into_iter().map(|(k, v)| (k, document(v)))),
	}
}
fn parse(text: &str) -> Result<Value, String> {
	event("parse");
	serde_json::from_str(text)
		.map(document)
		.map_err(|e| format!("fixture JSON: {e}"))
}
fn mutate() -> i64 {
	event("mutate");
	STATE.with(|s| {
		s.borrow().aliases[0].borrow_mut::<stock::Vec>().unwrap()[0] = Value::from(99i64)
	});
	99
}
fn inspect() -> i64 {
	event("inspect");
	STATE.with(|s| {
		s.borrow().aliases[0].borrow_ref::<stock::Vec>().unwrap()[0]
			.as_integer::<i64>()
			.unwrap()
	})
}
fn context() -> Context {
	cleanup();
	STATE.with(|s| *s.borrow_mut() = State::default());
	let mut c = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c10"]).unwrap();
	m.ty::<ProtocolMarker>().unwrap();
	m.function_meta(marker_display).unwrap();
	m.function_meta(marker_debug).unwrap();
	m.function_meta(marker_clone).unwrap();
	m.function("make", make).build().unwrap();
	m.function("fallible", fallible).build().unwrap();
	m.function("parse", parse).build().unwrap();
	m.function("mutate", mutate).build().unwrap();
	m.function("inspect", inspect).build().unwrap();
	c.install(m).unwrap();
	c
}

fn fresh_context() -> Context {
	cleanup();
	STATE.with(|s| *s.borrow_mut() = State::default());
	let mut c = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c10"]).unwrap();
	m.ty::<ProtocolMarker>().unwrap();
	m.function_meta(marker_display).unwrap();
	m.function_meta(marker_debug).unwrap();
	m.function_meta(marker_clone).unwrap();
	m.fresh_host_function("make", make).unwrap();
	m.fresh_host_function("fallible", fallible).unwrap();
	m.fresh_host_function("parse", parse).unwrap();
	m.fresh_host_function("mutate", mutate).unwrap();
	m.fresh_host_function("inspect", inspect).unwrap();
	c.install(m).unwrap();
	c
}
