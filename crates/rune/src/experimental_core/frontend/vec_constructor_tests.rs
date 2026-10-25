//! Stock constructor outputs, errors and exact provider certification.
use super::*;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
	value::Value,
};
use crate::Module;
use std::string::ToString;

fn compile_source(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 20)
}

#[test]
fn vec_constructors_match_stock_and_release_every_reservation() {
	let source = r#"fn make(n){Vec::with_capacity(n)} pub fn main(){
		let empty=Vec::new(); let total=empty.len() as i64;
		for n in [0,1,5,5000]{let v=make(n); v.push(n); total+=v[0];}
		total as i64
	}"#;
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	let mut stock = crate::prepare(&mut sources)
		.with_context(&context)
		.build_vm()
		.unwrap();
	let expected: i64 = crate::from_value(stock.call(["main"], ()).unwrap()).unwrap();
	assert_eq!(expected, 5006);
	let program = compile_source(source, &context).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	for _ in 0..2 {
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				10000,
			)
			.unwrap();
		assert!(matches!(store.import(&root).unwrap(), Value::I64(got) if got == expected));
		store.root_release(root).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		assert!(executor.is_idle());
	}
}

#[test]
fn vec_constructor_errors_match_stock_messages() {
	for (arg, kind) in [
		("-1", ErrorKind::IntegerCoercion),
		("true", ErrorKind::Type),
		("1.0", ErrorKind::Type),
		("9223372036854775807", ErrorKind::Allocation),
	] {
		let source = std::format!("pub fn main(){{Vec::with_capacity({arg})}}");
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(&source).unwrap())
			.unwrap();
		let mut stock = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let expected = stock.call(["main"], ()).unwrap_err().to_string();
		let program = compile_source(&source, &context).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap_err();
		assert_eq!(failure.error.kind, kind, "{arg}");
		assert!(failure.error.refusal.is_none());
		assert_eq!(failure.message(&program).to_string(), expected, "{arg}");
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		assert!(executor.is_idle());
	}
}

#[test]
fn vec_constructor_custom_providers_are_not_certified() {
	let mut module = Module::with_crate_item("std", ["vec", "Vec"]).unwrap();
	module.function("new", || 42i64).build().unwrap();
	let mut context = Context::new();
	context.install(module).unwrap();
	assert!(!context.vec_constructor(false));
	assert!(!context.vec_constructor(true));
	let error = compile_source("pub fn main(){std::vec::Vec::new()}", &context)
		.err()
		.unwrap();
	assert_eq!(error.refusal, Some(Refusal::Native));
	assert!(error.location.is_some());
}

#[test]
fn vec_capacity_and_bad_constructor_arity_stay_compile_time_refused() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"pub fn main(){Vec::new().capacity()}",
		"pub fn main(){Vec::new(1)}",
		"pub fn main(){Vec::with_capacity()}",
	] {
		let error = compile_source(source, &context).err().unwrap();
		assert!(error.refusal.is_some());
		assert!(error.location.is_some());
	}
}

#[test]
fn vec_constructor_capacity_contract_matches_stock_or_declares_allocator_divergence() {
	let contract: serde_json::Value =
		serde_json::from_str(include_str!("vec_constructor_contract.json")).unwrap();
	let limit = isize::MAX as usize / core::mem::size_of::<crate::runtime::Value>();
	for row in contract["rows"].as_array().unwrap() {
		let arg = match row["capacity"].as_str().unwrap() {
			"stock-layout-limit" => limit.to_string(),
			"stock-layout-limit-plus-one" => (limit + 1).to_string(),
			other => other.to_string(),
		};
		let source = row["source"].as_str().unwrap().replace("$capacity", &arg);
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(&source).unwrap())
			.unwrap();
		let mut stock = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let legacy_result = stock.call(["main"], ());
		if row["classification"] == "Intentional" {
			assert!(matches!(
				legacy_result.as_ref().unwrap_err().error().kind(),
				crate::runtime::VmErrorKind::AllocError { .. }
			));
			assert!(!row["rationale"].as_str().unwrap().is_empty());
		}
		let expected = legacy_result
			.map(|v| crate::from_value::<i64>(v).unwrap())
			.map_err(|e| e.to_string());
		let program = compile_source(&source, &context).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let actual = match executor.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			1000,
		) {
			Ok(root) => {
				let Value::I64(value) = store.import(&root).unwrap() else {
					panic!()
				};
				store.root_release(root).unwrap();
				Ok(value)
			}
			Err(failure) => {
				assert!(failure.error.refusal.is_none());
				Err(failure.message(&program).to_string())
			}
		};
		match row["classification"].as_str().unwrap() {
			"Intentional" => assert_eq!(actual, Ok(row["fresh"].as_i64().unwrap())),
			"Exact" => {
				assert_eq!(expected, Ok(row["fresh"].as_i64().unwrap()));
				assert_eq!(actual, expected);
			}
			"Exact-error" => {
				assert!(expected.is_err());
				assert_eq!(actual, expected, "{}", row["id"]);
			}
			other => panic!("unknown constructor contract classification: {other}"),
		}
		std::eprintln!(
			"C18B3C_VEC_ORACLE {} {} stock={expected:?} execution={actual:?}",
			row["id"],
			row["classification"]
		);
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		assert!(executor.is_idle());
	}
}
