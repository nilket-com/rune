//! Minimal Builder fixtures for native and focused Miri; no frontend or std modules.
use super::minimal_tests::{code, executor, installed};
use super::*;
use crate::experimental_core::{error::HostDynamicReason as Reason, heap::Limits};
use crate::{Context, Module};
use std::cell::{Cell, RefCell};

thread_local! {
	static OUTPUT: RefCell<Option<stock::Value>> = const { RefCell::new(None) };
	static CALLS: Cell<usize> = const { Cell::new(0) };
}
fn output() -> stock::Value {
	CALLS.with(|c| c.set(c.get() + 1));
	OUTPUT.with(|v| v.borrow().as_ref().unwrap().clone())
}
fn calls() -> usize {
	CALLS.with(Cell::get)
}
fn set(value: stock::Value) {
	OUTPUT.with(|v| *v.borrow_mut() = Some(value));
}
fn clear() {
	OUTPUT.with(|v| v.borrow_mut().take());
}
fn list(values: impl IntoIterator<Item = stock::Value>) -> stock::Value {
	let mut v = crate::alloc::Vec::new();
	for value in values {
		v.try_push(value).unwrap();
	}
	stock::Value::vec(v).unwrap()
}
fn text(value: &str) -> stock::Value {
	stock::Value::try_from(crate::alloc::String::try_from(value).unwrap()).unwrap()
}
fn context() -> Context {
	let mut module = Module::with_item(["tiny"]).unwrap();
	module.fresh_host_function("dynamic", output).unwrap();
	module
		.fresh_host_function(
			"result",
			|| -> core::result::Result<stock::Value, std::string::String> { Ok(output()) },
		)
		.unwrap();
	module.fresh_host_function("ok", || 7i64).unwrap();
	installed(module)
}
fn balanced(store: &Store, executor: &Executor, baseline: (usize, usize)) {
	assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
	assert!(executor.is_idle());
	assert!(executor.active_host_runtime.is_none());
	assert!(executor.pending_host_cleanup.is_none());
}
fn reusable(context: &Context, executor: &mut Executor, store: &mut Store) {
	let good = code(context, "ok", 0);
	let root = executor.invoke_code(&good, store, 0, &[], 100).unwrap();
	assert_eq!(root.value, Value::I64(7));
	store.root_release(root).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_scalars_empty_tuple_and_float_bits() {
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	for (native, expected) in [
		(stock::Value::unit(), Value::Unit),
		(stock::Value::from(true), Value::Bool(true)),
		(stock::Value::from(i64::MIN), Value::I64(i64::MIN)),
		(stock::Value::from(u64::MAX), Value::U64(u64::MAX)),
		(stock::Value::from('🦀'), Value::Char('🦀')),
		(
			stock::Value::from(core::cmp::Ordering::Less),
			Value::Ordering(core::cmp::Ordering::Less),
		),
	] {
		set(native);
		let root = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap();
		assert_eq!(root.value, expected);
		store.root_release(root).unwrap();
	}
	for bits in [0x8000000000000000, 0x7ff8000000000042, 0xfff800000000002a] {
		set(stock::Value::from(f64::from_bits(bits)));
		let root = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap();
		let Value::F64(value) = root.value else {
			panic!("lost float")
		};
		assert_eq!(value.to_bits(), bits);
		store.root_release(root).unwrap();
	}
	set(stock::Value::tuple(crate::alloc::Vec::<stock::Value>::new()).unwrap());
	let root = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap();
	assert_eq!(store.tag(root.value).unwrap(), Type::Tuple);
	assert_eq!(store.len(root.value).unwrap(), 0);
	store.root_release(root).unwrap();
	clear();
	balanced(&store, &executor, (0, 0));
	reusable(&context, &mut executor, &mut store);
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_dag_identity_and_external_snapshots() {
	let shared = list([stock::Value::from(7i64)]);
	set(list([
		shared.clone(),
		shared.clone(),
		list([stock::Value::from(7i64)]),
	]));
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let root = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap();
	let a = store.get(root.value, 0).unwrap();
	let b = store.get(root.value, 1).unwrap();
	let c = store.get(root.value, 2).unwrap();
	assert_eq!(a, b);
	assert_ne!(a, c);
	store.set(a, 0, Value::I64(42)).unwrap();
	assert_eq!(store.get(b, 0).unwrap(), Value::I64(42));
	assert_eq!(
		shared.borrow_ref::<stock::Vec>().unwrap()[0]
			.as_integer::<i64>()
			.unwrap(),
		7
	);
	shared.borrow_mut::<stock::Vec>().unwrap()[0] = stock::Value::from(99i64);
	assert_eq!(store.get(b, 0).unwrap(), Value::I64(42));
	let second = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap();
	let later = store.get(second.value, 0).unwrap();
	assert_ne!(later, a);
	assert_eq!(store.get(later, 0).unwrap(), Value::I64(99));
	for value in [a, b, c, later] {
		store.release(value).unwrap();
	}
	store.root_release(root).unwrap();
	store.root_release(second).unwrap();
	clear();
	balanced(&store, &executor, (0, 0));
	reusable(&context, &mut executor, &mut store);
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_cycle_access_and_unsupported_fail_once() {
	use crate::TypeHash;
	#[derive(crate::Any)]
	struct Opaque;
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let cycle = list([]);
	cycle
		.borrow_mut::<stock::Vec>()
		.unwrap()
		.push(cycle.clone())
		.unwrap();
	let borrowed = list([]);
	let held = borrowed.clone().into_mut::<stock::Vec>().unwrap();
	for (native, reason, hash) in [
		(cycle.clone(), Reason::Cycle, None),
		(borrowed, Reason::OutputAccess, Some(stock::Vec::HASH)),
		(
			stock::Value::new(Opaque).unwrap(),
			Reason::Unsupported,
			Some(Opaque::HASH),
		),
		(stock::Value::empty(), Reason::Unsupported, None),
	] {
		set(native);
		let before = calls();
		let trace = Executor::host_trace_begin();
		let failure = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap_err();
		let trace = trace.finish();
		assert_eq!(failure.error.kind, ErrorKind::Host);
		assert_eq!(
			failure.error.detail,
			Detail::HostDynamic {
				reason,
				type_hash: hash
			}
		);
		assert_eq!(calls() - before, 1);
		assert!(!trace.contains(&"put"));
		assert!(failure.cleanup_error.is_none());
		balanced(&store, &executor, (0, 0));
		reusable(&context, &mut executor, &mut store);
	}
	clear();
	drop(held);
	cycle.borrow_mut::<stock::Vec>().unwrap().clear();
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_depth_and_scratch_preflight_are_transactional() {
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	for (depth, reason) in [(128, None), (129, Some(Reason::DepthLimit))] {
		let mut value = stock::Value::unit();
		for _ in 0..depth {
			value = list([value]);
		}
		set(value);
		let mut store = Store::new(Limits::default()).unwrap();
		let result = executor.invoke_code(&code, &mut store, 0, &[], 100);
		match reason {
			None => store.root_release(result.unwrap()).unwrap(),
			Some(reason) => assert_eq!(
				result.unwrap_err().error.detail,
				Detail::HostDynamic {
					reason,
					type_hash: None
				}
			),
		}
		clear();
		balanced(&store, &executor, (0, 0));
	}
	// Payload fits, scratch does not. No Store node may be allocated/published.
	set(list([stock::Value::unit()]));
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 32,
	})
	.unwrap();
	let trace = Executor::host_trace_begin();
	let failure = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap_err();
	let trace = trace.finish();
	assert_eq!(
		failure.error.detail,
		Detail::HostDynamic {
			reason: Reason::ScratchLimit,
			type_hash: None
		}
	);
	assert!(!trace.contains(&"dynamic-build"));
	assert!(!trace.contains(&"put"));
	clear();
	balanced(&store, &executor, (0, 0));
	reusable(&context, &mut executor, &mut store);
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_malformed_outer_and_error_are_invalid_code() {
	for which in 0..2 {
		let mut module = Module::with_item(["tiny"]).unwrap();
		module
			.raw_function("bad", move |memory: &mut dyn stock::Memory, _, _, out| {
				if which == 0 {
					memory.store(out, 1i64)?;
				} else {
					memory.store(
						out,
						core::result::Result::<stock::Value, stock::Value>::Err(
							stock::Value::from(1i64),
						),
					)?;
				}
				Ok(())
			})
			.build()
			.unwrap();
		module.fresh_host_function("ok", || 7i64).unwrap();
		let mut context = installed(module);
		context
			.fresh_host_for_test(
				crate::Hash::type_hash(["tiny", "bad"]),
				HostSignature {
					arity: 0,
					arguments: [HostType::Unit; 4],
					output: HostOutput::ResultDynamicString,
				},
			)
			.unwrap();
		let code = code(&context, "bad", 0);
		let mut executor = executor(&context);
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
		assert!(failure.cleanup_error.is_none());
		balanced(&store, &executor, (0, 0));
		reusable(&context, &mut executor, &mut store);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "allocator sweep is native only; focused C10 Miri uses minimal fixtures"
)]
fn dynamic_every_real_budget_balances_and_recovers() {
	use super::fault_tests::Trace;
	let mut planning = 0;
	let mut building = 0;
	let mut midgraph = 0;
	let mut secondary = 0;
	for target in ["dynamic", "result"] {
		// All stock graph allocations precede the guard, so conversion failures
		// are distinguishable from the Result adapter allocation.
		let shared = list([text("host output")]);
		set(list([shared.clone(), shared, text("other")]));
		let context = context();
		let code = code(&context, target, 0);
		let mut failed = std::vec::Vec::new();
		let mut success = None;
		for budget in 0..=65536 {
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = executor(&context);
			let mut roots = std::vec::Vec::new();
			let capacity = store.slot_capacity_for_test();
			for _ in 0..capacity - 1 {
				roots.push(store.root_string("").unwrap());
			}
			assert_eq!(store.slot_capacity_for_test(), capacity);
			let baseline = (store.live_entries(), store.payload_bytes());
			let before = calls();
			let trace = Trace::start();
			let result = crate::alloc::limit::with(budget, || {
				executor
					.invoke_code(&code, &mut store, 0, &[], 100)
					.map_err(std::boxed::Box::new)
			})
			.call();
			let trace = trace.finish();
			let delta = calls() - before;
			assert!(delta <= 1);
			match result {
				Ok(root) => {
					assert_eq!(delta, 1);
					assert!(trace.contains(&"put"));
					assert_eq!(
						store.tag(root.value).unwrap(),
						if target == "dynamic" {
							Type::Vec
						} else {
							Type::Result
						}
					);
					store.root_release(root).unwrap();
					balanced(&store, &executor, baseline);
					for root in roots {
						store.root_release(root).unwrap();
					}
					success = Some(budget);
					println!("C10_BUDGET target={target} B={budget} success trace={trace:?}");
					break;
				}
				Err(failure) => {
					assert!(!trace.contains(&"put"));
					if failure.cleanup_error.is_some() {
						secondary += 1;
					}
					let path = if delta == 0 {
						"pre-call"
					} else if !trace.contains(&"output-start") {
						"adapter-return"
					} else if !trace.contains(&"dynamic-build") {
						planning += 1;
						assert_eq!(failure.error.kind, ErrorKind::Allocation);
						"dynamic-plan"
					} else {
						building += 1;
						assert_eq!(failure.error.kind, ErrorKind::Allocation);
						if trace.iter().filter(|s| **s == "dynamic-node").count() > 1 {
							midgraph += 1;
						}
						"dynamic-build"
					};
					balanced(&store, &executor, baseline);
					println!("C10_BUDGET target={target} bytes={budget} path={path} calls={delta} primary={:?} cleanup={:?} trace={trace:?}",failure.error,failure.cleanup_error);
					failed.push((store, executor, roots, baseline));
				}
			}
		}
		let b = success.expect("STOP: no success at or below 65536 bytes");
		assert_eq!(failed.len(), b);
		for (mut store, mut executor, roots, baseline) in failed {
			let before = calls();
			let root = crate::alloc::limit::with(65536, || {
				executor
					.invoke_code(&code, &mut store, 0, &[], 100)
					.map_err(std::boxed::Box::new)
			})
			.call()
			.unwrap();
			assert_eq!(calls() - before, 1);
			store.root_release(root).unwrap();
			balanced(&store, &executor, baseline);
			reusable(&context, &mut executor, &mut store);
			balanced(&store, &executor, baseline);
			for root in roots {
				store.root_release(root).unwrap();
			}
			balanced(&store, &executor, (0, 0));
		}
		println!("C10_SWEEP target={target} B={b} R=65536 recovery={b}/{b}");
		clear();
	}
	assert!(planning > 0);
	assert!(building > 0);
	assert!(midgraph > 0);
	println!("C10_COVERAGE planning={planning} build={building} midgraph={midgraph} secondary={secondary}");
	// Secondary cleanup is reported, not synthetically injected. For valid
	// acyclic admitted graphs release uses pre-reserved work/free metadata.
	assert_eq!(secondary, 0, "STOP: unexpected real cleanup failure");
}
