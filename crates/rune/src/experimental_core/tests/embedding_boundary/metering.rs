//! Isolated L1 work observations. The controller compares two live processes.
use super::*;
use crate::experimental_core::{
	error::{ErrorClass, ErrorProjection},
	execute::take_remaining_work_for_test,
};
use serde_json::{json, Value as Json};
use std::{fs, io::Write, path::Path};

const WORK: usize = 2_000_000;

fn observe(path: &Path, mode: &str, engine: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let text = fs::read_to_string(path).unwrap();
	assert!(text.len() < 1_000_000);
	let text = if mode == "script" {
		std::format!("pub fn main(){{\n{text}\n}}")
	} else {
		text
	};
	let entry = mode.strip_prefix("test:").unwrap_or("main");
	let parts: std::vec::Vec<_> = entry.split("::").collect();
	let hash = Hash::type_hash(parts.as_slice());
	if engine == "legacy" {
		let mut input = sources(&text);
		let compiled = crate::prepare(&mut input).with_context(&context).build_vm();
		let Ok(mut vm) = compiled else {
			return json!({"phase":"compile-error"});
		};
		if mode == "compile" {
			return json!({"phase":"compiled"});
		}
		let (result, remaining) = crate::runtime::budget::with(WORK, || {
			let result = vm.call(hash, ());
			(result, crate::runtime::budget::remaining().unwrap())
		})
		.call();
		let phase = match result {
			Ok(_) => "complete",
			Err(error)
				if matches!(
					error.error_class(),
					ErrorProjection::Class(ErrorClass::BudgetExhausted)
				) =>
			{
				"budget"
			}
			Err(_) => "runtime-error",
		};
		return json!({"phase":phase,"remaining":remaining,"used":WORK-remaining});
	}
	assert_eq!(engine, "execution");
	let compiled = frontend::compile(&mut sources(&text), &context, &mut Diagnostics::new(), 1);
	let program = match compiled {
		Ok(program) => program,
		Err(error) => {
			return json!({"phase":if error.refusal.is_some(){"refused"}else{"compile-error"},"reason":std::format!("{:?}",error.refusal)})
		}
	};
	if mode == "compile" {
		return json!({"phase":"compiled"});
	}
	let Some(function) = program.function(hash) else {
		return json!({"phase":"entry-error"});
	};
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	executor.set_format_limits(FormatLimits::permissive());
	let _ = take_remaining_work_for_test();
	let prepared = match executor.prepare(&program, &mut store, function, &[]) {
		Ok(prepared) => prepared,
		Err(PrepareError::Refused(reason)) => {
			return json!({"phase":"refused","reason":std::format!("{reason:?}")})
		}
		Err(PrepareError::Invalid(error)) => {
			return json!({"phase":"entry-error","error":std::format!("{error:?}")})
		}
	};
	let result = prepared.invoke(WORK);
	let remaining = take_remaining_work_for_test();
	let phase = match result {
		Ok(root) => {
			store.root_release(root).unwrap();
			"complete"
		}
		Err(failure) if failure.error.kind == ErrorKind::FuelExhausted => "budget",
		Err(_) => "runtime-error",
	};
	assert!(executor.is_idle());
	store.close().unwrap();
	json!({"phase":phase,"remaining":remaining,"used":remaining.map(|n|WORK-n)})
}

#[test]
#[ignore = "isolated corpus child; invoked by the L1 metering controller"]
fn child() {
	let path = std::env::var_os("GAP_CASE").unwrap();
	let mode = std::env::var("GAP_MODE").unwrap();
	let engine = std::env::var("GAP_ENGINE").unwrap();
	let result = observe(Path::new(&path), &mode, &engine);
	fs::write(
		std::env::var_os("GAP_RESULT").unwrap(),
		serde_json::to_vec(&result).unwrap(),
	)
	.unwrap();
	// The shared sandbox helper requires this sidecar. Actual warnings are
	// compared separately by the existing L1 semantic pass, not this observer.
	fs::write(std::env::var_os("GAP_L1_WARNINGS").unwrap(), b"[]").unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
