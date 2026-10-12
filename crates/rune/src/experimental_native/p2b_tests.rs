//! P2b: `Copy`, `Release` and `Literal Unit` in native loop regions (test-only).
//!
//! Every test that loads or executes generated code runs in a child process (this test
//! binary re-invoked with `--exact` on an `#[ignore]`d worker that refuses to run unless
//! `RUNE_P2B_NATIVE_CHILD` is set), with stdin null, `PERF_BUILDID_DIR` removed and a
//! deadline, exactly as the P1/P2 harnesses.
//!
//! - P1 hook, every fuel value: plain interpreter, simulator install and native install
//!   compared on the full observation; on every native entry the native and simulated
//!   status, the whole exit record (`units` included) and every written buffer cell are
//!   compared (`exec_tests::run_case`).
//! - Production, every fuel value: the region cache against the plain interpreter
//!   (`p2_tests::production_case`).
//!
//! Output lines prefixed `P2BCASE`, `P2BCOV` and `P2BGOLDEN` are report rows (with the
//! `TABLE` and `P2TABLE` rows of the reused harnesses).
use super::exec_tests::{
	add, arithmetic_inputs, compile_source, ints, run_case, rune, Arg, Case, Src,
};
use super::p2_tests::{production_case, Totals, V2_CALLS};
use super::tests::with_extra;
use super::{audit, audit::Finding};
use crate::experimental_core::{
	code::{Builder, Code, Function},
	region::{
		abi::ExitRecord,
		analyze, cache,
		hook::{self, Stats},
		p2b_tests as built, Exit,
	},
	value::Value,
};
use cranelift_codegen::ir::{types, InstBuilder, MemFlagsData};
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_P2B_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(900);

// ------------------------------------------------------------------ process isolation

fn run_child(inner: &str) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::p2b_tests::{inner}");
	let started = Instant::now();
	let mut child = Command::new(exe)
		.args([
			"--exact",
			name.as_str(),
			"--ignored",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD_ENV, "1")
		.env_remove("PERF_BUILDID_DIR")
		.stdin(Stdio::null())
		.spawn()
		.expect("spawn child");

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

fn require_child() {
	assert!(
		std::env::var_os(CHILD_ENV).is_some(),
		"P2b native worker tests run only in the child process started by their parent test"
	);
}

// ------------------------------------------------------------------------- the cases

fn code(functions: Vec<Function>) -> Code {
	let mut b = Builder::default();
	for f in functions {
		b.push(f).unwrap();
	}
	b.finish().unwrap()
}

fn built_case(
	name: &str,
	f: Function,
	inputs: Vec<Vec<Arg>>,
	loaded: usize,
	targets: &'static str,
) -> Case {
	Case {
		name: name.into(),
		src: Src::Code(code(vec![f])),
		inputs,
		loaded,
		refused: [0; 3],
		targets,
		entry_refusals: false,
	}
}

/// The overflow boundary inputs of a 4-iteration loop.
fn overflow_inputs() -> Vec<Vec<Arg>> {
	let max = i64::MAX;
	let min = i64::MIN;
	let mut inputs = Vec::new();
	for k in 0..4 {
		inputs.push(vec![Arg::I(max - k), Arg::I(1)]);
		inputs.push(vec![Arg::I(min + k), Arg::I(-1)]);
		inputs.push(vec![Arg::I(max / 4 - k), Arg::I(2)]);
		inputs.push(vec![Arg::I(min / 4 + k), Arg::I(-2)]);
	}
	inputs.push(vec![Arg::I(7), Arg::I(0)]);
	inputs.push(vec![Arg::I(min), Arg::I(-1)]);
	inputs
}

/// The region-test listing programs (`region/tests.rs` `LISTED`), verbatim.
const LISTED_3: &str = "pub fn main(a) { let i = 0; let s = 0; while i < a { let t = i * 2; if t > 6 { s = s + t; } i += 1; } s }";
const LISTED_4: &str =
	"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x = x + x; i += b; } x }";
const LISTED_6: &str = "pub fn main(a) { let i = 0; let s = 0; while i < a { let j = 0; while j < i { s += j; j += 1; } i += 1; } s }";
const LISTED_7: &str = "pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j = 0; while j < i { s += j; j += 1; } i += 1; } s }";

/// Every case P2b admits and executes natively.
fn admitted_cases() -> Vec<Case> {
	let mut cases = vec![
		rune(
			"p2b_listed_3_let_t",
			LISTED_3,
			ints(&[&[0], &[5], &[9]]),
			1,
			"CF",
		),
		rune(
			"p2b_listed_4_x_plus_x",
			LISTED_4,
			overflow_inputs(),
			1,
			"CFR",
		),
		rune(
			"p2b_listed_6_nested_let_j",
			LISTED_6,
			ints(&[&[0], &[3], &[6]]),
			2,
			"CF",
		),
		rune(
			"p2b_listed_7_j_reset",
			LISTED_7,
			ints(&[&[0], &[3], &[6]]),
			2,
			"CF",
		),
		rune(
			"p2b_and_and",
			built::AND_AND,
			ints(&[&[0, 5], &[5, 3], &[3, 7], &[6, 6]]),
			1,
			"CF",
		),
		rune(
			"p2b_unit_bool_i64_reuse",
			built::UNIT_BOOL_I64_REUSE,
			ints(&[&[0], &[2], &[6]]),
			1,
			"CF",
		),
		rune(
			"p2b_copy_overflow_retry",
			built::COPY_OVERFLOW_RETRY,
			arithmetic_inputs(),
			1,
			"CFR",
		),
		// The Rem's Retry exit is unreachable here: Cranelift sees the copied constant 17.
		rune(
			"p2b_copied_constant_divisor",
			built::COPY_NOT_A_DIVISOR,
			ints(&[&[0], &[5], &[40]]),
			1,
			"CF",
		),
		built_case(
			"p2b_transitions_i64_bool_unit_i64",
			built::transitions(),
			ints(&[&[0], &[1], &[2], &[3], &[4], &[5], &[6], &[7], &[9]]),
			1,
			"CF",
		),
		built_case(
			"p2b_overlapping_slots",
			built::overlapping(),
			overflow_inputs(),
			1,
			"CFR",
		),
		built_case(
			"p2b_self_copy",
			built::self_copy(),
			ints(&[&[0], &[1], &[3]]),
			1,
			"CF",
		),
		built_case(
			"p2b_literal_unit_ret_u",
			built::unit_and_release(4),
			ints(&[&[0], &[1], &[3], &[9]]),
			1,
			"CF",
		),
		built_case(
			"p2b_release_ret_r",
			built::unit_and_release(5),
			ints(&[&[0], &[1], &[3], &[9]]),
			1,
			"CF",
		),
		built_case(
			"p2b_copy_i64",
			built::copy_loop(Value::I64(3)),
			ints(&[&[0], &[1], &[4]]),
			1,
			"CF",
		),
		built_case(
			"p2b_copy_bool",
			built::copy_loop(Value::Bool(true)),
			ints(&[&[0], &[1], &[4]]),
			1,
			"CF",
		),
		built_case(
			"p2b_copy_unit",
			built::copy_loop(Value::Unit),
			ints(&[&[0], &[1], &[4]]),
			1,
			"CF",
		),
		built_case(
			"p2b_charged_copy_gap",
			built::charged_copy(),
			ints(&[&[0], &[1], &[3]]),
			1,
			"CF",
		),
		built_case(
			"p2b_untouched_f64_type_char_function_at_entry",
			built::untouched_scalars_at_entry(),
			ints(&[&[0], &[1], &[3]]),
			1,
			"CF",
		),
	];
	for (limit, ret) in [(5, 0), (4, 0), (5, 63), (4, 63)] {
		cases.push(built_case(
			&format!("p2b_sixty_four_limit{limit}_ret{ret}"),
			built::sixty_four(limit, ret),
			vec![vec![]],
			1,
			"CF",
		));
	}
	for (kind, name) in [
		(0, "copy_target"),
		(1, "release_target"),
		(2, "copy_source"),
	] {
		let mut case = built_case(
			&format!("p2b_heap_in_{name}_at_entry"),
			built::heap_at_entry(kind),
			ints(&[&[0], &[1], &[5]]),
			1,
			"CF",
		);
		case.entry_refusals = true;
		cases.push(case);
	}
	cases
}

/// Cases P2b refuses on purpose, with the expected analysis refusal of each candidate
/// header. They still run both differentials: zero native entries, equal observations.
fn refused_cases() -> Vec<(Case, Vec<Option<&'static str>>)> {
	vec![
		(
			rune(
				"p2b_copy_of_loop_variable",
				built::COPY_OF_LOOP_VARIABLE,
				ints(&[&[0], &[4]]),
				0,
				"",
			),
			vec![Some("Type")],
		),
		(
			built_case(
				"p2b_copy_from_parameter",
				built::copy_from_live_in(0),
				ints(&[&[0], &[3]]),
				0,
				"",
			),
			vec![Some("CopySource")],
		),
		(
			built_case(
				"p2b_copy_from_maybe_written",
				built::copy_from_live_in(1),
				ints(&[&[0], &[3]]),
				0,
				"",
			),
			vec![Some("CopySource")],
		),
		(
			built_case(
				"p2b_copy_from_top",
				built::copy_from_top(),
				ints(&[&[0], &[3]]),
				0,
				"",
			),
			vec![Some("CopySource")],
		),
		(
			built_case(
				"p2b_released_read_by_add",
				built::released_read(false),
				ints(&[&[0], &[2]]),
				0,
				"",
			),
			vec![Some("Type")],
		),
		(
			built_case(
				"p2b_released_branch_condition",
				built::released_read(true),
				ints(&[&[0], &[2]]),
				0,
				"",
			),
			vec![Some("Type")],
		),
		// B1: the calls kernel (refused here at its IterNext header through P2b) is admitted;
		// its coverage is `b1_tests::b1_calls_kernel_every_fuel`.
	]
}

/// The analysis outcome of every candidate header of every function, as text.
fn outcomes(code: &Code) -> Vec<String> {
	let mut out = Vec::new();
	for id in 0..code.functions() as u32 {
		for (header, region) in analyze(code, id).unwrap() {
			out.push(match region {
				Ok(_) => format!("f{id}h{header}:admitted"),
				Err(refusal) => format!("f{id}h{header}:{refusal:?}"),
			});
		}
	}
	out
}

fn row(
	kind: &str,
	case: &Case,
	outcomes: &[String],
	all: &Stats,
	production: &(usize, Totals, Totals),
) {
	println!(
		"P2BCOV\t{}\t{kind}\tanalysis={}\tloaded={}\trefused[runtime,const0,constneg1]={:?}\tinputs={}\tp1_native_entries={}\tp1_continue={}\tp1_fuel={}\tp1_retry={}\tunit_static={}\tunit_dynamic={}\tentry_refused={}\tmismatches={}\tproduction_points={}\tproduction_entries={}\tproduction_exits[C,F,R]={:?}\tproduction_entry_refused={}",
		case.name,
		outcomes.join(","),
		case.loaded,
		case.refused,
		case.inputs.len(),
		all.native_entries,
		all.native_continue,
		all.native_fuel,
		all.native_retry,
		all.unit_static,
		all.unit_dynamic,
		all.refused_tag + all.refused_store + all.refused_cancel,
		all.mismatches,
		production.0,
		production.2.entries,
		production.2.exits,
		production.2.tag + production.2.store + production.2.cancel,
	);
}

// --------------------------------------------- 23, 24, 25: native every-fuel differential

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn p2b_native_copy_release_differential() {
	run_child("child_p2b_differential");
}

#[test]
#[ignore = "P2b native worker: run by p2b_native_copy_release_differential in a child process"]
fn child_p2b_differential() {
	require_child();
	let mut total = Stats::default();
	let mut points = 0usize;
	let mut cases = 0usize;
	for case in admitted_cases() {
		let analysis = outcomes(case.src.code());
		// P1 hook: plain, simulator and native at every fuel value, native/simulator byte
		// equality on every entry (records with `units`, written buffer cells).
		hook::uninstall();
		let (all, covered) = run_case(&case);
		assert_eq!(all.mismatches, 0, "{}", case.name);
		assert_eq!(all.refused_call, 0, "{}", case.name);
		assert!(covered.native_entries > 0, "{}: no native entry", case.name);
		// Production: the region cache against the plain interpreter at every fuel value.
		let production = production_case(&case);
		assert!(
			production.1.entries > 0,
			"{}: no production entry",
			case.name
		);
		row("admitted", &case, &analysis, &all, &production);
		add(&mut total, &all);
		points += production.0;
		cases += 1;
	}
	println!(
		"P2BCOV\tTOTAL_ADMITTED\tcases={cases}\tproduction_points={points}\tp1_native_entries={}\tp1_continue={}\tp1_fuel={}\tp1_retry={}\tunit_static={}\tunit_dynamic={}\tmismatches={}\tinvalid_exits={}",
		total.native_entries,
		total.native_continue,
		total.native_fuel,
		total.native_retry,
		total.unit_static,
		total.unit_dynamic,
		total.mismatches,
		cache::invalid_exits()
	);
	// 25: a statically Unit-tagged and a dynamically Unit-tagged write-back each happened
	// in native mode.
	assert!(total.unit_static > 0, "no static Unit write-back");
	assert!(total.unit_dynamic > 0, "no dynamic Unit write-back");
	assert_eq!(total.mismatches, 0);
	assert_eq!(cache::invalid_exits(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: compiles and runs generated code in a child process"
)]
fn p2b_intentionally_refused_cases_stay_interpreted() {
	run_child("child_p2b_refused");
}

#[test]
#[ignore = "P2b native worker: run by p2b_intentionally_refused_cases_stay_interpreted in a child process"]
fn child_p2b_refused() {
	require_child();
	let mut cases = 0usize;
	for (case, expected) in refused_cases() {
		let analysis = outcomes(case.src.code());
		assert_eq!(
			analysis.len(),
			expected.len(),
			"{}: {analysis:?}",
			case.name
		);
		for (outcome, want) in analysis.iter().zip(&expected) {
			match want {
				Some(refusal) => assert!(
					outcome.contains(&format!(":{refusal}")),
					"{}: {outcome} is not {refusal}",
					case.name
				),
				None => assert!(outcome.ends_with(":admitted"), "{}: {outcome}", case.name),
			}
		}
		hook::uninstall();
		let (all, _) = run_case(&case);
		assert_eq!(all.native_entries, 0, "{}", case.name);
		let production = production_case(&case);
		assert_eq!(production.2.entries, 0, "{}", case.name);
		row("refused", &case, &analysis, &all, &production);
		cases += 1;
	}
	println!(
		"P2BCOV\tTOTAL_REFUSED\tcases={cases}\tinvalid_exits={}",
		cache::invalid_exits()
	);
	assert_eq!(cache::invalid_exits(), 0);
}

/// The bench kernels after P2b, as in the P3a golden: while and numeric admitted, fib
/// with no loop, shared_rows refused (side entry). B1 changed one row: calls, refused at its
/// IterNext header through P2b, is admitted (native IterNext, `add` inlined).
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn p2b_kernel_admission_unchanged() {
	for (name, text, expect) in [
		("v2/while", super::p2_tests::V2_WHILE, vec!["admitted"]),
		("v2/numeric", super::p2_tests::V2_NUMERIC, vec!["admitted"]),
		("v2/calls", V2_CALLS, vec!["f0h4:admitted"]),
		("v2/fib", super::p2_tests::V2_FIB, vec![]),
		(
			"v2/shared_rows",
			super::p2_tests::V2_SHARED_ROWS,
			vec!["f1h1:SideEntry { pc: 17 }"],
		),
	] {
		let program = compile_source(text);
		let found = outcomes(&program.code);
		assert_eq!(found.len(), expect.len(), "{name}: {found:?}");
		for (f, e) in found.iter().zip(&expect) {
			assert!(f.contains(e), "{name}: {f} vs {e}");
		}
		println!("P2BKERNEL\t{name}\t{}", found.join(","));
	}
	// The calls kernel's header is its IterNext (admitted since B1).
	let program = compile_source(V2_CALLS);
	let main = program.function(crate::Hash::type_hash(["main"])).unwrap();
	assert_eq!(main, 0);
	assert!(matches!(
		program.code.function(0).unwrap().operation(4).unwrap(),
		crate::experimental_core::code::Op::IterNext { .. }
	));
}

// ----------------------------------------------------- 26: the audit's record bound

#[test]
#[cfg_attr(miri, ignore = "Miri: builds Cranelift IR")]
fn p2b_audit_record_bound_is_48() {
	assert_eq!(ExitRecord::SIZE, 48);
	let store_at = |offset: i32| {
		with_extra(move |b, _, rec| {
			let mem = MemFlagsData::trusted();
			let v = b.ins().iconst(types::I64, 0);
			b.ins().store(mem, v, rec, offset);
		})
	};
	assert_eq!(
		audit::ir(
			&store_at(ExitRecord::UNITS),
			&audit::AccessMap::uniform_for_test(2)
		),
		Ok(()),
		"units at 40..48"
	);
	for offset in [44, 48, 56] {
		assert!(
			matches!(
				audit::ir(&store_at(offset), &audit::AccessMap::uniform_for_test(2)),
				Err(Finding::Memory(_))
			),
			"offset {offset}"
		);
	}
	// B1 (exact record field widths): a 4-byte store at 44 lies inside `units`
	// (40..48) and is refused since B1 (P2b's aggregate bound admitted it); at 48 it is past
	// the record either way.
	let store32 = |offset: i32| {
		with_extra(move |b, _, rec| {
			let mem = MemFlagsData::trusted();
			let v = b.ins().iconst(types::I32, 0);
			b.ins().store(mem, v, rec, offset);
		})
	};
	assert!(matches!(
		audit::ir(&store32(44), &audit::AccessMap::uniform_for_test(2)),
		Err(Finding::Memory(_))
	));
	assert!(matches!(
		audit::ir(&store32(48), &audit::AccessMap::uniform_for_test(2)),
		Err(Finding::Memory(_))
	));
}

// --------------------------------------------------- 27: the selection golden delta

/// The selection golden captured from the batch build (unchanged; sha256
/// 2f95b026f6426603c9baf156e39ca78893d3f059d409c40a04bf815bd64d8c9c).
const OLD_GOLDEN: &str = include_str!("fixtures/p3a_selection_golden.txt");
/// The selection golden re-captured at P2b (`RUNE_P3A_GOLDEN_OUT`).
pub(super) const P2B_GOLDEN: &str = include_str!("fixtures/p2b_selection_golden.txt");

/// The enumerated P2b selection delta: (program, function, header, old ANALYSIS outcome,
/// new ANALYSIS outcome, new COMPILE class). Every other ANALYSIS, COMPILE and LOADED line
/// of the old golden is unchanged.
pub(super) const DELTA: &[(&str, u32, u32, &str, &str, &str)] = &[
	("listed_2", 0, 2, "refused\tOp { pc: 10 }", "admitted", "ok"),
	("listed_3", 0, 2, "refused\tOp { pc: 6 }", "admitted", "ok"),
	("listed_5", 0, 2, "refused\tOp { pc: 13 }", "admitted", "ok"),
	("listed_6", 0, 3, "refused\tOp { pc: 6 }", "admitted", "ok"),
];

/// The source of each DELTA program (the golden's `listed_{i}` is `LISTED[i]` of p3a).
fn delta_source(program: &str) -> &'static str {
	match program {
		"listed_2" => LISTED_3,
		"listed_3" => LISTED_4,
		"listed_5" => LISTED_6,
		"listed_6" => LISTED_7,
		other => std::panic!("no source for {other}"),
	}
}

fn blocks(text: &str) -> Vec<(String, Vec<&str>)> {
	let mut out: Vec<(String, Vec<&str>)> = Vec::new();
	for line in text.lines() {
		if let Some(name) = line.strip_prefix("PROGRAM\t") {
			out.push((name.into(), Vec::new()));
		} else {
			out.last_mut().expect("PROGRAM first").1.push(line);
		}
	}
	out
}

fn field(line: &str, key: &str) -> u64 {
	let prefix = format!("{key}=");
	line.split([' ', '\t'])
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

/// `(function, header)` of an `ANALYSIS`, `COMPILE` or `LOADED` line, and the rest.
fn key(line: &str) -> (u32, u32, String) {
	let mut parts = line.split('\t');
	let _kind = parts.next();
	let _program = parts.next();
	let f = parts
		.next()
		.unwrap()
		.strip_prefix('f')
		.unwrap()
		.parse()
		.unwrap();
	let h = parts
		.next()
		.unwrap()
		.strip_prefix('h')
		.unwrap()
		.parse()
		.unwrap();
	(f, h, parts.collect::<Vec<_>>().join("\t"))
}

/// The `(function, header, rest)` of every line of `kind` in a program block.
fn lines(block: &[&str], kind: &str) -> Vec<(u32, u32, String)> {
	block
		.iter()
		.filter(|l| l.starts_with(kind))
		.map(|l| key(l))
		.collect()
}

/// The capacity `BVec::push` growth gives `len` exits (rune_alloc's amortized growth for an
/// element of at most 1,024 bytes: at least 4, doubling).
fn exit_capacity(len: u64) -> u64 {
	let mut cap = 0u64;
	while cap < len {
		cap = 4.max(cap * 2).max(cap + 1);
	}
	cap
}

/// Checks the old golden against the P2b golden: the two documented P3a differences
/// (`metadata_bytes` + one `Token` per loaded region; the capped program), the forced
/// `Exit` growth (`metadata_bytes` + the larger `Exit` per retained exit capacity), and
/// exactly the enumerated `DELTA`. Returns (identical lines, programs in DELTA).
pub(super) fn compare_p2b_delta(old: &str, new: &str) -> (usize, usize) {
	let token =
		core::mem::size_of::<crate::experimental_core::region::budget::Token<'static>>() as u64;
	let growth = core::mem::size_of::<Exit>() as u64 - 48;
	assert_eq!(growth, 8, "Exit grew by one u64 (units)");
	let old = blocks(old);
	let new = blocks(new);
	assert_eq!(old.len(), new.len(), "program count");
	let mut identical = 0;
	let mut delta_programs = 0;
	for ((name, o), (new_name, n)) in old.iter().zip(new.iter()) {
		assert_eq!(name, new_name);
		let delta: Vec<_> = DELTA.iter().filter(|d| d.0 == name).collect();
		if !delta.is_empty() {
			delta_programs += 1;
		}
		// ANALYSIS: same headers in the same order; identical unless enumerated.
		let (oa, na) = (lines(o, "ANALYSIS\t"), lines(n, "ANALYSIS\t"));
		assert_eq!(oa.len(), na.len(), "{name}: analysed headers");
		for (a, b) in oa.iter().zip(na.iter()) {
			assert_eq!((a.0, a.1), (b.0, b.1), "{name}");
			match delta.iter().find(|d| d.1 == a.0 && d.2 == a.1) {
				Some(d) => {
					assert_eq!(a.2, d.3, "{name} f{}h{}: old outcome", a.0, a.1);
					assert!(b.2.starts_with(d.4), "{name} f{}h{}: new {}", a.0, a.1, b.2);
					assert_ne!(a.2, b.2);
				}
				None => {
					assert_eq!(a, b, "{name}: unchanged ANALYSIS");
					identical += 1;
				}
			}
		}
		for d in &delta {
			assert!(
				oa.iter().any(|a| (a.0, a.1) == (d.1, d.2)),
				"{name}: enumerated header exists"
			);
		}

		// COMPILE: every old line unchanged; the new lines are exactly the newly admitted.
		let (oc, nc) = (lines(o, "COMPILE\t"), lines(n, "COMPILE\t"));
		for c in &oc {
			assert!(nc.contains(c), "{name}: COMPILE {c:?} kept");
			identical += 1;
		}
		let added: Vec<_> = nc.iter().filter(|c| !oc.contains(c)).collect();
		let admitted: Vec<_> = delta.iter().filter(|d| d.4 == "admitted").collect();
		assert_eq!(added.len(), admitted.len(), "{name}: new COMPILE lines");
		for d in &admitted {
			let c = added
				.iter()
				.find(|c| (c.0, c.1) == (d.1, d.2))
				.expect("COMPILE line");
			assert!(c.2.starts_with(d.5), "{name}: {c:?}");
		}

		// LOADED: every old line unchanged and in order; the new ones are the newly
		// admitted regions whose compile is `ok`, with its bytes.
		let (ol, nl) = (lines(o, "LOADED\t"), lines(n, "LOADED\t"));
		let kept: Vec<_> = nl.iter().filter(|l| ol.contains(l)).collect();
		assert_eq!(kept.len(), ol.len(), "{name}: old LOADED lines kept");
		assert!(
			kept.iter().zip(ol.iter()).all(|(a, b)| *a == b),
			"{name}: LOADED order"
		);
		identical += ol.len();
		let new_loaded: Vec<_> = nl.iter().filter(|l| !ol.contains(l)).collect();
		let ok: Vec<_> = admitted.iter().filter(|d| d.5 == "ok").collect();
		assert_eq!(new_loaded.len(), ok.len(), "{name}: new LOADED lines");
		let mut new_bytes = 0;
		let mut new_mapped = 0;
		for l in &new_loaded {
			let c = added
				.iter()
				.find(|c| (c.0, c.1) == (l.0, l.1))
				.expect("compiled");
			let bytes = field(&l.2, "code_bytes");
			assert_eq!(field(&c.2, "bytes"), bytes, "{name}: loaded bytes");
			new_bytes += bytes;
			new_mapped += bytes.div_ceil(4096).max(1) * 4096;
		}

		// SUMMARY.
		let os = o.iter().find(|l| l.starts_with("SUMMARY\t")).unwrap();
		let ns = n.iter().find(|l| l.starts_with("SUMMARY\t")).unwrap();
		let loaded = field(os, "loaded");
		let exits_retained: u64 = ol
			.iter()
			.map(|l| {
				let a = oa
					.iter()
					.find(|a| (a.0, a.1) == (l.0, l.1))
					.expect("loaded was admitted");
				exit_capacity(field(&a.2, "exits"))
			})
			.sum();
		let new_metadata: u64 = if new_loaded.is_empty() {
			0
		} else {
			let program = compile_source(delta_source(name));
			new_loaded
				.iter()
				.map(|l| {
					let (_, region) = analyze(&program.code, l.0)
						.unwrap()
						.into_iter()
						.find(|(h, _)| *h == l.1)
						.unwrap();
					// The P2b-era `Ready` (B1's `Region` fields grew it; the fixtures
					// predate B1).
					cache::region_metadata(&region.unwrap()) as u64
						- (cache::ready_bytes_for_test() as u64 - 688)
				})
				.sum()
		};
		assert_eq!(
			field(ns, "metadata_bytes"),
			field(os, "metadata_bytes") + loaded * token + growth * exits_retained + new_metadata,
			"{name}: metadata_bytes"
		);
		assert_eq!(
			field(ns, "loaded"),
			loaded + new_loaded.len() as u64,
			"{name}"
		);
		assert_eq!(
			field(ns, "code_bytes"),
			field(os, "code_bytes") + new_bytes,
			"{name}"
		);
		assert_eq!(
			field(ns, "mapped_bytes"),
			field(os, "mapped_bytes") + new_mapped,
			"{name}"
		);
		let cap_flag = field(ns, "region_cap") + field(ns, "compile_cap");
		let mut keys = vec![
			"metadata_bytes",
			"loaded",
			"code_bytes",
			"mapped_bytes",
			"regions",
		];
		if cap_flag == 0 {
			assert_eq!(
				field(ns, "regions"),
				field(os, "regions") + admitted.len() as u64,
				"{name}"
			);
		} else {
			assert_eq!(cap_flag, 1, "{name}");
			assert!(
				delta.is_empty(),
				"{name}: no P2b delta in the capped program"
			);
			keys.extend([
				"analysed_functions",
				"candidates",
				"region_cap",
				"compile_cap",
			]);
			assert!(field(ns, "analysed_functions") <= field(os, "analysed_functions"));
			assert_eq!(
				field(ns, "regions"),
				field(os, "regions") - field(os, "region_cap") - field(os, "compile_cap"),
				"{name}"
			);
			assert!(field(ns, "candidates") <= field(os, "candidates"));
		}
		assert_eq!(
			without(os, &keys),
			without(ns, &keys),
			"{name}: other SUMMARY counters"
		);
		if delta.is_empty() && cap_flag == 0 {
			assert_eq!(field(ns, "regions"), field(os, "regions"));
		}
	}
	(identical, delta_programs)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn p2b_selection_golden_delta() {
	let (identical, programs) = compare_p2b_delta(OLD_GOLDEN, P2B_GOLDEN);
	assert_eq!(
		programs,
		DELTA.len(),
		"one enumerated header per changed program"
	);
	println!(
		"P2BGOLDEN\told_lines={}\tnew_lines={}\tidentical_analysis_compile_loaded={identical}\tdelta_programs={programs}\tdelta_headers={}",
		OLD_GOLDEN.lines().count(),
		P2B_GOLDEN.lines().count(),
		DELTA.len()
	);
}
