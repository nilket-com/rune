//! P3a: resource bounds of the production native tier (test-only).
//!
//! The selection golden (`fixtures/p3a_selection_golden.txt`) was captured at 672a4eaa,
//! before the streaming refactor, by `child_selection_golden` with
//! `RUNE_P3A_GOLDEN_OUT` set (only test files differed from 672a4eaa then). For every
//! program of the corpus it pins:
//!
//! - `SUMMARY`: every counter of the build summary that existed at 672a4eaa, except the
//!   timings (production `cache::prepare`);
//! - `LOADED`: the loaded regions in load order, `(function, header, code bytes)`;
//! - `ANALYSIS`: the outcome of every analysed candidate header from `region::analyze`, an
//!   admitted region as an FNV-1a 64 fingerprint of all its fields;
//! - `COMPILE`: the classification of every admitted region by an independent
//!   `lower::compile(region, 0)`.
//!
//! Every test that loads or executes generated code runs in a child process, as in P2.
//! Output lines prefixed `P3A` are receipt rows.
use super::exec_tests::{
	anon_exec_kib, bin, branch, build, compile_source, corpus_cases, invoke, jump, lit, loc, new_cases,
	read_maps, refusal_cases, Arg, Observed, Src, BIG_FUEL,
};
use super::p2_tests::{many_headers, padded_loop, partial_refusal_case, probe_kernel_cases, straight};
use super::{audit::Finding, boundary, lower, NativeRefusal, MAX_PROGRAM_CODE_BYTES};
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Code, Function, Op},
	frontend::Program,
	region::{
		analyze,
		cache::{self, Checkout, Limits, NativeSummary},
		division_kind, hook, inject_analysis_error_for_test, DivisionKind, Region,
	},
	value::Value,
};
use crate::Hash;
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_P3A_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(900);

/// The selection golden captured at 672a4eaa.
const GOLDEN: &str = include_str!("fixtures/p3a_selection_golden.txt");

// ------------------------------------------------------------------ process isolation

fn run_child_with(inner: &str, env: &[(&str, &std::ffi::OsStr)]) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::p3a_tests::{inner}");
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
		"P3a native worker tests run only in the child process started by their parent test"
	);
}

/// A fresh thread state: no P1 install, no cached regions, production on, production
/// limits, the system page size.
fn fresh() {
	hook::uninstall();
	cache::set_limits_for_test(None);
	cache::set_page_size_for_test(None);
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

/// Native equals plain at full fuel and at a mid fuel (the caps never change outcomes).
fn same(code: &Code, main: u32, args: &[Arg], name: &str) {
	let reference = plain(code, main, args, BIG_FUEL);
	assert_eq!(native(code, main, args, BIG_FUEL), reference, "{name}: full fuel");
	let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
	let mid = total / 2;
	assert_eq!(native(code, main, args, mid), plain(code, main, args, mid), "{name}: fuel {mid}");
	assert_eq!(cache::invalid_exits(), 0, "{name}: invalid exit");
}

fn scaled(change: impl FnOnce(&mut Limits)) -> Limits {
	let mut limits = Limits::PRODUCTION;
	change(&mut limits);
	limits
}

/// A fresh build of `code` under `limits` on this thread (the cache is cleared first).
fn prep_with(limits: Option<Limits>, code: &Code) -> NativeSummary {
	cache::set_limits_for_test(limits);
	cache::clear_for_test();
	cache::prepare(code).expect("summary")
}

fn row(name: &str, s: &NativeSummary) {
	println!(
		"P3ALIMIT\t{name}\tattempted={}\tanalysed={}\tskipped_length={}\tskipped_headers={}\tskipped_budget={}\tskipped_after_caps={}\tcandidates={}\tregions={}\tloaded={}\tregion_cap={}\tcompile_cap={}\tmetadata={}\tthread_bound={}\tprocess_bound={}\tsize={}\tdivision_runtime={}\tpanicked={}\ttransient={}\tsteps={}\tpage_size={}\tload_refused={}\tload_panicked={}\tcode_bytes={}\tmapped_bytes={}\tmetadata_bytes={}\treserved_bytes={}\tanalysis_peak_bytes={}",
		s.attempted_functions,
		s.analysed_functions,
		s.skipped_length,
		s.skipped_headers,
		s.skipped_budget,
		s.skipped_after_caps,
		s.candidates,
		s.regions,
		s.loaded,
		s.region_cap,
		s.compile_cap,
		s.metadata,
		s.thread_bound,
		s.process_bound,
		s.size,
		s.division_runtime,
		s.panicked,
		s.transient,
		s.steps,
		s.page_size,
		s.load_refused,
		s.load_panicked,
		s.code_bytes,
		s.mapped_bytes,
		s.metadata_bytes,
		s.reserved_bytes,
		s.analysis_peak_bytes
	);
}

// ------------------------------------------------------------------ program generators

/// `fn(a) { i = 0; one = 1; while i < a { w_0..w_{W-1} = literals; then `adds` times
/// w_j = w_j + i (j cycling); i = i + one } i }`. Slots: 0 a, 1 i, 2 cond, 3 one, then the
/// `writes` w slots. Every Add has a Retry exit that stores every w slot, so the code grows
/// with `writes * adds`, the exits (metadata) with `adds`.
fn fat_loop(writes: u32, adds: u32) -> Function {
	let writes = writes.max(1);
	let mut f = Function::new(4 + writes, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	lit(&mut f, 3, Value::I64(1), 1);
	let h = 2u32;
	let exit = h + 2 + writes + adds + 2;
	bin(&mut f, 2, Binary::Lt, 1, 0, h);
	branch(&mut f, 2, h + 2, exit, h + 1);
	let mut pc = h + 2;
	for w in 0..writes {
		lit(&mut f, 4 + w, Value::I64(i64::from(w) + 7), pc);
		pc += 1;
	}
	for k in 0..adds {
		let w = 4 + k % writes;
		bin(&mut f, w, Binary::Add, w, 1, pc);
		pc += 1;
	}
	bin(&mut f, 1, Binary::Add, 1, 3, pc);
	jump(&mut f, h, pc + 1);
	f.push(Op::Return { value: 1 }, loc(exit)).unwrap();
	f
}

/// As [`fat_loop`], but the `count` ops are compares `cond = w_j < i` (no Retry exit), so
/// the body is large while the code and the exits stay small.
fn cmp_loop(writes: u32, count: u32) -> Function {
	let writes = writes.max(1);
	let mut f = Function::new(4 + writes, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0);
	lit(&mut f, 3, Value::I64(1), 1);
	let h = 2u32;
	let exit = h + 2 + writes + count + 2;
	bin(&mut f, 2, Binary::Lt, 1, 0, h);
	branch(&mut f, 2, h + 2, exit, h + 1);
	let mut pc = h + 2;
	for w in 0..writes {
		lit(&mut f, 4 + w, Value::I64(i64::from(w) + 7), pc);
		pc += 1;
	}
	for k in 0..count {
		bin(&mut f, 2, Binary::Lt, 4 + k % writes, 1, pc);
		pc += 1;
	}
	bin(&mut f, 1, Binary::Add, 1, 3, pc);
	jump(&mut f, h, pc + 1);
	f.push(Op::Return { value: 1 }, loc(exit)).unwrap();
	f
}

/// `count` copies of one function.
fn copies(count: usize, make: impl Fn() -> Function) -> Code {
	build((0..count).map(|_| make()).collect())
}

/// The one admitted region of function 0 of `code`.
fn region_of(code: &Code) -> Region {
	analyze(code.function(0).unwrap(), 0)
		.unwrap()
		.into_iter()
		.find_map(|(_, r)| r.ok())
		.expect("an admitted region")
}

/// The machine-code bytes of `fat_loop(writes, adds)`'s region; `usize::MAX` when it
/// exceeds the per-region bound (`NativeRefusal::Size`).
fn code_bytes(writes: u32, adds: u32) -> usize {
	let code = build(vec![fat_loop(writes, adds)]);
	match lower::compile(&region_of(&code), 0) {
		Ok(compiled) => compiled.bytes().len(),
		Err(NativeRefusal::Size) => usize::MAX,
		Err(other) => std::panic!("fat_loop({writes}, {adds}): {other:?}"),
	}
}

/// The adjacent pair with `bytes(adds - 1) <= target < bytes(adds)`, by bisection on an
/// invariant (no monotonicity assumed): `(adds, bytes(adds - 1), bytes(adds))`.
fn adds_crossing(writes: u32, target: usize, mut lo: u32, mut hi: u32) -> (u32, usize, usize) {
	assert!(code_bytes(writes, lo) <= target, "fat_loop({writes}, {lo}) over {target}");
	assert!(code_bytes(writes, hi) > target, "fat_loop({writes}, {hi}) not over {target}");
	while hi - lo > 1 {
		let mid = lo + (hi - lo) / 2;
		if code_bytes(writes, mid) <= target {
			lo = mid;
		} else {
			hi = mid;
		}
	}
	(hi, code_bytes(writes, lo), code_bytes(writes, hi))
}

/// `fn(a) { s = 0; loop { s = s; } }` reduced: pc 0 literal, pc 1 `jump 1`: a one-node
/// region that touches no slot (the zero-slot corner).
fn zero_slot_loop() -> Function {
	let mut f = Function::new(1, 1).unwrap();
	lit(&mut f, 0, Value::I64(0), 0);
	jump(&mut f, 1, 1);
	f
}

// ------------------------------------------------------------------ selection golden

/// The region-test listing programs (`region/tests.rs` `LISTED`), copied verbatim.
const LISTED: &[&str] = &[
	"pub fn main(_a) { let n = 0; while n < 10000 { n += 1; } n }",
	"pub fn main(_a) { let n = 0; let total = 0; while n < 10000 { total += n % 17; n += 1; } total }",
	"pub fn main(a) { let i = 0; let s = 0; while i < a { let t = i * 2; if t > 6 { s = s + t; } i += 1; } s }",
	"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x = x + x; i += b; } x }",
	"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += x; i += b; } x }",
	"pub fn main(a) { let i = 0; let s = 0; while i < a { let j = 0; while j < i { s += j; j += 1; } i += 1; } s }",
	"pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j = 0; while j < i { s += j; j += 1; } i += 1; } s }",
	"pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
	"pub fn main(a) { let i = 0; let s = 0; while i < a { if i > 3 { if i < 9 { s += 1; } } i += 1; } s }",
	"fn add(a, b) { a + b } pub fn main(_a) { let total = 0; for i in 0..10000 { total = add(total, i); } total }",
];

/// The 70-loop-function program of P2 bounds test (c).
fn seventy_loops() -> Program {
	let mut text = String::new();
	for k in 0..70 {
		text.push_str(&format!("fn f{k}(a) {{ let n = 0; while n < a {{ n += 1; }} n }}\n"));
	}
	text.push_str("pub fn main(a) { f0(a) }");
	compile_source(&text)
}

/// The P2 bounds programs of `child_bounds` (c) to (g), at the constants of 672a4eaa.
fn bounds_programs() -> Vec<(String, Src)> {
	let mut out = Vec::new();
	out.push(("bounds_program_regions".into(), Src::Program(seventy_loops())));
	out.push(("bounds_function_length_over".into(), Src::Code(build(vec![padded_loop(65_536)]))));
	out.push(("bounds_function_length_short".into(), Src::Code(build(vec![padded_loop(1_000)]))));
	out.push(("bounds_headers_over".into(), Src::Code(build(vec![many_headers(257)]))));
	out.push(("bounds_headers_at".into(), Src::Code(build(vec![many_headers(256)]))));

	// 1,000 instructions over 64 slots: analysis_work = 1,000 + 64,000 per function.
	let fit = ((1u64 << 24) / 65_000) as usize;
	out.push((
		"bounds_analysis_work".into(),
		Src::Code(build((0..fit + 5).map(|_| straight(1_000, 64)).collect())),
	));
	out.push((
		"bounds_analysed_functions".into(),
		Src::Code(build((0..1024 + 7).map(|_| straight(2, 1)).collect())),
	));
	out
}

/// Every program of the golden, by unique name.
fn golden_programs() -> Vec<(String, Src)> {
	let mut cases = corpus_cases();
	cases.extend(new_cases());
	cases.extend(refusal_cases());
	cases.extend(probe_kernel_cases());
	cases.push(partial_refusal_case());

	let mut out: Vec<(String, Src)> = cases.into_iter().map(|c| (c.name, c.src)).collect();
	for (i, text) in LISTED.iter().enumerate() {
		out.push((format!("listed_{i}"), Src::Program(compile_source(text))));
	}
	out.extend(bounds_programs());

	let mut names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
	names.sort_unstable();
	let count = names.len();
	names.dedup();
	assert_eq!(names.len(), count, "golden program names are unique");
	out
}

fn fnv1a(text: &str) -> u64 {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for byte in text.bytes() {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	hash
}

/// Every field of an admitted region, in a fixed textual form.
pub(super) fn region_text(r: &Region) -> String {
	format!(
		"function={} header={} header_node={} tracked={:#x} proven={:#x} slots={:?} entry={:?} nodes={:?} exits={:?}",
		r.function,
		r.header,
		r.header_node,
		r.tracked,
		r.proven,
		r.slots.as_slice(),
		r.entry.as_slice(),
		r.nodes.as_slice(),
		r.exits.as_slice()
	)
}

/// The summary counters that existed at 672a4eaa, in their declaration order, without the
/// timings.
pub(super) fn summary_fields(s: &NativeSummary) -> String {
	format!(
		"functions={} analysed_functions={} skipped_length={} skipped_headers={} skipped_budget={} analysis_errors={} candidates={} regions={} loaded={} division_runtime={} division_const0={} division_constneg1={} size={} codegen={} panicked={} audit_other={} other={} load_refused={} metadata={} region_cap={} compile_cap={} thread_bound={} environment={} code_bytes={} mapped_bytes={} metadata_bytes={}",
		s.functions,
		s.analysed_functions,
		s.skipped_length,
		s.skipped_headers,
		s.skipped_budget,
		s.analysis_errors,
		s.candidates,
		s.regions,
		s.loaded,
		s.division_runtime,
		s.division_const0,
		s.division_constneg1,
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
		s.code_bytes,
		s.mapped_bytes,
		s.metadata_bytes
	)
}

fn compile_class(region: &Region) -> String {
	match lower::compile(region, 0) {
		Ok(compiled) => format!("ok bytes={}", compiled.bytes().len()),
		Err(NativeRefusal::Audit(Finding::Division(_))) => match division_kind(region) {
			Some(DivisionKind::Const0) => "division_const0".into(),
			Some(DivisionKind::ConstNeg1) => "division_constneg1".into(),
			_ => "division_runtime".into(),
		},
		Err(NativeRefusal::Size) => "size".into(),
		Err(NativeRefusal::Codegen(_)) => "codegen".into(),
		Err(NativeRefusal::Audit(_)) => "audit_other".into(),
		Err(other) => format!("other {other:?}"),
	}
}

/// The golden text of one program: production build summary and loaded set, plus the
/// independent analysis and compile classification of every candidate header. Also returns
/// the full P3a summary.
fn golden_of(name: &str, code: &Code) -> (String, NativeSummary) {
	fresh();
	let mut out = String::new();
	let summary = cache::prepare(code).expect("summary");
	out.push_str(&format!("PROGRAM\t{name}\n"));
	out.push_str(&format!("SUMMARY\t{name}\t{}\n", summary_fields(&summary)));
	for r in cache::regions_for_test(code.id()).expect("resident") {
		out.push_str(&format!("LOADED\t{name}\tf{}\th{}\tcode_bytes={}\n", r.function, r.header, r.code_bytes));
	}
	cache::clear_for_test();

	for id in 0..code.functions() as u32 {
		let f = code.function(id).unwrap();
		match analyze(f, id) {
			Ok(found) => {
				for (header, region) in found {
					match region {
						Ok(region) => {
							out.push_str(&format!(
								"ANALYSIS\t{name}\tf{id}\th{header}\tadmitted\tfnv={:016x}\tnodes={}\texits={}\tslots={}\n",
								fnv1a(&region_text(&region)),
								region.nodes.len(),
								region.exits.len(),
								region.slots.len()
							));
							out.push_str(&format!("COMPILE\t{name}\tf{id}\th{header}\t{}\n", compile_class(&region)));
						}
						Err(refusal) => {
							out.push_str(&format!("ANALYSIS\t{name}\tf{id}\th{header}\trefused\t{refusal:?}\n"));
						}
					}
				}
			}
			Err(error) => out.push_str(&format!("ANALYSIS\t{name}\tf{id}\terror\t{:?}\n", error.kind)),
		}
	}
	(out, summary)
}

fn field(line: &str, key: &str) -> u64 {
	let prefix = format!("{key}=");
	line.split(|c| c == ' ' || c == '\t')
		.find_map(|f| f.strip_prefix(prefix.as_str()))
		.unwrap_or_else(|| std::panic!("{key} in {line}"))
		.parse()
		.unwrap()
}

fn without(line: &str, keys: &[&str]) -> String {
	line.split(' ')
		.filter(|f| !keys.iter().any(|k| f.starts_with(&format!("{k}="))))
		.collect::<Vec<_>>()
		.join(" ")
}

/// The golden comparison, with exactly two documented differences:
///
/// - `metadata_bytes` grows by `size_of::<Token>()` per loaded region: `Ready` now owns its
///   process reservation token (and `region_metadata` charges `size_of::<Ready>()`);
/// - in a program that reaches the region or compile cap, the regions after the cap are
///   no longer analysed: `analysed_functions` and `candidates` may drop, `regions` drops by
///   exactly the old cap refusals, and `region_cap`/`compile_cap` become a flag (1).
///
/// Every other SUMMARY counter, and every LOADED, ANALYSIS and COMPILE line, is identical.
fn compare_golden(new: &str) -> (usize, usize) {
	let token = core::mem::size_of::<crate::experimental_core::region::budget::Token<'static>>() as u64;
	let old: Vec<&str> = GOLDEN.lines().collect();
	let new: Vec<&str> = new.lines().collect();
	assert_eq!(old.len(), new.len(), "golden line count");
	let mut exact = 0;
	let mut capped = 0;
	for (o, n) in old.iter().zip(new.iter()) {
		if !o.starts_with("SUMMARY\t") {
			assert_eq!(o, n);
			exact += 1;
			continue;
		}
		let loaded = field(o, "loaded");
		assert_eq!(field(n, "loaded"), loaded, "{n}");
		assert_eq!(field(n, "metadata_bytes"), field(o, "metadata_bytes") + loaded * token, "{n}");
		let cap_flag = field(n, "region_cap") + field(n, "compile_cap");
		if cap_flag == 0 {
			assert_eq!(without(o, &["metadata_bytes"]), without(n, &["metadata_bytes"]));
			exact += 1;
		} else {
			assert_eq!(cap_flag, 1, "{n}");
			let keys = [
				"metadata_bytes",
				"analysed_functions",
				"candidates",
				"regions",
				"region_cap",
				"compile_cap",
			];
			assert_eq!(without(o, &keys), without(n, &keys));
			assert!(field(n, "analysed_functions") <= field(o, "analysed_functions"), "{n}");
			assert_eq!(
				field(n, "regions"),
				field(o, "regions") - field(o, "region_cap") - field(o, "compile_cap"),
				"{n}"
			);
			assert!(field(n, "candidates") <= field(o, "candidates"), "{n}");
			capped += 1;
		}
	}
	(exact, capped)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_selection_golden() {
	run_child("child_selection_golden");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_selection_golden() {
	require_child();
	let mut text = String::new();
	for (name, src) in golden_programs() {
		let (golden, s) = golden_of(&name, src.code());
		text.push_str(&golden);
		println!(
			"P3ASEL\t{name}\tattempted={}\tskipped_after_caps={}\ttransient={}\tsteps={}\tprocess_bound={}\tanalysis_peak_bytes={}\treserved_bytes={}",
			s.attempted_functions,
			s.skipped_after_caps,
			s.transient,
			s.steps,
			s.process_bound,
			s.analysis_peak_bytes,
			s.reserved_bytes
		);
		assert_eq!((s.transient, s.steps, s.process_bound, s.page_size), (0, 0, 0, 0), "{name}");
	}
	assert_eq!(cache::invalid_exits(), 0);

	if let Some(path) = std::env::var_os("RUNE_P3A_GOLDEN_OUT") {
		std::fs::write(&path, &text).expect("write golden");
		println!("P3AGOLDEN\twritten\tlines={}\tfnv={:016x}", text.lines().count(), fnv1a(&text));
		return;
	}

	let (exact, capped) = compare_golden(&text);
	println!(
		"P3AGOLDEN\tcompared\tlines={}\tgolden_fnv={:016x}\tnew_fnv={:016x}\tidentical_or_metadata_token_only={exact}\tcapped_programs={capped}",
		text.lines().count(),
		fnv1a(GOLDEN),
		fnv1a(&text)
	);
	assert_eq!(capped, 1, "only bounds_program_regions reaches a cap");
}

// ----------------------------------------------- A2/A3: streaming and work charging

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_streaming_and_attempt_bounds() {
	run_child("child_streaming");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_streaming() {
	require_child();
	fresh();

	// (a) The region cap stops the analysis: 64 headers analysed, not 70.
	let program = seventy_loops();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	cache::clear_for_test();
	let before = cache::analyzed_headers_for_test();
	let s = cache::prepare(&program.code).unwrap();
	row("a2_region_cap_streaming", &s);
	assert_eq!(cache::analyzed_headers_for_test() - before, 64);
	assert_eq!((s.loaded, s.regions, s.region_cap, s.skipped_after_caps), (64, 64, 1, 6));
	assert_eq!(s.analysed_functions, 65, "main and f1..f64; 672a4eaa analysed all 71");
	same(&program.code, main, &[Arg::I(30)], "seventy_loops");

	// (b) The scan is charged before it runs: work budget 3 * L for 10 functions of
	// length L with 257 headers each.
	let len = many_headers(257).len() as u64;
	let code = copies(10, || many_headers(257));
	let scans = cache::scans_for_test();
	let s = prep_with(Some(scaled(|l| l.program_analysis_work = 3 * len)), &code);
	row("a3_scan_charged_first", &s);
	assert_eq!(cache::scans_for_test() - scans, 3, "three scans, not ten");
	assert_eq!((s.skipped_headers, s.skipped_budget, s.attempted_functions), (3, 7, 10));

	// (c) The attempt cap counts functions every gate skips: k - 1, k, k + 1 around 8.
	for (functions, skipped) in [(7usize, 0u32), (8, 0), (9, 1), (20, 12)] {
		let code = copies(functions, || straight(20, 1));
		let scans = cache::scans_for_test();
		let limits = scaled(|l| {
			l.program_attempted_functions = 8;
			l.function_len = 10;
		});
		let s = prep_with(Some(limits), &code);
		row(&format!("a3_attempt_cap_{functions}"), &s);
		assert_eq!(cache::scans_for_test(), scans, "no scan of a skipped function");
		assert_eq!(
			(s.attempted_functions as usize, s.skipped_length as usize, s.skipped_budget),
			(functions.min(8), functions.min(8), skipped)
		);
	}

	// (d) The attempt cap at its real value: 4,097 tiny functions.
	let code = copies(cache::MAX_PROGRAM_ATTEMPTED_FUNCTIONS + 1, || straight(2, 1));
	let s = prep_with(None, &code);
	row("a3_attempt_cap_real_4096", &s);
	assert_eq!(s.attempted_functions as usize, cache::MAX_PROGRAM_ATTEMPTED_FUNCTIONS);
	assert_eq!(s.analysed_functions as usize, cache::MAX_PROGRAM_ANALYSED_FUNCTIONS);
	assert_eq!(
		s.skipped_budget as usize,
		cache::MAX_PROGRAM_ATTEMPTED_FUNCTIONS - cache::MAX_PROGRAM_ANALYSED_FUNCTIONS + 1
	);
	cache::set_limits_for_test(None);
	cache::clear_for_test();
	println!("P3ASTREAM\tregion cap stops analysis at 64 headers; scan charged first (3 scans); attempt cap 8 and 4096 count skipped functions");
}

// ------------------------------------------- A2: the hard transient analysis bound

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_transient_analysis_bound() {
	run_child("child_transient");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_transient() {
	require_child();
	fresh();

	// (a) Scaled: the exact peak P of a program, then the limit at P - 1, P, P + 1.
	let code = build(vec![fat_loop(40, 60), many_headers(20), fat_loop(8, 200)]);
	let unbounded = prep_with(Some(scaled(|l| l.transient_analysis_bytes = usize::MAX)), &code);
	let peak = unbounded.analysis_peak_bytes as usize;
	row("a2_transient_unbounded", &unbounded);
	assert_eq!(unbounded.transient, 0);
	for (limit, refused) in [(peak - 1, true), (peak, false), (peak + 1, false)] {
		let s = prep_with(Some(scaled(|l| l.transient_analysis_bytes = limit)), &code);
		row(&format!("a2_transient_limit_{limit}"), &s);
		assert!(s.analysis_peak_bytes as usize <= limit, "never charged past the limit");
		assert_eq!(s.transient > 0, refused, "limit {limit} (peak {peak})");
		if refused {
			assert!(s.loaded < unbounded.loaded || s.regions < unbounded.regions);
		} else {
			assert_eq!(s.loaded, unbounded.loaded);
		}
		same(&code, 0, &[Arg::I(5)], "transient");
	}

	// (b) A tiny limit refuses every function's scan, not the program.
	let s = prep_with(Some(scaled(|l| l.transient_analysis_bytes = 16)), &code);
	row("a2_transient_tiny", &s);
	assert_eq!((s.transient, s.loaded, s.analysed_functions), (3, 0, 0));
	same(&code, 0, &[Arg::I(5)], "transient tiny");

	// (c) Real constant: the largest function shape the gates admit (65,536 instructions,
	// 64 slots, so 2^22 inference cells) with a 1,000-instruction loop over 60 slots.
	let inner = cmp_loop(56, 940);
	assert!(inner.slots <= 64);
	let mut big = Function::new(64, 1).unwrap();
	let pad = 65_536 - inner.len();
	for pc in 0..pad {
		lit(&mut big, (pc % 64) as u32, Value::I64(1), pc as u32);
	}
	// The loop, shifted by `pad` instructions.
	for (pc, op) in shift(&inner, pad as u32).into_iter().enumerate() {
		big.push(op, loc((pad + pc) as u32)).unwrap();
	}
	assert_eq!(big.len(), 65_536);
	let code = build(vec![big]);
	let s = prep_with(None, &code);
	row("a2_transient_real_8mib_extreme", &s);
	assert!(s.analysis_peak_bytes as usize <= cache::MAX_TRANSIENT_ANALYSIS_BYTES);
	println!(
		"P3ATRANSIENT\tscaled peak={peak} (P-1 refused, P and P+1 admitted)\treal extreme: peak={} of {} transient={} loaded={}",
		s.analysis_peak_bytes,
		cache::MAX_TRANSIENT_ANALYSIS_BYTES,
		s.transient,
		s.loaded
	);
	same(&code, 0, &[Arg::I(3)], "transient real");
	cache::set_limits_for_test(None);
}

/// The operations of `f` with every jump and branch target shifted by `by`.
fn shift(f: &Function, by: u32) -> Vec<Op> {
	(0..f.len())
		.map(|pc| match f.operation(pc).unwrap() {
			Op::Jump { target } => Op::Jump { target: target + by },
			Op::Branch { condition, yes, no } => Op::Branch {
				condition,
				yes: yes + by,
				no: no + by,
			},
			other => other,
		})
		.collect()
}

// ------------------------------------------------- A2: the fixpoint transfer cutoff

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_transfer_cutoff() {
	run_child("child_steps");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_steps() {
	require_child();
	fresh();
	let code = build(vec![fat_loop(16, 64)]);
	let region = region_of(&code);
	let cutoff = 8 * (region.nodes.len()) * region.slots.len();

	// Factor 0: the cutoff is 0, so the region is refused (and the inference abandoned).
	let s = prep_with(Some(scaled(|l| l.transfer_factor = 0)), &code);
	row("a2_steps_factor0", &s);
	assert_eq!((s.steps, s.loaded), (1, 0));
	same(&code, 0, &[Arg::I(4)], "steps 0");

	// Production factor: admitted, and the transfers stay under the cutoff.
	let before = cache::max_transfers_for_test();
	let s = prep_with(None, &code);
	row("a2_steps_factor8", &s);
	let transfers = cache::max_transfers_for_test().max(before);
	assert_eq!((s.steps, s.loaded), (0, 1));
	assert!(transfers <= cutoff, "{transfers} > {cutoff}");
	same(&code, 0, &[Arg::I(4)], "steps 8");

	// The zero-slot corner: a region touching no slot is admitted (cutoff 8 * 1 * 1) and
	// runs to fuel exhaustion exactly like the interpreter.
	let zero = build(vec![zero_slot_loop()]);
	let r = region_of(&zero);
	assert_eq!((r.slots.len(), r.nodes.len()), (0, 1));
	let s = prep_with(None, &zero);
	row("a2_steps_zero_slots", &s);
	assert_eq!((s.steps, s.loaded), (0, 1));
	for fuel in [0, 1, 2, 17] {
		assert_eq!(native(&zero, 0, &[Arg::I(1)], fuel), plain(&zero, 0, &[Arg::I(1)], fuel), "fuel {fuel}");
	}
	println!(
		"P3ASTEPS\tfat_loop(16,64): nodes={} slots={} transfers={transfers} cutoff={cutoff}; factor 0 refuses; zero-slot region admitted",
		region.nodes.len(),
		region.slots.len()
	);
	cache::set_limits_for_test(None);
}

// ------------------------------------- A1: process reservations and page size

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_process_mapping_cap_across_threads() {
	run_child("child_process_threads");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_process_threads() {
	require_child();
	fresh();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// Four threads, process cap 3: exactly three loads, one process_bound, while every
	// thread keeps its entry until all have built.
	let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
	let handles: Vec<_> = (0..4)
		.map(|t| {
			let barrier = barrier.clone();
			std::thread::spawn(move || {
				fresh();
				cache::set_limits_for_test(Some(scaled(|l| l.process_mappings = 3)));
				let code = build(vec![fat_loop(2 + t, 3)]);
				let s = cache::prepare(&code).unwrap();
				barrier.wait();
				let held = cache::process_reserved_for_test();
				barrier.wait();
				same(&code, 0, &[Arg::I(6)], "process threads");
				(s.loaded, s.process_bound, held)
			})
		})
		.collect();
	let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
	let loaded: u32 = results.iter().map(|r| r.0).sum();
	let bound: u32 = results.iter().map(|r| r.1).sum();
	println!("P3APROCESS\tthreads=4 cap=3: loaded={loaded} process_bound={bound} held_at_barrier={:?}", results.iter().map(|r| r.2).collect::<Vec<_>>());
	assert_eq!((loaded, bound), (3, 1));
	assert!(results.iter().all(|r| r.2 == (3, 3 * cache::PAGE_BYTES)));
	assert_eq!(cache::process_reserved_for_test(), (0, 0), "thread exit released every token");

	// Concurrent reserve and drop with real loads: 8 threads, cap 5, build and evict.
	let handles: Vec<_> = (0..8)
		.map(|t| {
			std::thread::spawn(move || {
				fresh();
				cache::set_limits_for_test(Some(scaled(|l| l.process_mappings = 5)));
				let codes: Vec<Code> = (0..3).map(|k| build(vec![fat_loop(1 + (t + k) % 4, 2)])).collect();
				let (mut loaded, mut bound) = (0u32, 0u32);
				for round in 0..40 {
					let code = &codes[round % 3];
					let s = cache::prepare(code).unwrap();
					loaded += s.loaded;
					bound += s.process_bound;
					let (count, bytes) = cache::process_reserved_for_test();
					assert!(count <= 5 && bytes <= 5 * cache::PAGE_BYTES, "({count}, {bytes})");
					if round % 2 == 0 {
						cache::evict(code.id());
					}
				}
				cache::clear_for_test();
				(loaded, bound)
			})
		})
		.collect();
	let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
	println!("P3APROCESS\tconcurrent threads=8 cap=5 rounds=40: (loaded, process_bound) per thread {results:?}");
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// Single thread k - 1, k, k + 1 on both bounds, a program of three one-page regions.
	let code = copies(3, || fat_loop(2, 2));
	for (count, bytes, loaded) in [
		(2usize, usize::MAX, 2u32),
		(3, usize::MAX, 3),
		(4, usize::MAX, 3),
		(usize::MAX, 2 * cache::PAGE_BYTES, 2),
		(usize::MAX, 3 * cache::PAGE_BYTES - 1, 2),
		(usize::MAX, 3 * cache::PAGE_BYTES, 3),
		(usize::MAX, 3 * cache::PAGE_BYTES + 1, 3),
	] {
		let s = prep_with(
			Some(scaled(|l| {
				l.process_mappings = count;
				l.process_mapped_bytes = bytes;
			})),
			&code,
		);
		assert_eq!((s.loaded, s.process_bound), (loaded, 3 - loaded), "count {count} bytes {bytes}");
		assert_eq!(cache::process_reserved_for_test(), (loaded as usize, loaded as usize * cache::PAGE_BYTES));
		same(&code, 0, &[Arg::I(4)], "process k");
	}
	cache::set_limits_for_test(None);
	cache::clear_for_test();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_reservation_released_on_every_path() {
	run_child("child_reservation_paths");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_reservation_paths() {
	require_child();
	fresh();
	let one = build(vec![fat_loop(2, 2)]);
	let page = cache::PAGE_BYTES;

	// Injected load errors: nothing loaded, reservation released, memory freed.
	for stage in [boundary::InjectLoadError::AfterDefine, boundary::InjectLoadError::AfterFinalize] {
		let frees = boundary::load_error_frees_for_test();
		boundary::inject_load_error_for_test(Some(stage));
		let s = prep_with(None, &one);
		boundary::inject_load_error_for_test(None);
		assert_eq!((s.loaded, s.load_refused), (0, 1), "{stage:?}");
		assert_eq!(boundary::load_error_frees_for_test(), frees + 1);
		assert_eq!(cache::process_reserved_for_test(), (0, 0), "{stage:?}");
		assert_eq!(cache::cache_state_for_test().1, 0, "thread mapped charge returned");
		same(&one, 0, &[Arg::I(3)], "load error");
	}

	// Eviction (explicit and LRU) releases the token after the mapping is freed.
	let s = prep_with(None, &copies(5, || fat_loop(2, 2)));
	assert_eq!(s.loaded, 5);
	assert_eq!(cache::process_reserved_for_test(), (5, 5 * page));
	cache::clear_for_test();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));
	let codes: Vec<Code> = (0..cache::MAX_CACHED_PROGRAMS as u32 + 1).map(|k| build(vec![fat_loop(1 + k % 50, 1)])).collect();
	for (k, code) in codes.iter().enumerate() {
		cache::prepare(code).unwrap();
		let expected = (k + 1).min(cache::MAX_CACHED_PROGRAMS);
		assert_eq!(cache::process_reserved_for_test(), (expected, expected * page), "after {k}");
	}
	assert!(!cache::resident_for_test(codes[0].id()), "LRU evicted the first");
	assert_eq!(cache::entry_capacity_for_test(), cache::MAX_CACHED_PROGRAMS, "exact entry table");
	assert!(cache::evict(codes[1].id()));
	assert_eq!(cache::process_reserved_for_test().0, cache::MAX_CACHED_PROGRAMS - 1);
	cache::clear_for_test();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// A build panic (outside the compile's catch_unwind) after two loads: unwinding drops
	// the loaded regions, so their tokens are released and no charge stays.
	let five = copies(5, || fat_loop(2, 2));
	cache::inject_build_panic_after_for_test(Some(2));
	let unwound = std::panic::catch_unwind(|| {
		let _ = cache::checkout(&five);
	});
	cache::inject_build_panic_after_for_test(None);
	assert!(unwound.is_err());
	assert_eq!(cache::cache_state_for_test(), (0, 0, 0, 0));
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// A panic while checked out puts the entry back; the tokens stay with it.
	cache::clear_for_test();
	let unwound = std::panic::catch_unwind(|| {
		let guard = cache::checkout(&five);
		assert!(matches!(guard, Checkout::Native(_)));
		std::panic!("injected (test)");
	});
	assert!(unwound.is_err());
	assert_eq!(cache::process_reserved_for_test(), (5, 5 * page));
	cache::clear_for_test();

	// Thread exit: the thread-local cache is dropped, releasing every token.
	std::thread::spawn(move || {
		fresh();
		let code = build(vec![fat_loop(3, 3)]);
		assert_eq!(cache::prepare(&code).unwrap().loaded, 1);
		assert_eq!(cache::process_reserved_for_test(), (1, page));
	})
	.join()
	.unwrap();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// A panic inside `NativeRegion::load` after the code memory was allocated: the
	// module is dropped by unwinding, which leaks its mapping, so its reservation is
	// leaked too (by design: the pool never undercounts a live mapping). Last, since the
	// leak stays for this child process.
	boundary::inject_load_error_for_test(Some(boundary::InjectLoadError::PanicAfterDefine));
	let s = prep_with(None, &one);
	boundary::inject_load_error_for_test(None);
	assert_eq!((s.loaded, s.load_panicked), (0, 1));
	assert_eq!(cache::process_reserved_for_test(), (1, page), "leaked with its mapping");
	assert_eq!(cache::cache_state_for_test().1, 0, "the thread charge is returned");
	same(&one, 0, &[Arg::I(3)], "load panic");
	println!("P3ARESERVE\tload error x2 released; eviction/LRU released; build panic released; checked-out panic kept; thread exit released; load panic leaked with its mapping (1, {page})");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_page_size_fails_closed() {
	run_child("child_page_size");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_page_size() {
	require_child();
	fresh();
	let code = build(vec![fat_loop(2, 2)]);

	// The real query (once per process, production code path).
	let s = prep_with(None, &code);
	assert_eq!((s.loaded, s.page_size), (1, 0), "this host has 4 KiB pages");

	for size in [16_384usize, 65_536, 4095] {
		cache::set_page_size_for_test(Some(size));
		let s = prep_with(None, &code);
		row(&format!("a1_page_size_{size}"), &s);
		assert_eq!((s.loaded, s.page_size, s.regions), (0, 1, 1));
		assert_eq!(cache::process_reserved_for_test(), (0, 0));
		same(&code, 0, &[Arg::I(5)], "page size");
	}
	cache::set_page_size_for_test(Some(4096));
	assert_eq!(prep_with(None, &code).loaded, 1);
	cache::set_page_size_for_test(None);
	cache::clear_for_test();
	println!("P3APAGE\tsystem page size accepted; 16384/65536/4095 fail closed (page_size refusal, interpreter equal)");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_mapped_bytes_equal_anon_exec_delta() {
	run_child("child_mapped_exact");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_mapped_exact() {
	require_child();
	fresh();
	let page = cache::PAGE_BYTES;
	let (a1, b1lo, b1hi) = adds_crossing(16, page, 1, 16);
	let (a2, b2lo, b2hi) = adds_crossing(16, 2 * page, a1, 32);
	println!("P3AMAP\tcrossings: adds {} -> {b1lo}/{b1hi} bytes; adds {} -> {b2lo}/{b2hi} bytes", a1, a2);
	for adds in [a1 - 1, a1, a2 - 1, a2] {
		cache::clear_for_test();
		let code = build(vec![fat_loop(16, adds)]);
		let before = anon_exec_kib(&read_maps());
		let s = cache::prepare(&code).unwrap();
		let after = anon_exec_kib(&read_maps());
		let expected = (s.code_bytes as usize).div_ceil(page) * page;
		println!(
			"P3AMAP\tadds={adds}\tcode_bytes={}\tmapped_bytes={}\tanon_exec_delta={} KiB\tprocess={:?}",
			s.code_bytes,
			s.mapped_bytes,
			after - before,
			cache::process_reserved_for_test()
		);
		assert_eq!(s.loaded, 1);
		assert_eq!(s.mapped_bytes as usize, expected);
		assert_eq!(((after - before) * 1024) as usize, expected);
		assert_eq!(cache::process_reserved_for_test(), (1, expected));
		cache::clear_for_test();
		assert_eq!(anon_exec_kib(&read_maps()), before, "freed");
	}
}

// --------------------------------------------- A5: the remaining caps, at boundaries

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_compile_attempt_cap_boundary() {
	run_child("child_compile_cap");
}

/// `count` runtime-divisor loops (`t += a / b`), each admitted then refused by the audit.
fn divisor_loops(count: usize) -> Program {
	let mut text = String::new();
	let per = 43;
	let mut made = 0;
	let mut k = 0;
	while made < count {
		let n = per.min(count - made);
		text.push_str(&format!("fn d{k}(a, b) {{ let t = 0;"));
		for _ in 0..n {
			text.push_str(" let j = 0; while j < 2 { t += a / b; j += 1; }");
		}
		text.push_str(" t }\n");
		made += n;
		k += 1;
	}
	text.push_str("pub fn main(a, b) { d0(a, b) }");
	compile_source(&text)
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_compile_cap() {
	require_child();
	fresh();

	// Real constant, 128: k - 1, k, k + 1 attempts.
	for (count, cap) in [(127usize, 0u32), (128, 0), (129, 1)] {
		let program = divisor_loops(count);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let s = prep_with(None, &program.code);
		row(&format!("a5_compile_cap_real_{count}"), &s);
		assert_eq!((s.division_runtime as usize, s.compile_cap, s.loaded), (count.min(128), cap, 0));
		same(&program.code, main, &[Arg::I(7), Arg::I(2)], "compile cap");
	}

	// Scaled cap 4: refused and panicked attempts count too.
	let program = compile_source(
		"pub fn main(a, b) { let t = 0; let j = 0; while j < 2 { t += a / b; j += 1; } let k = 0; while k < 2 { t += a / b; k += 1; } let x = 0; while x < a { x += 1; } let y = 0; while y < a { y += 1; } let z = 0; while z < a { z += 1; } t + x + y + z }",
	);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	cache::inject_compile_panic_for_test(Some(2));
	let s = prep_with(Some(scaled(|l| l.program_compiles = 4)), &program.code);
	cache::inject_compile_panic_for_test(None);
	row("a5_compile_cap_scaled_4", &s);
	assert_eq!((s.division_runtime, s.panicked, s.loaded, s.compile_cap), (2, 1, 1, 1));
	same(&program.code, main, &[Arg::I(7), Arg::I(2)], "compile cap scaled");
	cache::set_limits_for_test(None);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_program_code_bound_boundary() {
	run_child("child_program_code");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_program_code() {
	require_child();
	fresh();

	// Real constant (1 MiB): identical regions just under the 64 KiB region bound.
	let (adds, _, _) = adds_crossing(56, super::MAX_REGION_CODE_BYTES, 64, 128);
	let fat = || fat_loop(56, adds - 1);
	let bytes = code_bytes(56, adds - 1);
	let fit = MAX_PROGRAM_CODE_BYTES / bytes;
	for (count, loaded) in [(fit, fit), (fit + 1, fit), (fit + 2, fit)] {
		let code = copies(count, fat);
		let s = prep_with(None, &code);
		row(&format!("a5_program_code_real_{count}"), &s);
		assert_eq!((s.loaded as usize, s.size as usize), (loaded, count - loaded));
		assert!(s.code_bytes as usize <= MAX_PROGRAM_CODE_BYTES);
		if count == fit + 2 {
			same(&code, 0, &[Arg::I(3)], "program code real");
		}
	}
	println!("P3ACODE\tregion code={bytes} bytes: {fit} fit in {MAX_PROGRAM_CODE_BYTES}; the next is Size, and later regions still attempt");

	// Scaled: three identical small regions, limit 2b - 1, 2b, 2b + 1.
	let b = code_bytes(4, 4);
	let code = copies(3, || fat_loop(4, 4));
	for (limit, loaded) in [(2 * b - 1, 1u32), (2 * b, 2), (2 * b + 1, 2)] {
		let s = prep_with(Some(scaled(|l| l.program_code_bytes = limit)), &code);
		row(&format!("a5_program_code_scaled_{limit}"), &s);
		assert_eq!((s.loaded, s.size), (loaded, 3 - loaded));
		same(&code, 0, &[Arg::I(3)], "program code scaled");
	}
	cache::set_limits_for_test(None);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_program_metadata_bound_boundary() {
	run_child("child_program_metadata");
}

/// The metadata one loaded region of `f` is charged (structure plus the module estimate).
fn charged_metadata(f: impl Fn() -> Function) -> usize {
	let code = build(vec![f()]);
	cache::region_metadata(&region_of(&code)) + cache::MODULE_RESERVE_BYTES
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_program_metadata() {
	require_child();
	fresh();

	// Real constant (1 MiB): exit-heavy, code-light regions.
	let heavy = || fat_loop(1, 129);
	let m = charged_metadata(heavy);
	let fit = cache::MAX_PROGRAM_METADATA_BYTES / m;
	assert!(fit * code_bytes(1, 129) <= MAX_PROGRAM_CODE_BYTES, "the code bound does not bind first");
	assert!(fit < cache::MAX_PROGRAM_REGIONS, "metadata binds before the region cap");
	for (count, loaded) in [(fit, fit), (fit + 1, fit)] {
		let code = copies(count, heavy);
		let s = prep_with(None, &code);
		row(&format!("a5_program_metadata_real_{count}"), &s);
		assert_eq!((s.loaded as usize, s.metadata as usize, s.size), (loaded, count - loaded, 0));
		assert!((s.metadata_bytes + s.reserved_bytes) as usize <= cache::MAX_PROGRAM_METADATA_BYTES);
		if count == fit + 1 {
			same(&code, 0, &[Arg::I(3)], "program metadata real");
		}
	}
	println!("P3AMETA\tprogram: region metadata {m} bytes (incl. module estimate): {fit} fit in {}", cache::MAX_PROGRAM_METADATA_BYTES);

	// Scaled: three regions, limit 2m - 1, 2m, 2m + 1.
	let small = || fat_loop(2, 2);
	let m = charged_metadata(small);
	let code = copies(3, small);
	for (limit, loaded) in [(2 * m - 1, 1u32), (2 * m, 2), (2 * m + 1, 2)] {
		let s = prep_with(Some(scaled(|l| l.program_metadata_bytes = limit)), &code);
		row(&format!("a5_program_metadata_scaled_{limit}"), &s);
		assert_eq!((s.loaded, s.metadata), (loaded, 3 - loaded));
		same(&code, 0, &[Arg::I(3)], "program metadata scaled");
	}
	cache::set_limits_for_test(None);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_thread_metadata_bound_boundary() {
	run_child("child_thread_metadata");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_thread_metadata() {
	require_child();
	fresh();
	let base = cache::thread_reserve_bytes(&Limits::PRODUCTION);

	// Real constant (4 MiB): programs of just under 1 MiB metadata each, built in order.
	let heavy = || fat_loop(1, 129);
	let m = charged_metadata(heavy);
	let per = cache::MAX_PROGRAM_METADATA_BYTES / m;
	let fit = (cache::MAX_THREAD_METADATA_BYTES - base) / (per * m);
	let codes: Vec<Code> = (0..fit + 1).map(|_| copies(per, heavy)).collect();
	cache::clear_for_test();
	for (k, code) in codes.iter().enumerate() {
		let s = cache::prepare(code).unwrap();
		let (_, _, metadata, _) = cache::cache_state_for_test();
		row(&format!("a5_thread_metadata_real_{k}"), &s);
		assert_eq!((s.loaded as usize, s.thread_bound), (per, 0));
		assert!(metadata + base <= cache::MAX_THREAD_METADATA_BYTES);
	}
	assert!(!cache::resident_for_test(codes[0].id()), "the least recently used program was evicted");
	assert!(codes[1..].iter().all(|c| cache::resident_for_test(c.id())));
	same(&codes[fit], 0, &[Arg::I(3)], "thread metadata real");
	println!("P3ATMETA\tthread: base {base} + {fit} programs of {per} regions x {m} bytes fit in {}; program {} evicts the first", cache::MAX_THREAD_METADATA_BYTES, fit + 1);

	// Scaled: limit base + 2m + {-1, 0, +1} with one-region programs P1..P3.
	let small = || fat_loop(2, 2);
	let m = charged_metadata(small);
	for (extra, evicted) in [(2 * m - 1, true), (2 * m, false), (2 * m + 1, false)] {
		let limits = scaled(|l| l.thread_metadata_bytes = base + extra);
		cache::set_limits_for_test(Some(limits));
		cache::clear_for_test();
		let codes: Vec<Code> = (0..2).map(|k| build(vec![fat_loop(2 + k, 2)])).collect();
		for code in &codes {
			assert_eq!(cache::prepare(code).unwrap().loaded, 1);
		}
		assert_eq!(!cache::resident_for_test(codes[0].id()), evicted, "extra {extra}");
	}
	cache::set_limits_for_test(None);
	cache::clear_for_test();
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_thread_mapped_bound_boundary() {
	run_child("child_thread_mapped");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_thread_mapped() {
	require_child();
	fresh();
	let page = cache::PAGE_BYTES;

	// Real constant (4 MiB): 64 regions of two pages per program, so 8 programs fill it
	// and the 9th evicts the least recently used.
	let (adds, _, _) = adds_crossing(16, page, 1, 16);
	let two = move || fat_loop(16, adds);
	let pages = code_bytes(16, adds).div_ceil(page);
	assert_eq!(pages, 2);
	let per = cache::MAX_PROGRAM_REGIONS;
	let fit = cache::MAX_THREAD_MAPPED_BYTES / (per * 2 * page);
	let codes: Vec<Code> = (0..fit + 1).map(|_| copies(per, two)).collect();
	cache::clear_for_test();
	let before = anon_exec_kib(&read_maps());
	for (k, code) in codes.iter().enumerate() {
		let s = cache::prepare(code).unwrap();
		let (_, mapped, _, _) = cache::cache_state_for_test();
		let delta = (anon_exec_kib(&read_maps()) - before) * 1024;
		row(&format!("a5_thread_mapped_real_{k}"), &s);
		assert_eq!((s.loaded as usize, s.thread_bound), (per, 0));
		assert!(mapped <= cache::MAX_THREAD_MAPPED_BYTES);
		assert_eq!(delta as usize, mapped, "mapped == anon-exec delta");
	}
	assert!(!cache::resident_for_test(codes[0].id()));
	same(&codes[fit], 0, &[Arg::I(3)], "thread mapped real");
	cache::clear_for_test();
	assert_eq!(anon_exec_kib(&read_maps()), before);
	println!("P3ATMAP\tthread: {fit} programs of {per} two-page regions fill {}; program {} evicts the first", cache::MAX_THREAD_MAPPED_BYTES, fit + 1);

	// Scaled: limit 3 pages +/- 1 byte, one-page programs; then the test-only seam of
	// holding checkouts (in-flight entries are never evicted) makes thread_bound reachable.
	for (limit, evicted) in [(3 * page - 1, true), (3 * page, false), (3 * page + 1, false)] {
		cache::set_limits_for_test(Some(scaled(|l| l.thread_mapped_bytes = limit)));
		cache::clear_for_test();
		let codes: Vec<Code> = (0..3).map(|k| build(vec![fat_loop(2 + k, 2)])).collect();
		for code in &codes {
			assert_eq!(cache::prepare(code).unwrap().loaded, 1);
		}
		assert_eq!(!cache::resident_for_test(codes[0].id()), evicted, "limit {limit}");
	}
	cache::set_limits_for_test(Some(scaled(|l| l.thread_mapped_bytes = 3 * page)));
	cache::clear_for_test();
	let codes: Vec<Code> = (0..4).map(|k| build(vec![fat_loop(2 + k, 2)])).collect();
	let held: Vec<Checkout> = codes[..3].iter().map(cache::checkout).collect();
	assert!(held.iter().all(|c| matches!(c, Checkout::Native(_))));
	let s = cache::prepare(&codes[3]).unwrap();
	row("a5_thread_bound_seam", &s);
	assert_eq!((s.loaded, s.thread_bound), (0, 1));
	drop(held);
	same(&codes[3], 0, &[Arg::I(3)], "thread bound");
	cache::set_limits_for_test(None);
	cache::clear_for_test();
}

// ------------------------------------------------------ A6: the shared ISA agrees

#[test]
#[cfg_attr(miri, ignore = "Miri: runs Cranelift")]
fn p3a_shared_isa_matches_fresh_isa() {
	let a = lower::shared_isa().unwrap();
	let b = lower::shared_isa().unwrap();
	assert!(std::sync::Arc::ptr_eq(&a, &b), "one ISA per thread");

	let mut compared = 0;
	for (name, src) in golden_programs() {
		let code = src.code();
		for id in 0..code.functions() as u32 {
			if code.function(id).unwrap().len() > cache::MAX_ANALYSED_FUNCTION_LEN {
				continue;
			}
			for (header, region) in analyze(code.function(id).unwrap(), id).unwrap() {
				let Ok(region) = region else { continue };
				let fresh = lower::diagnose_isa(&region, true);
				let shared = lower::diagnose_isa(&region, false);
				match (fresh, shared) {
					(Ok((fb, fa, fl)), Ok((sb, sa, sl))) => {
						assert_eq!((fb, fa), (sb, sa), "{name} f{id} h{header}: bytes");
						assert_eq!(fl.clif, sl.clif);
						assert_eq!(fl.optimized, sl.optimized);
						assert_eq!(fl.vcode, sl.vcode);
						assert_eq!(fl.report, sl.report, "{name} f{id} h{header}: audit");
					}
					(Err(f), Err(s)) => assert_eq!(format!("{f:?}"), format!("{s:?}")),
					(f, s) => std::panic!("{name} f{id} h{header}: {:?} vs {:?}", f.err(), s.err()),
				}
				compared += 1;
			}
		}
	}
	println!("P3AISA\tregions compared fresh vs shared ISA: {compared}, bytes/alignment/listings/audit identical");
	assert!(compared > 200);
}

// ------------------------------------- follow-up: a mid-function analysis error

/// `main` has three loops, `g` one; every candidate header is admitted and loads.
const PARTIAL: &str = "fn g(a) { let n = 0; while n < a { n += 1; } n }
pub fn main(a) { let s = 0; let i = 0; while i < a { s += i; i += 1; } let j = 0; while j < a { s += j * 2; j += 1; } let k = 0; while k < a { s -= k; k += 1; } s + g(a) }";

/// Native equals plain at EVERY fuel from 0 to the plain run's total (and one more): the
/// full observed outcome, including the remaining fuel.
fn same_every_fuel(code: &Code, main: u32, args: &[Arg], name: &str) -> usize {
	let reference = plain(code, main, args, BIG_FUEL);
	assert_eq!(native(code, main, args, BIG_FUEL), reference, "{name}: full fuel");
	let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
	for fuel in 0..=total + 1 {
		assert_eq!(native(code, main, args, fuel), plain(code, main, args, fuel), "{name}: fuel {fuel}");
	}
	assert_eq!(cache::invalid_exits(), 0, "{name}: invalid exit");
	total
}

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p3a_partial_analysis_failure() {
	run_child("child_partial_analysis");
}

#[test]
#[ignore = "P3a native worker: run by its parent test in a child process"]
fn child_partial_analysis() {
	require_child();
	fresh();
	let program = compile_source(PARTIAL);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let code = &program.code;
	let args = [Arg::I(7)];
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// The clean build: four regions, three in `main`, loaded in analysis order (function,
	// then header), so the injection countdown of `main`'s second header is its load index.
	cache::clear_for_test();
	let clean = cache::prepare(code).unwrap();
	row("follow1_partial_clean", &clean);
	assert_eq!(
		(clean.candidates, clean.regions, clean.loaded, clean.analysis_errors),
		(4, 4, 4, 0)
	);
	let clean_regions = cache::regions_for_test(code.id()).unwrap();
	let main_headers: Vec<u32> = clean_regions.iter().filter(|r| r.function == main).map(|r| r.header).collect();
	assert_eq!(main_headers.len(), 3);
	let second = clean_regions
		.iter()
		.position(|r| r.function == main && r.header == main_headers[1])
		.unwrap();
	assert_eq!(cache::budget_used_for_test(), (None, Some(0)), "clean build: no error, nothing left charged");
	cache::clear_for_test();
	assert_eq!(cache::process_reserved_for_test(), (0, 0));

	// The injected build: `main`'s first region loads, then the analysis of its second
	// header fails mid-way (body, ops and slots charged).
	let analysed = cache::analyzed_headers_for_test();
	inject_analysis_error_for_test(Some(second as u32));
	let s = cache::prepare(code).unwrap();
	inject_analysis_error_for_test(None);
	row("follow1_partial_injected", &s);

	// Later affected work refuses cleanly: `main`'s third header is never analysed, the
	// failed header admits nothing; `g` (another function) is unaffected.
	assert_eq!(cache::analyzed_headers_for_test() - analysed, 3, "main h1, main h2 (failed), g");
	assert_eq!((s.analysis_errors, s.accounting_invariant), (1, 0));
	assert_eq!((s.candidates, s.regions, s.loaded), (3, 2, 2));
	let regions = cache::regions_for_test(code.id()).unwrap();
	let kept: Vec<(u32, u32)> = regions.iter().map(|r| (r.function, r.header)).collect();
	assert!(kept.contains(&(main, main_headers[0])), "the earlier region stays: {kept:?}");
	assert!(!kept.iter().any(|&(f, h)| f == main && h != main_headers[0]), "{kept:?}");
	assert_eq!(kept.len(), 2);

	// No incomplete analysis products: at the error the budget held exactly the scan list
	// and the function context; at the end of the build nothing.
	let (at_error, at_end) = cache::budget_used_for_test();
	let (used, expected) = at_error.expect("the injected error was seen");
	assert_eq!(used, expected, "the failed header's buffers were released");
	assert_eq!(at_end, Some(0), "nothing charged after the build");

	// Reservations: process and thread charges are exactly the two loaded regions.
	let mapped = s.mapped_bytes as usize;
	assert_eq!(cache::process_reserved_for_test(), (2, mapped));
	assert_eq!(
		cache::cache_state_for_test(),
		(1, mapped, (s.metadata_bytes + s.reserved_bytes) as usize, 0)
	);

	// Native equals the interpreter at every fuel, and the surviving regions do run.
	let total = same_every_fuel(code, main, &args, "partial");
	let entered = cache::regions_for_test(code.id()).unwrap();
	assert!(entered.iter().all(|r| r.entries > 0), "{entered:?}");

	// Eviction balances every reservation.
	assert!(cache::evict(code.id()));
	assert_eq!(cache::process_reserved_for_test(), (0, 0));
	assert_eq!(cache::cache_state_for_test(), (0, 0, 0, 0));

	// Thread exit balances them too (the same injected build on a fresh thread).
	let source = std::thread::spawn(move || {
		fresh();
		let program = compile_source(PARTIAL);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		inject_analysis_error_for_test(Some(second as u32));
		let s = cache::prepare(&program.code).unwrap();
		inject_analysis_error_for_test(None);
		assert_eq!((s.analysis_errors, s.loaded), (1, 2));
		assert_eq!(cache::process_reserved_for_test(), (2, s.mapped_bytes as usize));
		same(&program.code, main, &[Arg::I(7)], "partial thread");
		s.mapped_bytes
	})
	.join()
	.unwrap();
	assert_eq!(cache::process_reserved_for_test(), (0, 0), "thread exit released every token");

	println!(
		"P3APARTIAL\tmain h{} loaded, h{} injected error (charged {used} == scan+context {expected}), h{} not analysed; g loaded; loaded=2 mapped={mapped} (thread run {source}); native == interpreter at every fuel 0..={}; evict and thread exit -> (0, 0)",
		main_headers[0],
		main_headers[1],
		main_headers[2],
		total + 1
	);
	cache::clear_for_test();
}
