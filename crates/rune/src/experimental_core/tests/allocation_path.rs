//! Item 9, executor side: on the five early-diagnostic programs, repeated invocation on one
//! executor and store, after warm-up, never grows the executor's bookkeeping buffers and leaves the store at its
//! baseline. Local construction check over the batch A sources; not a measurement.

use crate::experimental_core::{
	execute::Executor,
	frontend,
	heap::{Limits, Store},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};

/// The batch A sources, byte-identical to rnx probes/0184/reference/cases/*.rn.
const KERNELS: [(&str, &str); 5] = [
	(
		"numeric",
		"pub fn main(_a) {\n\tlet n = 0;\n\tlet total = 0;\n\twhile n < 10000 { total += n % 17; n += 1; }\n\tprintln!(\"{}\", total);\n}\n",
	),
	(
		"while",
		"pub fn main(_a) {\n\tlet n = 0;\n\twhile n < 10000 { n += 1; }\n\tprintln!(\"{}\", n);\n}\n",
	),
	(
		"calls",
		"fn add(a, b) { a + b }\npub fn main(_a) {\n\tlet total = 0;\n\tfor i in 0..10000 { total = add(total, i); }\n\tprintln!(\"{}\", total);\n}\n",
	),
	(
		"fib",
		"fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\npub fn main(_a) { println!(\"{}\", fib(20)); }\n",
	),
	(
		"shared_rows",
		"fn add(rows, key, value) {\n\tfor row in rows {\n\t\tif row[0] == key {\n\t\t\trow[1] = row[1] + value;\n\t\t\treturn;\n\t\t}\n\t}\n\trows.push([key, value]);\n}\n\npub fn main(_a) {\n\tlet rows = [];\n\tadd(rows, \"a\", 2);\n\tadd(rows, \"b\", 3);\n\tadd(rows, \"a\", 5);\n\tprintln!(\"rows {} {}\", rows[0][1], rows[1][1]);\n\tlet copy = rows.clone();\n\tlet alias = copy[0];\n\talias[1] = alias[1] + 1;\n\tcopy.push([\"c\", 9]);\n\tprintln!(\"nested {} {} {}\", rows[0][1], rows.len(), copy.len());\n}\n",
	),
];

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn warmed_invocation_never_grows_executor_buffers_or_leaves_store_state() {
	let context = Context::with_default_modules().unwrap();
	for (name, text) in KERNELS {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		// Warm actual high-water bookkeeping before recording its capacities.
		let unit = store.root_unit();
		let out = executor
			.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
			.unwrap();
		store.root_release(out).unwrap();
		let capacities = executor.capacities_for_test();
		let baseline = (
			store.live_entries(),
			store.payload_bytes(),
			store.reserved_metadata_bytes(),
		);
		let unit = store.root_unit();

		for k in 0..3 {
			let out = executor
				.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
				.unwrap();
			store.root_release(out).unwrap();
			assert!(executor.is_idle(), "{name}: idle after invocation {k}");
			assert_eq!(
				executor.capacities_for_test(),
				capacities,
				"{name}: executor buffers grew at invocation {k}"
			);
			assert_eq!(
				(
					store.live_entries(),
					store.payload_bytes(),
					store.reserved_metadata_bytes()
				),
				baseline,
				"{name}: store not at baseline after invocation {k}"
			);
		}
	}
}

use crate::experimental_core::error::ErrorKind;
use rune_alloc::limit;

fn compiled(text: &str) -> (frontend::Program, u32) {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	(program, main)
}

/// Executor construction reaches each of its three reservations (frames, slots, call-chain
/// diagnostics, in that order): one byte short of each cumulative boundary is refused with a
/// named error and nothing published; the full size succeeds.
#[test]
fn executor_construction_refuses_at_each_reservation() {
	let [frames, slots, diagnostics] = Executor::reservation_bytes_for_test();
	let total = frames + slots + diagnostics;

	for (site, budget) in [
		("frames", frames - 1),
		("slots", frames + slots - 1),
		("diagnostics", total - 1),
	] {
		let refused = limit::with(budget, || Executor::new().err().map(|e| e.kind)).call();
		assert_eq!(
			refused,
			Some(ErrorKind::Allocation),
			"{site}: budget {budget}"
		);
	}

	let built = limit::with(total, || Executor::new().is_ok()).call();
	assert!(built, "construction fits in exactly {total} bytes");
}

/// After a failed invocation moved the call-chain buffer out, the next invocation's
/// re-reservation can fail: a named allocation failure before any frame, slot or retain,
/// and the executor stays reusable.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn a_refused_diagnostics_reservation_leaves_the_executor_reusable() {
	let (failing, failing_main) = compiled("fn f(x) { 1 / x } pub fn main(_a) { f(0) }");
	let (ok, ok_main) = compiled("pub fn main(_a) { 7 }");
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let unit = store.root_unit();
	let failure = executor
		.invoke(&failing, &mut store, failing_main, &[&unit], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
	assert!(
		!failure.call_chain.is_empty(),
		"the buffer moved out with the failure"
	);
	let refused = limit::with(0, || {
		executor
			.invoke(&ok, &mut store, ok_main, &[&unit], 1000)
			.err()
	})
	.call()
	.unwrap();
	assert!(
		refused.cleanup_error.is_none(),
		"no secondary cleanup error"
	);
	assert_eq!(refused.error.kind, ErrorKind::Allocation);
	assert!(refused.call_chain.is_empty());
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	let out = executor
		.invoke(&ok, &mut store, ok_main, &[&unit], 1000)
		.unwrap();
	assert_eq!(
		store.root_scalar(&out).unwrap(),
		Some(crate::experimental_core::value::Scalar::I64(7))
	);
	store.root_release(out).unwrap();
}

/// Sampled allocation-budget sweep (not exhaustive site injection, not a peak proof) over each of the five programs on a prepared executor and store:
/// at every budget the invocation either succeeds or fails with a named error, and in both
/// cases the executor is idle, the store is back at its baseline (no partial publication,
/// lost charge or leaked root), and an unlimited invocation then succeeds. This exercises
/// the executor's own sites (list scratch, frame and slot use) and, as a by-product, the
/// store and formatter sites on the path; it is not a peak measurement. Before each limited
/// invocation an unlimited one restores the call-chain buffer that a previous failure moved
/// out; otherwise every later budget below its 24 KiB would stop at that re-reservation and
/// the rest of the path would never be reached (found while writing this test).
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn sampled_allocation_budgets_leave_clean_reusable_state_on_the_five_programs() {
	for (name, text) in KERNELS {
		let (program, main) = compiled(text);
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let unit = store.root_unit();
		let baseline = (
			store.live_entries(),
			store.payload_bytes(),
			store.reserved_metadata_bytes(),
		);
		let mut failures = 0;
		let mut budget = 0usize;
		let mut reached = std::collections::BTreeSet::new();

		loop {
			let warm = executor
				.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
				.unwrap();
			store.root_release(warm).unwrap();
			let outcome = limit::with(budget, || {
				executor
					.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
					.map(|out| store.root_release(out).is_ok())
					.map_err(|f| {
						assert!(
							f.cleanup_error.is_none(),
							"secondary cleanup error {:?}",
							f.cleanup_error
						);
						(f.error.kind, f.error.location.map(|l| l.start))
					})
			})
			.call();
			assert!(executor.is_idle(), "{name}: not idle at budget {budget}");
			assert_eq!(
				(
					store.live_entries(),
					store.payload_bytes(),
					store.reserved_metadata_bytes()
				),
				baseline,
				"{name}: store not at baseline at budget {budget}"
			);

			match outcome {
				Ok(released) => {
					assert!(released, "{name}: result release at budget {budget}");
					break;
				}

				Err((kind, at)) => {
					reached.insert(at);
					assert!(
						matches!(
							kind,
							ErrorKind::Allocation
								| ErrorKind::StoreCapacity
								| ErrorKind::FormatLimit
						),
						"{name}: unexpected {kind:?} at budget {budget}"
					);
					failures += 1;
				}
			}

			budget = if budget < 256 { budget + 8 } else { budget * 2 };
			assert!(budget < 1 << 30, "{name}: never succeeded");
		}

		let out = executor
			.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
			.unwrap();
		store.root_release(out).unwrap();
		assert_eq!(store.live_entries(), 0, "{name}");

		// Not vacuous: small budgets refuse, and at more than one source location.
		assert!(failures >= 1, "{name}: no budget refused");

		if name == "shared_rows" {
			assert!(
				reached.len() >= 2,
				"shared_rows: refusals reached only {reached:?}"
			);
		}

		std::eprintln!(
			"{name}: {failures} refused budgets before success; refusal sites {reached:?}"
		);
	}
}

/// Codex's store/scratch bound, not a process RSS measurement: summing every
/// requested payload and runtime scratch allocation in this fixed invocation
/// gives 455 bytes, including buffers that never coexist. Prepared metadata,
/// code/frontend allocations and stdout's global buffer are separate.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
// `Failure`, the public error type of `invoke`, is large by design.
#[allow(clippy::result_large_err)]
fn shared_rows_fits_the_conservative_payload_and_scratch_formula() {
	let text = KERNELS
		.iter()
		.find(|(name, _)| *name == "shared_rows")
		.unwrap()
		.1;
	let (program, main) = compiled(text);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let unit = store.root_unit();
	let baseline = (
		store.live_entries(),
		store.payload_bytes(),
		store.reserved_metadata_bytes(),
	);
	let strings = 4 + 15; // Four one-byte keys; the two templates' literal parts.
	let formatted = 8 + 12; // "rows 7 3" and "nested 8 2 3".
	let inner_lists = 3 * 2 * 16;
	let outer_buffers = (1 + 2 + 2 + 4) * 16; // Growth, clone, then clone growth.
	let scratch = (3 * 2 + 2) * 16; // List literals and clone_outer's first copy.
	let iterators = 3 * 16;
	let bound = strings + formatted + inner_lists + outer_buffers + scratch + iterators;
	assert_eq!(bound, 455);
	let out = limit::with(bound, || {
		executor.invoke(&program, &mut store, main, &[&unit], 1_000_000_000)
	})
	.call()
	.unwrap();
	store.root_release(out).unwrap();
	assert!(executor.is_idle());
	assert_eq!(
		(
			store.live_entries(),
			store.payload_bytes(),
			store.reserved_metadata_bytes()
		),
		baseline
	);
}

/// D1 resource inspection on the unchanged five programs; no performance claim.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn decoded_cache_footprints_are_exact_for_each_kernel() {
	let context = Context::with_default_modules().unwrap();
	for (name, text) in KERNELS {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
		let mut bytes = 0usize;

		for id in 0..program.code.functions() {
			let function = program.code.function(id as u32).unwrap();
			let (len, capacity, allocation) = function.cache_layout().unwrap();
			assert_eq!(len, function.instructions().len());
			assert_eq!(capacity, len);
			assert_eq!(
				allocation,
				len * crate::experimental_core::code::Code::cache_type_layout().0
			);
			bytes += allocation;
			std::println!(
				"D1 {name} function {id}: len={len} capacity={capacity} requested_bytes={allocation}"
			);
		}

		let footprint = program.code.footprint().unwrap();
		assert_eq!(footprint.cache_payload, bytes);
		assert_eq!(
			footprint.resident,
			footprint.original_buffers + footprint.function_records + bytes
		);
		std::println!(
			"D1 {name}: layout={:?} footprint={footprint:?}",
			crate::experimental_core::code::Code::cache_type_layout()
		);
	}
}
