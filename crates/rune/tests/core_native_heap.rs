//! P3a heap measurement of the native tier (feature `native-code`, x86-64 Linux).
//!
//! A DEDICATED test binary (`harness = false`): `stats_alloc` (MIT, dev-only) is the global
//! allocator of THIS binary only, never of the library or of its other tests. It uses the
//! public execution API only, as a host would, so the production tier is the one measured.
//!
//! The parent re-runs this binary twice as a child: with the tier ON, and with it OFF
//! (`PERF_BUILDID_DIR` set in the child's environment, which makes the tier refuse every
//! load; the parent's own environment is not changed). Each child runs on one thread with
//! serialized, controlled allocations, after a warm-up, and prints its counters.
//!
//! What is measured: LIVE REQUESTED BYTES, `bytes_allocated - bytes_deallocated` of the
//! global allocator API (reallocation growth counts as allocated, shrinkage as
//! deallocated). That is NOT the malloc footprint (no allocator headers, rounding or
//! fragmentation) and NOT RSS. A per-region figure from this one configuration is an
//! estimate for that configuration, not a bound for other modules, signatures, ISAs or
//! compiler versions, and no hard memory guarantee is derived from it.
// Not under Miri: it measures the native tier, which Miri refuses, from child processes.
#[cfg(all(
	feature = "native-code",
	target_arch = "x86_64",
	target_os = "linux",
	not(miri)
))]
mod heap {
	use rune::experimental_core::execute::Executor;
	use rune::experimental_core::frontend::{self, Program};
	use rune::experimental_core::heap::{Limits, Store};
	use rune::{Context, Diagnostics, Hash, Source, Sources};
	use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};
	use std::alloc::System;
	use std::process::{Command, Stdio};

	#[global_allocator]
	static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

	const CHILD: &str = "RUNE_P3A_HEAP_CHILD";

	/// Regions of the measured program (64 loop functions, one region each).
	const REGIONS: i64 = 64;

	/// `MODULE_RESERVE_BYTES` of `experimental_core::region::cache` (crate-private), the
	/// estimate this measurement checks for this one configuration.
	const MODULE_RESERVE_BYTES: i64 = 4096;

	/// `ISA_RESERVE_BYTES` of the same module.
	const ISA_RESERVE_BYTES: i64 = 64 << 10;

	/// Our own retained metadata per region of this program shape, by capacity: the P3a
	/// unit tests' summary of the same 64-loop program (`metadata_bytes = 59136` for 64
	/// regions, `a2_region_cap_streaming`).
	const OUR_METADATA_PER_REGION: i64 = 59_136 / 64;

	fn live() -> i64 {
		let s = GLOBAL.stats();
		s.bytes_allocated as i64 - s.bytes_deallocated as i64
	}

	fn compile(context: &Context, text: &str) -> Program {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1).unwrap()
	}

	fn loops(count: usize, tag: &str) -> String {
		let mut text = String::new();
		for k in 0..count {
			text.push_str(&format!(
				"fn {tag}{k}(a) {{ let n = 0; while n < a {{ n += 1; }} n }}\n"
			));
		}
		text.push_str(&format!("pub fn main(a) {{ {tag}0(a) }}"));
		text
	}

	/// One function with one large loop region (120 additions over 30 accumulators).
	fn big() -> String {
		let mut text = String::from("pub fn main(a) { let i = 0;");
		for k in 0..30 {
			text.push_str(&format!(" let t{k} = 0;"));
		}
		text.push_str(" while i < a {");
		for _ in 0..4 {
			for k in 0..30 {
				text.push_str(&format!(" t{k} += i;"));
			}
		}
		text.push_str(" i += 1; } t0 }");
		text
	}

	/// One invocation of `main(x)`; every root is released.
	fn run(program: &Program, store: &mut Store, executor: &mut Executor, x: i64) {
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let arg = store.root_i64(x);
		let out = executor
			.invoke(program, store, main, &[&arg], 1_000_000)
			.unwrap();
		store.root_release(out).unwrap();
		store.root_release(arg).unwrap();
	}

	fn child(mode: &str) {
		let context = Context::with_default_modules().unwrap();
		let warm = compile(&context, &loops(1, "w"));
		let program = compile(&context, &loops(REGIONS as usize, "f"));
		let large = compile(&context, &big());
		let cycle: Vec<Program> = (0..17)
			.map(|k| compile(&context, &loops(1, &format!("c{k}_"))))
			.collect();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();

		// Warm-up: the first build on this thread creates the thread-local cache, the shared
		// ISA (tier on), the process page-size query and the executor's buffers.
		run(&warm, &mut store, &mut executor, 3);
		let w0 = live();
		run(&warm, &mut store, &mut executor, 3);
		println!("P3AHEAP\t{mode}\twarm_rerun_delta={}", live() - w0);

		// A second warm program on a FRESH thread measures the first build's thread costs
		// (cache table, shared ISA) separately from the main thread's warm-up.
		let t0 = live();
		let first_build = std::thread::scope(|scope| {
			scope
				.spawn(|| {
					let mut store = Store::new(Limits::default()).unwrap();
					let mut executor = Executor::new().unwrap();
					let before = live();
					run(&warm, &mut store, &mut executor, 3);
					let after = live();
					after - before
				})
				.join()
				.unwrap()
		});
		println!("P3AHEAP\t{mode}\tfirst_build_on_fresh_thread_retained={first_build}\tthread_exit_returned_to_baseline_delta={}", live() - t0);

		// The 64-region program: retained after its build (compile transients freed).
		let l0 = live();
		let region = Region::new(GLOBAL);
		run(&program, &mut store, &mut executor, 5);
		let stats = region.change();
		let l1 = live();
		println!(
			"P3AHEAP\t{mode}\tprogram64_retained={}\tprogram64_bytes_allocated_during={}\tallocations={}",
			l1 - l0,
			stats.bytes_allocated,
			stats.allocations
		);

		// One large region: cumulative bytes allocated while it is built and compiled (an
		// upper bound of the live-bytes increase during that interval, since live bytes only
		// grow through allocation; descriptive, not a gate).
		let l2 = live();
		let region = Region::new(GLOBAL);
		run(&large, &mut store, &mut executor, 2);
		let stats = region.change();
		println!(
			"P3AHEAP\t{mode}\tlarge_region_bytes_allocated_during={}\tretained={}",
			stats.bytes_allocated,
			live() - l2
		);

		// Drift over 1,000 invocations cycling 17 one-region programs through the 16-entry
		// cache: every invocation evicts the least recently used program and rebuilds.
		let mut at100 = 0;
		for k in 0..1_000 {
			run(&cycle[k % 17], &mut store, &mut executor, 2);
			if k + 1 == 100 {
				at100 = live();
			}
		}
		println!("P3AHEAP\t{mode}\tdrift_cycle100_to_1000={}", live() - at100);
	}

	fn field(text: &str, mode: &str, key: &str) -> i64 {
		text.lines()
			.filter(|l| l.starts_with(&format!("P3AHEAP\t{mode}\t")))
			.flat_map(|l| l.split('\t'))
			.find_map(|f| f.strip_prefix(&format!("{key}=")))
			.unwrap_or_else(|| panic!("{mode} {key}"))
			.parse()
			.unwrap()
	}

	fn parent() {
		let exe = std::env::current_exe().unwrap();
		let mut out = String::new();
		for mode in ["on", "off"] {
			let mut command = Command::new(&exe);
			command.env(CHILD, mode).stdin(Stdio::null());
			if mode == "off" {
				command.env("PERF_BUILDID_DIR", "/nonexistent-p3a-heap");
			} else {
				command.env_remove("PERF_BUILDID_DIR");
			}
			let output = command.output().unwrap();
			let text = String::from_utf8(output.stdout).unwrap();
			print!("{text}");
			eprint!("{}", String::from_utf8_lossy(&output.stderr));
			assert!(output.status.success(), "{mode} child: {:?}", output.status);
			out.push_str(&text);
		}

		let on = field(&out, "on", "program64_retained");
		let off = field(&out, "off", "program64_retained");
		let per_region = (on - off) / REGIONS;
		let module = per_region - OUR_METADATA_PER_REGION;
		let first_on = field(&out, "on", "first_build_on_fresh_thread_retained");
		let first_off = field(&out, "off", "first_build_on_fresh_thread_retained");
		// The first build on a thread: one region, plus the thread's cache and shared ISA.
		let thread_first = first_on - first_off - per_region;

		println!("P3AHEAPSUM\tlabel=live requested bytes (allocator API sizes; not malloc footprint, not RSS)");
		println!("P3AHEAPSUM\tretained_per_loaded_region={per_region}\tof_which_our_metadata={OUR_METADATA_PER_REGION}\tmodule_heap_estimate={module}\tMODULE_RESERVE_BYTES={MODULE_RESERVE_BYTES}");
		println!("P3AHEAPSUM\tfirst_build_thread_costs_estimate(isa+cache+lazies)={thread_first}\tISA_RESERVE_BYTES={ISA_RESERVE_BYTES}");
		for mode in ["on", "off"] {
			println!(
				"P3AHEAPSUM\t{mode}\tthread_exit_delta={}\tdrift_cycle100_to_1000={}\tlarge_region_bytes_allocated_during={}",
				field(&out, mode, "thread_exit_returned_to_baseline_delta"),
				field(&out, mode, "drift_cycle100_to_1000"),
				field(&out, mode, "large_region_bytes_allocated_during")
			);
		}

		// Checks of THIS configuration only (not universal bounds).
		assert!(
			module > 0 && module <= MODULE_RESERVE_BYTES,
			"module estimate {module}"
		);
		assert!(
			thread_first <= ISA_RESERVE_BYTES,
			"thread first-build estimate {thread_first}"
		);
		for mode in ["on", "off"] {
			assert_eq!(
				field(&out, mode, "thread_exit_returned_to_baseline_delta"),
				0,
				"{mode}: thread exit frees everything"
			);
			assert_eq!(
				field(&out, mode, "drift_cycle100_to_1000"),
				0,
				"{mode}: no drift"
			);
		}
		println!("P3AHEAPSUM\tok");
	}

	pub(super) fn main() {
		match std::env::var(CHILD) {
			Ok(mode) => child(&mode),
			Err(_) => parent(),
		}
	}
}

fn main() {
	#[cfg(all(
		feature = "native-code",
		target_arch = "x86_64",
		target_os = "linux",
		not(miri)
	))]
	heap::main();
}
