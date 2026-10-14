//! Fresh-only setup controls. Names/sources are from the immutable H1 catalogue.
use super::*;
use crate::experimental_core::{code::FIRST_HOST, error::ErrorKind};
use std::sync::atomic::{AtomicUsize, Ordering};

// Only this one test calls the function. This counter proves absence and recovery
// without replacing the function pointer with a capturing closure.
static STATIC_CALLS: AtomicUsize = AtomicUsize::new(0);
fn static_mark(_: &str, value: i64) -> i64 {
	STATIC_CALLS.fetch_add(1, Ordering::SeqCst);
	value
}
fn same_fn_context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["h1"]).unwrap();
	module.fresh_host_function("mark", static_mark).unwrap();
	context.install(module).unwrap();
	context
}
fn program(context: &Context, source: &str) -> crate::experimental_core::frontend::Program {
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1).unwrap()
}

#[test]
#[cfg_attr(
	miri,
	ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
)]
fn ten_frozen_identity_modes_check_admission_ownership_and_recovery() {
	let cases: Vec<_> = CASES.iter().filter(|case| !case.stock).collect();
	assert_eq!(cases.len(), 10);
	let mut done = std::collections::BTreeSet::new();
	for case in cases {
		assert!(done.insert(case.mode));
		STATIC_CALLS.store(0, Ordering::SeqCst);
		let tape = fixture::Recorder::new();
		let is_fn = case.mode == "same-fn-new-allocation";
		let context_a = if is_fn {
			same_fn_context()
		} else {
			fixture::fresh_context(&tape)
		};
		let other_tape = fixture::Recorder::new();
		let context_b = if is_fn {
			same_fn_context()
		} else {
			fixture::fresh_context(&other_tape)
		};
		let source = if case.mode == "unused-host-descriptor" {
			format!("{} pub fn idle(){{7}}", case.source)
		} else {
			case.source.into()
		};
		let use_b = case.mode == "two-programs-same-hash";
		let mut compiled_context = Some(if use_b { context_b } else { context_a });
		let correct =
			crate::sync::Arc::try_new(compiled_context.as_ref().unwrap().runtime().unwrap())
				.unwrap();
		let compiled = program(compiled_context.as_ref().unwrap(), &source);
		let entry = if case.mode == "unused-host-descriptor" {
			"idle"
		} else {
			"main"
		};
		let function = compiled.function(Hash::type_hash([entry])).unwrap();
		if entry == "idle" {
			assert!(
				compiled.code.host(FIRST_HOST).is_some(),
				"unreachable descriptor must exist"
			);
			assert!(compiled.code.functions() >= 2);
		}
		let accept = matches!(
			case.mode,
			"clone-runtime" | "context-dropped" | "configured-runtime-reuse"
		);
		let runtime = match case.mode {
			"clone-runtime" | "context-dropped" | "configured-runtime-reuse" => {
				Some(correct.clone())
			}
			"different-capture" | "same-fn-new-allocation" | "two-programs-same-hash" => {
				let wrong = if is_fn {
					same_fn_context()
				} else {
					fixture::fresh_context(&other_tape)
				};
				Some(crate::sync::Arc::try_new(wrong.runtime().unwrap()).unwrap())
			}
			"missing-handler" | "unused-host-descriptor" => {
				Some(crate::sync::Arc::try_new(crate::runtime::RuntimeContext::default()).unwrap())
			}
			"clear-runtime" | "missing-runtime" => None,
			_ => panic!("unreviewed identity mode"),
		};
		if case.mode == "context-dropped" {
			drop(compiled_context.take());
		}
		let mut executor = Executor::new().unwrap();
		if case.mode == "clear-runtime" {
			executor.set_host_runtime(Some(correct.clone())).unwrap();
		}
		executor.set_host_runtime(runtime).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let caller = store.root_string("caller-owned").unwrap();
		let before = (store.live_entries(), store.payload_bytes());
		let before_state = store.state_for_test();
		if accept {
			let invocations = if case.mode == "configured-runtime-reuse" {
				2
			} else {
				1
			};
			for _ in 0..invocations {
				let root = executor
					.invoke(&compiled, &mut store, function, &[], FUEL)
					.unwrap();
				assert_eq!(
					store.root_scalar(&root).unwrap(),
					Some(crate::experimental_core::value::Scalar::I64(7))
				);
				store.root_release(root).unwrap();
				assert!(executor.is_idle());
				assert_eq!(before, (store.live_entries(), store.payload_bytes()));
			}
			assert_eq!(tape.snapshot().0, invocations);
		} else {
			executor.request_cancel();
			let failure = executor
				.invoke(&compiled, &mut store, function, &[], FUEL)
				.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::InvalidCode, "{}", case.label);
			assert!(failure.error.location.is_none());
			assert!(failure.call_chain.is_empty());
			assert!(failure.cleanup_error.is_none());
			assert_eq!(tape.snapshot(), (0, Vec::new()));
			assert_eq!(other_tape.snapshot(), (0, Vec::new()));
			assert_eq!(STATIC_CALLS.load(Ordering::SeqCst), 0);
			assert_eq!(before, (store.live_entries(), store.payload_bytes()));
			assert_eq!(before_state, store.state_for_test());
			assert_eq!(store.root_text(&caller).unwrap(), "caller-owned");
			assert!(executor.is_idle());
			// Correct binding admits; preserved cancellation must then fire at charge.
			executor.set_host_runtime(Some(correct.clone())).unwrap();
			let cancelled = executor
				.invoke(&compiled, &mut store, function, &[], FUEL)
				.unwrap_err();
			assert_eq!(cancelled.error.kind, ErrorKind::Cancelled);
			assert_eq!(STATIC_CALLS.load(Ordering::SeqCst), 0);
			let root = executor
				.invoke(&compiled, &mut store, function, &[], FUEL)
				.unwrap();
			assert_eq!(
				store.root_scalar(&root).unwrap(),
				Some(crate::experimental_core::value::Scalar::I64(7))
			);
			store.root_release(root).unwrap();
			if is_fn {
				assert_eq!(STATIC_CALLS.load(Ordering::SeqCst), 1);
			}
			assert_eq!(before, (store.live_entries(), store.payload_bytes()));
			if case.mode == "clear-runtime" {
				executor.set_host_runtime(None).unwrap();
				let plain = program(
					&Context::with_default_modules().unwrap(),
					"pub fn main(){7}",
				);
				let root = executor
					.invoke(
						&plain,
						&mut store,
						plain.function(Hash::type_hash(["main"])).unwrap(),
						&[],
						FUEL,
					)
					.unwrap();
				store.root_release(root).unwrap();
			}
		}
		store.root_release(caller).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
	contract_completed("fresh", 10);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
)]
fn fuel_frontiers_keep_complete_host_call_effect_prefixes_and_clean_state() {
	let rows: Vec<_> = include_str!("stock.tsv")
		.lines()
		.skip(1)
		.map(|s| s.split('\t').collect::<Vec<_>>())
		.filter(|r| r[10] == "source-only-fuel-no-numeric-parity")
		.collect();
	assert_eq!(rows.len(), 17);
	assert!(rows.iter().all(|r| r[2] == rows[0][2]));
	let expected = ["mark", "left", "mark", "right", "add"];
	let mut frontier = Vec::new();
	let mut succeeded = false;
	for budget in 0..=16 {
		let tape = fixture::Recorder::new();
		let context = fixture::fresh_context(&tape);
		let compiled = program(&context, &unhex(rows[0][2]));
		let main = compiled.function(Hash::type_hash(["main"])).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let outcome = executor.invoke(&compiled, &mut store, main, &[], budget);
		let (calls, events) = tape.snapshot();
		assert!(matches!(events.len(), 0 | 2 | 4 | 5));
		assert_eq!(events, expected[..events.len()]);
		assert_eq!(
			calls,
			events
				.iter()
				.filter(|s| s.as_str() == "mark" || s.as_str() == "add")
				.count()
		);
		let phase = match outcome {
			Ok(root) => {
				assert_eq!(
					store.root_scalar(&root).unwrap(),
					Some(crate::experimental_core::value::Scalar::I64(15))
				);
				store.root_release(root).unwrap();
				assert_eq!(calls, 3);
				assert_eq!(events, expected);
				succeeded = true;
				"value"
			}
			Err(failure) => {
				assert_eq!(failure.error.kind, ErrorKind::FuelExhausted);
				assert!(failure.cleanup_error.is_none());
				"fuel"
			}
		};
		if budget == 0 {
			assert_eq!(calls, 0);
			assert_eq!(phase, "fuel");
		}
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		frontier.push((budget, phase, calls));
	}
	assert!(succeeded, "frontier bound must include completion");
	println!("H1 FUEL FRONTIER {frontier:?}");
}
