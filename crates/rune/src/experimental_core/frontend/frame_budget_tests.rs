//! S6 recursion observations, frozen before default frame and slot changes.
use super::*;
use serde_json::{json, Value as Json};

fn rows() -> std::vec::Vec<Json> {
	let data: Json = serde_json::from_str(include_str!("frame_budget_contract.json")).unwrap();
	data["rows"].as_array().unwrap().clone()
}

fn sources(row: &Json) -> Sources {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	sources
}

#[test]
#[ignore = "operator-only stock child; S6_CASE and new S6_RESULT required"]
fn frame_budget_stock_child() {
	use std::io::Write;
	let index: usize = std::env::var("S6_CASE").unwrap().parse().unwrap();
	let row = rows().remove(index);
	let context = Context::with_default_modules().unwrap();
	let mut sources = sources(&row);
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	let outcome = match vm.call(["main"], ()) {
		Ok(value) => json!({"status":"value","value":crate::from_value::<i64>(value).unwrap()}),
		Err(error) => {
			json!({"status":"error","kind":std::format!("{:?}",error.error().kind()),"display":std::format!("{error}")})
		}
	};
	let observation =
		json!({"outcome":outcome,"diagnostics":std::format!("{:?}",diagnostics.diagnostics())});
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("S6_RESULT").unwrap())
		.unwrap();
	file.write_all(
		serde_json::to_string_pretty(&observation)
			.unwrap()
			.as_bytes(),
	)
	.unwrap();
}

#[test]
#[ignore = "operator-only pre-change frame and slot observation"]
fn frame_budget_observe_before() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	for row in rows() {
		let context = Context::with_default_modules().unwrap();
		let mut sources = sources(&row);
		let mut diagnostics = Diagnostics::new();
		let compiled = super::compile(&mut sources, &context, &mut diagnostics, 6);
		let result = match compiled {
			Err(error) => json!({"compiled":false,"error":std::format!("{error:?}")}),
			Ok(program) => {
				let slots: std::vec::Vec<_> = (0..program.code.functions())
					.map(|i| program.code.function(i as u32).unwrap().slots)
					.collect();
				let mut store = Store::new(Limits::default()).unwrap();
				let mut executor = Executor::new().unwrap();
				let outcome = match executor.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					200_000_000,
				) {
					Ok(root) => {
						let Value::I64(value) = store.import(&root).unwrap() else {
							panic!("expected integer");
						};
						store.root_release(root).unwrap();
						json!({"status":"value","value":value})
					}
					Err(failure) => {
						json!({"status":"error","kind":std::format!("{:?}",failure.error.kind),"location":std::format!("{:?}",failure.error.location),"callers":failure.call_chain.len(),"cleanup_error":failure.cleanup_error.is_some()})
					}
				};
				assert!(executor.is_idle());
				assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
				json!({"compiled":true,"slots":slots,"outcome":outcome,"clean":true,"diagnostics":std::format!("{:?}",diagnostics.diagnostics())})
			}
		};
		std::println!(
			"S6_BEFORE {}",
			json!({"name":row["name"],"source_sha256":row["source_sha256"],"result":result})
		);
	}
}

#[test]
fn frame_budget_sources_are_pinned() {
	use sha2::{Digest, Sha256};
	let rows = rows();
	assert_eq!(rows.len(), 27);
	for row in rows {
		let hash = Sha256::digest(row["source"].as_str().unwrap().as_bytes());
		let text = hash
			.iter()
			.map(|b| std::format!("{b:02x}"))
			.collect::<std::string::String>();
		assert_eq!(text, row["source_sha256"].as_str().unwrap());
	}
}

#[test]
fn frame_budget_callback_progressions_are_narrow() {
	let data: Json = serde_json::from_str(include_str!("frame_budget_c11_contract.json")).unwrap();
	let rows = data["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 2);
	for (row, (index, label)) in rows.iter().zip([(106, "depth/511"), (107, "depth/512")]) {
		assert_eq!(row["index"], index);
		assert_eq!(row["label"], label);
		assert_eq!(row["classification"], "Retained-depth");
		assert_eq!(row["stock_limit"], 32);
		assert_eq!(
			row["source_sha256"],
			"439465a86c3ee721280200e97c934cc7d629910a96ef20c605e2f80fecd6c9aa"
		);
	}
}
