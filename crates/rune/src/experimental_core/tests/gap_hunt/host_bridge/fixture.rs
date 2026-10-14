//! H1 stock-only host fixture, SOURCE DRAFT. No enrolment API is invoked yet.
use crate::runtime::ProtocolCaller;
use crate::{Context, Module};
use std::{
	borrow::ToOwned,
	string::String,
	sync::{Arc, Mutex},
	vec::Vec,
};
#[derive(Default)]
pub(super) struct Tape {
	pub calls: usize,
	pub events: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Recorder(Arc<Mutex<Tape>>);
impl Recorder {
	pub fn new() -> Self {
		Self(Arc::new(Mutex::new(Tape::default())))
	}
	pub fn call(&self, name: &str) {
		let mut tape = self.0.lock().unwrap();
		tape.calls += 1;
		tape.events.push(name.to_owned());
	}
	pub fn event(&self, name: &str) {
		self.0.lock().unwrap().events.push(name.to_owned());
	}
	pub fn snapshot(&self) -> (usize, Vec<String>) {
		let tape = self.0.lock().unwrap();
		(tape.calls, tape.events.clone())
	}
}
// A new Recorder per child/context; no global events leak between tests.
pub(super) fn stock_context(tape: &Recorder) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = Module::with_item(["h1"]).unwrap();
	{
		let tape = tape.clone();
		module
			.function("id_unit", move |v: ()| -> () {
				tape.call("id_unit");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("id_bool", move |v: bool| -> bool {
				tape.call("id_bool");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("id_i64", move |v: i64| -> i64 {
				tape.call("id_i64");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("id_u64", move |v: u64| -> u64 {
				tape.call("id_u64");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("id_f64", move |v: f64| -> f64 {
				tape.call("id_f64");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("id_char", move |v: char| -> char {
				tape.call("id_char");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("greet", move |v: &str| -> String {
				tape.call("greet");
				format!("hello:{v}")
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("unit", move || -> () {
				tape.call("unit");
				()
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("add", move |a: i64, b: i64| -> i64 {
				tape.call("add");
				a + b
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("mix3", move |a: bool, b: i64, c: &str| -> String {
				tape.call("mix3");
				format!("{a}:{b}:{c}")
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("mix4", move |a: (), b: u64, c: f64, d: char| -> String {
				tape.call("mix4");
				{
					let _ = a;
					format!("{b}:{c}:{d}")
				}
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("mark", move |tag: &str, v: i64| -> i64 {
				tape.call("mark");
				{
					tape.event(tag);
					v
				}
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("parse", move |v: &str| -> Result<i64, String> {
				tape.call("parse");
				v.parse::<i64>().map_err(|_| "invalid integer".to_owned())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("maybe", move |v: i64| -> Option<String> {
				tape.call("maybe");
				if v == 0 {
					None
				} else {
					Some(format!("value:{v}"))
				}
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("f64_bits", move |v: u64| -> f64 {
				tape.call("f64_bits");
				f64::from_bits(v)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("sized_string", move |v: u64| -> String {
				tape.call("sized_string");
				{
					assert!(v <= 4096);
					"α".repeat(v as usize)
				}
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_unit", move || -> Option<()> {
				tape.call("some_unit");
				Some(())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_unit", move || -> Option<()> {
				tape.call("none_unit");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_unit", move || -> Result<(), ()> {
				tape.call("ok_unit");
				Ok(())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_unit", move || -> Result<(), ()> {
				tape.call("err_unit");
				Err(())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_bool", move || -> Option<bool> {
				tape.call("some_bool");
				Some(true)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_bool", move || -> Option<bool> {
				tape.call("none_bool");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_bool", move || -> Result<bool, bool> {
				tape.call("ok_bool");
				Ok(true)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_bool", move || -> Result<bool, bool> {
				tape.call("err_bool");
				Err(true)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_i64", move || -> Option<i64> {
				tape.call("some_i64");
				Some(-7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_i64", move || -> Option<i64> {
				tape.call("none_i64");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_i64", move || -> Result<i64, i64> {
				tape.call("ok_i64");
				Ok(-7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_i64", move || -> Result<i64, i64> {
				tape.call("err_i64");
				Err(-7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_u64", move || -> Option<u64> {
				tape.call("some_u64");
				Some(7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_u64", move || -> Option<u64> {
				tape.call("none_u64");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_u64", move || -> Result<u64, u64> {
				tape.call("ok_u64");
				Ok(7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_u64", move || -> Result<u64, u64> {
				tape.call("err_u64");
				Err(7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_f64", move || -> Option<f64> {
				tape.call("some_f64");
				Some(1.25)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_f64", move || -> Option<f64> {
				tape.call("none_f64");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_f64", move || -> Result<f64, f64> {
				tape.call("ok_f64");
				Ok(1.25)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_f64", move || -> Result<f64, f64> {
				tape.call("err_f64");
				Err(1.25)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_char", move || -> Option<char> {
				tape.call("some_char");
				Some('α')
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_char", move || -> Option<char> {
				tape.call("none_char");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_char", move || -> Result<char, char> {
				tape.call("ok_char");
				Ok('α')
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_char", move || -> Result<char, char> {
				tape.call("err_char");
				Err('α')
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("some_string", move || -> Option<String> {
				tape.call("some_string");
				Some("α\0\n".to_owned())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("none_string", move || -> Option<String> {
				tape.call("none_string");
				None
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("ok_string", move || -> Result<String, String> {
				tape.call("ok_string");
				Ok("α\0\n".to_owned())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("err_string", move || -> Result<String, String> {
				tape.call("err_string");
				Err("α\0\n".to_owned())
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("mixed_error", move || -> Result<String, i64> {
				tape.call("mixed_error");
				Err(-7)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("rust_panic", move || -> i64 {
				tape.call("rust_panic");
				panic!("h1 rust panic")
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("plain_unenrolled", move |v: i64| -> i64 {
				tape.call("plain_unenrolled");
				v
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("owned_string", move |v: String| -> i64 {
				tape.call("owned_string");
				v.len() as i64
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("mut_str", move |v: &mut str| -> () {
				tape.call("mut_str");
				v.make_ascii_uppercase()
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("vec_input", move |v: &crate::runtime::Vec| -> i64 {
				tape.call("vec_input");
				v.len() as i64
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("needs_environment", move |v: i64| -> i64 {
				tape.call("needs_environment");
				crate::runtime::EnvProtocolCaller
					.call_protocol_fn(
						&crate::runtime::Protocol::CLONE,
						crate::runtime::Value::from(v),
						&mut (),
					)
					.and_then(|value| {
						<i64 as crate::runtime::FromValue>::from_value(value)
							.map_err(crate::runtime::VmError::from)
					})
					.unwrap_or(-999)
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.function("async_creation", move |v: i64| {
				tape.call("async_creation");
				async move { v }
			})
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.raw_function(
				"vm_error_control",
				move |_: &mut dyn crate::runtime::Memory, _, _, _| {
					tape.call("vm_error_control");
					Err(crate::runtime::VmError::panic("h1 vm fault"))
				},
			)
			.build()
			.unwrap();
	}
	{
		let tape = tape.clone();
		module
			.raw_function(
				"raw_unknown",
				move |memory: &mut dyn crate::runtime::Memory, _, _, out| {
					tape.call("raw_unknown");
					memory.store(out, 7i64)?;
					Ok(())
				},
			)
			.build()
			.unwrap();
	}
	context.install(module).unwrap();
	context
}
