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
	OUTPUT.with(|v| v.borrow_mut().take().unwrap())
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
	module.function("dynamic", output).build().unwrap();
	module
		.function(
			"result",
			|| -> core::result::Result<stock::Value, std::string::String> { Ok(output()) },
		)
		.build()
		.unwrap();
	module.fresh_host_function("ok", || 7i64).unwrap();
	let mut context = installed(module);
	for (name, output) in [
		("dynamic", HostOutput::Dynamic),
		("result", HostOutput::ResultDynamicString),
	] {
		context
			.fresh_host_for_test(
				crate::Hash::type_hash(["tiny", name]),
				HostSignature {
					arity: 0,
					arguments: [HostType::Unit; 4],
					output,
				},
			)
			.unwrap();
	}
	context
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
fn dynamic_dag_identity_and_external_alias_contract() {
	let shared = list([stock::Value::from(7i64)]);
	set(list([
		shared.clone(),
		shared.clone(),
		list([stock::Value::from(7i64)]),
	]));
	drop(shared);
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
	for value in [a, b, c] {
		store.release(value).unwrap();
	}
	store.root_release(root).unwrap();
	let external = list([stock::Value::from(7i64)]);
	set(list([external.clone()]));
	let failure = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Host);
	assert!(matches!(
		failure.error.detail,
		Detail::HostDynamic {
			reason: Reason::ExternalAlias,
			..
		}
	));
	assert_eq!(
		external.borrow_ref::<stock::Vec>().unwrap()[0]
			.as_integer::<i64>()
			.unwrap(),
		7
	);
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
#[cfg_attr(miri, ignore = "focused C18b3b Miri: exact-name operator only")]
fn dynamic_native_collections_are_rejected_before_publication() {
	use crate::modules::collections::{HashMap, HashSet};
	use crate::runtime::Bytes;
	use crate::TypeHash;

	let context = context();
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let retained = store.root_string("retained").unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());
	for (native, hash) in [
		(stock::Value::new(Bytes::new()).unwrap(), Bytes::HASH),
		(
			stock::Value::new(HashMap::with_capacity(0).unwrap()).unwrap(),
			HashMap::HASH,
		),
		(
			stock::Value::new(HashSet::with_capacity(0).unwrap()).unwrap(),
			HashSet::HASH,
		),
	] {
		for target in ["dynamic", "result"] {
			let code = code(&context, target, 0);
			for nested in [false, true] {
				// A supported sibling is planned before the unsupported leaf.
				// Neither it nor the enclosing Vec/Result may reach the Store.
				set(if nested {
					list([list([text("supported sibling")]), native.clone()])
				} else {
					native.clone()
				});
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
						reason: Reason::Unsupported,
						type_hash: Some(hash),
					}
				);
				assert_eq!(calls() - before, 1);
				assert!(!trace.contains(&"dynamic-build"));
				assert!(!trace.contains(&"put"));
				assert!(failure.cleanup_error.is_none());
				clear();
				balanced(&store, &executor, baseline);
				assert_eq!(store.tag(retained.value).unwrap(), Type::String);
				assert_eq!(store.len(retained.value).unwrap(), 8);
				reusable(&context, &mut executor, &mut store);
				balanced(&store, &executor, baseline);
			}
		}
	}
	store.root_release(retained).unwrap();
	balanced(&store, &executor, (0, 0));
}

#[test]
#[cfg_attr(miri, ignore = "focused C10 Miri: exact-name operator only")]
fn dynamic_depth_and_scratch_are_memory_bound() {
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	for depth in [128, 129, 256] {
		let mut value = stock::Value::unit();
		for _ in 0..depth {
			value = list([value]);
		}
		set(value);
		let mut store = Store::new(Limits::default()).unwrap();
		let root = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap();
		store.root_release(root).unwrap();
		clear();
		balanced(&store, &executor, (0, 0));
	}
	// Scratch no longer consumes the payload budget. This payload fits 32 bytes.
	set(list([stock::Value::unit()]));
	let mut store = Store::new(Limits {
		entries: 8,
		bytes: 32,
	})
	.unwrap();
	let root = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap();
	store.root_release(root).unwrap();
	clear();
	balanced(&store, &executor, (0, 0));
	reusable(&context, &mut executor, &mut store);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "large graph capacity witness; small graph importer defenses run under Miri"
)]
fn dynamic_visit_and_scratch_caps_do_not_refuse_valid_outputs() {
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	set(list((0..300_000).map(|_| stock::Value::unit())));
	let root = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap();
	assert_eq!(store.len(root.value).unwrap(), 300_000);
	store.root_release(root).unwrap();
	clear();
	balanced(&store, &executor, (0, 0));
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
		let prepare = || {
			let shared = list([text("host output")]);
			set(list([shared.clone(), shared, text("other")]));
		};
		let context = context();
		let code = code(&context, target, 0);
		let mut failed = std::vec::Vec::new();
		let mut success = None;
		for budget in 0..=65536 {
			prepare();
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
			prepare();
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

/// The audit's reference-count equation applies to every represented stock kind.
#[test]
#[cfg_attr(miri, ignore = "focused S7b1 Miri: exact-name operator only")]
fn dynamic_all_noninline_kinds_are_owned_any_roots_trees_and_dags() {
	fn sample(kind: usize) -> stock::Value {
		match kind {
			0 => text(""),
			1 => text("same"),
			2 => list([]),
			3 => list([stock::Value::from(7i64)]),
			4 => stock::Value::tuple(crate::alloc::Vec::<stock::Value>::new()).unwrap(),
			5 => {
				let mut fields = crate::alloc::Vec::new();
				fields.try_push(stock::Value::from(7i64)).unwrap();
				stock::Value::tuple(fields).unwrap()
			}
			6 | 7 => {
				let mut object = stock::Object::new();
				if kind == 7 {
					object
						.insert(
							crate::alloc::String::try_from("n").unwrap(),
							stock::Value::from(7i64),
						)
						.unwrap();
				}
				stock::Value::new(object).unwrap()
			}
			8 => stock::Value::new(None::<stock::Value>).unwrap(),
			9 => stock::Value::new(Some(stock::Value::from(7i64))).unwrap(),
			10 => stock::Value::new(Ok::<stock::Value, stock::Value>(stock::Value::from(7i64)))
				.unwrap(),
			11 => stock::Value::new(Err::<stock::Value, stock::Value>(stock::Value::from(7i64)))
				.unwrap(),
			_ => unreachable!(),
		}
	}
	let context = context();
	let code = code(&context, "dynamic", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	for kind in 0..12 {
		for placement in 0..3 {
			let child = sample(kind);
			let stock::Repr::Any(any) = child.as_ref() else {
				panic!("kind {kind} is not Any");
			};
			assert_eq!(any.owned_ref_count(), Some(1), "kind {kind} root");
			let value = match placement {
				0 => child,
				1 => list([child]),
				_ => {
					let copy = child.clone();
					let stock::Repr::Any(any) = child.as_ref() else {
						unreachable!()
					};
					assert_eq!(any.owned_ref_count(), Some(2), "kind {kind} DAG");
					list([child, copy])
				}
			};
			set(value);
			let root = executor
				.invoke_code(&code, &mut store, 0, &[], 100)
				.unwrap();
			if placement == 2 {
				let a = store.get(root.value, 0).unwrap();
				let b = store.get(root.value, 1).unwrap();
				assert_eq!(a, b, "kind {kind} shared child");
				store.release(a).unwrap();
				store.release(b).unwrap();
			}
			store.root_release(root).unwrap();
			clear();
			balanced(&store, &executor, (0, 0));
		}
	}
}
