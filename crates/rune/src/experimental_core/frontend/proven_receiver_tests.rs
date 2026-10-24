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
		if row["before"]["admitted"] == true {
			assert_eq!(baseline(row), row["before"], "{}", row["name"]);
		}
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
fn proven_receiver_protocol_frontier() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_protocol_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(
			digest(row["source"].as_str().unwrap().as_bytes()),
			row["source_sha256"].as_str().unwrap()
		);
		let actual = baseline(row);
		if row["after"] == "admitted" {
			assert_eq!(actual["admitted"], true, "{}: {}", row["name"], actual);
		} else {
			let mut expected = row["before"].clone();
			match row["name"].as_str().unwrap() {
				"item-context-override" => expected["refusal"] = json!("Some(Native)"),
				"proven-receiver-wrong-arity" => expected["refusal"] = json!("Some(Arity)"),
				"resolved-method-before-async" => {
					expected["refusal"] = json!("Some(AsyncBlock)");
					let source = row["source"].as_str().unwrap();
					let start = source.find("async { 1 }").unwrap();
					expected["location"] = json!([0, start, start + "async { 1 }".len()]);
				}
				_ => (),
			}
			assert_eq!(actual, expected, "{}", row["name"]);
		}
	}
}

#[test]
#[ignore = "operator-only receiver proof frontier report"]
fn proven_receiver_observe_frontier() {
	for fixture in [
		include_str!("proven_receiver_contract.json"),
		include_str!("proven_receiver_protocol_contract.json"),
		include_str!("proven_receiver_exclusions_contract.json"),
		include_str!("proven_receiver_builtin_contract.json"),
	] {
		let contract: Json = serde_json::from_str(fixture).unwrap();
		for row in contract["rows"].as_array().unwrap() {
			std::println!(
				"C18E_FRONTIER {}",
				json!({"name":row["name"],"after":baseline(row)})
			);
		}
	}
}

fn fresh(row: &Json) -> (Program, Context) {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let context = context(row);
	let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 23).unwrap();
	(program, context)
}

#[test]
fn proven_receiver_private_and_closure_entries_refuse_before_work() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_contract.json")).unwrap();
	for row in contract["rows"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|row| !row["host_policy"].is_null())
	{
		let (program, context) = fresh(row);
		let hash = row["entry_hash"]
			.as_u64()
			.map(Hash::new)
			.unwrap_or_else(|| Hash::type_hash(["take"]));
		let target = program.function(hash).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		assert!(!program.entry_eligible(target));
		assert!(program.entry_eligible(main));
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let wrong = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["wrong"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		let foreign_store = Store::new(Limits::default()).unwrap();
		let foreign = foreign_store.root_i64(1);
		let baseline = (store.live_entries(), store.payload_bytes());
		EFFECTS.with(|count| count.set(0));
		executor.request_cancel();
		for fuel in [0, 1000] {
			for args in [
				std::vec![],
				std::vec![&wrong],
				std::vec![&foreign],
				std::vec![&wrong, &wrong],
			] {
				let failure = executor
					.invoke(&program, &mut store, target, &args, fuel)
					.unwrap_err();
				assert_eq!(failure.error.refusal, Some(Refusal::Entry));
				assert!(
					failure.call_chain.is_empty()
						&& failure.cleanup_error.is_none()
						&& executor.is_idle()
				);
				assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
				assert_eq!(EFFECTS.with(Cell::get), 0);
			}
		}
		let failure = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::Cancelled);
		assert_eq!(EFFECTS.with(Cell::get), 0);
		let result = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap();
		assert!(matches!(store.import(&result).unwrap(), Value::I64(7)));
		assert_eq!(EFFECTS.with(Cell::get), 1);
		store.root_release(result).unwrap();
		store.root_release(wrong).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
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
fn proven_receiver_test_builtin_macro_frontier() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_builtin_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(baseline(row)["admitted"], true, "{}", row["name"]);
	}
}

fn expected_admission(row: &Json) -> bool {
	let fields: Json =
		serde_json::from_str(include_str!("field_receiver_progressions.json")).unwrap();
	if let Some(entry) = fields["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|entry| entry["name"] == row["name"])
	{
		assert_eq!(entry["source_sha256"], row["source_sha256"]);
		return entry["after_admitted"].as_bool().unwrap();
	}
	let overlay: Json =
		serde_json::from_str(include_str!("proven_receiver_prediction_overlay.json")).unwrap();
	overlay["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|entry| entry["name"] == row["name"])
		.map_or_else(
			|| {
				row["after_admitted"]
					.as_bool()
					.unwrap_or_else(|| row["after"] == "admitted")
			},
			|entry| entry["after_admitted"].as_bool().unwrap(),
		)
}

#[test]
fn proven_receiver_frozen_frontier_and_execution() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	for fixture in [
		include_str!("proven_receiver_contract.json"),
		include_str!("proven_receiver_builtin_contract.json"),
		include_str!("proven_receiver_protocol_contract.json"),
		include_str!("proven_receiver_exclusions_contract.json"),
	] {
		let contract: Json = serde_json::from_str(fixture).unwrap();
		for row in contract["rows"].as_array().unwrap() {
			let actual = baseline(row);
			assert_eq!(
				actual["admitted"],
				expected_admission(row),
				"{}: {}",
				row["name"],
				actual
			);
			if !expected_admission(row) {
				if row["after_admitted"].is_boolean() {
					assert_eq!(actual, row["before"], "{}", row["name"]);
				}
				continue;
			}
			let (program, context) = fresh(row);
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			assert!(program.entry_eligible(main), "{}", row["name"]);
			let mut executor = Executor::new().unwrap();
			executor
				.set_host_runtime(Some(
					crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
				))
				.unwrap();
			let mut store = Store::new(Limits::default()).unwrap();
			let argument = row["args"]
				.get(0)
				.and_then(Json::as_i64)
				.map(|n| store.root_i64(n));
			let arguments = argument.iter().collect::<std::vec::Vec<_>>();
			EFFECTS.with(|count| count.set(0));
			let result = executor
				.invoke(&program, &mut store, main, &arguments, 10000)
				.unwrap();
			let expected = if row["host_policy"].is_null() {
				row["stock"]["result"]["value"].as_i64().unwrap()
			} else {
				7
			};
			assert!(
				matches!(store.import(&result).unwrap(), Value::I64(value) if value == expected),
				"{}",
				row["name"]
			);
			assert_eq!(
				EFFECTS.with(Cell::get),
				usize::from(!row["host_policy"].is_null()),
				"{}",
				row["name"]
			);
			store.root_release(result).unwrap();
			if let Some(argument) = argument {
				store.root_release(argument).unwrap();
			}
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		}
	}
}

#[test]
#[ignore = "operator-only unmodified AWFY compilation frontier"]
fn proven_receiver_awfy_frontier() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benches/awfy");
	for name in [
		"bounce",
		"cd",
		"havlak",
		"json",
		"list",
		"mandelbrot",
		"nbody",
		"permute",
		"queens",
		"sieve",
		"storage",
		"towers",
	] {
		let files = std::vec![
			std::string::String::from("harness.rn"),
			std::string::String::from("benchmark.rn")
		];
		let mut files = files;
		if matches!(name, "bounce" | "cd" | "havlak" | "json" | "storage") {
			files.push(std::string::String::from("som.rn"));
		}
		files.push(std::format!("{name}.rn"));
		let mut sources = Sources::new();
		let mut inputs = std::vec::Vec::new();
		for file in files {
			let text = std::fs::read_to_string(root.join(&file)).unwrap();
			inputs.push((file.clone(), text.clone()));
			sources
				.insert(crate::Source::new(std::format!("awfy/{file}"), text).unwrap())
				.unwrap();
		}
		let result = super::compile(
			&mut sources,
			&Context::with_default_modules().unwrap(),
			&mut Diagnostics::new(),
			23,
		);
		let observation = match result {
			Ok(program) => {
				let ineligible: std::vec::Vec<_> = program.entry_ineligible.iter().map(|&id| {
					let f = program.code.function(id).unwrap();
					json!({"id":id,"hash":std::format!("{:?}",program.functions[id as usize]),"parameters":f.parameters,"first_location":f.locations.first().map(|at|json!([at.source,at.start,at.end]))})
				}).collect();
				json!({"admitted":true,"functions":program.functions.len(),"ineligible":ineligible,"main_eligible":program.entry_eligible(program.function(Hash::type_hash(["main"])).unwrap())})
			}
			Err(error) => {
				let location = error.location.map(|at| {
					let input = inputs.get(at.source as usize);
					json!({"source":at.source,"span":[at.start,at.end],"file":input.map(|(file,_)|file),"text":input.and_then(|(_,text)|text.get(at.start as usize..at.end as usize))})
				});
				json!({"admitted":false,"refusal":std::format!("{:?}",error.refusal),"location":location})
			}
		};
		std::println!(
			"C18E_AWFY {}",
			json!({"program":name,"observation":observation})
		);
	}
}

#[test]
fn proven_receiver_both_context_absences_are_checked_before_execution() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
	};
	for key in [
		Hash::associated_function(Hash::type_hash(["R"]), Hash::ident("capacity")),
		Hash::type_hash(["R", "capacity"]),
	] {
		let row = json!({"source":"struct R; impl R { fn capacity(self) { 7 } } fn make() { R } pub fn main() { mark(); make().capacity() }"});
		let (program, mut context) = fresh(&row);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let original = crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap();
		let handler = context
			.lookup_function(Hash::type_hash(["mark"]))
			.unwrap()
			.clone();
		context.c13_insert_raw_fixture(key, handler).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		EFFECTS.with(|count| count.set(0));
		let failure = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
		assert_eq!(EFFECTS.with(Cell::get), 0);
		assert!(executor.is_idle());
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		executor.set_host_runtime(Some(original)).unwrap();
		let result = executor
			.invoke(&program, &mut store, main, &[], 1000)
			.unwrap();
		assert_eq!(EFFECTS.with(Cell::get), 1);
		store.root_release(result).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}

#[test]
#[ignore = "operator-only conservative return exclusions freeze"]
fn proven_receiver_freeze_exclusions_oracle() {
	use std::io::Write;
	let mut contract: Json =
		serde_json::from_str(include_str!("proven_receiver_exclusions_contract.json")).unwrap();
	for row in contract["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
		std::println!("C18E_EXCLUSION_ROW {}", row);
	}
	let mut output = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18E_EXCLUSIONS_OUT").unwrap())
		.unwrap();
	output
		.write_all(serde_json::to_string_pretty(&contract).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn proven_receiver_exclusions_stock_is_frozen() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_exclusions_contract.json")).unwrap();
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
fn proven_receiver_exclusions_keep_m4_refusals() {
	let contract: Json =
		serde_json::from_str(include_str!("proven_receiver_exclusions_contract.json")).unwrap();
	for row in contract["rows"].as_array().unwrap() {
		assert_eq!(
			digest(row["source"].as_str().unwrap().as_bytes()),
			row["source_sha256"].as_str().unwrap()
		);
		assert_eq!(baseline(row), row["before"], "{}", row["name"]);
	}
}

#[test]
fn deferred_refusal_keeps_the_frozen_object_diagnostic() {
	let source = "use c17b::tap;struct P{x}struct T(a)enum E{A,B(x)}impl T{fn len(self){99}fn is_empty(self){false}}pub fn main(){(T(1).len(),T(1).is_empty(),#{}.len(),#{}.is_empty())}pub fn c17b_reuse(){7}";
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c17b"]).unwrap();
	module
		.fresh_host_function("tap", |_: i64, value: i64| value)
		.unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::new("c17b.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let error = super::compile(&mut sources, &context, &mut diagnostics, 16)
		.err()
		.unwrap();
	assert_eq!(error.refusal, Some(Refusal::Method));
	assert_eq!(error.location.unwrap().start, 140);
	assert_eq!(error.location.unwrap().end, 149);
	assert_eq!(std::format!("{:?}", diagnostics.diagnostics()), "[Fatal(FatalDiagnostic { source_id: 0, kind: CompileError(Error { span: 140:149, kind: Custom { error: \"Method\" } }) }), Warning(WarningDiagnostic { source_id: 0, kind: NotUsed { span: 0:13, context: None } })]");
}

#[test]
fn deferred_refusal_paths_always_emit_the_compile_diagnostic() {
	for source in [
		"struct R;impl R{fn capacity(self){7}}fn make(x){x}pub fn main(){make(1).capacity()}",
		"struct R;impl R{fn capacity(self){7}}fn probe(x){x.capacity()}pub fn main(){probe(R)}fn later(){probe(1)}",
	] {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources.insert(crate::Source::memory(source).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		let error = super::compile(&mut sources, &context, &mut diagnostics, 23).err().unwrap();
		assert_eq!(error.refusal, Some(Refusal::Method));
		let at = error.location.unwrap();
		assert!(diagnostics.has_error());
		assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
			let crate::diagnostics::Diagnostic::Fatal(fatal) = diagnostic else { return false };
			fatal.source_id() == crate::SourceId::new(at.source)
				&& fatal.as_compile_error().is_some_and(|compile| {
					compile.span() == Span::new(at.start, at.end)
						&& compile.to_string() == "Method"
				})
		}), "{source}: {:?}", diagnostics.diagnostics());
	}
}

#[test]
fn deferred_refusals_keep_all_body_diagnostics_and_the_closure_span() {
	let cases: Json =
		serde_json::from_str(include_str!("../tests/gap_hunt/numeric_methods/cases.json")).unwrap();
	let rules: Json = serde_json::from_str(include_str!(
		"../tests/gap_hunt/numeric_methods/dispositions.json"
	))
	.unwrap();
	let row = &cases[316];
	assert_eq!(row["label"], "script/impl-partial_cmp-sort");
	let source = row["source"].as_str().unwrap();
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c17c"]).unwrap();
	module
		.fresh_host_function("tap", |_: i64, value: i64| value)
		.unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::new("c17c.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let error = super::compile(&mut sources, &context, &mut diagnostics, 16)
		.err()
		.unwrap();
	assert_eq!(error.refusal, Some(Refusal::Method));
	assert_eq!(error.location.unwrap().start, 156);
	assert_eq!(error.location.unwrap().end, 172);
	assert_eq!(
		json!(std::format!("{:?}", diagnostics.diagnostics())),
		rules["rows"][316]["diagnostics"]
	);
}

#[path = "field_receiver_tests.rs"]
mod field_receiver_tests;
