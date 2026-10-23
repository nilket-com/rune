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
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	super::compile(&mut sources, &context(row), &mut Diagnostics::new(), 22)
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
