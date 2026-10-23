//! Frozen stock resize behavior and the pre-record admission boundary.
use super::*;
use serde_json::{json, Value as Json};
use std::cell::{Cell, RefCell};
use std::string::ToString;

thread_local! {
	static CLONES: Cell<usize> = const { Cell::new(0) };
	static WITNESS: RefCell<Option<crate::runtime::Value>> = const { RefCell::new(None) };
}

fn keep(value: crate::runtime::Value) {
	WITNESS.with(|w| *w.borrow_mut() = Some(value));
}

fn clone_range(_: &crate::runtime::Range) -> core::result::Result<i64, crate::runtime::VmError> {
	let count = CLONES.with(|c| {
		c.set(c.get() + 1);
		c.get()
	});
	if count == 3 {
		return Err(crate::runtime::VmError::panic("third clone"));
	}
	Ok(count as i64)
}

fn context(row: &Json) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	if row["context"] == "custom-range-clone" {
		let mut module = crate::Module::with_item(["c17f"]).unwrap();
		module.function("keep", keep).build().unwrap();
		module
			.associated_function(&crate::runtime::Protocol::CLONE, clone_range)
			.unwrap();
		context.install(module).unwrap();
	}
	context
}

fn stock_observation(row: &Json) -> Json {
	let context = context(row);
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	CLONES.with(|c| c.set(0));
	WITNESS.with(|w| *w.borrow_mut() = None);
	let result = match vm.call(["main"], ()) {
		Ok(value) => json!({"value":crate::from_value::<i64>(value).unwrap()}),
		Err(error) => {
			let site = error.first_location().map(|at| {
				let span = at
					.unit
					.debug_info()
					.unwrap()
					.instruction_at(at.ip)
					.unwrap()
					.span
					.range();
				json!([span.start, span.end])
			});
			json!({"error":error.to_string(),"site":site})
		}
	};
	let witness = WITNESS.with(|w| {
		let value = w.borrow_mut().take()?;
		let values = value.borrow_ref::<crate::runtime::Vec>().unwrap();
		Some(
			values
				.iter()
				.map(|v| crate::from_value::<i64>(v.clone()).unwrap())
				.collect::<std::vec::Vec<_>>(),
		)
	});
	json!({"result":result,"clones":CLONES.with(Cell::get),"witness":witness,"diagnostics":std::format!("{:?}",diagnostics.diagnostics())})
}

fn compile_source(row: &Json) -> Result<Program> {
	compile_in(row, &context(row))
}

fn compile_in(row: &Json, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 22)
}

fn normalized_op(program: &Program, op: crate::experimental_core::code::Op) -> std::string::String {
	let raw = std::format!("{op:?}").replace(
		&std::format!("code: {}", program.code.id()),
		"code: PROGRAM",
	);
	let mut out = std::string::String::new();
	let mut rest = raw.as_str();
	while let Some(at) = rest.find("Nominal(") {
		out.push_str(&rest[..at]);
		let tail = &rest[at + 8..];
		let end = tail.find(')').unwrap();
		let id = tail[..end].parse::<u32>().unwrap();
		let hash = crate::experimental_core::nominal::hash_copy(
			crate::experimental_core::value::Nominal(id),
		)
		.unwrap();
		let name = ["S", "T"]
			.into_iter()
			.find(|name| Hash::type_hash([*name]).into_inner() == hash)
			.unwrap();
		out.push_str(&std::format!("Nominal({name})"));
		rest = &tail[end + 1..];
	}
	out.push_str(rest);
	out
}

fn baseline(row: &Json) -> Json {
	match compile_source(row) {
		Err(error) => {
			json!({"admitted":false,"refusal":std::format!("{:?}",error.refusal),"located":error.location.is_some()})
		}
		Ok(program) => {
			let ops = (0..program.code.functions())
				.map(|id| {
					let f = program.code.function(id as u32).unwrap();
					(
						std::format!("{:?}", program.functions[id]),
						f.parameters,
						f.slots,
						(0..f.code.len())
							.map(|pc| normalized_op(&program, f.cached(pc).unwrap().op))
							.collect::<std::vec::Vec<_>>(),
					)
				})
				.collect::<std::vec::Vec<_>>();
			json!({"admitted":true,"ops":ops,"ineligible":program.entry_ineligible})
		}
	}
}

#[test]
#[ignore = "operator-only stock freeze; requires a new C17F_ORACLE_OUT"]
fn vec_resize_freeze_stock_oracle() {
	use std::io::Write;
	let path = std::env::var_os("C17F_ORACLE_OUT").unwrap();
	let mut contract: Json =
		serde_json::from_str(include_str!("vec_resize_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock_observation(row);
		row["before"] = baseline(row);
		std::println!("C17F_ORACLE_ROW {}", row["name"]);
	}
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(path)
		.unwrap();
	file.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn vec_resize_stock_oracle_is_frozen() {
	let contract: Json = serde_json::from_str(include_str!("vec_resize_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(stock_observation(row), row["stock"], "{}", row["name"]);
	}
}

#[test]
fn vec_resize_sources_and_prior_lowering_are_pinned() {
	use sha2::{Digest, Sha256};
	let contract: Json = serde_json::from_str(include_str!("vec_resize_contract.json")).unwrap();
	let mut names = std::collections::HashSet::new();
	let mut prior = 0;
	for row in contract["rows"].as_array().unwrap() {
		assert!(names.insert(row["name"].as_str().unwrap()));
		let digest = Sha256::digest(row["source"].as_str().unwrap().as_bytes());
		let hash = digest
			.iter()
			.map(|b| std::format!("{b:02x}"))
			.collect::<std::string::String>();
		assert_eq!(hash, row["source_sha256"].as_str().unwrap());
		if row["before"]["admitted"] == true {
			assert_eq!(baseline(row), row["before"], "{}", row["name"]);
			prior += 1;
		}
	}
	assert_eq!(contract["rows"].as_array().unwrap().len(), 40);
	assert_eq!(prior, 3);
}

#[test]
fn vec_resize_fresh_matches_the_frozen_contract() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let contract: Json = serde_json::from_str(include_str!("vec_resize_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		let name = row["name"].as_str().unwrap();
		let context = context(row);
		let compiled = compile_in(row, &context);
		if row["classification"]
			.as_str()
			.unwrap()
			.starts_with("Refused")
			|| name == "script-parameter"
		{
			let error = compiled.err().expect(name);
			assert!(
				error.refusal.is_some() && error.location.is_some(),
				"{name}"
			);
			continue;
		}
		let program = compiled.unwrap_or_else(|e| panic!("{name}: {e:?}"));
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let result = executor.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			100000,
		);
		if row["classification"] == "Known-gap" {
			let failure = result.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::StoreCapacity, "{name}");
			assert!(failure.error.refusal.is_none());
		} else if let Some(expected) = row["stock"]["result"]["value"].as_i64() {
			let root = result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
			assert!(
				matches!(store.import(&root).unwrap(),Value::I64(value)if value==expected),
				"{name}"
			);
			store.root_release(root).unwrap();
		} else {
			let failure = result.unwrap_err();
			assert!(failure.error.refusal.is_none(), "{name}");
			assert_eq!(
				failure.message(&program).to_string(),
				row["stock"]["result"]["error"].as_str().unwrap(),
				"{name}"
			);
			let at = failure.error.location.unwrap();
			assert_eq!(
				json!([at.start, at.end]),
				row["stock"]["result"]["site"],
				"{name}"
			);
		}
		assert!(executor.is_idle(), "{name}");
		if name == "self-retained-child" {
			store.close().unwrap();
		}
		assert_eq!(
			(store.live_entries(), store.payload_bytes()),
			(0, 0),
			"{name}"
		);
	}
}

#[test]
fn vec_resize_unknown_or_mutated_wrapper_fills_refuse_before_execution() {
	for source in [
		"fn f(v,x){v.resize(3,x)}pub fn main(){f([],None)}",
		"pub fn main(){let fill=Some([1]);fill.0=0..2;let v=[];v.resize(2,fill)}",
		"fn mutate(x){x.0=0..2}pub fn main(){let fill=Some([1]);mutate(fill);let v=[];v.resize(2,fill)}",
		"pub fn main(){let fill=Some([1]);let alias=fill;alias.0=0..2;let v=[];v.resize(2,fill)}",
		"pub fn main(){let fill=Some([1]);let f=||{fill.0=0..2};f();let v=[];v.resize(2,fill)}",
	] {
		let row=json!({"source":source,"context":"default"});let error=compile_source(&row).err().expect(source);assert!(error.refusal.is_some()&&error.location.is_some(),"{source}");
	}
}

#[test]
fn vec_resize_native_adapter_cannot_be_guessed_as_a_host_entry() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let row =
		json!({"source":"fn f(v){v.resize(3,None)}pub fn main(){f([]);7}","context":"default"});
	let context = context(&row);
	let program = compile_in(&row, &context).unwrap();
	let id=(0..program.code.functions()).find(|&id| {
		let f=program.code.function(id as u32).unwrap();(0..f.code.len()).any(|pc|matches!(f.cached(pc).unwrap().op,crate::experimental_core::code::Op::Call{function,..}if function==crate::experimental_core::code::VEC_RESIZE))&&program.functions[id]==Hash::EMPTY
	}).unwrap()as u32;
	assert!(!program.entry_eligible(id));
	let mut store = Store::new(Limits::default()).unwrap();
	let receiver = store
		.root_list(crate::experimental_core::value::Type::Vec, &[])
		.unwrap();
	let n = store.root_i64(3);
	let fill = store.root_i64(9);
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	executor.request_cancel();
	for fuel in [0, 1000] {
		let failure = executor
			.invoke(&program, &mut store, id, &[&receiver, &n, &fill], fuel)
			.unwrap_err();
		assert_eq!(failure.error.refusal, Some(Refusal::Entry));
		assert!(failure.call_chain.is_empty() && executor.is_idle());
		assert_eq!(store.root_len(&receiver).unwrap(), 0);
	}
	let failure = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			1000,
		)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert!(matches!(store.import(&fill).unwrap(), Value::I64(9)));
	store.root_release(receiver).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn vec_resize_moved_heap_fills_refuse_before_execution() {
	for fill in ["[1]", "\"x\"", "#{x:1}", "Some([1])", "([1],)"] {
		let source=std::format!("pub fn main(){{let fill={fill};let alias=fill;std::mem::drop(alias);let v=[];v.resize(2,fill);v.len() as i64}}");
		let row = json!({"source":source,"context":"default"});
		let stock = stock_observation(&row);
		assert!(
			stock["result"]["error"].as_str().unwrap().contains("moved"),
			"{fill}: {stock}"
		);
		let error = compile_source(&row).err().expect(fill);
		assert!(
			error.refusal.is_some() && error.location.is_some(),
			"{fill}"
		);
	}
}
