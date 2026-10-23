//! Frozen stock observations before local and parameter receiver proofs.
use super::*;
use crate as rune;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::string::ToString;

thread_local! {
	static EFFECTS: Cell<usize> = const { Cell::new(0) };
}

fn digest(bytes: &[u8]) -> std::string::String {
	Sha256::digest(bytes)
		.iter()
		.map(|byte| std::format!("{byte:02x}"))
		.collect()
}

fn context(row: &Json) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.fresh_host_function("mark", || {
			EFFECTS.with(|count| count.set(count.get() + 1));
			0i64
		})
		.unwrap();
	if row["macros"] == true {
		use crate::macros::quote;
		module
			.macro_(["mk"], |cx, _| {
				let id = cx.insert_source("c18e-call", "R::new()")?;
				let expr = cx.parse_source::<crate::ast::Expr>(id)?;
				Ok(quote!(#expr).into_token_stream(cx)?)
			})
			.unwrap();
		module
			.macro_(["ret"], |cx, _| {
				let id = cx.insert_source("c18e-return", "return R")?;
				let expr = cx.parse_source::<crate::ast::Expr>(id)?;
				Ok(quote!(#expr).into_token_stream(cx)?)
			})
			.unwrap();
	}
	context.install(module).unwrap();
	if row["test_builtin"] == true {
		// Test-only handlers occupy existing certified hash slots. The product
		// hash allowlist and ordinary public macro registration remain intact.
		let mut call = crate::Module::with_crate_item("std", ["fmt"]).unwrap();
		call.macro_("format", |cx, _| {
			let id = cx.insert_source("c18e-call", "R::new()")?;
			let expr = cx.parse_source::<crate::ast::Expr>(id)?;
			Ok(crate::macros::quote!(#expr).into_token_stream(cx)?)
		})
		.unwrap();
		call.fresh_mark_builtin_macro();
		context.install(call).unwrap();
		let mut returns = crate::Module::with_crate_item("std", ["test"]).unwrap();
		returns
			.macro_("assert", |cx, _| {
				let id = cx.insert_source("c18e-return", "return R")?;
				let expr = cx.parse_source::<crate::ast::Expr>(id)?;
				Ok(crate::macros::quote!(#expr).into_token_stream(cx)?)
			})
			.unwrap();
		returns.fresh_mark_builtin_macro();
		context.install(returns).unwrap();
		assert!(context.fresh_builtin_macro(Hash::type_hash(crate::item!(::std::fmt::format))));
		assert!(context.fresh_builtin_macro(Hash::type_hash(crate::item!(::std::test::assert))));
	}
	if let Some(kind) = row["context_override"].as_str() {
		let mut module = crate::Module::with_item(["host"]).unwrap();
		module
			.function("override", |_: crate::runtime::Value| 99i64)
			.build()
			.unwrap();
		context.install(module).unwrap();
		let handler = context
			.lookup_function(Hash::type_hash(["host", "override"]))
			.unwrap()
			.clone();
		let key = match kind {
			"associated" => {
				Hash::associated_function(Hash::type_hash(["R"]), Hash::ident("capacity"))
			}
			"item" => Hash::type_hash(["R", "capacity"]),
			_ => panic!("unknown Context override fixture"),
		};
		context.c13_insert_raw_fixture(key, handler).unwrap();
	}
	context
}

#[cfg(feature = "serde")]
fn canonical(value: Json) -> Json {
	match value {
		Json::Object(object) => {
			let mut entries: std::vec::Vec<_> = object.into_iter().collect();
			entries.sort_by(|a, b| a.0.cmp(&b.0));
			Json::Object(
				entries
					.into_iter()
					.map(|(key, value)| (key, canonical(value)))
					.collect(),
			)
		}
		Json::Array(values) => Json::Array(values.into_iter().map(canonical).collect()),
		value => value,
	}
}

fn stock(row: &mut Json) -> Json {
	let context = context(row);
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.build_vm()
		.unwrap();
	#[cfg(feature = "serde")]
	let unit = canonical(serde_json::to_value(&**vm.unit()).unwrap());
	#[cfg(feature = "serde")]
	let unit_hash = Some(digest(&serde_json::to_vec(&unit).unwrap()));
	#[cfg(not(feature = "serde"))]
	let unit_hash: Option<std::string::String> = None;

	if row["guess_closure"] == true && row["entry_hash"].is_null() {
		#[cfg(feature = "serde")]
		{
			let candidates: std::vec::Vec<_> = unit["functions"]
				.as_object()
				.unwrap()
				.iter()
				.filter(|(_, function)| function["Offset"]["captures"] == 1)
				.map(|(hash, _)| hash.parse::<u64>().unwrap())
				.collect();
			assert_eq!(candidates.len(), 1, "{}", row["name"]);
			row["entry_hash"] = json!(candidates[0]);
		}
		#[cfg(not(feature = "serde"))]
		panic!("the operator-only closure hash freeze needs serde");
	}

	let argument = row["stock_arg_function"]
		.as_str()
		.map(|name| vm.call([name], ()).unwrap());
	EFFECTS.with(|count| count.set(0));
	let result = if let Some(argument) = argument {
		let hash = row["entry_hash"]
			.as_u64()
			.map(Hash::new)
			.unwrap_or_else(|| Hash::type_hash([row["stock_entry"].as_str().unwrap()]));
		vm.call(hash, (argument,))
	} else if let Some(argument) = row["args"].get(0).and_then(Json::as_i64) {
		vm.call(["main"], (argument,))
	} else {
		vm.call(["main"], ())
	};
	let observed = match result {
		Ok(value) => json!({"value":crate::from_value::<i64>(value).unwrap()}),
		Err(error) => {
			let site = error.first_location().and_then(|at| {
				let span = at.unit.debug_info()?.instruction_at(at.ip)?.span.range();
				Some(json!([span.start, span.end]))
			});
			json!({"error":error.to_string(),"site":site})
		}
	};
	if let Some(expected) = row["expected_stock_value"].as_i64() {
		assert_eq!(observed["value"], expected, "{}", row["name"]);
	}
	json!({"result":observed,"effects":EFFECTS.with(Cell::get),"unit_sha256":unit_hash})
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
		let nominal = crate::experimental_core::value::Nominal(tail[..end].parse().unwrap());
		let hash = crate::experimental_core::nominal::hash_copy(nominal).unwrap();
		out.push_str(&std::format!("Nominal(hash={hash:016x})"));
		rest = &tail[end + 1..];
	}
	out.push_str(rest);
	out
}

fn baseline(row: &Json) -> Json {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	match super::compile(&mut sources, &context(row), &mut diagnostics, 23) {
		Err(error) => {
			json!({"admitted":false,"refusal":std::format!("{:?}",error.refusal),"location":error.location.map(|at|json!([at.source,at.start,at.end]))})
		}
		Ok(program) => {
			let functions: std::vec::Vec<_> = (0..program.code.functions()).map(|id| {
				let function = program.code.function(id as u32).unwrap();
				let ops: std::vec::Vec<_> = (0..function.code.len()).map(|pc| normalized_op(&program, function.cached(pc).unwrap().op)).collect();
				json!({"hash":std::format!("{:?}",program.functions[id]),"parameters":function.parameters,"slots":function.slots,"ops":ops})
			}).collect();
			json!({"admitted":true,"functions":functions,"ineligible":program.entry_ineligible})
		}
	}
}

#[test]
#[ignore = "operator-only pre-implementation stock freeze; requires new C18E_ORACLE_OUT"]
fn proven_receiver_freeze_oracle() {
	use std::io::Write;
	let path = std::env::var_os("C18E_ORACLE_OUT").unwrap();
	let mut contract: Json =
		serde_json::from_str(include_str!("proven_receiver_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
		std::println!("C18E_ORACLE_ROW {}", row);
	}
	let mut output = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(path)
		.unwrap();
	output
		.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn proven_receiver_stock_oracle_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_contract.json")).unwrap();
	for original in contract["rows"].as_array().unwrap() {
		let mut row = original.clone();
		let actual = stock(&mut row);
		let expected = original["stock"].clone();
		#[cfg(not(feature = "serde"))]
		let expected = {
			let mut expected = expected;
			expected["unit_sha256"] = Json::Null;
			expected
		};
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

#[test]
fn proven_receiver_m4_lowering_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(
			digest(row["source"].as_str().unwrap().as_bytes()),
			row["source_sha256"].as_str().unwrap()
		);
		assert_eq!(baseline(row), row["before"], "{}", row["name"]);
	}
	assert_eq!(contract["rows"].as_array().unwrap().len(), 42);
}

#[test]
#[ignore = "operator-only protocol freeze; requires new C18E_PROTOCOL_OUT"]
fn proven_receiver_freeze_protocol_oracle() {
	use std::io::Write;
	let mut contract: Json =
		serde_json::from_str(include_str!("proven_receiver_protocol_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
		std::println!("C18E_PROTOCOL_ROW {}", row);
	}
	let mut output = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18E_PROTOCOL_OUT").unwrap())
		.unwrap();
	output
		.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn proven_receiver_protocol_stock_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_protocol_contract.json")).unwrap();
	assert_eq!(contract["rows"].as_array().unwrap().len(), 9);
	for original in contract["rows"].as_array().unwrap() {
		let mut row = original.clone();
		let actual = stock(&mut row);
		let expected = original["stock"].clone();
		#[cfg(not(feature = "serde"))]
		let expected = {
			let mut expected = expected;
			expected["unit_sha256"] = Json::Null;
			expected
		};
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

#[test]
fn proven_receiver_protocol_m4_lowering_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_protocol_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(
			digest(row["source"].as_str().unwrap().as_bytes()),
			row["source_sha256"].as_str().unwrap()
		);
		assert_eq!(baseline(row), row["before"], "{}", row["name"]);
	}
}

#[test]
fn proven_receiver_public_macro_registration_stays_uncertified() {
	let row = json!({"macros":true});
	let context = context(&row);
	assert!(!context.fresh_builtin_macro(Hash::type_hash(["mk"])));
	assert!(!context.fresh_builtin_macro(Hash::type_hash(["ret"])));
	let mut context = context;
	let mut public = crate::Module::with_crate_item("std", ["fmt"]).unwrap();
	public
		.macro_("format", |cx, _| {
			let value = cx.lit(1i64)?;
			Ok(crate::macros::quote!(#value).into_token_stream(cx)?)
		})
		.unwrap();
	context.install(public).unwrap();
	assert!(!context.fresh_builtin_macro(Hash::type_hash(crate::item!(::std::fmt::format))));
}

#[test]
#[ignore = "operator-only deterministic builtin-fixture freeze"]
fn proven_receiver_freeze_macro_oracle() {
	use std::io::Write;
	let mut contract: Json =
		serde_json::from_str(include_str!("proven_receiver_builtin_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
		std::println!("C18E_MACRO_ROW {}", row);
	}
	let mut output = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18E_MACRO_OUT").unwrap())
		.unwrap();
	output
		.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn proven_receiver_test_builtin_macro_stock_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_builtin_contract.json")).unwrap();
	for original in contract["rows"].as_array().unwrap() {
		let mut row = original.clone();
		let actual = stock(&mut row);
		let expected = original["stock"].clone();
		#[cfg(not(feature = "serde"))]
		let expected = {
			let mut expected = expected;
			expected["unit_sha256"] = Json::Null;
			expected
		};
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

#[test]
fn proven_receiver_test_builtin_macro_m4_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_builtin_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(baseline(row), row["before"], "{}", row["name"]);
	}
}
