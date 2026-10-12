//! B1: native IterNext over i64 ranges, inlined leaf scalar callees and the
//! explicit Replay checkpoint, EXECUTED (test-only).
//!
//! Every test that loads or executes generated code runs in a child process (this test
//! binary re-invoked with `--exact` on an `#[ignore]`d worker that refuses to run unless
//! `RUNE_B1_NATIVE_CHILD` is set), with stdin null, `PERF_BUILDID_DIR` removed and a
//! deadline, exactly as the P1/P2/P2b harnesses. A signal or a timeout is a failure.
//!
//! "Three modes" at EVERY fuel value 0..=total+1 of every input, each compared with the plain
//! interpreter on the full observation (value or full error, call chain, cleanup error,
//! idle, live entries, the store's whole state, remaining fuel):
//!
//! 1. the P1 hook in simulator mode (with the traced Replay checkpoint check);
//! 2. the P1 hook in native mode, cross-checked against the simulator on every entry (whole
//!    record, every written cell, `next` on every exit, `end` unchanged) and the traced
//!    checkpoint;
//! 3. the production tier (region cache), whose entries, exits by kind and refusals must
//!    equal the P1 native hook's at every point (the resume rule).
//!
//! Native coverage (entries, exits by kind) is reported separately from intentional
//! refusals (Tag, Iter, Room, cancel, compile refusals). Output lines prefixed `B1COV`,
//! `B1CASE` and `B1GOLDEN` are machine-readable report rows.
use super::exec_tests::{compile_source, invoke, Arg, Observed, Outcome, Src, BIG_FUEL, MAX_SWEEP};
use super::{
	audit, audit::AccessMap, audit::Finding, boundary::NativeRegion, lower, NativeRefusal,
};
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Code, Function, Op},
	error::ErrorKind,
	execute::{cancel_at_charge_for_test, Executor},
	frontend,
	heap::{Limits, Store},
	region::{
		abi::ExitRecord,
		analyze, b1_tests as built,
		cache::{self, NativeSummary, State},
		commit::{self, Refused, Room},
		hook::{self, Mode, NativeInstall, Stats},
		simulate::simulate,
		Exit, NodeOp, Region, RegionRefusal, ReplayReason,
	},
	value::{Type, Value},
};
use crate::Hash;
use cranelift_codegen::ir::{types, InstBuilder, MemFlagsData};
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_B1_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(1200);

// ------------------------------------------------------------------ process isolation

fn run_child(inner: &str) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::b1_tests::{inner}");
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
		"B1 native worker tests run only in the child process started by their parent test"
	);
}

// --------------------------------------------------------------------- the harness

fn code_of(functions: Vec<Function>) -> Code {
	let mut b = Builder::default();
	for f in functions {
		b.push(f).unwrap();
	}
	b.finish().unwrap()
}

fn ints(rows: &[&[i64]]) -> Vec<Vec<Arg>> {
	rows.iter()
		.map(|r| r.iter().map(|&v| Arg::I(v)).collect())
		.collect()
}

/// One B1 differential case.
struct B1Case {
	name: String,
	src: Src,
	inputs: Vec<Vec<Arg>>,
	/// Regions expected compiled, audited and loaded (P1 and production).
	loaded: usize,
	/// Compile refusals: runtime divisor, constant 0, constant -1.
	refused: [usize; 3],
	/// Exits targeted over fuel >= 1 in native mode: C continue, D continue at the none edge
	/// (`iter_done`), F fuel, R retry, P Replay(CalleeFuel), Q Replay(CalleeFault).
	targets: &'static str,
	/// Expected entry refusals (each kind must occur): T tag, I iterator, M room, X cancel.
	entry: &'static str,
	/// P1 only: a test-only transform applied to every analysed region (forced Replay).
	transform: Option<fn(&mut Region)>,
	/// Run the production tier too.
	production: bool,
}

fn case(
	name: &str,
	text: &str,
	inputs: Vec<Vec<Arg>>,
	loaded: usize,
	targets: &'static str,
) -> B1Case {
	B1Case {
		name: name.into(),
		src: Src::Program(compile_source(text)),
		inputs,
		loaded,
		refused: [0; 3],
		targets,
		entry: "",
		transform: None,
		production: true,
	}
}

fn built_case(
	name: &str,
	code: Code,
	inputs: Vec<Vec<Arg>>,
	loaded: usize,
	targets: &'static str,
) -> B1Case {
	B1Case {
		name: name.into(),
		src: Src::Code(code),
		inputs,
		loaded,
		refused: [0; 3],
		targets,
		entry: "",
		transform: None,
		production: true,
	}
}

/// Production counters summed over a code's resident regions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Prod {
	entries: u64,
	exits: [u64; 4],
	cancel: u64,
	store: u64,
	tag: u64,
	iter: u64,
	room: u64,
}

fn prod(code: &Code) -> Prod {
	let mut t = Prod::default();
	for r in cache::regions_for_test(code.id()).unwrap_or_default() {
		t.entries += r.entries;
		for k in 0..4 {
			t.exits[k] += r.exits[k];
		}
		t.cancel += r.refused_cancel;
		t.store += r.refused_store;
		t.tag += r.refused_tag;
		t.iter += r.refused_iter;
		t.room += r.refused_room;
	}
	t
}

fn prod_minus(a: Prod, b: Prod) -> Prod {
	let mut d = Prod {
		entries: a.entries - b.entries,
		cancel: a.cancel - b.cancel,
		store: a.store - b.store,
		tag: a.tag - b.tag,
		iter: a.iter - b.iter,
		room: a.room - b.room,
		..Prod::default()
	};
	for k in 0..4 {
		d.exits[k] = a.exits[k] - b.exits[k];
	}
	d
}

fn prod_add(a: &mut Prod, d: Prod) {
	a.entries += d.entries;
	for k in 0..4 {
		a.exits[k] += d.exits[k];
	}
	a.cancel += d.cancel;
	a.store += d.store;
	a.tag += d.tag;
	a.iter += d.iter;
	a.room += d.room;
}

fn stats_add(total: &mut Stats, s: &Stats) {
	super::exec_tests::add(total, s);
}

/// What one case covered.
#[derive(Default)]
struct Cov {
	points: usize,
	install: NativeInstall,
	native_all: Stats,
	native_covered: Stats,
	sim_all: Stats,
	prod_all: Prod,
	prod_covered: Prod,
	analysis: Vec<String>,
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

/// Every refusal kind of a production build other than the division kinds.
fn unexpected(s: &NativeSummary) -> [u32; 12] {
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

fn plain(code: &Code, main: u32, args: &[Arg], fuel: usize) -> Observed {
	hook::suspend();
	cache::enable_for_test(false);
	let o = invoke(code, main, args, fuel);
	cache::enable_for_test(true);
	hook::resume();
	o
}

/// The three modes against the plain interpreter at every fuel value of every input.
fn three_modes(case: &B1Case) -> Cov {
	let code = case.src.code();
	let main = case.src.main();
	let mut cov = Cov {
		analysis: outcomes(code),
		..Cov::default()
	};

	hook::uninstall();
	cache::clear_for_test();
	cache::enable_for_test(true);
	hook::set_region_transform_for_test(case.transform);
	let install = hook::install_native(code, None);
	hook::set_region_transform_for_test(None);
	let refused = [
		install.division_runtime,
		install.division_const0,
		install.division_constneg1,
	];
	assert_eq!(
		install.loaded, case.loaded,
		"{}: loaded {install:?} analysis {:?}",
		case.name, cov.analysis
	);
	assert_eq!(refused, case.refused, "{}: {install:?}", case.name);
	assert_eq!(
		(
			install.size,
			install.codegen,
			install.audit_other,
			install.other,
			install.load_refused
		),
		(0, 0, 0, 0, 0),
		"{}: unexpected compile refusal {install:?}",
		case.name
	);
	cov.install = install;

	let production = case.production && case.transform.is_none();
	let rebuild = case.entry.contains('T') || case.entry.contains('I');
	if production {
		hook::suspend();
		let s = cache::prepare(code).expect("summary");
		hook::resume();
		assert_eq!(
			s.loaded as usize, case.loaded,
			"{}: production {s:?}",
			case.name
		);
		assert_eq!(
			[
				s.division_runtime as usize,
				s.division_const0 as usize,
				s.division_constneg1 as usize
			],
			case.refused,
			"{}",
			case.name
		);
		assert_eq!(unexpected(&s), [0; 12], "{}: {s:?}", case.name);
	}

	for args in &case.inputs {
		let reference = plain(code, main, args, BIG_FUEL);
		if let Outcome::Failure { error, .. } = &reference.outcome {
			assert_ne!(
				error.kind,
				ErrorKind::FuelExhausted,
				"{}: {args:?}",
				case.name
			);
		}
		let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
		assert!(
			total < MAX_SWEEP,
			"{}: {args:?} charges {total}: reduce the case",
			case.name
		);

		for fuel in 0..=total + 1 {
			let p = plain(code, main, args, fuel);

			hook::set_mode(Mode::Simulate);
			hook::take_stats();
			let s = invoke(code, main, args, fuel);
			let sim = hook::take_stats().unwrap();
			if let Some(m) = hook::take_mismatch() {
				std::panic!(
					"SIMULATOR TRACE MISMATCH {} fuel {fuel} {args:?}\n{m}",
					case.name
				);
			}

			hook::set_mode(Mode::Native);
			let n = invoke(code, main, args, fuel);
			let nat = hook::take_stats().unwrap();
			if let Some(m) = hook::take_mismatch() {
				std::panic!("NATIVE MISMATCH {} fuel {fuel} {args:?}\n{m}", case.name);
			}

			assert_eq!(p, s, "{}: simulator, fuel {fuel}, {args:?}", case.name);
			assert_eq!(p, n, "{}: native, fuel {fuel}, {args:?}", case.name);
			assert!(
				p.idle && p.live_entries == 0 || case.name.starts_with("host_root"),
				"{}",
				case.name
			);
			assert_eq!(nat.mismatches, 0, "{}", case.name);
			assert_eq!(
				sim.replay_exits, sim.replay_checked,
				"{}: every simulated Replay traced",
				case.name
			);
			assert_eq!(
				nat.replay_exits, nat.replay_checked,
				"{}: every native Replay traced",
				case.name
			);

			// The simulator and native installs enter and exit identically.
			if case.loaded == install.regions {
				assert_eq!(
					(
						sim.entries,
						sim.continue_exits,
						sim.fuel_exits,
						sim.retry_exits,
						sim.replay_exits
					),
					(
						nat.native_entries,
						nat.native_continue,
						nat.native_fuel,
						nat.native_retry,
						nat.native_replay
					),
					"{}: fuel {fuel}, {args:?}",
					case.name
				);
			}

			if production {
				hook::suspend();
				if rebuild {
					cache::evict(code.id());
				}
				let before = prod(code);
				let r = invoke(code, main, args, fuel);
				let d = prod_minus(prod(code), before);
				hook::resume();
				assert_eq!(p, r, "{}: production, fuel {fuel}, {args:?}", case.name);
				// The resume rule: production enters and exits exactly as the P1 hook.
				if case.loaded == install.regions {
					assert_eq!(
						(d.entries, d.exits, d.tag, d.iter, d.room, d.store, d.cancel),
						(
							nat.native_entries as u64,
							[
								nat.native_continue as u64,
								nat.native_fuel as u64,
								nat.native_retry as u64,
								nat.native_replay as u64
							],
							nat.refused_tag as u64,
							nat.refused_iter as u64,
							nat.refused_room as u64,
							nat.refused_store as u64,
							nat.refused_cancel as u64
						),
						"{}: production vs P1, fuel {fuel}, {args:?}",
						case.name
					);
				}
				prod_add(&mut cov.prod_all, d);
				if fuel >= 1 {
					prod_add(&mut cov.prod_covered, d);
				}
			}

			stats_add(&mut cov.native_all, &nat);
			stats_add(&mut cov.sim_all, &sim);
			if fuel >= 1 {
				stats_add(&mut cov.native_covered, &nat);
			}
			cov.points += 1;
		}
	}
	hook::uninstall();
	assert_eq!(cache::invalid_exits(), 0, "{}: invalid exit", case.name);
	check_coverage(case, &cov);
	row(case, &cov);
	cov
}

fn check_coverage(case: &B1Case, cov: &Cov) {
	let n = &cov.native_covered;
	let name = &case.name;
	assert_eq!(cov.native_all.refused_call, 0, "{name}: refused_call");
	if case.loaded == 0 {
		assert_eq!(cov.native_all.native_entries, 0, "{name}");
		assert_eq!(cov.prod_all.entries, 0, "{name}");
	} else if !case.targets.is_empty() {
		assert!(n.native_entries > 0, "{name}: no native entry");
		if case.production && case.transform.is_none() {
			assert!(cov.prod_covered.entries > 0, "{name}: no production entry");
		}
	}
	let refusals = [
		('T', n.refused_tag),
		('I', n.refused_iter),
		('M', n.refused_room),
		('X', n.refused_cancel),
		('S', n.refused_store),
	];
	for (k, count) in refusals {
		if case.entry.contains(k) {
			assert!(count > 0, "{name}: expected entry refusal {k}");
		} else if case.loaded > 0 {
			assert_eq!(count, 0, "{name}: unexpected entry refusal {k}");
		}
	}
	for (k, count) in [
		('C', n.native_continue),
		('D', n.native_iter_done),
		('F', n.native_fuel),
		('R', n.native_retry),
		('P', n.native_replay_fuel),
		('Q', n.native_replay_fault),
	] {
		if case.targets.contains(k) {
			assert!(count > 0, "{name}: no {k} exit in native mode");
		}
	}
}

fn row(case: &B1Case, cov: &Cov) {
	let n = &cov.native_all;
	println!(
		"B1COV\t{}\tanalysis={}\tloaded={}\trefused[runtime,const0,constneg1]={:?}\tinputs={}\tpoints={}\tnative_entries={}\tcontinue={}\titer_done={}\tfuel={}\tretry={}\treplay_fuel={}\treplay_fault={}\treplay_checked={}\tmismatches={}\tscratch_diffs={}\trefused_tag={}\trefused_iter={}\trefused_room={}\trefused_cancel={}\trefused_store={}\tsim_entries={}\tsim_replay={}\tproduction_entries={}\tproduction_exits[C,F,R,P]={:?}\tproduction_refused[tag,iter,room,cancel,store]={:?}\ttargets={}\tentry={}",
		case.name,
		cov.analysis.join(","),
		cov.install.loaded,
		case.refused,
		case.inputs.len(),
		cov.points,
		n.native_entries,
		n.native_continue,
		n.native_iter_done,
		n.native_fuel,
		n.native_retry,
		n.native_replay_fuel,
		n.native_replay_fault,
		n.replay_checked,
		n.mismatches,
		n.scratch_differences,
		n.refused_tag,
		n.refused_iter,
		n.refused_room,
		n.refused_cancel,
		n.refused_store,
		cov.sim_all.entries,
		cov.sim_all.replay_exits,
		cov.prod_all.entries,
		cov.prod_all.exits,
		[cov.prod_all.tag, cov.prod_all.iter, cov.prod_all.room, cov.prod_all.cancel, cov.prod_all.store],
		if case.targets.is_empty() { "-" } else { case.targets },
		if case.entry.is_empty() { "-" } else { case.entry },
	);
}

fn run_cases(cases: &[B1Case]) -> Vec<Cov> {
	cases.iter().map(three_modes).collect()
}

// ------------------------------------------------------------------- the corpus

const RANGE_SUM: &str = "pub fn main(a, b) { let s = 0; for i in a..b { s += i; } s }";
/// A range loop with a `break` (hand-built: in Rune source, the dead `Jump` the frontend
/// emits after `break` is a predecessor from outside the loop, which the P0 analysis
/// refuses as a SideEntry, independent of B1). `count`: `s += 1; if s == 3 { break }`;
/// otherwise `if i > 4 { break } s += i`. Slots: 0 a, 1 b, 2 s, 3 range, 4 it, 5 i, 6 k,
/// 7 c, 8 three.
fn break_loop(count: bool) -> Code {
	use built::{binary, op};
	let mut f = Function::new(9, 2).unwrap();
	op(
		&mut f,
		Op::Literal {
			out: 2,
			value: Value::I64(0),
		},
		0,
	); // 0
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		1,
	); // 1
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 2); // 2
	op(
		&mut f,
		Op::Literal {
			out: 8,
			value: Value::I64(3),
		},
		3,
	); // 3
	op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 4,
			none: 10,
		},
		4,
	); // 4: header
	if count {
		op(
			&mut f,
			Op::Literal {
				out: 6,
				value: Value::I64(1),
			},
			5,
		); // 5
		op(
			&mut f,
			Op::AssignBinary {
				out: 2,
				op: Binary::Add,
				lhs: 2,
				rhs: 6,
			},
			6,
		); // 6
		binary(&mut f, 7, Binary::Eq, 2, 8, 7); // 7
		op(
			&mut f,
			Op::Branch {
				condition: 7,
				yes: 10,
				no: 9,
			},
			8,
		); // 8: break
	} else {
		op(
			&mut f,
			Op::Literal {
				out: 6,
				value: Value::I64(4),
			},
			5,
		); // 5
		binary(&mut f, 7, Binary::Gt, 5, 6, 6); // 6
		op(
			&mut f,
			Op::Branch {
				condition: 7,
				yes: 10,
				no: 8,
			},
			7,
		); // 7: break
		op(
			&mut f,
			Op::AssignBinary {
				out: 2,
				op: Binary::Add,
				lhs: 2,
				rhs: 5,
			},
			8,
		); // 8
	}
	op(&mut f, Op::Jump { target: 4 }, 9); // 9
	op(&mut f, Op::Release { value: 4 }, 10); // 10
	op(&mut f, Op::Release { value: 3 }, 11); // 11
	op(&mut f, Op::Return { value: 2 }, 12); // 12
	code_of(vec![f])
}

fn range_cases() -> Vec<B1Case> {
	let (min, max) = (i64::MIN, i64::MAX);
	vec![
		case(
			"range_sum",
			RANGE_SUM,
			ints(&[
				&[5, 2],
				&[0, 0],
				&[0, 1],
				&[0, 30],
				&[max - 3, max],
				&[min, min + 3],
				&[-5, 5],
			]),
			1,
			"CDFR",
		),
		built_case(
			"range_break_min_max",
			break_loop(true),
			ints(&[&[min, max], &[0, 2]]),
			1,
			"CDF",
		),
		built_case(
			"range_break_in_if",
			break_loop(false),
			ints(&[&[0, 10], &[0, 3], &[-3, 2]]),
			1,
			"CDF",
		),
	]
}

fn calls_cases() -> Vec<B1Case> {
	vec![case(
		"v2_calls",
		built::V2_CALLS,
		ints(&[&[0], &[1], &[2], &[5], &[30]]),
		1,
		"CDFP",
	)]
}

const F_SRC: &str = "fn f(a, b) { if a < b { a * b } else { a - b } } pub fn main(n, t) { let s = 0; for i in 0..n { s = f(i, t); } s }";
const G_SRC: &str = "fn g(a) { if a > 3 { return a * a; } a + 1 } pub fn main(n, t) { let s = 0; for i in 0..n { s = g(t + i); } s }";
// `s = s + h(i)` would copy the live-in `s` before the call (a P2b Type refusal): the call
// result is bound first.
const H_SRC: &str =
	"fn h(a) { a % 7 } pub fn main(n) { let s = 0; for i in 0..n { let t = h(i); s += t; } s }";
const K_SRC: &str =
	"fn k(a, b) { a / b } pub fn main(n, d) { let s = 0; for i in 0..n { s = k(i, d); } s }";
const LT_SRC: &str = "fn lt(a, b) { a < b } pub fn main(n) { let s = 0; for i in 0..n { if lt(i, 3) { s += 1; } } s }";
const U_SRC: &str = "fn u(a) { } pub fn main(n) { let s = 0; for i in 0..n { u(i); s += 1; } s }";

fn fault_cases() -> Vec<B1Case> {
	let max = i64::MAX;
	let mut k = case(
		"callee_k_runtime_div",
		K_SRC,
		ints(&[&[4, 2], &[3, 0], &[3, -1]]),
		0,
		"",
	);
	k.refused = [1, 0, 0];
	vec![
		// f(i, t): MIN faults on the first iteration (0 - MIN), MAX/2 + 1 on the third
		// (2 * t) of five (middle) and of three (last).
		case(
			"callee_f_two_arms",
			F_SRC,
			ints(&[
				&[5, i64::MIN],
				&[5, max / 2 + 1],
				&[3, max / 2 + 1],
				&[4, 7],
				&[4, -7],
			]),
			1,
			"CDFPQ",
		),
		// g(t + i): sqrt(MAX) ~ 3037000499.98: t faults first, t - 1 second.
		case(
			"callee_g_early_return",
			G_SRC,
			ints(&[
				&[4, 3_037_000_500],
				&[4, 3_037_000_499],
				&[3, 3_037_000_498],
				&[5, -9],
				&[3, max - 1],
			]),
			1,
			"CDFPQ",
		),
		case("callee_h_const_rem", H_SRC, ints(&[&[9], &[0]]), 1, "CDFP"),
		k,
		case(
			"callee_lt_caller_branch",
			LT_SRC,
			ints(&[&[6], &[1]]),
			1,
			"CDFP",
		),
		case(
			"callee_u_unit_return",
			U_SRC,
			ints(&[&[5], &[0]]),
			1,
			"CDFP",
		),
	]
}

const SUB_I_S: &str =
	"fn sub(a, b) { a - b } pub fn main(n, t) { let s = t; for i in 0..n { s = sub(i, s); } s }";
const SUB_S_I: &str =
	"fn sub(a, b) { a - b } pub fn main(n, t) { let s = t; for i in 0..n { s = sub(s, i); } s }";
const ADD_I_I: &str = "fn add(a, b) { a + b } pub fn main(n) { let s = 0; for i in 0..n { let t = add(i, i); s += t; } s }";
const PICK: &str = "fn p0(a, b, c) { a + 0 } fn p1(a, b, c) { b + 0 } fn p2(a, b, c) { c + 0 } pub fn main(n) { let s = 0; for i in 0..n { let x = p0(i, s, 7); let y = p1(i, s, 7); let z = p2(i, s, 7); s = x + y; s += z; } s }";
const UNUSED_PARAM: &str =
	"fn k(a, b) { a + 1 } pub fn main(n, u) { let s = 0; for i in 0..n { s = k(s, u); } s }";
const UNUSED_UNIT: &str = "fn k(a, b) { a + 1 } pub fn main(n) { let u = (); let s = 0; for i in 0..n { s = k(s, u); } s }";

fn argument_cases() -> Vec<B1Case> {
	let (min, max) = (i64::MIN, i64::MAX);
	vec![
		case(
			"arg_sub_i_s",
			SUB_I_S,
			ints(&[&[5, 3], &[3, min + 1], &[3, min]]),
			1,
			"CDFPQ",
		),
		case(
			"arg_sub_s_i",
			SUB_S_I,
			ints(&[&[5, 3], &[3, min + 2]]),
			1,
			"CDFPQ",
		),
		case("arg_add_i_i", ADD_I_I, ints(&[&[6]]), 1, "CDFP"),
		case("arg_pick_each_parameter", PICK, ints(&[&[5]]), 1, "CDFP"),
		case(
			"arg_unused_f64_bool_i64",
			UNUSED_PARAM,
			vec![
				vec![Arg::I(4), Arg::F(1.5)],
				vec![Arg::I(4), Arg::B(true)],
				vec![Arg::I(4), Arg::I(7)],
				vec![Arg::I(2), Arg::I(max)],
			],
			1,
			"CDFP",
		),
		case("arg_unused_unit", UNUSED_UNIT, ints(&[&[4]]), 1, "CDFP"),
	]
}

/// Hand-built output/input aliases around `add` (f1); slots of `built::range_loop`.
fn alias_cases() -> Vec<B1Case> {
	use built::{add_fn, call, op, range_loop};
	let inputs = || ints(&[&[0, 6], &[1, 4], &[i64::MAX - 3, i64::MAX]]);
	let mk = |name: &str, body: fn(&mut Function)| {
		built_case(
			name,
			code_of(vec![range_loop(8, body), add_fn()]),
			inputs(),
			1,
			"CDFPQ",
		)
	};
	vec![
		mk("alias_out_is_arg0", |f| call(f, 2, 1, &[2, 5], 5)),
		mk("alias_out_is_arg1", |f| call(f, 2, 1, &[5, 2], 5)),
		mk("alias_out_is_both_duplicated_args", |f| {
			call(f, 2, 1, &[2, 2], 5);
			// s + i keeps s non-zero so the doubling can overflow.
			op(
				f,
				Op::AssignBinary {
					out: 2,
					op: Binary::Add,
					lhs: 2,
					rhs: 5,
				},
				6,
			);
		}),
		mk("alias_out_is_iternext_binding", |f| {
			call(f, 5, 1, &[2, 5], 5);
			op(f, Op::Copy { out: 2, value: 5 }, 6);
		}),
		mk("alias_copy_target_is_call_temp", |f| {
			call(f, 6, 1, &[2, 5], 5);
			op(f, Op::Copy { out: 2, value: 6 }, 6);
			op(f, Op::Copy { out: 6, value: 5 }, 7);
		}),
	]
}

/// Hand-built return and release shapes.
fn return_release_cases() -> Vec<B1Case> {
	use built::{binary, call, op, range_loop};
	// Releases its first parameter, returns the second plus 0. Slots 0 a, 1 b, 2 r, 3 z.
	let mut releases = Function::new(4, 2).unwrap();
	op(&mut releases, Op::Release { value: 0 }, 100);
	op(
		&mut releases,
		Op::Literal {
			out: 3,
			value: Value::I64(0),
		},
		101,
	);
	binary(&mut releases, 2, Binary::Add, 1, 3, 102);
	op(&mut releases, Op::Return { value: 2 }, 103);
	// Returns a written non-parameter Unit.
	let mut unit = Function::new(3, 2).unwrap();
	op(
		&mut unit,
		Op::Literal {
			out: 2,
			value: Value::Unit,
		},
		100,
	);
	op(&mut unit, Op::Return { value: 2 }, 101);
	let inputs = || ints(&[&[0, 5], &[i64::MAX - 2, i64::MAX]]);
	vec![
		built_case(
			"ret_release_param_return_other",
			code_of(vec![
				range_loop(8, |f| {
					call(f, 6, 1, &[2, 5], 5);
					op(f, Op::Copy { out: 2, value: 6 }, 6);
				}),
				releases,
			]),
			inputs(),
			1,
			"CDFP",
		),
		built_case(
			"ret_written_unit",
			code_of(vec![
				range_loop(8, |f| {
					call(f, 6, 1, &[2, 5], 5);
					op(
						f,
						Op::Literal {
							out: 7,
							value: Value::I64(1),
						},
						6,
					);
					binary(f, 2, Binary::Add, 2, 7, 7);
				}),
				unit,
			]),
			inputs(),
			1,
			"CDFP",
		),
		built_case(
			"ret_copy_then_release",
			code_of(vec![
				range_loop(8, |f| {
					call(f, 6, 1, &[2, 5], 5);
					op(f, Op::Copy { out: 2, value: 6 }, 6);
					op(f, Op::Release { value: 6 }, 7);
				}),
				built::add_fn(),
			]),
			inputs(),
			1,
			"CDFPQ",
		),
		built_case(
			"ret_release_then_reuse_as_bool",
			code_of(vec![
				range_loop(8, |f| {
					call(f, 6, 1, &[2, 5], 5);
					op(f, Op::Copy { out: 2, value: 6 }, 6);
					op(f, Op::Release { value: 6 }, 7);
					binary(f, 6, Binary::Lt, 5, 2, 8);
					op(
						f,
						Op::Branch {
							condition: 6,
							yes: 10,
							no: 10,
						},
						9,
					);
				}),
				built::add_fn(),
			]),
			inputs(),
			1,
			"CDFPQ",
		),
	]
}

const IGNORE_HEAP: &str =
	"fn ignore(a, b) { a + 1 } pub fn main(n) { let t = 0; let s = \"x\"; for i in 0..n { t = ignore(t, s); } t }";

fn heap_argument_cases() -> Vec<B1Case> {
	let mut c = case(
		"heap_unused_argument",
		IGNORE_HEAP,
		ints(&[&[4], &[0]]),
		1,
		"",
	);
	c.entry = "T";
	vec![c]
}

/// `fn(a, b)` iterating `a..b` with the iterator also held at `alias` (slot 7), breaking at
/// `i == 3`, then one IterNext through the alias. Slots: 0 a, 1 b, 2 s, 3 range, 4 it,
/// 5 i, 6 j, 7 alias, 8 c, 9 three, 10 list, 11 zero.
fn alias_untouched(kind: &str) -> Code {
	use built::{binary, op};
	let mut f = Function::new(12, 2).unwrap();
	op(
		&mut f,
		Op::Literal {
			out: 2,
			value: Value::I64(0),
		},
		0,
	); // 0
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		1,
	); // 1
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 2); // 2
	match kind {
		"frame_slot" => {
			op(&mut f, Op::Copy { out: 7, value: 4 }, 3); // 3
		}
		_ => {
			let (args_start, args_len) = f.arguments(&[4]).unwrap();
			op(
				&mut f,
				Op::List {
					out: 10,
					tag: Type::Vec,
					args_start,
					args_len,
				},
				3,
			); // 3
		}
	}
	op(
		&mut f,
		Op::Literal {
			out: 9,
			value: Value::I64(3),
		},
		4,
	); // 4
	op(
		&mut f,
		Op::IterNext {
			out: 5,
			iterator: 4,
			none: 10,
		},
		5,
	); // 5: header
	binary(&mut f, 2, Binary::Add, 2, 5, 6); // 6
	binary(&mut f, 8, Binary::Eq, 5, 9, 7); // 7
	op(
		&mut f,
		Op::Branch {
			condition: 8,
			yes: 10,
			no: 9,
		},
		8,
	); // 8: break
	op(&mut f, Op::Jump { target: 5 }, 9); // 9
	if kind != "frame_slot" {
		op(
			&mut f,
			Op::Literal {
				out: 11,
				value: Value::I64(0),
			},
			10,
		); // 10
		op(
			&mut f,
			Op::Get {
				out: 7,
				value: 10,
				index: 11,
			},
			11,
		); // 11
		op(&mut f, Op::Jump { target: 13 }, 12); // 12
	} else {
		op(&mut f, Op::Jump { target: 13 }, 10); // 10
		op(&mut f, Op::Jump { target: 13 }, 11); // 11
		op(&mut f, Op::Jump { target: 13 }, 12); // 12
	}
	op(
		&mut f,
		Op::IterNext {
			out: 6,
			iterator: 7,
			none: 15,
		},
		13,
	); // 13: through the alias
	binary(&mut f, 2, Binary::Add, 2, 6, 14); // 14
	op(&mut f, Op::Return { value: 2 }, 15); // 15
	code_of(vec![f])
}

/// The iterator handle is a parameter: the caller frame (main) holds an alias.
fn alias_in_caller_frame() -> Code {
	use built::{binary, call, op};
	// L(it): slots 0 it, 1 s, 2 i, 3 c, 4 three.
	let mut l = Function::new(5, 1).unwrap();
	op(
		&mut l,
		Op::Literal {
			out: 1,
			value: Value::I64(0),
		},
		100,
	); // 0
	op(
		&mut l,
		Op::Literal {
			out: 4,
			value: Value::I64(3),
		},
		101,
	); // 1
	op(
		&mut l,
		Op::IterNext {
			out: 2,
			iterator: 0,
			none: 7,
		},
		102,
	); // 2: header
	binary(&mut l, 1, Binary::Add, 1, 2, 103); // 3
	binary(&mut l, 3, Binary::Eq, 2, 4, 104); // 4
	op(
		&mut l,
		Op::Branch {
			condition: 3,
			yes: 7,
			no: 6,
		},
		105,
	); // 5
	op(&mut l, Op::Jump { target: 2 }, 106); // 6
	op(&mut l, Op::Return { value: 1 }, 107); // 7
										   // main(a, b): slots 0 a, 1 b, 2 range, 3 it, 4 r, 5 j.
	let mut m = Function::new(6, 2).unwrap();
	op(
		&mut m,
		Op::Range {
			out: 2,
			start: 0,
			end: 1,
		},
		0,
	); // 0
	op(&mut m, Op::IterStart { out: 3, value: 2 }, 1); // 1
	call(&mut m, 4, 1, &[3], 2); // 2
	op(
		&mut m,
		Op::IterNext {
			out: 5,
			iterator: 3,
			none: 6,
		},
		3,
	); // 3: the caller's alias
	binary(&mut m, 4, Binary::Add, 4, 5, 4); // 4
	op(&mut m, Op::Return { value: 4 }, 5); // 5
	op(&mut m, Op::Return { value: 4 }, 6); // 6
	code_of(vec![m, l])
}

fn alias_untouched_cases() -> Vec<B1Case> {
	let inputs = || ints(&[&[0, 10], &[0, 2], &[5, 1], &[i64::MAX - 5, i64::MAX]]);
	vec![
		built_case(
			"heap_alias_frame_slot",
			alias_untouched("frame_slot"),
			inputs(),
			1,
			"CDF",
		),
		built_case(
			"heap_alias_list_payload",
			alias_untouched("list"),
			inputs(),
			1,
			"CDF",
		),
		built_case(
			"heap_alias_caller_frame",
			alias_in_caller_frame(),
			inputs(),
			1,
			"CDF",
		),
	]
}

/// The handle sits at entry in a touched slot (copied there before the loop). Slots as
/// `built::range_loop`; `add` is f1.
fn alias_touched_cases() -> Vec<B1Case> {
	use built::{add_fn, call, first_plus_one, op};
	let mk = |name: &str, alias: u32, callee: Function, body: fn(&mut Function)| {
		let mut f = built::range_loop(8, body);
		// pc 3 jumps to the header; replace it with the aliasing Copy (falls through).
		f.patch(
			3,
			Op::Copy {
				out: alias,
				value: 4,
			},
		)
		.unwrap();
		let mut c = built_case(
			name,
			code_of(vec![f, callee]),
			ints(&[&[0, 5], &[0, 0]]),
			1,
			"",
		);
		c.entry = "T";
		c
	};
	vec![
		mk("touched_iternext_out", 5, add_fn(), |f| {
			call(f, 2, 1, &[2, 5], 5)
		}),
		mk("touched_call_out", 6, add_fn(), |f| {
			call(f, 6, 1, &[2, 5], 5);
			op(f, Op::Copy { out: 2, value: 6 }, 6);
		}),
		// `k(a, b) { a + 1 }`: the handle sits in the never-read argument.
		mk("touched_unused_call_argument", 7, first_plus_one(), |f| {
			call(f, 6, 1, &[2, 7], 5);
			op(f, Op::Copy { out: 2, value: 6 }, 6);
		}),
		mk("touched_copy_source_overwritten_first", 7, add_fn(), |f| {
			op(
				f,
				Op::Literal {
					out: 7,
					value: Value::I64(1),
				},
				5,
			);
			op(f, Op::Copy { out: 6, value: 7 }, 6);
			call(f, 2, 1, &[2, 6], 7);
		}),
		mk("touched_release_target", 7, add_fn(), |f| {
			op(f, Op::Release { value: 7 }, 5);
			call(f, 2, 1, &[2, 5], 6);
		}),
	]
}

/// `out == iterator` on the header: refused IterSlot, never compiled.
fn iter_out_is_iterator() -> Code {
	use built::op;
	let mut f = Function::new(6, 2).unwrap();
	op(
		&mut f,
		Op::Range {
			out: 3,
			start: 0,
			end: 1,
		},
		0,
	);
	op(&mut f, Op::IterStart { out: 4, value: 3 }, 1);
	op(
		&mut f,
		Op::IterNext {
			out: 4,
			iterator: 4,
			none: 4,
		},
		2,
	);
	op(&mut f, Op::Jump { target: 2 }, 3);
	op(&mut f, Op::Return { value: 0 }, 4);
	code_of(vec![f])
}

/// Every differential case of the B1 corpus (24–38 and 41).
fn corpus() -> Vec<B1Case> {
	let mut all = range_cases();
	all.extend(calls_cases());
	all.extend(fault_cases());
	all.extend(argument_cases());
	all.extend(alias_cases());
	all.extend(return_release_cases());
	all.extend(heap_argument_cases());
	all.extend(alias_untouched_cases());
	all.extend(alias_touched_cases());
	all.extend(branch_checkpoint_cases());
	all
}

/// The B1 corpus as (name, program) pairs (the W13 compile-only layout and edge tests).
pub(super) fn corpus_programs() -> Vec<(String, Src)> {
	corpus().into_iter().map(|c| (c.name, c.src)).collect()
}

// ------------------------------------------------------- 24, 25: ranges and calls

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_range_loops_every_fuel() {
	run_child("child_range_loops");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_range_loops() {
	require_child();
	run_cases(&range_cases());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_calls_kernel_every_fuel() {
	run_child("child_calls_kernel");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_calls_kernel() {
	require_child();
	let covs = run_cases(&calls_cases());
	// The calls kernel admits and enters natively.
	assert!(
		covs[0].native_all.native_entries > 0,
		"calls: native entries"
	);
	assert!(covs[0].prod_all.entries > 0, "calls: production entries");

	// The stage kernel (10,000 iterations, beyond the sweep bound): admitted and loaded, and
	// equal in the three modes at full fuel and at fuels that stop in the header, in the
	// callee's charge and at the end.
	let program = compile_source(built::STAGE_CALLS);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let code = &program.code;
	hook::uninstall();
	cache::clear_for_test();
	let install = hook::install_native(code, None);
	assert_eq!(install.loaded, 1, "stage calls loaded");
	let reference = plain(code, main, &[Arg::I(0)], BIG_FUEL);
	let total = BIG_FUEL - reference.fuel_left.unwrap();
	let mut points = 0;
	for fuel in [
		0,
		1,
		2,
		3,
		4,
		5,
		6,
		7,
		100,
		101,
		10_001,
		total - 2,
		total - 1,
		total,
		total + 1,
		BIG_FUEL,
	] {
		let p = plain(code, main, &[Arg::I(0)], fuel);
		for mode in [Mode::Simulate, Mode::Native] {
			hook::set_mode(mode);
			let o = invoke(code, main, &[Arg::I(0)], fuel);
			let s = hook::take_stats().unwrap();
			assert!(hook::take_mismatch().is_none(), "stage fuel {fuel}");
			assert_eq!(o, p, "stage {mode:?} fuel {fuel}");
			if mode == Mode::Native && fuel >= 3 {
				assert!(s.native_entries > 0, "stage fuel {fuel}: native entry");
			}
		}
		hook::suspend();
		let o = invoke(code, main, &[Arg::I(0)], fuel);
		hook::resume();
		assert_eq!(o, p, "stage production fuel {fuel}");
		points += 1;
	}
	hook::uninstall();
	println!("B1CASE\tstage_calls\tloaded=1\ttotal_charge={total}\tpoints={points} (not every fuel: beyond MAX_SWEEP; v2_calls covers every fuel)");
	assert_eq!(cache::invalid_exits(), 0);
}

// ---------------------------------------------- 26: callee-fuel location and chain

const NESTED_CALLS: &str = "fn add(a, b) { a + b } fn mid(x) { let t = 0; for i in 0..x { t = add(t, i); } t } pub fn main(x) { mid(x) + 1 }";

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_callee_fuel_replay_location_and_chain() {
	run_child("child_callee_fuel");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_callee_fuel() {
	require_child();
	for (name, text, x) in [
		("v2_calls", built::V2_CALLS, 5i64),
		("nested", NESTED_CALLS, 5),
	] {
		let program = compile_source(text);
		let code = &program.code;
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let add = program.function(Hash::type_hash(["add"])).unwrap();
		let loop_fn = if name == "nested" {
			program.function(Hash::type_hash(["mid"])).unwrap()
		} else {
			main
		};
		let lf = code.function(loop_fn).unwrap();
		let call_pc = (0..lf.len())
			.find(
				|&p| matches!(lf.operation(p).unwrap(), Op::Call { function, .. } if function == add),
			)
			.unwrap();
		let add_loc = code.function(add).unwrap().locations[0];
		let mut chain = vec![lf.locations[call_pc]];
		if name == "nested" {
			let mf = code.function(main).unwrap();
			let mid_pc = (0..mf.len())
				.find(|&p| matches!(mf.operation(p).unwrap(), Op::Call { .. }))
				.unwrap();
			chain.push(mf.locations[mid_pc]);
		}

		hook::uninstall();
		cache::clear_for_test();
		assert_eq!(hook::install_native(code, None).loaded, 1);
		let reference = plain(code, main, &[Arg::I(x)], BIG_FUEL);
		let total = BIG_FUEL - reference.fuel_left.unwrap();
		let mut at_callee = 0;
		let mut replays = 0;
		for fuel in 0..=total + 1 {
			let p = plain(code, main, &[Arg::I(x)], fuel);
			hook::set_mode(Mode::Native);
			hook::take_stats();
			let n = invoke(code, main, &[Arg::I(x)], fuel);
			let s = hook::take_stats().unwrap();
			assert!(hook::take_mismatch().is_none());
			assert_eq!(n, p, "{name} fuel {fuel}");
			if let Outcome::Failure {
				error, call_chain, ..
			} = &p.outcome
			{
				if error.kind == ErrorKind::FuelExhausted && error.location == Some(add_loc) {
					at_callee += 1;
					assert_eq!(call_chain, &chain, "{name} fuel {fuel}");
					assert_eq!(p.fuel_left, Some(0));
					// The region committed the CalleeFuel checkpoint (record fuel 0) and the
					// interpreter raised the error in the real callee frame.
					assert_eq!(s.native_replay_fuel, 1, "{name} fuel {fuel}");
					replays += s.native_replay_fuel;
				}
			}
		}
		hook::uninstall();
		println!("B1CASE\tcallee_fuel_{name}\tpoints={}\tfuel_exhausted_at_callee_pc0={at_callee}\treplay_callee_fuel={replays}", total + 2);
		assert!(at_callee > 0 && replays > 0, "{name}");
	}
}

// --------------------------------------------------------- 27..30: inline shapes

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_callee_fault_branches_every_fuel() {
	run_child("child_callee_faults");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_callee_faults() {
	require_child();
	let covs = run_cases(&fault_cases());
	// k: compiled is refused (runtime divisor), the simulator still Replays its faults.
	let k = &covs[3];
	assert_eq!(k.install.division_runtime, 1);
	assert!(k.sim_all.replay_exits > 0, "k: simulated Replay");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_argument_retain_and_order_every_fuel() {
	run_child("child_arguments");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_arguments() {
	require_child();
	run_cases(&argument_cases());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_output_input_alias_every_fuel() {
	run_child("child_aliases");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_aliases() {
	require_child();
	run_cases(&alias_cases());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_return_release_every_fuel() {
	run_child("child_return_release");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_return_release() {
	require_child();
	let covs = run_cases(&return_release_cases());
	assert!(
		covs.iter()
			.map(|c| c.native_all.unit_static + c.native_all.unit_dynamic)
			.sum::<usize>()
			> 0
	);
}

// ---------------------------------------------------- 31: heap argument refusal

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_heap_argument_refuses_entry_every_fuel() {
	run_child("child_heap_argument");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_heap_argument() {
	require_child();
	let covs = run_cases(&heap_argument_cases());
	assert_eq!(covs[0].native_all.native_entries, 0, "never entered");
	assert!(covs[0].native_all.refused_tag > 0);
}

// ---------------------------------------------------- 32, 33: capacity boundaries

const DEEP: &str = "fn add(a, b) { a + b } fn lp(x) { let t = 0; for i in 0..x { t = add(t, i); } t } fn deep(d, x) { if d == 0 { lp(x) } else { deep(d - 1, x) } } pub fn main(d, x) { deep(d, x) }";

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_frame_cap_boundary() {
	run_child("child_frame_cap");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_frame_cap() {
	require_child();
	let program = compile_source(DEEP);
	let code = &program.code;
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let lp = program.function(Hash::type_hash(["lp"])).unwrap();
	let lf = code.function(lp).unwrap();
	let call_pc = (0..lf.len())
		.find(|&p| matches!(lf.operation(p).unwrap(), Op::Call { .. }))
		.unwrap();

	// The depth at which the interpreter's Call in `lp` hits FrameCapacity.
	hook::uninstall();
	let fails = |d: i64| match plain(code, main, &[Arg::I(d), Arg::I(3)], BIG_FUEL).outcome {
		Outcome::Failure { error, .. } => {
			error.kind == ErrorKind::FrameCapacity && error.location == Some(lf.locations[call_pc])
		}
		_ => false,
	};
	let at = (1000..1030)
		.find(|&d| fails(d))
		.expect("a FrameCapacity depth");
	assert!(!fails(at - 1));
	println!(
		"B1CASE\tframe_cap\tFrameCapacity at lp's Call from depth {at}; depth {} succeeds",
		at - 1
	);

	let mut over = case(
		"frame_cap_over",
		DEEP,
		vec![vec![Arg::I(at), Arg::I(3)]],
		1,
		"",
	);
	over.entry = "M";
	let under = case(
		"frame_cap_under",
		DEEP,
		vec![vec![Arg::I(at - 1), Arg::I(3)]],
		1,
		"CDFP",
	);
	let covs = run_cases(&[over, under]);
	assert_eq!(covs[0].native_all.native_entries, 0);
	assert!(covs[0].native_all.refused_room > 0 && covs[0].prod_all.room > 0);
	assert!(covs[1].native_all.native_entries > 0);
}

/// main(x) with `slots` slots calls the calls kernel loop (f1), which calls add (f2).
fn slot_cap(slots: u32) -> Code {
	use built::{add_fn, call, op};
	let mut m = Function::new(slots, 1).unwrap();
	call(&mut m, 1, 1, &[0], 0);
	op(&mut m, Op::Return { value: 1 }, 1);
	let mut l = built::calls_main(&[1, 4]);
	// calls_main calls function 1; here the callee is 2.
	let args = l.arguments(&[1, 4]).unwrap();
	l.patch(
		5,
		Op::Call {
			out: 5,
			function: 2,
			args_start: args.0,
			args_len: args.1,
		},
	)
	.unwrap();
	code_of(vec![m, l, add_fn()])
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_slot_cap_boundary() {
	run_child("child_slot_cap");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_slot_cap() {
	require_child();
	// When the loop runs: main's frame (`slots`) plus the loop's 6 slots are reserved, and
	// add needs 3 more: slots + 6 + 3 <= 65,536 enters.
	let fit = 65_536 - 6 - 3;
	let mut over = built_case(
		"slot_cap_over",
		slot_cap(fit + 1),
		ints(&[&[3], &[0]]),
		1,
		"",
	);
	over.entry = "M";
	let under = built_case(
		"slot_cap_exact",
		slot_cap(fit),
		ints(&[&[3], &[0]]),
		1,
		"CDFP",
	);
	let covs = run_cases(&[over, under]);
	assert_eq!(covs[0].native_all.native_entries, 0);
	assert!(covs[0].native_all.refused_room > 0);
	assert!(covs[1].native_all.native_entries > 0);
}

// ------------------------------------------------------------- 34: cancellation

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_cancel_pending_and_tick() {
	run_child("child_cancel");
}

fn on(executor: &mut Executor, code: &Code, main: u32, x: i64) -> String {
	let mut s = Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap();
	let root = s.root_i64(x);
	match executor.invoke_code(code, &mut s, main, &[&root], BIG_FUEL) {
		Ok(root) => format!("{:?}", s.root_scalar(&root).unwrap()),
		Err(f) => format!(
			"{:?} at {:?} chain {:?}",
			f.error.kind,
			f.error.location,
			f.call_chain.iter().collect::<Vec<_>>()
		),
	}
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_cancel() {
	require_child();
	let program = compile_source(built::V2_CALLS);
	let code = &program.code;
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	hook::uninstall();
	cache::clear_for_test();
	cache::enable_for_test(true);
	assert_eq!(cache::prepare(code).unwrap().loaded, 1);

	// A pending cancel is consumed by the first charged instruction, main's pc 0, which
	// precedes the range header (a range loop's header is never pc 0): the region is never
	// consulted with the request pending; the next invocation enters.
	let run = |production: bool| {
		cache::enable_for_test(production);
		let mut executor = Executor::new().unwrap();
		executor.request_cancel();
		let first = on(&mut executor, code, main, 6);
		let second = on(&mut executor, code, main, 6);
		cache::enable_for_test(true);
		(first, second, executor.is_idle())
	};
	let reference = run(false);
	let before = prod(code);
	let observed = run(true);
	let d = prod_minus(prod(code), before);
	assert_eq!(observed, reference);
	assert!(observed.0.starts_with("Cancelled"), "{}", observed.0);
	assert_eq!(
		(d.cancel, d.entries),
		(0, 1),
		"never consulted while pending; then entered"
	);

	// The commit refuses a pending cancel at entry (the interpreter consumes it at the header).
	let region = analyze(code, main).unwrap().remove(0).1.unwrap();
	let mut s = Store::new(Limits {
		entries: 64,
		bytes: 65_536,
	})
	.unwrap();
	let r = s.range(0, 3).unwrap();
	let it = s.iterator(r).unwrap();
	let mut w = vec![
		Value::I64(3),
		Value::I64(0),
		r,
		it,
		Value::Unit,
		Value::Unit,
	];
	let room = Room {
		frames: 1024,
		slots: 65_536,
	};
	assert_eq!(
		commit::enter(
			&region,
			&mut w,
			&mut s,
			true,
			50,
			room,
			|_, _, _| std::panic!("not entered")
		)
		.unwrap(),
		Err(Refused::Cancel)
	);

	// The armed cancel-at-charge hook disables entry; outcomes stay equal.
	for n in [0, 1, 3, 4, 5, 9] {
		cancel_at_charge_for_test(Some(n));
		let p = plain(code, main, &[Arg::I(6)], BIG_FUEL);
		cancel_at_charge_for_test(Some(n));
		let before = prod(code);
		let o = invoke(code, main, &[Arg::I(6)], BIG_FUEL);
		cancel_at_charge_for_test(None);
		assert_eq!(o, p, "tick {n}");
		assert_eq!(
			prod_minus(prod(code), before),
			Prod::default(),
			"tick {n}: no entry while armed"
		);
	}
	let o = invoke(code, main, &[Arg::I(6)], BIG_FUEL);
	assert_eq!(o, plain(code, main, &[Arg::I(6)], BIG_FUEL));
	println!("B1CASE\tcancel\tpending consumed at main pc 0 ({}), next invocation entered; armed tick: no entry", observed.0);
	assert_eq!(cache::invalid_exits(), 0);
}

// ------------------------------------------------------- 35..38: aliases, vectors

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_heap_alias_untouched_sees_committed_next() {
	run_child("child_alias_untouched");
}

/// One invocation of `code`'s function 1 (`L(it)`) with the iterator handed in as a host
/// root, which keeps an alias (strong 2) for the whole run.
fn host_root_alias(
	code: &Code,
	start: i64,
	end: i64,
	fuel: usize,
) -> (Observed, Option<(i64, i64)>) {
	let mut s = Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap();
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	s.release(range).unwrap();
	let root = s.export(it);
	let mut executor = Executor::new().unwrap();
	hook::take_final_fuel();
	let outcome = match executor.invoke_code(code, &mut s, 1, &[&root], fuel) {
		Ok(r) => Outcome::Value(s.root_scalar(&r).unwrap()),
		Err(f) => Outcome::Failure {
			error: f.error,
			call_chain: f.call_chain.iter().copied().collect(),
			cleanup_error: f.cleanup_error,
		},
	};
	let fuel_left = hook::take_final_fuel();
	let next = s.range_iterator_for_native(root.value);
	(
		Observed {
			outcome,
			live_entries: s.live_entries(),
			idle: executor.is_idle(),
			fuel_left,
			store_state: s.state_for_test(),
		},
		next,
	)
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_alias_untouched() {
	require_child();
	run_cases(&alias_untouched_cases());

	// A host root alias: the host observes the committed `next` after the run.
	let code = alias_in_caller_frame();
	hook::uninstall();
	cache::clear_for_test();
	assert_eq!(hook::install_native(&code, None).loaded, 1);
	let mut points = 0;
	for (start, end) in [(0, 10), (0, 2), (5, 1)] {
		hook::suspend();
		cache::enable_for_test(false);
		let (reference, _) = host_root_alias(&code, start, end, BIG_FUEL);
		cache::enable_for_test(true);
		hook::resume();
		let total = BIG_FUEL - reference.fuel_left.unwrap();
		for fuel in 0..=total + 1 {
			hook::suspend();
			cache::enable_for_test(false);
			let p = host_root_alias(&code, start, end, fuel);
			cache::enable_for_test(true);
			hook::resume();
			for mode in [Mode::Simulate, Mode::Native] {
				hook::set_mode(mode);
				let o = host_root_alias(&code, start, end, fuel);
				assert!(hook::take_mismatch().is_none());
				assert_eq!(o, p, "host root {start}..{end} {mode:?} fuel {fuel}");
			}
			hook::suspend();
			let o = host_root_alias(&code, start, end, fuel);
			hook::resume();
			assert_eq!(o, p, "host root production fuel {fuel}");
			points += 1;
		}
	}
	hook::uninstall();
	println!("B1CASE\theap_alias_host_root\tpoints={points}\tthe host root's next equals the interpreter's at every fuel");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_heap_alias_in_touched_slot_refuses_entry() {
	run_child("child_alias_touched");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_alias_touched() {
	require_child();
	run_cases(&alias_touched_cases());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_iter_out_equals_iterator_slot_never_compiled() {
	run_child("child_iter_out_is_iterator");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_iter_out_is_iterator() {
	require_child();
	let code = iter_out_is_iterator();
	assert_eq!(
		analyze(&code, 0).unwrap()[0].1.as_ref().err(),
		Some(&RegionRefusal::IterSlot { pc: 2 })
	);
	let c = built_case(
		"iter_out_is_iterator",
		code,
		ints(&[&[0, 3], &[0, 0]]),
		0,
		"",
	);
	let covs = run_cases(&[c]);
	assert_eq!(covs[0].install.regions, 0);
}

const VECTOR_LOOP: &str = "pub fn main(n) { let s = 0; let k = 0; while k < n { for x in [1, 2, 3] { s += 1; } k += 1; } s }";

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_vector_iterator_loop_goes_cold() {
	run_child("child_vector_cold");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_vector_cold() {
	require_child();
	let mut c = case("vector_iterator", VECTOR_LOOP, ints(&[&[3], &[0]]), 1, "");
	c.entry = "I";
	let covs = run_cases(&[c]);
	assert_eq!(covs[0].native_all.native_entries, 0);

	// Production: Cold after COLD_AFTER_REFUSALS consecutive Iter refusals, outcome equal.
	let program = compile_source(VECTOR_LOOP);
	let code = &program.code;
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	hook::uninstall();
	cache::clear_for_test();
	let n = i64::from(cache::COLD_AFTER_REFUSALS) + 6;
	let p = plain(code, main, &[Arg::I(n)], BIG_FUEL);
	let o = invoke(code, main, &[Arg::I(n)], BIG_FUEL);
	assert_eq!(o, p);
	let r = &cache::regions_for_test(code.id()).unwrap()[0];
	assert_eq!(r.state, State::Cold, "{r:?}");
	assert_eq!(r.refused_iter, u64::from(cache::COLD_AFTER_REFUSALS));
	assert_eq!(r.entries, 0);
	println!(
		"B1CASE\tvector_iterator_cold\touter_iterations={n}\trefused_iter={}\tstate={:?}",
		r.refused_iter, r.state
	);
}

// ------------------------------------------------- 39, 40: byte equality, coverage

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_native_simulator_byte_equal() {
	run_child("child_byte_equal");
}

/// P1 native mode over the whole corpus at every fuel: the hook cross-checks every entry
/// (record, written cells, `next` on every exit, `end` unchanged) and every Replay's traced
/// checkpoint.
#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_byte_equal() {
	require_child();
	let mut total = Stats::default();
	let mut points = 0;
	for case in corpus() {
		let code = case.src.code();
		let main = case.src.main();
		hook::uninstall();
		hook::install_native(code, None);
		hook::set_mode(Mode::Native);
		for args in &case.inputs {
			let reference = plain(code, main, args, BIG_FUEL);
			let t = BIG_FUEL - reference.fuel_left.unwrap();
			for fuel in 0..=t + 1 {
				hook::take_stats();
				let o = invoke(code, main, args, fuel);
				let s = hook::take_stats().unwrap();
				if let Some(m) = hook::take_mismatch() {
					std::panic!("NATIVE MISMATCH {} fuel {fuel} {args:?}\n{m}", case.name);
				}
				assert_eq!(
					o,
					plain(code, main, args, fuel),
					"{} fuel {fuel}",
					case.name
				);
				stats_add(&mut total, &s);
				points += 1;
			}
		}
		hook::uninstall();
	}
	println!(
		"B1COV\tTOTAL_BYTE_EQUAL\tpoints={points}\tnative_entries={}\tmismatches={}\treplay={}\treplay_checked={}\tscratch_diffs={}",
		total.native_entries, total.mismatches, total.native_replay, total.replay_checked, total.scratch_differences
	);
	assert_eq!(total.mismatches, 0);
	assert!(total.native_entries > 0 && total.native_replay > 0);
	assert_eq!(total.native_replay, total.replay_checked);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_exit_coverage() {
	run_child("child_exit_coverage");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_exit_coverage() {
	require_child();
	let cases = corpus();
	let covs = run_cases(&cases);
	let mut native = Stats::default();
	let mut refusals = (0usize, 0usize, 0usize);
	for (case, cov) in cases.iter().zip(&covs) {
		stats_add(&mut native, &cov.native_covered);
		refusals.0 += cov.native_all.refused_tag;
		refusals.1 += cov.native_all.refused_iter;
		refusals.2 += cov.native_all.refused_room;
		if case.loaded > 0 && !case.targets.is_empty() {
			assert!(cov.native_covered.native_entries > 0, "{}", case.name);
		}
	}
	println!(
		"B1COV\tTOTAL\tcases={}\tnative_entries={}\tcontinue={}\titer_done={}\tfuel={}\tretry={}\treplay_fuel={}\treplay_fault={}\tintentional_refusals[tag,iter,room]={:?}\tinvalid_exits={}",
		cases.len(),
		native.native_entries,
		native.native_continue,
		native.native_iter_done,
		native.native_fuel,
		native.native_retry,
		native.native_replay_fuel,
		native.native_replay_fault,
		refusals,
		cache::invalid_exits()
	);
	assert!(
		native.native_continue > native.native_iter_done && native.native_iter_done > 0,
		"iter_done and break"
	);
	assert!(native.native_fuel > 0 && native.native_retry > 0);
	assert!(native.native_replay_fuel > 0 && native.native_replay_fault > 0);
	assert_eq!(cache::invalid_exits(), 0);
}

// ---------------------------------------------------- 41, 42: checkpoint survival

const BRANCH_CHECKPOINT: &str = "fn f(a, b) { let t = a * 2; if a < b { t * b } else { let u = b - 1; u * t } } pub fn main(n, b) { let s = 0; for i in 0..n { s = f(i, b); } s }";

fn branch_checkpoint_cases() -> Vec<B1Case> {
	let (min, max) = (i64::MIN, i64::MAX);
	vec![
		case(
			"checkpoint_two_arms",
			BRANCH_CHECKPOINT,
			ints(&[&[4, max / 2], &[4, min / 2], &[4, min], &[4, 3]]),
			1,
			"CDFPQ",
		),
		built_case(
			"checkpoint_three_returns",
			code_of(vec![
				built::range_loop(8, |f| {
					built::call(f, 6, 1, &[5], 5);
					built::op(f, Op::Copy { out: 2, value: 6 }, 6);
				}),
				built::three_returns(),
			]),
			ints(&[
				&[0, 4],
				&[0, 2],
				&[-1, 3],
				&[i64::MAX - 3, i64::MAX],
				&[i64::MIN, i64::MIN + 2],
			]),
			1,
			"CDFPQ",
		),
	]
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_checkpoint_survives_callee_branches_every_fuel() {
	run_child("child_checkpoint_branches");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_checkpoint_branches() {
	require_child();
	let covs = run_cases(&branch_checkpoint_cases());
	for c in &covs {
		assert!(c.native_all.replay_checked > 0);
		assert_eq!(c.native_all.replay_checked, c.native_all.native_replay);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_forced_replay_protocol_every_fuel() {
	run_child("child_forced_replay");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_forced_replay() {
	require_child();
	let mut cases = vec![
		case(
			"forced_v2_calls",
			built::V2_CALLS,
			ints(&[&[0], &[3], &[7]]),
			1,
			"CDFPQ",
		),
		case(
			"forced_two_arms",
			BRANCH_CHECKPOINT,
			ints(&[&[4, 3]]),
			1,
			"CDFPQ",
		),
		// No genuine CalleeFault exit (a Compare callee): forced only at checkpoint 0.
		case("forced_lt_no_fault_exit", LT_SRC, ints(&[&[4]]), 1, "CDFP"),
	];
	for c in &mut cases {
		c.transform = Some(hook::force_replay);
	}
	let covs = run_cases(&cases);
	// Every call with checkpoint > 0 Replays: the interpreter then runs the callee, which
	// succeeds, and the outcome is still equal.
	assert!(covs[0].native_all.native_replay_fault > covs[0].native_all.native_replay_fuel);

	// Production validation is unchanged on the transformed region: a CalleeFuel record
	// with fuel 1 is refused.
	let program = compile_source(built::V2_CALLS);
	let mut region = analyze(&program.code, 0).unwrap().remove(0).1.unwrap();
	hook::force_replay(&mut region);
	assert!(region
		.nodes
		.iter()
		.any(|n| matches!(n.op, NodeOp::ForceReplay { .. })));
	let mut s = Store::new(Limits {
		entries: 64,
		bytes: 65_536,
	})
	.unwrap();
	let r = s.range(0, 3).unwrap();
	let it = s.iterator(r).unwrap();
	let mut w = vec![
		Value::I64(3),
		Value::I64(0),
		r,
		it,
		Value::Unit,
		Value::Unit,
	];
	let before = (w.clone(), s.state_for_test());
	let room = Room {
		frames: 1024,
		slots: 65_536,
	};
	let result = commit::enter(&region, &mut w, &mut s, false, 1, room, |r, b, rec| {
		let status = simulate(r, b, rec);
		assert_eq!(region_exit(r, rec).replay, Some(ReplayReason::CalleeFuel));
		rec.fuel = 1;
		status
	})
	.unwrap();
	assert_eq!(result, Err(Refused::InvalidExit));
	assert_eq!((w, s.state_for_test()), before);

	fn region_exit(r: &Region, rec: &ExitRecord) -> Exit {
		r.exits[rec.exit as usize]
	}
}

// ----------------------------------------------------------- 43: the resume rule

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_resume_rule_entry_counts() {
	run_child("child_resume_rule");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_resume_rule() {
	require_child();
	// `three_modes` asserts, at every fuel point, that production entries, exits by kind
	// and refusals equal the P1 native hook's. Here: the calls and branch corpus, and the
	// static half: every inlined Call pc is uncharged, so a Replay's resume pc never
	// consults either hook.
	let mut cases = calls_cases();
	cases.extend(branch_checkpoint_cases());
	cases.extend(range_cases());
	for c in &cases {
		let code = c.src.code();
		for id in 0..code.functions() as u32 {
			let f = code.function(id).unwrap();
			for (_, region) in analyze(code, id).unwrap() {
				let Ok(region) = region else { continue };
				for site in region.sites.iter() {
					assert!(
						!f.cached_ref(site.pc as usize).unwrap().charged,
						"{}: Call pc {} charged",
						c.name,
						site.pc
					);
					for e in [Some(site.fuel_exit), site.fault_exit]
						.into_iter()
						.flatten()
					{
						assert_eq!(region.exits[e as usize].pc, site.pc);
					}
				}
			}
		}
	}
	let covs = run_cases(&cases);
	let entries: u64 = covs.iter().map(|c| c.prod_all.entries).sum();
	println!("B1CASE\tresume_rule\tcases={}\tproduction_entries={entries} (equal to the P1 hook's at every point)", cases.len());
	assert!(entries > 0);
}

// ----------------------------------------------------- 45..47: audit, boundary

/// A hand-built function with one extra instruction (the audit's cases).
fn with_extra(
	extra: impl FnOnce(
		&mut cranelift_frontend::FunctionBuilder<'_>,
		cranelift_codegen::ir::Value,
		cranelift_codegen::ir::Value,
	),
) -> cranelift_codegen::ir::Function {
	super::tests::with_extra(extra)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: builds Cranelift IR")]
fn b1_audit_per_cell_permissions() {
	let region = analyze(&built::calls_kernel(), 0)
		.unwrap()
		.remove(0)
		.1
		.unwrap();
	let n = region.slots.len() as i32;
	assert_eq!(n, 3);
	let map = AccessMap::of(&region);
	let mem = MemFlagsData::trusted();
	let load = |offset: i32| {
		with_extra(move |b, buf, _| {
			b.ins().load(types::I64, mem, buf, offset);
		})
	};
	let store = |offset: i32| {
		with_extra(move |b, buf, _| {
			let v = b.ins().iconst(types::I64, 0);
			b.ins().store(mem, v, buf, offset);
		})
	};
	let rec_load = |offset: i32, ty: cranelift_codegen::ir::Type| {
		with_extra(move |b, _, rec| {
			b.ins().load(ty, mem, rec, offset);
		})
	};
	let rec_store = |offset: i32, ty: cranelift_codegen::ir::Type| {
		with_extra(move |b, _, rec| {
			let v = b.ins().iconst(ty, 0);
			b.ins().store(mem, v, rec, offset);
		})
	};
	let ok = |f: &cranelift_codegen::ir::Function, what: &str| {
		assert_eq!(audit::ir(f, &map), Ok(()), "{what}")
	};
	let refused = |f: &cranelift_codegen::ir::Function, what: &str| {
		assert!(
			matches!(audit::ir(f, &map), Err(Finding::Memory(_))),
			"{what}: {}",
			f.display()
		)
	};

	// Cell 0 is slot 1 (`total`): a Guard live-in that is also written: load AND store.
	ok(&load(0), "load at a Guard cell");
	ok(&store(0), "store at a Guard cell that is written (union)");
	ok(&store(8), "store at a possibly-written Scalar cell");
	ok(&store(16), "store at a possibly-written Scalar cell");
	ok(&load(8 * n), "load of next");
	ok(&store(8 * n), "store of next");
	ok(&load(8 * (n + 1)), "load of end");
	ok(&rec_load(ExitRecord::FUEL, types::I64), "record fuel load");
	for (offset, ty) in [
		(ExitRecord::FUEL, types::I64),
		(ExitRecord::WRITTEN, types::I64),
		(ExitRecord::BOOLS, types::I64),
		(ExitRecord::STATUS, types::I32),
		(ExitRecord::EXIT, types::I32),
		(ExitRecord::PC, types::I32),
		(ExitRecord::UNITS, types::I64),
	] {
		ok(&rec_store(offset, ty), "record out field store");
	}

	refused(&load(8), "load at a Scalar-class cell");
	refused(&store(8 * (n + 1)), "store of end");
	refused(&load(8 * (n + 2)), "load past the cells");
	refused(&store(8 * (n + 2)), "store past the cells");
	refused(&load(4), "unaligned");
	refused(
		&with_extra(move |b, buf, _| {
			b.ins().load(types::I32, mem, buf, 0);
		}),
		"4-byte cell access",
	);
	refused(
		&rec_load(ExitRecord::WRITTEN, types::I64),
		"load of record written",
	);
	refused(&rec_store(36, types::I32), "store to reserved");
	refused(
		&rec_store(24, types::I64),
		"8-byte store spanning status and exit",
	);
	refused(
		&rec_store(ExitRecord::STATUS, types::I64),
		"8-byte store at the 4-byte status",
	);
	refused(
		&rec_store(ExitRecord::UNITS + 4, types::I32),
		"4-byte store inside units",
	);
	refused(&rec_store(48, types::I64), "past the record");

	// A cell that is touched but never written and not a live-in: no access at all.
	let unused = analyze(&built_calling_unused(), 0)
		.unwrap()
		.remove(0)
		.1
		.unwrap();
	let l = unused.slots.iter().position(|&s| s == 7).unwrap() as i32;
	let map2 = AccessMap::of(&unused);
	for f in [load(8 * l), store(8 * l)] {
		assert!(matches!(audit::ir(&f, &map2), Err(Finding::Memory(_))));
	}

	fn built_calling_unused() -> Code {
		code_of(vec![
			built::range_loop(8, |f| {
				built::call(f, 6, 1, &[2, 7], 5);
				built::op(f, Op::Copy { out: 2, value: 6 }, 6);
			}),
			built::first_plus_one(),
		])
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn b1_audit_accepts_every_corpus_region() {
	let mut programs: Vec<(String, Src)> = Vec::new();
	for c in corpus() {
		programs.push((c.name, c.src));
	}
	for c in super::exec_tests::corpus_cases()
		.into_iter()
		.chain(super::exec_tests::new_cases())
		.chain(super::p2_tests::bench_kernel_cases())
	{
		programs.push((c.name, c.src));
	}
	let mut compiled = 0;
	for (name, src) in &programs {
		let code = src.code();
		for id in 0..code.functions() as u32 {
			for (_, region) in analyze(code, id).unwrap() {
				let Ok(region) = region else { continue };
				match lower::compile(&region, 0) {
					Ok(c) => {
						assert_eq!(c.cells(), region.cells(), "{name}");
						compiled += 1;
					}
					Err(NativeRefusal::Audit(Finding::Division(_))) => (),
					Err(other) => std::panic!("{name} f{id} h{}: {other:?}", region.header),
				}
			}
		}
	}
	println!(
		"B1CASE\taudit_corpus\tprograms={}\tcompiled={compiled}\tno Audit(Memory) refusal",
		programs.len()
	);
	assert!(compiled > 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn b1_boundary_rejects_wrong_cell_count() {
	run_child("child_boundary_cells");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_boundary_cells() {
	require_child();
	let region = analyze(&built::calls_kernel(), 0)
		.unwrap()
		.remove(0)
		.1
		.unwrap();
	let compiled = lower::compile(&region, 0).unwrap();
	assert_eq!(compiled.cells(), 5);
	let native = NativeRegion::load(&compiled).unwrap();
	let n = region.slots.len();
	for len in [n, n + 1, n + 3, 0] {
		let mut buf = vec![0i64; len];
		let mut record = ExitRecord::new(10);
		assert_eq!(native.call(&mut buf, &mut record), None, "len {len}");
		assert_eq!(record, ExitRecord::new(10), "nothing ran");
	}
	// The right length runs: 0..3 with total 0 and fuel 100.
	let mut buf = vec![0i64; n + 2];
	buf[n + 1] = 3;
	let mut record = ExitRecord::new(100);
	let status = native.call(&mut buf, &mut record).expect("runs");
	let mut sim = vec![0i64; n + 2];
	sim[n + 1] = 3;
	let mut sim_record = ExitRecord::new(100);
	assert_eq!(status, simulate(&region, &mut sim, &mut sim_record));
	assert_eq!(record, sim_record);
	assert_eq!((buf[0], buf[n], buf[n + 1]), (3, 3, 3));
}

// ------------------------------------------------- 48..51: admission, golden, Send

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn b1_bench_kernel_admission() {
	run_child("child_bench_admission");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_bench_admission() {
	require_child();
	for (name, text, loaded) in [
		("v2/while", super::p2_tests::V2_WHILE, 1),
		("v2/numeric", super::p2_tests::V2_NUMERIC, 1),
		("v2/calls", built::V2_CALLS, 1),
		("v2/fib", super::p2_tests::V2_FIB, 0),
		("v2/shared_rows", super::p2_tests::V2_SHARED_ROWS, 0),
		("stage/calls", built::STAGE_CALLS, 1),
	] {
		hook::uninstall();
		cache::clear_for_test();
		cache::enable_for_test(true);
		let program = compile_source(text);
		let s = cache::prepare(&program.code).expect("summary");
		println!(
			"B1ADMIT\t{name}\tcandidates={}\tregions={}\tloaded={}\tcode_bytes={}\tmetadata_bytes={}\trefusals={:?}\tanalysis={}",
			s.candidates,
			s.regions,
			s.loaded,
			s.code_bytes,
			s.metadata_bytes,
			unexpected(&s),
			outcomes(&program.code).join(",")
		);
		assert_eq!(s.loaded, loaded, "{name}: {s:?}");
		assert_eq!(unexpected(&s), [0; 12], "{name}");
	}
}

// ------------------------------------------ W13: branch-condition sources (W3 guard)

/// A guarded `Bool` live-in that is also written in the loop: at the first `if c` the
/// condition is the entry payload (unwritten, but stored at every exit as a possibly
/// written slot), later a Compare result. `i64::MAX - 1` makes `k += 2` take its Retry
/// exit right after the yes edge, with `c` still unwritten.
const W13_GUARD_BOOL: &str =
	"pub fn main(f, a, n) { let k = a; let c = f; while k < n { if c { k += 2; } else { k += 1; } c = k < 5; } k }";
/// A `Bool` literal condition (the header Branch reads a guarded live-in first).
const W13_LITERAL_BOOL: &str = "pub fn main(n) { let k = 0; let s = 0; while k < n { let t = if k < 2 { true } else { false }; if t { s += 1; } k += 1; } s }";
/// A condition copied from a Compare result (P2b `Copy`).
const W13_COPY_BOOL: &str = "pub fn main(n) { let k = 0; let s = 0; while k < n { let c = k < 3; let d = c; if d { s += 1; } k += 1; } s }";

fn w13_branch_cases() -> Vec<B1Case> {
	let b = |f: bool, a: i64, n: i64| vec![Arg::B(f), Arg::I(a), Arg::I(n)];
	vec![
		case(
			"w13_guard_bool",
			W13_GUARD_BOOL,
			vec![
				b(true, 0, 7),
				b(false, 0, 4),
				b(true, i64::MAX - 1, i64::MAX),
				b(false, i64::MAX - 1, i64::MAX),
			],
			1,
			"CFR",
		),
		case(
			"w13_literal_bool",
			W13_LITERAL_BOOL,
			ints(&[&[0], &[1], &[5]]),
			1,
			"CF",
		),
		case(
			"w13_copy_bool",
			W13_COPY_BOOL,
			ints(&[&[0], &[2], &[6]]),
			1,
			"CF",
		),
	]
}

/// The B1 corpus cases whose branch conditions are virtual locals inside an inlined callee
/// (`callee_f_two_arms`, `checkpoint_two_arms`) or a callee's Ret-fed Bool read by the
/// caller (`callee_lt_caller_branch`); re-run here so T5 covers them in its own counts.
const W13_FROM_CORPUS: [&str; 3] = [
	"callee_f_two_arms",
	"checkpoint_two_arms",
	"callee_lt_caller_branch",
];

/// T5's cases: the three W13 sources, then the corpus cases above.
fn w13_t5_cases() -> Vec<B1Case> {
	let mut all = w13_branch_cases();
	for name in W13_FROM_CORPUS {
		let mut found: Vec<B1Case> = corpus().into_iter().filter(|c| c.name == name).collect();
		assert_eq!(found.len(), 1, "{name}");
		all.push(found.remove(0));
	}
	all
}

/// T5's programs as (name, program) pairs (the compile-only source census in `w13_tests`).
pub(super) fn w13_t5_programs() -> Vec<(String, Src)> {
	w13_t5_cases()
		.into_iter()
		.map(|c| (c.name, c.src))
		.collect()
}

/// W13 T5 (W3 guard; passes before and after the change): Branch conditions from every
/// source a region admits (Compare, Literal, Copy, guarded live-in, unwritten at a Retry
/// exit, a virtual local inside an inlined callee and a callee's Ret-fed Bool) in the three
/// modes at every fuel. `w13_tests::w13_t5_branch_condition_sources` checks statically that
/// each case's admitted region has the Branch source it is here for.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn w13_branch_condition_sources_every_fuel() {
	run_child("child_w13_branch_sources");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_w13_branch_sources() {
	require_child();
	let cases = w13_t5_cases();
	let covs = run_cases(&cases);
	let (mut total, mut points) = (Stats::default(), 0);
	for (case, c) in cases.iter().zip(&covs) {
		// Admitted, not merely run: every case loads its one region and enters it natively.
		assert_eq!(c.install.loaded, 1, "{}", case.name);
		assert!(
			c.analysis.iter().any(|a| a.ends_with(":admitted")),
			"{}: {:?}",
			case.name,
			c.analysis
		);
		assert!(c.native_covered.native_entries > 0, "{}", case.name);
		assert_eq!(c.native_all.mismatches, 0, "{}", case.name);
		// Unwritten stored cells equal the simulator's too: W3 stores the same integers.
		assert_eq!(c.native_all.scratch_differences, 0, "{}", case.name);
		assert_eq!(
			c.native_all.replay_checked, c.native_all.native_replay,
			"{}",
			case.name
		);
		stats_add(&mut total, &c.native_all);
		points += c.points;
	}
	println!(
		"W13COV\tTOTAL\tcases={}\tpoints={points}\tnative_entries={}\tcontinue={}\titer_done={}\tfuel={}\tretry={}\treplay_fuel={}\treplay_fault={}\treplay_checked={}\tmismatches={}\tscratch_diffs={}\tinvalid_exits={}",
		cases.len(),
		total.native_entries,
		total.native_continue,
		total.native_iter_done,
		total.native_fuel,
		total.native_retry,
		total.native_replay_fuel,
		total.native_replay_fault,
		total.replay_checked,
		total.mismatches,
		total.scratch_differences,
		cache::invalid_exits()
	);
	assert_eq!(cache::invalid_exits(), 0);
}

/// The selection golden re-captured at B1 (`RUNE_P3A_GOLDEN_OUT`).
pub(super) const B1_GOLDEN: &str = include_str!("fixtures/b1_selection_golden.txt");

/// The enumerated B1 selection delta against the P2b golden: (program, function, header,
/// old ANALYSIS outcome, new ANALYSIS outcome prefix, new COMPILE class or "" when the
/// header stays refused).
pub(super) const DELTA: &[(&str, u32, u32, &str, &str, &str)] = &[
	(
		"inner_loop_in_refused_outer",
		0,
		2,
		"refused\tOp { pc: 12 }",
		"refused\tInline { pc: 12, reason: ReturnSource { callee_pc: 0 } }",
		"",
	),
	(
		"region_in_function_called_from_loop",
		0,
		2,
		"refused\tOp { pc: 4 }",
		"refused\tInline { pc: 4, reason: Charged {",
		"",
	),
	("v2_calls", 0, 4, "refused\tOp { pc: 4 }", "admitted", "ok"),
	("listed_9", 0, 5, "refused\tOp { pc: 5 }", "admitted", "ok"),
];

fn delta_source(program: &str) -> Code {
	let text = match program {
		"v2_calls" => built::V2_CALLS,
		"listed_9" => "fn add(a, b) { a + b } pub fn main(_a) { let total = 0; for i in 0..10000 { total = add(total, i); } total }",
		other => std::panic!("no source for {other}"),
	};
	compile_source(text).code
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

fn lines(block: &[&str], kind: &str) -> Vec<(u32, u32, String)> {
	block
		.iter()
		.filter(|l| l.starts_with(kind))
		.map(|l| key(l))
		.collect()
}

/// The size of `cache::Ready` before B1 (measured on x86_64 Linux).
const BASE_READY_BYTES: u64 = 688;

/// Checks the P2b golden against the B1 golden: every ANALYSIS, COMPILE and LOADED line
/// identical except exactly `DELTA`; SUMMARY lines identical except `metadata_bytes`
/// (old + loaded * the `Ready` growth forced by B1's `Region` fields, + the new regions'
/// own metadata) and the new regions' counts and bytes. Returns (identical lines, programs
/// in DELTA).
pub(super) fn compare_b1_delta(old: &str, new: &str) -> (usize, usize) {
	assert_eq!(
		core::mem::size_of::<Exit>(),
		56,
		"Exit keeps its P2b size (B1 fields fit its padding)"
	);
	let growth = cache::ready_bytes_for_test() as u64 - BASE_READY_BYTES;
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

		let os = o.iter().find(|l| l.starts_with("SUMMARY\t")).unwrap();
		let ns = n.iter().find(|l| l.starts_with("SUMMARY\t")).unwrap();
		let loaded = field(os, "loaded");
		let new_metadata: u64 = if new_loaded.is_empty() {
			0
		} else {
			let code = delta_source(name);
			new_loaded
				.iter()
				.map(|l| {
					let (_, region) = analyze(&code, l.0)
						.unwrap()
						.into_iter()
						.find(|(h, _)| *h == l.1)
						.unwrap();
					cache::region_metadata(&region.unwrap()) as u64
				})
				.sum()
		};
		assert_eq!(
			field(ns, "metadata_bytes"),
			field(os, "metadata_bytes") + loaded * growth + new_metadata,
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
		assert_eq!(
			field(ns, "regions"),
			field(os, "regions") + admitted.len() as u64,
			"{name}"
		);
		let keys = [
			"metadata_bytes",
			"loaded",
			"code_bytes",
			"mapped_bytes",
			"regions",
		];
		assert_eq!(
			without(os, &keys),
			without(ns, &keys),
			"{name}: other SUMMARY counters"
		);
	}
	(identical, delta_programs)
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn b1_selection_golden_delta() {
	let (identical, programs) = compare_b1_delta(super::p2b_tests::P2B_GOLDEN, B1_GOLDEN);
	assert_eq!(
		programs,
		DELTA.len(),
		"one enumerated header per changed program"
	);
	println!(
		"B1GOLDEN\tp2b_lines={}\tb1_lines={}\tidentical_analysis_compile_loaded={identical}\tdelta_programs={programs}\tready_growth={}",
		super::p2b_tests::P2B_GOLDEN.lines().count(),
		B1_GOLDEN.lines().count(),
		cache::ready_bytes_for_test() as u64 - BASE_READY_BYTES
	);
}

#[allow(dead_code)]
fn assert_send<T: Send>() {}

#[test]
fn b1_types_stay_send() {
	assert_send::<Executor>();
	assert_send::<frontend::Program>();
	assert_send::<Code>();
	// The analysed region (plain data) is Send too; loaded code stays thread-bound.
	assert_send::<Region>();
}

/// Two B1 loops in `main`: the injected analysis error fires in the second header, after
/// its admission built the inline summaries and site buffers.
const PARTIAL_B1: &str = "fn add(a, b) { a + b } pub fn main(x) { let t = 0; for i in 0..x { t = add(t, i); } let u = 0; for j in 0..x { u = add(u, j); } t + u }";

#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn b1_partial_analysis_failure_with_inline_sites() {
	run_child("child_partial_b1");
}

#[test]
#[ignore = "B1 native worker: run by its parent test in a child process"]
fn child_partial_b1() {
	require_child();
	let program = compile_source(PARTIAL_B1);
	let code = &program.code;
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	hook::uninstall();
	cache::clear_for_test();
	cache::enable_for_test(true);
	let clean = cache::prepare(code).unwrap();
	assert_eq!(
		(clean.candidates, clean.regions, clean.loaded),
		(2, 2, 2),
		"{clean:?}"
	);
	let headers: Vec<u32> = cache::regions_for_test(code.id())
		.unwrap()
		.iter()
		.map(|r| r.header)
		.collect();
	cache::clear_for_test();

	crate::experimental_core::region::inject_analysis_error_for_test(Some(1));
	let s = cache::prepare(code).unwrap();
	crate::experimental_core::region::inject_analysis_error_for_test(None);
	assert_eq!((s.analysis_errors, s.accounting_invariant), (1, 0));
	assert_eq!((s.candidates, s.regions, s.loaded), (2, 1, 1));
	let kept: Vec<u32> = cache::regions_for_test(code.id())
		.unwrap()
		.iter()
		.map(|r| r.header)
		.collect();
	assert_eq!(kept, vec![headers[0]], "the earlier B1 region stays usable");
	let (at_error, at_end) = cache::budget_used_for_test();
	let (used, expected) = at_error.expect("the injected error was seen");
	assert_eq!(
		used, expected,
		"the failed header's inline buffers were released"
	);
	assert_eq!(at_end, Some(0), "nothing charged after the build");

	// The surviving region runs, equal to the interpreter at every fuel.
	let args = [Arg::I(6)];
	let reference = plain(code, main, &args, BIG_FUEL);
	let total = BIG_FUEL - reference.fuel_left.unwrap();
	for fuel in 0..=total + 1 {
		hook::suspend();
		let o = invoke(code, main, &args, fuel);
		hook::resume();
		assert_eq!(o, plain(code, main, &args, fuel), "fuel {fuel}");
	}
	let entered = cache::regions_for_test(code.id()).unwrap();
	assert!(entered[0].entries > 0, "{entered:?}");
	println!("B1CASE\tpartial_b1\th{} loaded, h{} injected error after inline summaries (charged {used} == scan+context {expected}); equal at every fuel 0..={}", headers[0], headers[1], total + 1);
	cache::clear_for_test();
	assert_eq!(cache::invalid_exits(), 0);
}
