//! Real Rune allocator budgets; the scoped trace only observes reached boundaries.
use super::minimal_tests::{code, executor, installed};
use super::*;
use crate::experimental_core::heap::Limits;
use crate::Module;
use std::cell::RefCell;
use std::sync::{
	atomic::{AtomicUsize, Ordering},
	Arc,
};

std::thread_local! {
	static TRACE: RefCell<Option<std::vec::Vec<&'static str>>> = const { RefCell::new(None) };
}
pub(super) fn observe(boundary: &'static str) {
	TRACE.with(|trace| {
		if let Some(trace) = trace.borrow_mut().as_mut() {
			trace.push(boundary);
		}
	});
}
pub(crate) struct Trace;
impl Trace {
	pub(crate) fn start() -> Self {
		TRACE.with(|trace| assert!(trace.replace(Some(std::vec::Vec::new())).is_none()));
		Self
	}
	pub(crate) fn finish(self) -> std::vec::Vec<&'static str> {
		TRACE.with(|trace| trace.borrow_mut().take().unwrap())
	}
}
impl Drop for Trace {
	fn drop(&mut self) {
		TRACE.with(|trace| {
			trace.borrow_mut().take();
		});
	}
}

struct Fixture {
	store: Store,
	executor: Executor,
	roots: std::vec::Vec<Root>,
	calls: Arc<AtomicUsize>,
}
impl Fixture {
	fn new(context: &crate::Context, calls: Arc<AtomicUsize>, parent: bool) -> Self {
		let mut store = Store::new(Limits::default()).unwrap();
		let mut roots = std::vec::Vec::new();
		if parent {
			let capacity = store.slot_capacity_for_test();
			assert!(capacity > 1);
			for _ in 0..capacity - 1 {
				roots.push(store.root_string("").unwrap());
			}
			assert_eq!(store.live_entries(), capacity - 1);
			assert_eq!(store.slot_capacity_for_test(), capacity);
		}
		Self {
			store,
			executor: executor(context),
			roots,
			calls,
		}
	}
	fn balanced(&self, live: usize, bytes: usize) {
		assert_eq!(self.store.live_entries(), live);
		assert_eq!(self.store.payload_bytes(), bytes);
		for root in &self.roots {
			assert_eq!(self.store.root_text(root).unwrap(), "");
		}
		assert!(self.executor.is_idle());
		assert!(self.executor.active_host_runtime.is_none());
		assert!(self.executor.pending_host_cleanup.is_none());
	}
	fn release_roots(&mut self) {
		for root in self.roots.drain(..) {
			self.store.root_release(root).unwrap();
		}
		assert_eq!(self.store.live_entries(), 0);
		assert_eq!(self.store.payload_bytes(), 0);
	}
}
fn check_output(store: &Store, root: &Root, target: &str) {
	let value = if target == "s" {
		root.value
	} else {
		let (tag, flag, child) = store.variant_parts(root.value).unwrap();
		assert_eq!(
			tag,
			if target == "o" {
				Type::Option
			} else {
				Type::Result
			}
		);
		assert_eq!(flag, target != "e");
		child.unwrap()
	};
	assert_eq!(store.text(value).unwrap(), "host output");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "allocator-budget census is native-only; minimal fixtures cover Miri"
)]
fn every_real_allocation_budget_balances_and_recovers() {
	let mut leaf_failures = 0;
	let mut parent_failures = 0;
	let mut secondary = 0;
	for target in ["s", "o", "r", "e"] {
		let calls = Arc::new(AtomicUsize::new(0));
		let captured = calls.clone();
		let mut module = Module::with_item(["tiny"]).unwrap();
		match target {
			"s" => {
				module
					.fresh_host_function(target, move || -> std::string::String {
						captured.fetch_add(1, Ordering::SeqCst);
						"host output".into()
					})
					.unwrap();
			}
			"o" => {
				module
					.fresh_host_function(target, move || -> Option<std::string::String> {
						captured.fetch_add(1, Ordering::SeqCst);
						Some("host output".into())
					})
					.unwrap();
			}
			"r" => {
				module
					.fresh_host_function(
						target,
						move || -> core::result::Result<std::string::String, i64> {
							captured.fetch_add(1, Ordering::SeqCst);
							Ok("host output".into())
						},
					)
					.unwrap();
			}
			"e" => {
				module
					.fresh_host_function(
						target,
						move || -> core::result::Result<i64, std::string::String> {
							captured.fetch_add(1, Ordering::SeqCst);
							Err("host output".into())
						},
					)
					.unwrap();
			}
			_ => unreachable!(),
		}
		let context = installed(module);
		let code = code(&context, target, 0);
		let mut failed = std::vec::Vec::new();
		let mut histogram = std::collections::BTreeMap::new();
		let mut first_success = None;
		for budget in 0..=65536 {
			let mut fixture = Fixture::new(&context, calls.clone(), target != "s");
			let live = fixture.store.live_entries();
			let bytes = fixture.store.payload_bytes();
			let before = calls.load(Ordering::SeqCst);
			let trace = Trace::start();
			let result = crate::alloc::limit::with(budget, || {
				fixture
					.executor
					.invoke_code(&code, &mut fixture.store, 0, &[], 100)
					.map_err(std::boxed::Box::new)
			})
			.call();
			let trace = trace.finish();
			let delta = calls.load(Ordering::SeqCst) - before;
			assert!(
				delta <= 1,
				"target={target} budget={budget}: duplicate call"
			);
			let path = match result {
				Ok(root) => {
					assert_eq!(delta, 1);
					assert!(trace.contains(&"put"));
					check_output(&fixture.store, &root, target);
					fixture.store.root_release(root).unwrap();
					fixture.balanced(live, bytes);
					fixture.release_roots();
					first_success = Some(budget);
					"success"
				}
				Err(failure) => {
					assert!(!trace.contains(&"put"), "{target} {budget}: {trace:?}");
					if trace.contains(&"cleanup-error") {
						secondary += 1;
						assert!(failure.cleanup_error.is_some());
					} else {
						assert!(failure.cleanup_error.is_none());
					}
					let path = if delta == 0 {
						"pre-call"
					} else if !trace.contains(&"output-start") {
						"adapter-return-primary"
					} else if trace.contains(&"parent-start") {
						assert_eq!(failure.error.kind, ErrorKind::Allocation);
						parent_failures += 1;
						"variant-parent-primary"
					} else {
						assert!(trace.contains(&"leaf-start"));
						assert_eq!(failure.error.kind, ErrorKind::Allocation);
						leaf_failures += 1;
						"leaf-primary"
					};
					fixture.balanced(live, bytes);
					println!("BUDGET target={target} bytes={budget} path={path} calls={delta} primary={:?} cleanup={:?} trace={trace:?}", failure.error, failure.cleanup_error);
					failed.push(fixture);
					path
				}
			};
			*histogram.entry(path).or_insert(0usize) += 1;
			if first_success.is_some() {
				break;
			}
		}
		let budget = first_success.expect("STOP: no success at or below 65536 bytes");
		for mut fixture in failed {
			let live = fixture.store.live_entries();
			let bytes = fixture.store.payload_bytes();
			let before = fixture.calls.load(Ordering::SeqCst);
			let result = crate::alloc::limit::with(65536, || {
				fixture
					.executor
					.invoke_code(&code, &mut fixture.store, 0, &[], 100)
					.map_err(std::boxed::Box::new)
			})
			.call()
			.unwrap();
			assert_eq!(fixture.calls.load(Ordering::SeqCst) - before, 1);
			check_output(&fixture.store, &result, target);
			fixture.store.root_release(result).unwrap();
			fixture.balanced(live, bytes);
			fixture.release_roots();
		}
		println!(
			"SWEEP target={target} B={budget} R=65536 histogram={histogram:?} recovery={budget}/{budget}"
		);
	}
	assert!(leaf_failures > 0);
	assert!(parent_failures > 0);
	println!("COVERAGE leaf={leaf_failures} parent={parent_failures} secondary={secondary}");
}
