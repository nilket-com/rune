use crate as rune;
use crate::runtime::{self as stock, Value as RuntimeValue};
use std::cell::RefCell;

#[derive(Default)]
struct State {
	events: Vec<String>,
	aliases: Vec<RuntimeValue>,
	cycles: Vec<RuntimeValue>,
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
fn values(items: impl IntoIterator<Item = RuntimeValue>) -> crate::alloc::Vec<RuntimeValue> {
	let mut out = crate::alloc::Vec::new();
	for value in items {
		out.try_push(value).unwrap();
	}
	out
}
fn vector(items: impl IntoIterator<Item = RuntimeValue>) -> RuntimeValue {
	RuntimeValue::vec(values(items)).unwrap()
}
fn tuple(items: impl IntoIterator<Item = RuntimeValue>) -> RuntimeValue {
	RuntimeValue::tuple(values(items)).unwrap()
}
fn text(value: &str) -> RuntimeValue {
	RuntimeValue::try_from(crate::alloc::String::try_from(value).unwrap()).unwrap()
}
fn object(items: impl IntoIterator<Item = (String, RuntimeValue)>) -> RuntimeValue {
	let mut out = stock::Object::new();
	for (key, value) in items {
		out.insert(crate::alloc::String::try_from(key).unwrap(), value)
			.unwrap();
	}
	RuntimeValue::new(out).unwrap()
}
fn nested(depth: usize) -> RuntimeValue {
	let mut value = RuntimeValue::unit();
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

// All factories are shared by the later stock/new fixtures. Only registration changes.
fn make(name: &str) -> RuntimeValue {
	event(&format!("make/{name}"));
	match name {
		"unit" => RuntimeValue::unit(),
		"bool" => RuntimeValue::from(true),
		"i64-min" => RuntimeValue::from(i64::MIN),
		"i64-max" => RuntimeValue::from(i64::MAX),
		"u64-max" => RuntimeValue::from(u64::MAX),
		"char" => RuntimeValue::from('🦀'),
		"less" => RuntimeValue::from(core::cmp::Ordering::Less),
		"equal" => RuntimeValue::from(core::cmp::Ordering::Equal),
		"greater" => RuntimeValue::from(core::cmp::Ordering::Greater),
		"string-empty" => text(""),
		"string" => text("α\0🦀"),
		"vec-empty" => vector([]),
		"tuple-empty" => tuple([]),
		"object-empty" => object([]),
		"none" => RuntimeValue::new(Option::<RuntimeValue>::None).unwrap(),
		"some" => RuntimeValue::new(Some(text("α"))).unwrap(),
		"ok" => RuntimeValue::new(Ok::<RuntimeValue, RuntimeValue>(vector([RuntimeValue::from(7i64)]))).unwrap(),
		"err" => RuntimeValue::new(Err::<RuntimeValue, RuntimeValue>(text("payload"))).unwrap(),
		"mixed" => object([
			("α\0".into(), vector([RuntimeValue::from(u64::MAX), text("🦀")])),
			("tuple".into(), tuple([RuntimeValue::unit(), RuntimeValue::from(false)])),
			(
				"result".into(),
				RuntimeValue::new(Ok::<RuntimeValue, RuntimeValue>(
					RuntimeValue::new(Some(RuntimeValue::from(core::cmp::Ordering::Less))).unwrap(),
				))
				.unwrap(),
			),
		]),
		"dag-vec" => {
			let v = vector([RuntimeValue::from(7i64)]);
			vector([v.clone(), v])
		}
		"dag-object" => {
			let v = object([("n".into(), RuntimeValue::from(7i64))]);
			vector([v.clone(), v])
		}
		"dag-string" => {
			let v = text("same");
			vector([v.clone(), v])
		}
		"diamond" => {
			let leaf = vector([RuntimeValue::from(7i64)]);
			vector([vector([leaf.clone()]), vector([leaf])])
		}
		"distinct-equal" => vector([vector([RuntimeValue::from(7i64)]), vector([RuntimeValue::from(7i64)])]),
		"external" => STATE.with(|s| {
			let mut s = s.borrow_mut();
			if s.aliases.is_empty() {
				s.aliases.push(vector([RuntimeValue::from(7i64)]));
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
				"cycle-option" => RuntimeValue::new(Some(v.clone())).unwrap(),
				_ => RuntimeValue::new(Ok::<RuntimeValue, RuntimeValue>(v.clone())).unwrap(),
			};
			v.borrow_mut::<stock::Vec>().unwrap().push(next).unwrap();
			STATE.with(|s| s.borrow_mut().cycles.push(v.clone()));
			v
		}
		"shared-read" | "exclusive" | "moved" | "deep-exclusive" | "deep-moved" => {
			let v = vector([RuntimeValue::from(7i64)]);
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
		"empty" => RuntimeValue::empty(),
		"type" => RuntimeValue::from(stock::Type::new(crate::Hash::EMPTY)),
		"hash" => RuntimeValue::from(crate::Hash::new(7)),
		"bytes" => RuntimeValue::new(stock::Bytes::from_slice([0, 1, 255]).unwrap()).unwrap(),
		"bytes-empty" => RuntimeValue::new(stock::Bytes::new()).unwrap(),
		"custom" => RuntimeValue::new(ProtocolMarker).unwrap(),
		"nested-custom" => vector([RuntimeValue::new(ProtocolMarker).unwrap()]),
		"function" => RuntimeValue::new(
			stock::Function::new(|| {
				event("forbidden/function");
				7i64
			})
			.unwrap(),
		)
		.unwrap(),
		"future" => RuntimeValue::new(
			stock::Future::new(async {
				event("forbidden/poll");
				Ok::<_, stock::VmError>(RuntimeValue::unit())
			})
			.unwrap(),
		)
		.unwrap(),
		"range" => RuntimeValue::new(stock::Range::new(RuntimeValue::from(1i64), RuntimeValue::from(3i64))).unwrap(),
		"nominal" => RuntimeValue::empty_struct(
			crate::sync::Arc::try_new(stock::Rtti {
				variant_index: 0,
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
			RuntimeValue::from(f64::from_bits(u64::from_str_radix(&name[4..], 16).unwrap()))
		}
		_ if name.starts_with("depth/") => nested(name[6..].parse().unwrap()),
		_ if name.starts_with("visits/") => {
			vector((0..name[7..].parse::<usize>().unwrap()).map(|_| RuntimeValue::unit()))
		}
		_ if name.starts_with("key/") => {
			object([("k".repeat(name[4..].parse().unwrap()), RuntimeValue::unit())])
		}
		_ => panic!("unknown C10 factory name"),
	}
}
fn fallible(name: &str) -> Result<RuntimeValue, String> {
	if name == "error" {
		event("fallible/error");
		Err("host error α\0".into())
	} else {
		Ok(make(name))
	}
}
fn document(value: serde_json::Value) -> RuntimeValue {
	match value {
		serde_json::Value::Null => RuntimeValue::unit(),
		serde_json::Value::Bool(v) => RuntimeValue::from(v),
		serde_json::Value::Number(v) => {
			if let Some(n) = v.as_i64() {
				RuntimeValue::from(n)
			} else if let Some(n) = v.as_u64() {
				RuntimeValue::from(n)
			} else {
				RuntimeValue::from(v.as_f64().unwrap())
			}
		}
		serde_json::Value::String(v) => text(&v),
		serde_json::Value::Array(v) => vector(v.into_iter().map(document)),
		serde_json::Value::Object(v) => object(v.into_iter().map(|(k, v)| (k, document(v)))),
	}
}
fn parse(text: &str) -> Result<RuntimeValue, String> {
	event("parse");
	serde_json::from_str(text)
		.map(document)
		.map_err(|e| format!("fixture JSON: {e}"))
}
fn mutate() -> i64 {
	event("mutate");
	STATE.with(|s| {
		s.borrow().aliases[0].borrow_mut::<stock::Vec>().unwrap()[0] = RuntimeValue::from(99i64)
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
fn legacy_context() -> Context {
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

fn context() -> Context {
	cleanup();
	STATE.with(|s| *s.borrow_mut() = State::default());
	let mut c = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c10"]).unwrap();
	m.ty::<ProtocolMarker>().unwrap();
	m.function_meta(marker_display).unwrap();
	m.function_meta(marker_debug).unwrap();
	m.function_meta(marker_clone).unwrap();
	m.host_function("make", make).unwrap();
	m.host_function("fallible", fallible).unwrap();
	m.host_function_with_output(
		"parse",
		parse,
		crate::function::host::OutputCertificate::JSON_RESULT,
	)
	.unwrap();
	m.host_function("mutate", mutate).unwrap();
	m.host_function("inspect", inspect).unwrap();
	c.install(m).unwrap();
	c
}
