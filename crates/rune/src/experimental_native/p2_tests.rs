//! P2a: the PRODUCTION native path (thread-local region cache and executor wiring),
//! test-only checks. No simulator runs here: the production tier calls the loaded code
//! directly, and each result is compared with the plain interpreter.
//!
//! Every test that loads or executes generated code runs in a child process (this test
//! binary re-invoked with `--exact` on an `#[ignore]`d worker that refuses to run unless
//! `RUNE_P2_NATIVE_CHILD` is set), with stdin null, `PERF_BUILDID_DIR` removed unless the
//! test sets it, and a deadline. A signal or a timeout is a failure.
//!
//! Comparisons are of the whole observation of an invocation: value or full error (kind,
//! location, operation, refusal), call chain, cleanup error, executor idle state, live store
//! entries, the store's whole state and the remaining fuel, at every fuel value from 0 to
//! the case's total charge + 1.
//!
//! Output lines prefixed `P2TABLE`, `P2LIMIT`, `P2MAP` and `P2RSS` are machine-readable
//! report rows.
use super::exec_tests::{
	anon_exec_kib, bin, branch, build, compile_source, corpus_cases, ints, invoke, jump, lit, loc,
	new_cases, read_maps, refusal_cases, rune, status_kib, store, Arg, Case, Observed, Outcome,
	BIG_FUEL, MAX_SWEEP,
};
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Code, Function, Op},
	error::ErrorKind,
	execute::{cancel_at_charge_for_test, Executor},
	frontend,
	region::{
		cache::{self, Checkout, NativeSummary, RegionCounters, State},
		hook::{self, Mode},
	},
	value::{Root, Value},
};
use crate::Hash;
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_P2_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(600);

// ------------------------------------------------------------------ process isolation

fn run_child_with(inner: &str, env: &[(&str, &std::ffi::OsStr)]) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::p2_tests::{inner}");
	let started = Instant::now();
	let mut command = Command::new(exe);
	command
		.args([
			"--exact",
			name.as_str(),
			"--ignored",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD_ENV, "1")
		.env_remove("PERF_BUILDID_DIR")
		.stdin(Stdio::null());
	for (key, value) in env {
		command.env(key, value);
	}
	let mut child = command.spawn().expect("spawn child");

	let status = loop {
		if let Some(status) = child.try_wait().expect("wait child") {
			break status;
		}
		if started.elapsed() > DEADLINE {
			let _ = child.kill();
			let status = child.wait().expect("reap child");
			std::panic!(
				"{inner}: TIMEOUT after {:?}, killed (status {status:?}, signal {:?})",
				DEADLINE,
				status.signal()
			);
		}
		std::thread::sleep(Duration::from_millis(20));
	};

	println!(
		"CHILD\t{inner}\tstatus={status:?}\tsignal={:?}\twall_ms={}",
		status.signal(),
		started.elapsed().as_millis()
	);
	assert_eq!(status.signal(), None, "{inner}: child killed by a signal");
	assert!(status.success(), "{inner}: child failed ({status:?})");
}

fn run_child(inner: &str) {
	run_child_with(inner, &[]);
}

fn require_child() {
	assert!(
		std::env::var_os(CHILD_ENV).is_some(),
		"P2 native worker tests run only in the child process started by their parent test"
	);
}

/// A new thread state: no P1 install, no cached regions, production on.
fn execution() {
	hook::uninstall();
	cache::clear_for_test();
	cache::enable_for_test(true);
}

fn plain(code: &Code, main: u32, args: &[Arg], fuel: usize) -> Observed {
	cache::enable_for_test(false);
	let observed = invoke(code, main, args, fuel);
	cache::enable_for_test(true);
	observed
}

fn native(code: &Code, main: u32, args: &[Arg], fuel: usize) -> Observed {
	cache::enable_for_test(true);
	invoke(code, main, args, fuel)
}

fn counters(code: &Code) -> Vec<RegionCounters> {
	cache::regions_for_test(code.id()).expect("resident entry")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Totals {
	pub(super) entries: u64,
	pub(super) exits: [u64; 3],
	pub(super) cancel: u64,
	pub(super) store: u64,
	pub(super) tag: u64,
}

fn totals(code: &Code) -> Totals {
	let mut t = Totals::default();
	for r in cache::regions_for_test(code.id()).unwrap_or_default() {
		t.entries += r.entries;
		for k in 0..3 {
			t.exits[k] += r.exits[k];
		}
		t.cancel += r.refused_cancel;
		t.store += r.refused_store;
		t.tag += r.refused_tag;
	}
	t
}

fn minus(a: Totals, b: Totals) -> Totals {
	Totals {
		entries: a.entries - b.entries,
		exits: [
			a.exits[0] - b.exits[0],
			a.exits[1] - b.exits[1],
			a.exits[2] - b.exits[2],
		],
		cancel: a.cancel - b.cancel,
		store: a.store - b.store,
		tag: a.tag - b.tag,
	}
}

/// Every refusal kind other than the three division kinds, which cases set explicitly.
fn unexpected_refusals(s: &NativeSummary) -> [u32; 12] {
	[
		s.size,
		s.codegen,
		s.panicked,
		s.audit_other,
		s.other,
		s.load_refused,
		s.metadata,
		s.region_cap,
		s.compile_cap,
		s.thread_bound,
		s.environment,
		s.analysis_errors,
	]
}

fn total_charge(code: &Code, main: u32, args: &[Arg], name: &str) -> usize {
	let reference = plain(code, main, args, BIG_FUEL);
	if let Outcome::Failure { error, .. } = &reference.outcome {
		assert_ne!(error.kind, ErrorKind::FuelExhausted, "{name}: {args:?}");
	}
	let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
	assert!(
		total < MAX_SWEEP,
		"{name}: {args:?} charges {total}: reduce the case"
	);
	total
}

/// The production path against the plain interpreter at every fuel value of every input.
/// Returns (points, production totals over fuel >= 1, all totals).
pub(super) fn production_case(case: &Case) -> (usize, Totals, Totals) {
	let code = case.src.code();
	let main = case.src.main();
	execution();

	let summary = cache::prepare(code).expect("summary");
	let refused = [
		summary.division_runtime as usize,
		summary.division_const0 as usize,
		summary.division_constneg1 as usize,
	];
	assert_eq!(
		summary.loaded as usize, case.loaded,
		"{}: loaded {summary:?}",
		case.name
	);
	assert_eq!(refused, case.refused, "{}: refusals {summary:?}", case.name);
	assert_eq!(
		unexpected_refusals(&summary),
		[0; 12],
		"{}: {summary:?}",
		case.name
	);

	let mut points = 0usize;
	let mut covered = Totals::default();
	let start = totals(code);
	for args in &case.inputs {
		let total = total_charge(code, main, args, &case.name);
		for fuel in 0..=total + 1 {
			let before = totals(code);
			let p = plain(code, main, args, fuel);
			let n = native(code, main, args, fuel);
			assert_eq!(p, n, "{}: production, fuel {fuel}, {args:?}", case.name);
			assert!(p.idle && p.live_entries == 0, "{}", case.name);
			if fuel >= 1 {
				let d = minus(totals(code), before);
				covered.entries += d.entries;
				for k in 0..3 {
					covered.exits[k] += d.exits[k];
				}
				covered.cancel += d.cancel;
				covered.store += d.store;
				covered.tag += d.tag;
			}
			points += 1;
		}
	}
	let all = minus(totals(code), start);
	let refusals = all.cancel + all.store + all.tag;
	let states: Vec<String> = counters(code)
		.iter()
		.map(|r| format!("f{}h{}:{:?}", r.function, r.header, r.state))
		.collect();

	println!(
		"P2TABLE\t{}\tcandidates={}\tregions={}\tloaded={}\trefused[runtime,const0,constneg1]={:?}\tinputs={}\tpoints={}\tentries={}\tcontinue={}\tfuel_exits={}\tretry={}\tentry_refused={}(tag {} store {} cancel {})\tcode_bytes={}\tmapped_bytes={}\tmetadata_bytes={}\tstates={}\ttargets={}",
		case.name,
		summary.candidates,
		summary.regions,
		summary.loaded,
		refused,
		case.inputs.len(),
		points,
		all.entries,
		all.exits[0],
		all.exits[1],
		all.exits[2],
		refusals,
		all.tag,
		all.store,
		all.cancel,
		summary.code_bytes,
		summary.mapped_bytes,
		summary.metadata_bytes,
		states.join(","),
		if case.targets.is_empty() { "-" } else { case.targets }
	);

	if case.loaded == 0 {
		assert_eq!(all.entries, 0, "{}", case.name);
	} else if !case.entry_refusals || !case.targets.is_empty() {
		assert!(covered.entries > 0, "{}: no native entry", case.name);
	}
	if !case.entry_refusals {
		assert_eq!(refusals, 0, "{}: unexpected entry fallback", case.name);
	} else {
		assert!(refusals > 0, "{}: expected entry refusals", case.name);
	}
	for (k, c) in ['C', 'F', 'R'].into_iter().enumerate() {
		if case.targets.contains(c) {
			assert!(covered.exits[k] > 0, "{}: no {c} exit", case.name);
		}
	}
	assert_eq!(cache::invalid_exits(), 0, "{}: invalid exit", case.name);
	(points, covered, all)
}

// ------------------------------------------------------------ bench kernels (v2 ports)

/// The v2 bench kernels (while, numeric, calls, fib, shared_rows), as Rune source.
pub(super) const V2_WHILE: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../tools/xtask/measure/kernels/while.rn"
));
pub(super) const V2_NUMERIC: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../tools/xtask/measure/kernels/numeric.rn"
));
pub(super) const V2_CALLS: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../tools/xtask/measure/kernels/calls.rn"
));
pub(super) const V2_FIB: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../tools/xtask/measure/kernels/fib.rn"
));
pub(super) const V2_SHARED_ROWS: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../tools/xtask/measure/kernels/shared_rows.rn"
));

/// The official fixed-input stage kernels, as Rune source.
const STAGE_NUMERIC: &str = "pub fn main(_a) {\n\tlet n = 0;\n\tlet total = 0;\n\twhile n < 10000 { total += n % 17; n += 1; }\n\tprintln!(\"{}\", total);\n}\n";
const STAGE_WHILE: &str =
	"pub fn main(_a) {\n\tlet n = 0;\n\twhile n < 10000 { n += 1; }\n\tprintln!(\"{}\", n);\n}\n";
const STAGE_CALLS: &str = "fn add(a, b) { a + b }\npub fn main(_a) {\n\tlet total = 0;\n\tfor i in 0..10000 { total = add(total, i); }\n\tprintln!(\"{}\", total);\n}\n";
const STAGE_FIB: &str =
	"fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\npub fn main(_a) { println!(\"{}\", fib(20)); }\n";
const STAGE_SHARED_ROWS: &str = "fn add(rows, key, value) {\n\tfor row in rows {\n\t\tif row[0] == key {\n\t\t\trow[1] = row[1] + value;\n\t\t\treturn;\n\t\t}\n\t}\n\trows.push([key, value]);\n}\n\npub fn main(_a) {\n\tlet rows = [];\n\tadd(rows, \"a\", 2);\n\tadd(rows, \"b\", 3);\n\tadd(rows, \"a\", 5);\n\tprintln!(\"rows {} {}\", rows[0][1], rows[1][1]);\n\tlet copy = rows.clone();\n\tlet alias = copy[0];\n\talias[1] = alias[1] + 1;\n\tcopy.push([\"c\", 9]);\n\tprintln!(\"nested {} {} {}\", rows[0][1], rows.len(), copy.len());\n}\n";

pub(super) fn bench_kernel_cases() -> Vec<Case> {
	vec![
		rune("v2_while", V2_WHILE, ints(&[&[25], &[1], &[7]]), 1, "CF"),
		rune(
			"v2_while_tiny",
			V2_WHILE,
			ints(&[&[0], &[1], &[2], &[3]]),
			1,
			"CF",
		),
		rune("v2_numeric", V2_NUMERIC, ints(&[&[40], &[18]]), 1, "CF"),
		rune(
			"v2_numeric_tiny",
			V2_NUMERIC,
			ints(&[&[0], &[1], &[2], &[3]]),
			1,
			"CF",
		),
		// B1: admitted (native IterNext, `add` inlined).
		rune("v2_calls", V2_CALLS, ints(&[&[0], &[5], &[30]]), 1, "CF"),
		rune("v2_fib", V2_FIB, ints(&[&[0], &[1], &[9]]), 0, ""),
		rune(
			"v2_shared_rows",
			V2_SHARED_ROWS,
			ints(&[&[0], &[123_456], &[999_999]]),
			0,
			"",
		),
	]
}

/// One admitted region and one runtime-divisor region in the same function.
pub(super) fn partial_refusal_case() -> Case {
	let mut case = rune(
		"partial_refusal",
		"pub fn main(a, b) { let i = 0; let s = 0; while i < a { s += i; i += 1; } let j = 0; let t = 0; while j < 3 { t += a / b; j += 1; } s + t }",
		ints(&[&[5, 2], &[5, 0], &[0, -1], &[i64::MIN, -1]]),
		1,
		"CF",
	);
	case.refused = [1, 0, 0];
	case
}

// ------------------------------------------------------------------------ 1. corpus

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_production_matches_interpreter_every_fuel() {
	run_child("child_production_every_fuel");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_production_every_fuel() {
	require_child();
	let mut cases = corpus_cases();
	cases.extend(new_cases());
	cases.extend(refusal_cases());
	cases.extend(bench_kernel_cases());
	cases.push(partial_refusal_case());
	let mut points = 0;
	for case in &cases {
		points += production_case(case).0;
	}
	println!(
		"P2TABLE\tTOTAL\tcases={}\tpoints={points}\tinvalid_exits={}",
		cases.len(),
		cache::invalid_exits()
	);
	assert_eq!(cache::invalid_exits(), 0);
}

// -------------------------------------------------------- bench kernel admission

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p2_bench_kernel_admission() {
	run_child("child_bench_kernel_admission");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_bench_kernel_admission() {
	require_child();
	for (name, text, loaded) in [
		("v2/while", V2_WHILE, 1),
		("v2/numeric", V2_NUMERIC, 1),
		("v2/calls", V2_CALLS, 1),
		// fib has no loop candidate; shared_rows has a side entry and is refused.
		("v2/fib", V2_FIB, 0),
		("v2/shared_rows", V2_SHARED_ROWS, 0),
		("stage/while", STAGE_WHILE, 1),
		("stage/numeric", STAGE_NUMERIC, 1),
		("stage/calls", STAGE_CALLS, 1),
		("stage/fib", STAGE_FIB, 0),
		("stage/shared_rows", STAGE_SHARED_ROWS, 0),
	] {
		execution();
		let program = compile_source(text);
		let s = cache::prepare(&program.code).expect("summary");
		println!(
			"P2ADMIT\t{name}\tfunctions={}\tcandidates={}\tregions={}\tloaded={}\tcode_bytes={}\tmapped_bytes={}\tmetadata_bytes={}\trefusals={:?}\tanalysis_ns={}\tcompile_ns={}\tload_ns={}\tprofile={}",
			s.functions,
			s.candidates,
			s.regions,
			s.loaded,
			s.code_bytes,
			s.mapped_bytes,
			s.metadata_bytes,
			unexpected_refusals(&s),
			s.analysis_ns,
			s.compile_ns,
			s.load_ns,
			if cfg!(debug_assertions) { "debug-assertions" } else { "release" }
		);
		assert_eq!(s.loaded, loaded, "{name}: {s:?}");
		assert_eq!(unexpected_refusals(&s), [0; 12], "{name}: {s:?}");
		if loaded == 1 {
			let r = counters(&program.code);
			assert_eq!(r.len(), 1);
			assert_eq!(r[0].state, State::Live);
		}
	}
	// The `% 17` constant: numeric has no runtime division refusal and no divide.
	execution();
	let program = compile_source(V2_NUMERIC);
	let s = cache::prepare(&program.code).unwrap();
	assert_eq!(
		(s.division_runtime, s.division_const0, s.division_constneg1),
		(0, 0, 0)
	);
}

// ---------------------------------------------------- 2. P1 hook mutual exclusion

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: compiles and runs generated code in a child process"
)]
fn p2_production_never_runs_with_test_hook_installed() {
	run_child("child_hook_exclusion");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_hook_exclusion() {
	require_child();
	execution();
	let program = compile_source(V2_WHILE);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let args = [Arg::I(20)];
	let reference = plain(&program.code, main, &args, BIG_FUEL);

	// P1 hook installed (simulator mode), production enabled: only the hook runs.
	assert_eq!(hook::install(&program.code), 1);
	hook::set_mode(Mode::Simulate);
	hook::take_stats();
	let (_, builds) = cache::counters_for_test();
	let observed = native(&program.code, main, &args, BIG_FUEL);
	let stats = hook::take_stats().unwrap();
	assert_eq!(observed, reference);
	assert!(stats.entries > 0, "the P1 hook ran");
	assert_eq!(
		cache::counters_for_test().1,
		builds,
		"production did not build"
	);
	assert!(
		!cache::resident_for_test(program.code.id()),
		"production cached nothing"
	);

	// Uninstalled: only production runs.
	hook::uninstall();
	let observed = native(&program.code, main, &args, BIG_FUEL);
	assert_eq!(observed, reference);
	assert_eq!(cache::counters_for_test().1, builds + 1);
	assert_eq!(totals(&program.code).entries, 1);
	println!(
		"P2HOOK\tinstalled: hook entries {} production builds 0; uninstalled: production entries 1",
		stats.entries
	);
}

// -------------------------------------------------- 3. Send additivity, thread-local

/// Compile-time: these stay `Send` with the feature on.
#[allow(dead_code)]
fn assert_send<T: Send>() {}

#[test]
fn p2_types_stay_send_and_regions_stay_thread_bound() {
	// `Store` (and so `Root`'s owner) is deliberately `!Send` in the base (it carries a
	// `PhantomData<Rc<()>>`); the feature does not change that. The types that were `Send`
	// stay `Send`:
	assert_send::<Executor>();
	assert_send::<frontend::Program>();
	assert_send::<Code>();

	// Compile-time: the region types are neither `Send` nor `Sync` (this resolves only
	// when the bound does NOT hold; otherwise the impl choice is ambiguous and the test
	// does not compile).
	trait AmbiguousIfSend<A> {
		fn check() {}
	}
	impl<T: ?Sized> AmbiguousIfSend<()> for T {}
	impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
	trait AmbiguousIfSync<A> {
		fn check() {}
	}
	impl<T: ?Sized> AmbiguousIfSync<()> for T {}
	impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}

	<super::boundary::NativeRegion as AmbiguousIfSend<_>>::check();
	<super::boundary::NativeRegion as AmbiguousIfSync<_>>::check();
	<cache::ProgramRegions as AmbiguousIfSend<_>>::check();
	<cache::ProgramRegions as AmbiguousIfSync<_>>::check();
	<cache::Guard as AmbiguousIfSend<_>>::check();
	<cache::Guard as AmbiguousIfSync<_>>::check();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_cache_is_thread_local() {
	run_child("child_thread_local");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_thread_local() {
	require_child();
	execution();
	let program = compile_source(V2_NUMERIC);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let id = program.code.id();
	let reference = plain(&program.code, main, &[Arg::I(40)], BIG_FUEL);

	// The program moves to each thread in turn (it is Send); each thread builds its own
	// regions and enters them.
	let mut program = program;
	let mut results = Vec::new();
	for t in 0..2 {
		let (back, observed, builds, entries, resident_elsewhere) = std::thread::spawn(move || {
			cache::enable_for_test(true);
			let (_, before) = cache::counters_for_test();
			let observed = invoke(&program.code, main, &[Arg::I(40)], BIG_FUEL);
			let builds = cache::counters_for_test().1 - before;
			let entries = totals(&program.code).entries;
			(program, observed, builds, entries, false)
		})
		.join()
		.expect("thread");
		program = back;
		assert_eq!(observed, reference, "thread {t}");
		assert_eq!((builds, entries), (1, 1), "thread {t}: own build and entry");
		assert!(!resident_elsewhere);
		results.push(observed);
	}
	assert!(
		!cache::resident_for_test(id),
		"the main thread built nothing"
	);
	assert_eq!(results[0], results[1]);
	println!("P2THREAD\ttwo threads: one build and one native entry each, equal results; main thread none");
}

// ------------------------------------------------------------- 4. partial refusal

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_partial_refusal_keeps_other_regions() {
	run_child("child_partial_refusal");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_partial_refusal() {
	require_child();
	let case = partial_refusal_case();
	let (_, covered, _) = production_case(&case);
	assert!(covered.entries > 0);
	let s = cache::summary(case.src.code().id()).unwrap();
	assert_eq!((s.regions, s.loaded, s.division_runtime), (2, 1, 1));
}

// ------------------------------------------------- 5. prepare touches no store/program

fn snapshot(code: &Code) -> (u32, Vec<String>) {
	let mut ops = Vec::new();
	for id in 0..code.functions() as u32 {
		let f = code.function(id).unwrap();
		for pc in 0..f.len() {
			ops.push(format!(
				"{id}:{pc}:{:?}:{}:{:?}",
				f.operation(pc).unwrap(),
				f.instructions()[pc].is_charged(),
				f.locations[pc]
			));
		}
	}
	(code.id(), ops)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p2_prepare_touches_neither_store_nor_program() {
	run_child("child_prepare_pure");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_prepare_pure() {
	require_child();
	execution();
	let program = compile_source(
		"fn inner(n) { let s = 0; let i = 0; while i < n { s += i % 5; i += 1; } s }
		pub fn main(a) { let t = 0; let k = 0; while k < a { t += inner(k); k += 1; } t }",
	);
	let s = store();
	let before = (
		snapshot(&program.code),
		s.state_for_test(),
		s.live_entries(),
	);
	let summary = cache::prepare(&program.code).unwrap();
	let after = (
		snapshot(&program.code),
		s.state_for_test(),
		s.live_entries(),
	);
	assert!(summary.loaded >= 1, "{summary:?}");
	assert_eq!(before, after);
	// `prepare` takes only `&Code`: it has no Context, Sources or Store to run the frontend
	// or touch a heap with (by construction, not by counting).
	println!(
		"P2PURE\tloaded={} instructions={} code id unchanged, operations/charges/locations unchanged, store unchanged",
		summary.loaded,
		before.0 .1.len()
	);
}

// ------------------------------------------------------- 6. invalid exit, poisoned

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_invalid_exit_poisons_region_and_falls_back() {
	run_child("child_invalid_exit");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_invalid_exit() {
	require_child();
	execution();
	let program = compile_source(V2_WHILE);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let code = &program.code;
	let args = [Arg::I(25)];
	let reference = plain(code, main, &args, BIG_FUEL);
	assert_eq!(cache::prepare(code).unwrap().loaded, 1);
	assert_eq!(cache::invalid_exits(), 0);

	// The native call runs; its record is then corrupted into "status not written".
	cache::inject_invalid_exit_for_test(1);
	let observed = native(code, main, &args, BIG_FUEL);
	assert_eq!(observed, reference, "fallback with the original state");
	let r = counters(code);
	assert_eq!((r[0].state, r[0].entries), (State::Poisoned, 0));
	assert_eq!(cache::invalid_exits(), 1, "the event is counted");

	// Never entered again; still the interpreter's result.
	for fuel in [0, 3, 10, BIG_FUEL] {
		assert_eq!(
			native(code, main, &args, fuel),
			plain(code, main, &args, fuel)
		);
	}
	let r = counters(code);
	assert_eq!((r[0].state, r[0].entries), (State::Poisoned, 0));
	assert_eq!(cache::invalid_exits(), 1);
	println!("P2INVALID\tinjected 1: fallback equal, region Poisoned, entries 0 afterwards, invalid_exits=1 (stderr line above)");
}

// --------------------------------------------------- 7. pending cancel at header pc 0

/// `fn(a, i) { while i < a { i += 1 } i }` with the header at pc 0.
fn header_at_pc0() -> Code {
	// slots: 0 a, 1 i, 2 c, 3 one
	let mut f = Function::new(4, 2).unwrap();
	bin(&mut f, 2, Binary::Lt, 1, 0, 0); // 0: header
	branch(&mut f, 2, 2, 5, 1); // 1
	lit(&mut f, 3, Value::I64(1), 2); // 2
	bin(&mut f, 1, Binary::Add, 1, 3, 3); // 3
	jump(&mut f, 0, 4); // 4
	f.push(Op::Return { value: 1 }, loc(5)).unwrap(); // 5
	build(vec![f])
}

/// One invocation on a given executor: (value or (kind, location, operation), chain).
fn on(executor: &mut Executor, code: &Code, args: &[i64]) -> (String, bool, usize) {
	let mut s = store();
	let roots: Vec<Root> = args.iter().map(|&v| s.root_i64(v)).collect();
	let refs: Vec<&Root> = roots.iter().collect();
	let text = match executor.invoke_code(code, &mut s, 0, &refs, BIG_FUEL) {
		Ok(root) => format!("{:?}", s.root_scalar(&root).unwrap()),
		Err(f) => format!(
			"{:?} at {:?} op {:?} refusal {:?} chain {:?} cleanup {:?}",
			f.error.kind,
			f.error.location,
			f.error.operation,
			f.error.refusal,
			f.call_chain.iter().collect::<Vec<_>>(),
			f.cleanup_error
		),
	};
	(text, executor.is_idle(), s.live_entries())
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_cancel_pending_at_header_pc0_is_consumed_by_interpreter() {
	run_child("child_cancel_pc0");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_cancel_pc0() {
	require_child();
	execution();
	let code = header_at_pc0();
	let s = cache::prepare(&code).unwrap();
	assert_eq!(s.loaded, 1, "{s:?}");
	assert_eq!(counters(&code)[0].header, 0);

	let run = |production: bool| {
		cache::enable_for_test(production);
		let mut executor = Executor::new().unwrap();
		executor.request_cancel();
		let first = on(&mut executor, &code, &[5, 0]);
		let second = on(&mut executor, &code, &[5, 0]);
		cache::enable_for_test(true);
		(first, second)
	};
	let reference = run(false);
	let before = totals(&code);
	let observed = run(true);
	let d = minus(totals(&code), before);

	assert_eq!(observed, reference);
	assert!(
		observed.0 .0.starts_with("Cancelled at"),
		"{}",
		observed.0 .0
	);
	assert!(
		observed.0 .0.contains(&format!("{:?}", loc(0))),
		"{}",
		observed.0 .0
	);
	assert!(observed.0 .1 && observed.1 .1, "executor idle");
	assert_eq!(
		(d.cancel, d.entries),
		(1, 1),
		"refused once (pending), then entered"
	);
	println!(
		"P2CANCEL\theader pc 0: {} ; refused_cancel=1 then native entry=1, executor idle",
		observed.0 .0
	);
}

// ------------------------------------------------- 8. armed test tick: no entry

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_cancel_tick_armed_disables_production_entry() {
	run_child("child_cancel_tick");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_cancel_tick() {
	require_child();
	execution();
	let program = compile_source(V2_WHILE);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let code = &program.code;
	assert_eq!(cache::prepare(code).unwrap().loaded, 1);
	for n in [0, 1, 3, 7, 20] {
		cancel_at_charge_for_test(Some(n));
		let p = plain(code, main, &[Arg::I(25)], BIG_FUEL);
		cancel_at_charge_for_test(Some(n));
		let before = totals(code);
		let o = native(code, main, &[Arg::I(25)], BIG_FUEL);
		cancel_at_charge_for_test(None);
		assert_eq!(o, p, "tick {n}");
		assert_eq!(
			minus(totals(code), before),
			Totals::default(),
			"tick {n}: no entry while armed"
		);
	}
	// Disarmed, it enters.
	let o = native(code, main, &[Arg::I(25)], BIG_FUEL);
	assert_eq!(o, plain(code, main, &[Arg::I(25)], BIG_FUEL));
	assert_eq!(totals(code).entries, 1);
	let assignment = compile_source(crate::experimental_core::region::n1_tests::FOR);
	let main = assignment.function(Hash::type_hash(["main"])).unwrap();
	let code = &assignment.code;
	assert_eq!(cache::prepare(code).unwrap().loaded, 1);
	for n in [0, 1, 3, 7, 20] {
		cancel_at_charge_for_test(Some(n));
		let p = plain(code, main, &[Arg::I(25)], BIG_FUEL);
		cancel_at_charge_for_test(Some(n));
		let before = totals(code);
		let observed = native(code, main, &[Arg::I(25)], BIG_FUEL);
		cancel_at_charge_for_test(None);
		assert_eq!(observed, p, "N1 assignment tick {n}");
		assert_eq!(minus(totals(code), before), Totals::default());
	}
}

// ---------------------------------------- 10. resume rule: equal to the P1 hook entries

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_resume_rule_matches_p1_hook_entry_counts() {
	run_child("child_resume_rule");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_resume_rule() {
	require_child();
	let mut cases = corpus_cases();
	cases.extend(new_cases());
	let mut compared = 0usize;
	for case in &cases {
		let code = case.src.code();
		let main = case.src.main();
		execution();
		// The P1 install stays aside (suspended) while production runs: never both.
		hook::install_native(code, None);
		hook::set_mode(Mode::Native);
		hook::suspend();
		for args in &case.inputs {
			let total = total_charge(code, main, args, &case.name);
			for fuel in 0..=total + 1 {
				// P1 hook, native mode (with its simulator cross-check).
				hook::resume();
				hook::take_stats();
				let p1 = invoke(code, main, args, fuel);
				let stats = hook::take_stats().unwrap();
				assert!(hook::take_mismatch().is_none());
				hook::suspend();

				// Production. A case with expected entry refusals is rebuilt each time, so
				// a region going Cold after repeated refusals cannot change the counts.
				if case.entry_refusals {
					cache::evict(code.id());
				}
				let before = totals(code);
				let p2 = native(code, main, args, fuel);
				let d = minus(totals(code), before);

				assert_eq!(p1, p2, "{}: fuel {fuel} {args:?}", case.name);
				assert_eq!(
					(
						stats.native_entries as u64,
						stats.native_continue as u64,
						stats.native_fuel as u64,
						stats.native_retry as u64,
						stats.refused_tag as u64,
						stats.refused_store as u64,
						stats.refused_cancel as u64
					),
					(d.entries, d.exits[0], d.exits[1], d.exits[2], d.tag, d.store, d.cancel),
					"{}: fuel {fuel} {args:?}",
					case.name
				);
				compared += 1;
			}
		}
		hook::resume();
		hook::uninstall();
	}
	println!("P2RESUME\tcases={} points={compared}: production entries, exits by kind and refusals equal the P1 hook's", cases.len());
	assert_eq!(cache::invalid_exits(), 0);
}

// ------------------------------------------------------ 11. repeated refusal: Cold

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_repeated_tag_refusal_marks_region_cold() {
	run_child("child_cold");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_cold() {
	require_child();
	// `k` is read only on a branch never taken, so the interpreter never fails on it, but
	// its guard (I64) refuses every arrival when it holds an F64: the header is refused at
	// each of the loop's arrivals, back edges included.
	let mut case = rune(
		"refused_every_arrival",
		"pub fn main(a, k) { let j = 0; let s = 0; while j < a { if j > a { s += k; } j += 1; } s }",
		vec![
			vec![Arg::I(0), Arg::F(1.5)],
			vec![Arg::I(3), Arg::F(1.5)],
			vec![Arg::I(10), Arg::F(1.5)],
		],
		1,
		"",
	);
	case.entry_refusals = true;
	production_case(&case);
	let code = case.src.code();
	let main = case.src.main();

	execution();
	assert_eq!(cache::prepare(code).unwrap().loaded, 1);
	let big = [Arg::I(100), Arg::F(1.5)];
	assert_eq!(
		native(code, main, &big, BIG_FUEL),
		plain(code, main, &big, BIG_FUEL)
	);
	let r = counters(code);
	assert_eq!(r.len(), 1);
	assert_eq!(
		(r[0].state, r[0].refused_tag, r[0].entries),
		(State::Cold, cache::COLD_AFTER_REFUSALS as u64, 0),
		"{r:?}"
	);
	// Cold stays cold and stops trying; results unchanged, also for inputs it could enter.
	for args in [big, [Arg::I(100), Arg::I(2)]] {
		assert_eq!(
			native(code, main, &args, BIG_FUEL),
			plain(code, main, &args, BIG_FUEL)
		);
	}
	let r = counters(code);
	assert_eq!(
		(r[0].state, r[0].refused_tag, r[0].entries),
		(State::Cold, cache::COLD_AFTER_REFUSALS as u64, 0)
	);
	println!(
		"P2COLD\t101 arrivals: {} consecutive tag refusals then Cold; no further attempts (also for an enterable input)",
		cache::COLD_AFTER_REFUSALS
	);
}

// -------------------------------------------------------- 12. LRU eviction frees code

const LRU_PROGRAMS: usize = 64;
const LRU_CYCLES: usize = 1_000;
const LRU_BASELINE: usize = 100;
const LRU_RSS_TOLERANCE_KIB: i64 = 8 * 1024;
const LRU_DATA_TOLERANCE_KIB: i64 = 16 * 1024;
const LRU_EXEC_TOLERANCE_KIB: i64 = 1024;

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_lru_eviction_frees_code() {
	run_child("child_lru");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_lru() {
	require_child();
	execution();
	let programs: Vec<frontend::Program> = (0..LRU_PROGRAMS)
		.map(|k| {
			compile_source(&format!(
				"pub fn main(a) {{ let n = {k}; while n < a {{ n += 1; }} n }}"
			))
		})
		.collect();
	let mains: Vec<u32> = programs
		.iter()
		.map(|p| p.function(Hash::type_hash(["main"])).unwrap())
		.collect();

	let maps0 = read_maps();
	let exec0 = anon_exec_kib(&maps0) as i64;
	let (rss0, data0) = (status_kib("VmRSS:") as i64, status_kib("VmData:") as i64);
	println!("P2RSS\tcycle=0\tVmRSS={rss0} kB\tVmData={data0} kB\tanon_exec={exec0} kB");

	// Heap overhead per loaded region (Cranelift JITModule and our metadata), measured, not
	// bounded: the first 16 programs, each building one region.
	for (p, &main) in programs.iter().zip(&mains).take(cache::MAX_CACHED_PROGRAMS) {
		let o = native(&p.code, main, &[Arg::I(80)], BIG_FUEL);
		assert_eq!(
			o.outcome,
			Outcome::Value(Some(crate::experimental_core::value::Scalar::I64(80)))
		);
	}
	let (entries, mapped, metadata, in_flight) = cache::cache_state_for_test();
	let exec16 = anon_exec_kib(&read_maps()) as i64;
	let (rss16, data16) = (status_kib("VmRSS:") as i64, status_kib("VmData:") as i64);
	println!(
		"P2MAP\t16 resident programs: entries={entries} mapped={mapped} B metadata={metadata} B in_flight={in_flight}; anon_exec +{} kB; VmRSS +{} kB; VmData +{} kB (heap per region incl. JITModule: ~{} kB RSS, ~{} kB data)",
		exec16 - exec0,
		rss16 - rss0,
		data16 - data0,
		(rss16 - rss0) / 16,
		(data16 - data0) / 16
	);
	assert_eq!(entries, 16);
	assert_eq!(mapped, 16 * cache::PAGE_BYTES);
	assert_eq!(in_flight, 0);
	assert_eq!(
		exec16 - exec0,
		(16 * cache::PAGE_BYTES / 1024) as i64,
		"one page per region"
	);

	let mut baseline = None;
	let mut end = None;
	for cycle in 1..=LRU_CYCLES {
		let k = cycle % LRU_PROGRAMS;
		let x = (cycle % 97) as i64;
		let o = native(&programs[k].code, mains[k], &[Arg::I(x)], BIG_FUEL);
		let want = x.max(k as i64);
		assert_eq!(
			o.outcome,
			Outcome::Value(Some(crate::experimental_core::value::Scalar::I64(want))),
			"cycle {cycle}"
		);
		let (entries, mapped, _, in_flight) = cache::cache_state_for_test();
		assert!(entries <= cache::MAX_CACHED_PROGRAMS && in_flight == 0);
		assert!(mapped <= cache::MAX_CACHED_PROGRAMS * cache::PAGE_BYTES);
		if cycle == LRU_BASELINE || cycle % 100 == 0 {
			let now = (
				status_kib("VmRSS:") as i64,
				status_kib("VmData:") as i64,
				anon_exec_kib(&read_maps()) as i64,
			);
			println!(
				"P2RSS\tcycle={cycle}\tVmRSS={} kB\tVmData={} kB\tanon_exec={} kB",
				now.0, now.1, now.2
			);
			if cycle == LRU_BASELINE {
				baseline = Some(now);
			}
			end = Some(now);
		}
	}
	let (b, e) = (baseline.unwrap(), end.unwrap());
	let (rss, data, exec) = (e.0 - b.0, e.1 - b.1, e.2 - b.2);
	let (_, builds) = cache::counters_for_test();
	println!(
		"P2RSS\tgrowth cycle {LRU_BASELINE}..{LRU_CYCLES}: VmRSS {rss:+} kB (tolerance {LRU_RSS_TOLERANCE_KIB}), VmData {data:+} kB (tolerance {LRU_DATA_TOLERANCE_KIB}), anon_exec {exec:+} kB (tolerance {LRU_EXEC_TOLERANCE_KIB}); builds={builds}; anon_exec at end {} kB vs start {exec0} kB",
		e.2
	);
	assert!(rss <= LRU_RSS_TOLERANCE_KIB, "VmRSS grew {rss} kB");
	assert!(data <= LRU_DATA_TOLERANCE_KIB, "VmData grew {data} kB");
	assert!(exec <= LRU_EXEC_TOLERANCE_KIB, "anon exec grew {exec} kB");
	assert!(e.2 - exec0 <= (cache::MAX_CACHED_PROGRAMS * cache::PAGE_BYTES / 1024) as i64);

	// Clearing the cache frees every mapping.
	cache::clear_for_test();
	assert_eq!(
		anon_exec_kib(&read_maps()) as i64,
		exec0,
		"all region code freed"
	);
	assert_eq!(cache::invalid_exits(), 0);
}

// ----------------------------------------------------- 13. PERF_BUILDID_DIR refused

#[test]
#[cfg_attr(miri, ignore = "Miri: runs Cranelift in a child process")]
fn p2_perf_buildid_dir_refuses_native() {
	let dir = std::env::temp_dir().join(format!("rune-p2-perf-{}", std::process::id()));
	std::fs::create_dir_all(&dir).unwrap();
	run_child_with("child_perf_env", &[("PERF_BUILDID_DIR", dir.as_os_str())]);
	let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
	assert!(
		left.is_empty(),
		"the tier wrote into PERF_BUILDID_DIR: {left:?}"
	);
	std::fs::remove_dir(&dir).unwrap();
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_perf_env() {
	require_child();
	assert!(
		std::env::var_os("PERF_BUILDID_DIR").is_some(),
		"the parent sets it"
	);
	execution();
	let program = compile_source(V2_WHILE);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let exec_before = anon_exec_kib(&read_maps());
	let s = cache::prepare(&program.code).unwrap();
	assert_eq!((s.regions, s.loaded, s.environment), (1, 0, 1), "{s:?}");
	for fuel in [0, 5, BIG_FUEL] {
		assert_eq!(
			native(&program.code, main, &[Arg::I(25)], fuel),
			plain(&program.code, main, &[Arg::I(25)], fuel)
		);
	}
	let map = std::path::PathBuf::from(format!("/tmp/perf-{}.map", std::process::id()));
	assert!(!map.exists(), "{map:?} was written");
	assert_eq!(
		anon_exec_kib(&read_maps()),
		exec_before,
		"no executable mapping"
	);
	println!("P2PERF\tPERF_BUILDID_DIR set: regions=1 loaded=0 environment=1, no {map:?}");
}

// -------------------------------------------------------- 14. compile panic caught

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2_catch_unwind_maps_compile_panic() {
	run_child("child_compile_panic");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_compile_panic() {
	require_child();
	execution();
	let program = compile_source(
		"pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
	);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let code = &program.code;

	cache::inject_compile_panic_for_test(Some(0));
	let s = cache::prepare(code).unwrap();
	assert_eq!((s.regions, s.loaded, s.panicked), (2, 1, 1), "{s:?}");
	let (entries, _, _, in_flight) = cache::cache_state_for_test();
	assert_eq!((entries, in_flight), (1, 0));
	let total = total_charge(code, main, &[Arg::I(6)], "compile_panic");
	for fuel in 0..=total + 1 {
		assert_eq!(
			native(code, main, &[Arg::I(6)], fuel),
			plain(code, main, &[Arg::I(6)], fuel)
		);
	}
	assert!(totals(code).entries > 0, "the other region runs natively");

	// Retry after eviction: rebuilt from scratch, both regions load.
	assert!(cache::evict(code.id()));
	let s = cache::prepare(code).unwrap();
	assert_eq!((s.loaded, s.panicked), (2, 0));
	println!("P2PANIC\tinjected compile panic: panicked=1 loaded=1 of 2, equal at every fuel; after eviction loaded=2");
}

// ------------------------------------- 15. in-flight markers never compile a duplicate

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p2_in_flight_markers_and_unwinding() {
	run_child("child_in_flight");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_in_flight() {
	require_child();
	execution();
	let program = compile_source(V2_WHILE);
	let code = &program.code;
	let main = program.function(Hash::type_hash(["main"])).unwrap();

	// (a) A nested checkout while checked out runs interpreted and builds nothing.
	let (_, b0) = cache::counters_for_test();
	let outer = cache::checkout(code);
	assert!(matches!(outer, Checkout::Native(_)));
	assert!(matches!(
		cache::checkout(code),
		Checkout::Interpret { no_regions: false }
	));
	// An invocation inside that window is interpreted and equal.
	let nested = native(code, main, &[Arg::I(9)], BIG_FUEL);
	assert_eq!(nested, plain(code, main, &[Arg::I(9)], BIG_FUEL));
	assert_eq!(cache::counters_for_test().1, b0 + 1, "one build only");
	assert_eq!(cache::cache_state_for_test().3, 1, "one in-flight marker");
	drop(outer);
	assert_eq!(cache::cache_state_for_test().3, 0);
	assert!(matches!(cache::checkout(code), Checkout::Native(_)));
	assert_eq!(cache::counters_for_test().1, b0 + 1);

	// (b) A panic during a build (outside the compile's catch_unwind) leaves no marker.
	assert!(cache::evict(code.id()));
	cache::inject_build_panic_for_test();
	let unwound = std::panic::catch_unwind(|| {
		let _ = cache::checkout(code);
	});
	assert!(unwound.is_err());
	assert_eq!(
		cache::cache_state_for_test(),
		(0, 0, 0, 0),
		"no marker, no charge left"
	);
	assert!(
		matches!(cache::checkout(code), Checkout::Native(_)),
		"rebuilt"
	);

	// (c) A panic while checked out puts the entry back.
	let unwound = std::panic::catch_unwind(|| {
		let guard = cache::checkout(code);
		assert!(matches!(guard, Checkout::Native(_)));
		std::panic!("injected (test)");
	});
	assert!(unwound.is_err());
	assert!(cache::resident_for_test(code.id()));
	assert_eq!(cache::cache_state_for_test().3, 0);
	let (_, builds) = cache::counters_for_test();
	assert_eq!(
		native(code, main, &[Arg::I(9)], BIG_FUEL),
		plain(code, main, &[Arg::I(9)], BIG_FUEL)
	);
	assert_eq!(
		cache::counters_for_test().1,
		builds,
		"no rebuild after the put-back"
	);
	println!("P2INFLIGHT\tnested checkout interpreted (1 build); build panic leaves no marker; panic while checked out puts back");
}

// ---------------------------------------------- 16. negative caches and the bounds

/// A function of `len` instructions: literals, then `return`.
pub(super) fn straight(len: usize, slots: u32) -> Function {
	let mut f = Function::new(slots, 0).unwrap();
	for pc in 0..len - 1 {
		lit(
			&mut f,
			(pc % slots as usize) as u32,
			Value::I64(1),
			pc as u32,
		);
	}
	f.push(Op::Return { value: 0 }, loc(len as u32)).unwrap();
	f
}

/// `fn(a) { padding literals; n = 0; while n < a { n += 1 } n }`, `len` instructions or
/// more.
pub(super) fn padded_loop(padding: usize) -> Function {
	// slots: 0 a, 1 n, 2 c, 3 one
	let mut f = Function::new(4, 1).unwrap();
	for pc in 0..padding {
		lit(&mut f, 1, Value::I64(0), pc as u32);
	}
	let h = padding as u32 + 1;
	lit(&mut f, 1, Value::I64(0), h - 1);
	bin(&mut f, 2, Binary::Lt, 1, 0, h);
	branch(&mut f, 2, h + 2, h + 5, h + 1);
	lit(&mut f, 3, Value::I64(1), h + 2);
	bin(&mut f, 1, Binary::Add, 1, 3, h + 3);
	jump(&mut f, h, h + 4);
	f.push(Op::Return { value: 1 }, loc(h + 5)).unwrap();
	f
}

/// `count` sequential counted loops in one function: `count` candidate headers.
pub(super) fn many_headers(count: u32) -> Function {
	// slots: 0 a, 1 n, 2 c, 3 one
	let mut f = Function::new(4, 1).unwrap();
	for k in 0..count {
		let h = 5 * k + 1;
		lit(&mut f, 1, Value::I64(0), h - 1);
		bin(&mut f, 2, Binary::Lt, 1, 0, h);
		branch(&mut f, 2, h + 2, h + 4, h + 1);
		bin(&mut f, 1, Binary::Add, 1, 3, h + 2);
		jump(&mut f, h, h + 3);
	}
	f.push(Op::Return { value: 1 }, loc(5 * count)).unwrap();
	f
}

fn summary_of(code: &Code) -> NativeSummary {
	cache::clear_for_test();
	cache::prepare(code).unwrap()
}

fn limit_row(name: &str, s: &NativeSummary) {
	println!(
		"P2LIMIT\t{name}\tfunctions={}\tanalysed={}\tskipped_length={}\tskipped_headers={}\tskipped_budget={}\tcandidates={}\tregions={}\tloaded={}\tregion_cap={}\tcompile_cap={}\tmetadata={}\tthread_bound={}\tcode_bytes={}\tmapped_bytes={}\tmetadata_bytes={}\tanalysis_ns={}\tcompile_ns={}\tload_ns={}",
		s.functions,
		s.analysed_functions,
		s.skipped_length,
		s.skipped_headers,
		s.skipped_budget,
		s.candidates,
		s.regions,
		s.loaded,
		s.region_cap,
		s.compile_cap,
		s.metadata,
		s.thread_bound,
		s.code_bytes,
		s.mapped_bytes,
		s.metadata_bytes,
		s.analysis_ns,
		s.compile_ns,
		s.load_ns
	);
}

#[test]
fn p2_analysis_work_estimate() {
	assert_eq!(cache::analysis_work(10, 0, 4), 10 + 40);
	assert_eq!(cache::analysis_work(10, 3, 4), 40 + 40);
	assert_eq!(
		cache::analysis_work(10, 1000, 4),
		10 * 65 + 40,
		"headers capped at 64"
	);
	// Past the inference bound the function-level cells are not run, so not charged.
	assert_eq!(cache::analysis_work(1 << 16, 0, 1 << 7), 1 << 16);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p2_negative_caches_and_program_bounds() {
	run_child("child_bounds");
}

#[test]
#[ignore = "P2 native worker: run by its parent test in a child process"]
fn child_bounds() {
	require_child();
	execution();

	// (a) No admitted region: an Empty entry, and the executor hint skips the cache.
	let program = compile_source("pub fn main(a) { a + 1 }");
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut s = store();
	let arg = s.root_i64(4);
	let mut executor = Executor::new().unwrap();
	let (c0, b0) = cache::counters_for_test();
	executor
		.invoke(&program, &mut s, main, &[&arg], BIG_FUEL)
		.unwrap();
	executor
		.invoke(&program, &mut s, main, &[&arg], BIG_FUEL)
		.unwrap();
	let (c1, b1) = cache::counters_for_test();
	assert_eq!(
		(c1 - c0, b1 - b0),
		(1, 1),
		"second invocation skipped the cache (hint)"
	);
	let mut other = Executor::new().unwrap();
	other
		.invoke(&program, &mut s, main, &[&arg], BIG_FUEL)
		.unwrap();
	assert_eq!(
		cache::counters_for_test(),
		(c1 + 1, b1),
		"new executor: Empty entry, no rebuild"
	);
	assert_eq!(cache::regions_for_test(program.code.id()), Some(Vec::new()));

	// (b) Every region refused: a negative entry; rebuilt only after eviction.
	let refused = compile_source(
		"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a / b; i += 1; } t }",
	);
	let rmain = refused.function(Hash::type_hash(["main"])).unwrap();
	let (_, b0) = cache::counters_for_test();
	for _ in 0..3 {
		native(&refused.code, rmain, &[Arg::I(7), Arg::I(2)], BIG_FUEL);
	}
	assert_eq!(
		cache::counters_for_test().1,
		b0 + 1,
		"refusals remembered while resident"
	);
	assert_eq!(
		cache::summary(refused.code.id()).unwrap().division_runtime,
		1
	);
	assert!(cache::evict(refused.code.id()));
	native(&refused.code, rmain, &[Arg::I(7), Arg::I(2)], BIG_FUEL);
	assert_eq!(
		cache::counters_for_test().1,
		b0 + 2,
		"retried after eviction"
	);

	// (c) Loaded regions per program.
	let mut text = String::new();
	for k in 0..(cache::MAX_PROGRAM_REGIONS + 6) {
		text.push_str(&format!(
			"fn f{k}(a) {{ let n = 0; while n < a {{ n += 1; }} n }}\n"
		));
	}
	text.push_str("pub fn main(a) { f0(a) }");
	let many = compile_source(&text);
	let s = summary_of(&many.code);
	limit_row("program_regions", &s);
	// P3a streaming: the six regions past the cap are no longer analyzed (batch build:
	// regions 70, region_cap 6); `region_cap` is now the flag that the cap stopped it.
	assert_eq!(
		(s.regions, s.loaded, s.region_cap, s.skipped_after_caps),
		(64, 64, 1, 6)
	);
	assert_eq!(s.mapped_bytes, 64 * cache::PAGE_BYTES as u64);
	assert!(s.metadata_bytes > 0 && s.metadata_bytes <= cache::MAX_PROGRAM_METADATA_BYTES as u64);
	let mmain = many.function(Hash::type_hash(["main"])).unwrap();
	assert_eq!(
		native(&many.code, mmain, &[Arg::I(30)], BIG_FUEL),
		plain(&many.code, mmain, &[Arg::I(30)], BIG_FUEL)
	);

	// (d) Function length.
	let long = build(vec![padded_loop(cache::MAX_ANALYZED_FUNCTION_LEN)]);
	let s = summary_of(&long);
	limit_row("function_length", &s);
	assert_eq!((s.skipped_length, s.regions, s.loaded), (1, 0, 0));
	let short = build(vec![padded_loop(1_000)]);
	let s = summary_of(&short);
	assert_eq!((s.skipped_length, s.loaded), (0, 1));
	assert_eq!(
		native(&short, 0, &[Arg::I(5)], BIG_FUEL),
		plain(&short, 0, &[Arg::I(5)], BIG_FUEL)
	);

	// (e) Candidate headers per function.
	let over = build(vec![many_headers(
		cache::MAX_FUNCTION_CANDIDATE_HEADERS as u32 + 1,
	)]);
	let s = summary_of(&over);
	limit_row("candidate_headers", &s);
	assert_eq!((s.skipped_headers, s.candidates), (1, 0));
	let at = build(vec![many_headers(
		cache::MAX_FUNCTION_CANDIDATE_HEADERS as u32,
	)]);
	let s = summary_of(&at);
	limit_row("candidate_headers_at_bound", &s);
	assert_eq!(s.skipped_headers, 0);
	assert_eq!(
		s.candidates as usize,
		crate::experimental_core::region::MAX_REGIONS_PER_FUNCTION
	);

	// (f) The analysis-work budget: functions of 1,000 instructions over 64 slots.
	let per = cache::analysis_work(1_000, 0, 64);
	let fit = (cache::MAX_PROGRAM_ANALYSIS_WORK / per) as usize;
	let budget = build((0..fit + 5).map(|_| straight(1_000, 64)).collect());
	let s = summary_of(&budget);
	limit_row("analysis_work", &s);
	assert_eq!((s.analysed_functions as usize, s.skipped_budget), (fit, 5));

	// (g) Functions analyzed per program.
	let count = cache::MAX_PROGRAM_ANALYZED_FUNCTIONS + 7;
	let functions = build((0..count).map(|_| straight(2, 1)).collect());
	let s = summary_of(&functions);
	limit_row("analysed_functions", &s);
	assert_eq!(
		(s.analysed_functions as usize, s.skipped_budget),
		(cache::MAX_PROGRAM_ANALYZED_FUNCTIONS, 7)
	);

	// (h) One region's mapping is one page: the summary's mapped bytes against
	// /proc/self/maps.
	cache::clear_for_test();
	let before = anon_exec_kib(&read_maps());
	let w = compile_source(V2_WHILE);
	let s = cache::prepare(&w.code).unwrap();
	let after = anon_exec_kib(&read_maps());
	println!(
		"P2MAP\tv2/while: code_bytes={} mapped_bytes={} metadata_bytes={} anon_exec_delta={} KiB",
		s.code_bytes,
		s.mapped_bytes,
		s.metadata_bytes,
		after - before
	);
	assert_eq!(s.mapped_bytes, cache::PAGE_BYTES as u64);
	assert_eq!((after - before) * 1024, cache::PAGE_BYTES as u64);
	assert_eq!(cache::invalid_exits(), 0);
}

/// B3: compare native routing counters across the eager-buffer baseline and
/// growable bookkeeping. This checks deterministic execution, not timings.
#[test]
#[cfg_attr(miri, ignore = "Miri: executes native code in a child process")]
fn b3_kernel_room_gate() {
	run_child("child_b3_kernel_room_gate");
}

#[test]
#[ignore = "Native worker: launched by b3_kernel_room_gate"]
fn child_b3_kernel_room_gate() {
	require_child();
	for (name, text) in [
		("numeric", STAGE_NUMERIC),
		("while", STAGE_WHILE),
		("calls", STAGE_CALLS),
		("fib", STAGE_FIB),
		("shared_rows", STAGE_SHARED_ROWS),
	] {
		execution();
		let program = compile_source(text);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		cache::prepare(&program.code).unwrap();
		let observed = native(&program.code, main, &[Arg::I(0)], 1_000_000_000);
		assert!(
			matches!(observed.outcome, Outcome::Value(_)),
			"{observed:?}"
		);
		let rows: Vec<_> = cache::regions_for_test(program.code.id())
			.unwrap_or_default()
			.into_iter()
			.map(|r| {
				(
					r.function,
					r.header,
					r.state,
					r.entries,
					r.exits,
					[
						r.refused_cancel,
						r.refused_store,
						r.refused_tag,
						r.refused_iter,
						r.refused_room,
					],
				)
			})
			.collect();
		// Frozen from B2 (7c448146) with the identical counter-only driver.
		let (fuel, header) = match name {
			"numeric" => (999989998, Some(2)),
			"while" => (999989998, Some(1)),
			"calls" => (999979998, Some(5)),
			"fib" => (999978108, None),
			"shared_rows" => (999999992, None),
			_ => unreachable!(),
		};
		assert_eq!(observed.fuel_left, Some(fuel), "{name}");
		if let Some(header) = header {
			assert_eq!(
				rows,
				vec![(0, header, State::Live, 1, [1, 0, 0, 0], [0; 5])],
				"{name}"
			);
		} else {
			assert!(rows.is_empty(), "{name}: {rows:?}");
		}
		println!("B3ROUTE {name} fuel={:?} rows={rows:?}", observed.fuel_left);
	}
	cache::clear_for_test();
	cache::enable_for_test(false);
}

#[test]
#[cfg_attr(miri, ignore = "native code execution in isolated child")]
fn raw_print_beside_a_loop_preserves_native_entry() {
	run_child("child_raw_print_beside_loop");
}

#[test]
#[ignore = "native worker invoked only by parent"]
fn child_raw_print_beside_loop() {
	require_child();
	execution();
	let program = compile_source(
		"pub fn main(n) { print!(\"\"); let i=0; while i<n { i+=1; } print!(\"\"); i }",
	);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let args = [Arg::I(20)];
	let reference = plain(&program.code, main, &args, BIG_FUEL);
	let observed = native(&program.code, main, &args, BIG_FUEL);
	assert_eq!(observed, reference);
	assert!(
		totals(&program.code).entries > 0,
		"the adjacent pure loop must run natively"
	);
}
