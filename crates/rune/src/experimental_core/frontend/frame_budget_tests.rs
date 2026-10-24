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

fn run_default_row(
	row: &Json,
	stack_bytes: Option<usize>,
	native: bool,
	limits: Option<crate::experimental_core::execute::ExecutorLimits>,
) {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	let context = Context::with_default_modules().unwrap();
	let mut sources = sources(row);
	let mut diagnostics = Diagnostics::new();
	let program = super::compile(&mut sources, &context, &mut diagnostics, 6).unwrap();
	assert!(diagnostics.diagnostics().is_empty());
	let expected = row["expected_i64"].as_i64().unwrap();
	let run = move || {
		#[cfg(all(
			feature = "fresh-core-native",
			target_arch = "x86_64",
			target_os = "linux"
		))]
		if native {
			crate::experimental_core::region::cache::clear_for_test();
			crate::experimental_core::region::cache::enable_for_test(true);
		}
		#[cfg(not(all(
			feature = "fresh-core-native",
			target_arch = "x86_64",
			target_os = "linux"
		)))]
		assert!(!native);
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::with_limits(limits.unwrap_or_default()).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		for _ in 0..2 {
			let root = executor
				.invoke(&program, &mut store, main, &[], 200_000_000)
				.unwrap();
			assert!(matches!(store.import(&root).unwrap(), Value::I64(n) if n == expected));
			store.root_release(root).unwrap();
			assert!(executor.is_idle());
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		}
		#[cfg(all(
			feature = "fresh-core-native",
			target_arch = "x86_64",
			target_os = "linux"
		))]
		if native {
			let entries =
				crate::experimental_core::region::cache::regions_for_test(program.code.id())
					.unwrap();
			assert!(entries.iter().any(|region| region.entries > 0));
			assert_eq!(crate::experimental_core::region::cache::invalid_exits(), 0);
			crate::experimental_core::region::cache::clear_for_test();
			crate::experimental_core::region::cache::enable_for_test(false);
		}
	};
	if let Some(bytes) = stack_bytes {
		// Compilation of Rune source is complete before entering the runtime stack.
		std::thread::Builder::new()
			.stack_size(bytes)
			.spawn(run)
			.unwrap()
			.join()
			.unwrap();
	} else {
		run();
	}
}

#[test]
fn frame_budget_defaults_match_values_and_named_callback_policy() {
	for row in rows() {
		let stock = &row["stock"]["outcome"];
		if row["classification"] == "Exact" {
			assert_eq!(stock["status"], "value");
			assert_eq!(stock["value"], row["expected_i64"]);
		} else {
			assert_eq!(
				row["classification"],
				"Intentional resource-policy divergence"
			);
			assert_eq!(stock["kind"], "MaxExecutionDepth { max: 32 }");
		}
		run_default_row(&row, None, false, None);
	}
}

#[test]
fn frame_budget_deep_runtime_uses_a_small_machine_stack() {
	for row in rows().into_iter().filter(|row| row["depth"] == 100_000) {
		run_default_row(&row, Some(256 * 1024), false, None);
	}
}

#[test]
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
#[ignore = "operator-only isolated cold native 512 KiB stack child"]
fn frame_budget_native_adequate_stack_child() {
	let row = rows()
		.into_iter()
		.find(|row| row["kind"] == "native-loop" && row["depth"] == 100_000)
		.unwrap();
	run_default_row(&row, Some(512 * 1024), true, None);
}

#[test]
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
#[ignore = "operator-only isolated native stack attribution"]
fn frame_budget_native_stack_probe_child() {
	let depth: i64 = std::env::var("S6_NATIVE_DEPTH").unwrap().parse().unwrap();
	let mut row = rows()
		.into_iter()
		.find(|row| row["kind"] == "native-loop" && row["depth"] == 100_000)
		.unwrap();
	row["source"] = json!(row["source"]
		.as_str()
		.unwrap()
		.replace("100000", &std::format!("{depth}")));
	row["expected_i64"] = json!(depth);
	let limits = std::env::var_os("S6_NATIVE_HISTORICAL").map(|_| {
		crate::experimental_core::execute::ExecutorLimits {
			frames: 1024,
			slots: 65_536,
		}
	});
	run_default_row(
		&row,
		std::env::var_os("S6_NATIVE_SMALL").map(|_| {
			std::env::var("S6_NATIVE_STACK_BYTES")
				.ok()
				.map(|n| n.parse().unwrap())
				.unwrap_or(256 * 1024)
		}),
		true,
		limits,
	);
}

#[test]
fn frame_budget_slot_sandbox_is_independent_of_the_frame_sandbox() {
	use crate::experimental_core::{
		error::ErrorKind,
		execute::{Executor, ExecutorLimits},
		heap::{Limits, Store},
	};
	let row = rows()
		.into_iter()
		.find(|row| row["kind"] == "wide")
		.unwrap();
	let context = Context::with_default_modules().unwrap();
	let mut sources = sources(&row);
	let program = super::compile(&mut sources, &context, &mut Diagnostics::new(), 6).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut depths = std::vec::Vec::new();
	for limits in [
		ExecutorLimits {
			slots: 65_536,
			..Default::default()
		},
		ExecutorLimits {
			frames: 1024,
			..Default::default()
		},
	] {
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::with_limits(limits).unwrap();
		for _ in 0..2 {
			let failure = executor
				.invoke(&program, &mut store, main, &[], 200_000_000)
				.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::FrameCapacity);
			assert!(failure.cleanup_error.is_none());
			assert!(executor.is_idle());
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
			depths.push(failure.call_chain.len());
		}
	}
	// The 64-slot recursive window makes the two isolated historical caps
	// coincide here; neither run uses the other historical cap.
	assert_eq!(depths[0], 1023);
	assert_eq!(depths[0], depths[1]);
	assert_eq!(&depths[2..], &[1023, 1023]);
}
