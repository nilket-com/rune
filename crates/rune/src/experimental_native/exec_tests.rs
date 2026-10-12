//! P1: generated region code EXECUTED under the differential (test-only).
//!
//! Every native-executing test runs in a child process: the parent test re-invokes this
//! test binary with `--exact` on an `#[ignore]`d worker that refuses to run unless
//! `RUNE_P1_NATIVE_CHILD` is set, with stdin null and `PERF_BUILDID_DIR` removed, under a
//! 120 s deadline. A signal or a timeout is a failure with the signal recorded.
//!
//! Per case and per fuel value 0..=total_charged+1 (at most 20,000), three engines run on
//! fresh stores: the plain interpreter (no install at all), the simulator install and the
//! native install. Their observations must be equal: value or full error (kind, location,
//! operation, refusal), call chain, cleanup error, executor idle state, live store entries,
//! the store's whole state, and the remaining fuel. On every native entry the hook also
//! compares the native and simulated status, exit record and written buffer entries from
//! independent copies of the same pre-entry input.
//!
//! Output lines prefixed `TABLE` and `RSS` are the receipt rows.
use super::boundary::{
	inject_load_error_for_test, load_error_frees_for_test, maps_overlapping, parse_maps_line,
	InjectLoadError, MapInfo, NativeRegion,
};
use super::lower::compile as compile_region;
use super::{NativeRefusal, MAX_PROGRAM_CODE_BYTES};
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Code, Function, Op},
	error::{Error, ErrorKind, Location},
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	region::{
		abi::ExitRecord,
		analyze,
		hook::{self, Mode, NativeInstall, Stats},
		simulate::simulate,
		EntryClass, Region, Scalar, Status,
	},
	value::{Root, Scalar as HostScalar, Value},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::panic;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_P1_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(120);
/// The fuel sweep bound per case: larger cases are reduced, never sampled.
pub(super) const MAX_SWEEP: usize = 20_000;
/// Fuel for the reference run that measures a case's total charge.
pub(super) const BIG_FUEL: usize = 1_000_000;

// ------------------------------------------------------------------ process isolation

/// Runs the worker test `inner` in a child process under the deadline.
fn run_child(inner: &str) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::exec_tests::{inner}");
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
			panic!(
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
		"native worker tests run only in the child process started by their parent test"
	);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn p1_native_differential_corpus() {
	run_child("child_corpus");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn p1_native_differential_new_cases() {
	run_child("child_new_cases");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn p1_native_refusals() {
	run_child("child_refusals");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn p1_native_lifecycle_and_contract() {
	run_child("child_lifecycle");
}

// ---------------------------------------------------------------------------- helpers

pub(super) fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

pub(super) fn store() -> Store {
	Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap()
}

pub(super) fn compile_source(text: &str) -> frontend::Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap()
}

pub(super) fn build(functions: Vec<Function>) -> Code {
	let mut b = Builder::default();
	for f in functions {
		b.push(f).unwrap();
	}
	b.finish().unwrap()
}

pub(super) fn lit(f: &mut Function, out: u32, value: Value, at: u32) {
	f.push(Op::Literal { out, value }, loc(at)).unwrap();
}

pub(super) fn bin(f: &mut Function, out: u32, op: Binary, lhs: u32, rhs: u32, at: u32) {
	f.push(Op::Binary { out, op, lhs, rhs }, loc(at)).unwrap();
}

pub(super) fn branch(f: &mut Function, condition: u32, yes: u32, no: u32, at: u32) {
	f.push(Op::Branch { condition, yes, no }, loc(at)).unwrap();
}

pub(super) fn jump(f: &mut Function, target: u32, at: u32) {
	f.push(Op::Jump { target }, loc(at)).unwrap();
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Arg {
	I(i64),
	U(u64),
	F(f64),
	B(bool),
}

#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
	Value(Option<HostScalar>),
	Failure {
		error: Error,
		call_chain: Vec<Location>,
		cleanup_error: Option<Error>,
	},
}

/// Everything observable about one invocation.
#[derive(Debug, PartialEq)]
pub(super) struct Observed {
	pub(super) outcome: Outcome,
	pub(super) live_entries: usize,
	pub(super) idle: bool,
	pub(super) fuel_left: Option<usize>,
	pub(super) store_state: String,
}

pub(super) fn invoke(code: &Code, function: u32, args: &[Arg], fuel: usize) -> Observed {
	let mut s = store();
	let roots: Vec<Root> = args
		.iter()
		.map(|a| match *a {
			Arg::I(v) => s.root_i64(v),
			Arg::U(v) => s.root_u64(v),
			Arg::F(v) => s.root_f64(v),
			Arg::B(v) => s.root_bool(v),
		})
		.collect();
	let refs: Vec<&Root> = roots.iter().collect();

	let mut executor = Executor::new().unwrap();
	hook::take_final_fuel();
	let outcome = match executor.invoke_code(code, &mut s, function, &refs, fuel) {
		Ok(root) => Outcome::Value(s.root_scalar(&root).unwrap()),
		Err(failure) => Outcome::Failure {
			error: failure.error,
			call_chain: failure.call_chain.iter().copied().collect(),
			cleanup_error: failure.cleanup_error,
		},
	};
	let fuel_left = hook::take_final_fuel();

	Observed {
		outcome,
		live_entries: s.live_entries(),
		idle: executor.is_idle(),
		fuel_left,
		store_state: s.state_for_test(),
	}
}

fn add(total: &mut Stats, s: &Stats) {
	total.entries += s.entries;
	total.continue_exits += s.continue_exits;
	total.fuel_exits += s.fuel_exits;
	total.retry_exits += s.retry_exits;
	total.refused_cancel += s.refused_cancel;
	total.refused_store += s.refused_store;
	total.refused_tag += s.refused_tag;
	total.refused_other += s.refused_other;
	total.native_entries += s.native_entries;
	total.native_continue += s.native_continue;
	total.native_fuel += s.native_fuel;
	total.native_retry += s.native_retry;
	total.refused_call += s.refused_call;
	total.mismatches += s.mismatches;
	total.scratch_differences += s.scratch_differences;
}

// ------------------------------------------------------------------------- the cases

pub(super) enum Src {
	Program(frontend::Program),
	Code(Code),
}

impl Src {
	pub(super) fn code(&self) -> &Code {
		match self {
			Src::Program(p) => &p.code,
			Src::Code(c) => c,
		}
	}

	pub(super) fn main(&self) -> u32 {
		match self {
			Src::Program(p) => p.function(Hash::type_hash(["main"])).unwrap(),
			Src::Code(_) => 0,
		}
	}
}

/// Expected compile refusals by kind: runtime divisor, constant 0, constant -1.
pub(super) type Refusals = [usize; 3];

pub(super) struct Case {
	pub(super) name: String,
	pub(super) src: Src,
	pub(super) inputs: Vec<Vec<Arg>>,
	/// Regions expected compiled, audited and loaded.
	pub(super) loaded: usize,
	pub(super) refused: Refusals,
	/// Exit kinds this case targets ("C", "F", "R"); asserted over fuel >= 1.
	pub(super) targets: &'static str,
	/// Entry refusals (tag/store/cancel) are expected: recorded, not failures, and no
	/// native entry is required.
	pub(super) entry_refusals: bool,
}

pub(super) fn rune(name: &str, text: &str, inputs: Vec<Vec<Arg>>, loaded: usize, targets: &'static str) -> Case {
	Case {
		name: name.into(),
		src: Src::Program(compile_source(text)),
		inputs,
		loaded,
		refused: [0; 3],
		targets,
		entry_refusals: false,
	}
}

pub(super) fn arithmetic_inputs() -> Vec<Vec<Arg>> {
	let max = i64::MAX;
	let min = i64::MIN;
	let mut inputs = Vec::new();
	for k in 0..6 {
		inputs.push(vec![Arg::I(max - k), Arg::I(1)]);
		inputs.push(vec![Arg::I(min + k), Arg::I(1)]);
		inputs.push(vec![Arg::I(max / 8 - k), Arg::I(2)]);
		inputs.push(vec![Arg::I(min / 8 + k), Arg::I(2)]);
	}
	inputs.push(vec![Arg::I(1 << 60), Arg::I(1)]);
	inputs.push(vec![Arg::I(7), Arg::I(0)]);
	inputs.push(vec![Arg::I(min), Arg::I(-1)]);
	inputs.push(vec![Arg::I(min), Arg::I(1)]);
	inputs.push(vec![Arg::I(max), Arg::I(-1)]);
	inputs
}

/// `fn(limit, x) { i = 0; acc = 0; while i < limit { d = D; t = x op d; acc = acc + t;
/// one = 1; x = x - one; i = i + one } return acc }`, where `d` is the literal `divisor`
/// written inside the loop, or (with `None`) the parameter `limit` (a runtime divisor).
pub(super) fn divided(op: Binary, divisor: Option<i64>) -> Code {
	// slots: 0 limit, 1 x, 2 i, 3 cond, 4 one, 5 d, 6 t, 7 acc
	let mut f = Function::new(8, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0); // 0
	lit(&mut f, 7, Value::I64(0), 1); // 1
	bin(&mut f, 3, Binary::Lt, 2, 0, 2); // 2: header
	branch(&mut f, 3, 4, 11, 3); // 3
	match divisor {
		Some(d) => lit(&mut f, 5, Value::I64(d), 4), // 4
		None => lit(&mut f, 4, Value::I64(1), 4),   // 4 (d unused; divide by limit)
	}
	bin(&mut f, 6, op, 1, if divisor.is_some() { 5 } else { 0 }, 5); // 5
	bin(&mut f, 7, Binary::Add, 7, 6, 6); // 6
	lit(&mut f, 4, Value::I64(1), 7); // 7
	bin(&mut f, 1, Binary::Sub, 1, 4, 8); // 8
	bin(&mut f, 2, Binary::Add, 2, 4, 9); // 9
	jump(&mut f, 2, 10); // 10
	f.push(Op::Return { value: 7 }, loc(11)).unwrap(); // 11
	build(vec![f])
}

pub(super) fn divided_inputs() -> Vec<Vec<Arg>> {
	vec![
		vec![Arg::I(12), Arg::I(100)],
		vec![Arg::I(12), Arg::I(-1_000_003)],
		vec![Arg::I(6), Arg::I(i64::MIN + 3)],
		vec![Arg::I(6), Arg::I(i64::MAX)],
		vec![Arg::I(12), Arg::I(-7)],
		vec![Arg::I(4), Arg::I(0)],
	]
}

/// Out/in slot aliasing: `x = x * x` (out == lhs == rhs) and `x = one - x` (out == rhs).
fn alias_square() -> Code {
	// slots: 0 limit, 1 x, 2 i, 3 cond, 4 one
	let mut f = Function::new(5, 2).unwrap();
	lit(&mut f, 2, Value::I64(0), 0); // 0
	bin(&mut f, 3, Binary::Lt, 2, 0, 1); // 1: header
	branch(&mut f, 3, 3, 8, 2); // 2
	bin(&mut f, 1, Binary::Mul, 1, 1, 3); // 3: out == lhs == rhs
	lit(&mut f, 4, Value::I64(1), 4); // 4
	bin(&mut f, 1, Binary::Sub, 4, 1, 5); // 5: out == rhs
	bin(&mut f, 2, Binary::Add, 2, 4, 6); // 6
	jump(&mut f, 1, 7); // 7
	f.push(Op::Return { value: 1 }, loc(8)).unwrap(); // 8
	build(vec![f])
}

/// One temporary slot `t` reused across types: `Bool` on iterations with `i < 3`, `I64`
/// after; its last tag is observable through the return value.
fn reused_temp() -> Code {
	// slots: 0 limit, 1 i, 2 c, 3 t, 4 one, 5 three
	let mut f = Function::new(6, 1).unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	lit(&mut f, 3, Value::Bool(false), 1); // 1
	bin(&mut f, 2, Binary::Lt, 1, 0, 2); // 2: header
	branch(&mut f, 2, 4, 14, 3); // 3
	lit(&mut f, 4, Value::I64(1), 4); // 4
	bin(&mut f, 1, Binary::Add, 1, 4, 5); // 5
	lit(&mut f, 5, Value::I64(3), 6); // 6
	bin(&mut f, 2, Binary::Lt, 1, 5, 7); // 7
	branch(&mut f, 2, 9, 11, 8); // 8
	lit(&mut f, 3, Value::Bool(true), 9); // 9: t is a Bool
	jump(&mut f, 2, 10); // 10
	lit(&mut f, 3, Value::I64(7), 11); // 11: t is an I64
	jump(&mut f, 2, 12); // 12
	f.push(Op::Return { value: 3 }, loc(13)).unwrap(); // 13 (unreached)
	f.push(Op::Return { value: 3 }, loc(14)).unwrap(); // 14
	build(vec![f])
}

/// Slot 2 holds a live string when the loop is first reached (the P0 heap case).
fn heap_slot() -> Code {
	let mut f = Function::new(5, 1).unwrap();
	let text = f.string("owned").unwrap();
	lit(&mut f, 1, Value::I64(0), 0); // 0
	f.push(Op::String { out: 2, text }, loc(1)).unwrap(); // 1
	bin(&mut f, 3, Binary::Lt, 1, 0, 2); // 2 header
	branch(&mut f, 3, 4, 8, 3); // 3
	lit(&mut f, 2, Value::I64(1), 4); // 4: overwrites the string
	bin(&mut f, 1, Binary::Add, 1, 2, 5); // 5
	jump(&mut f, 2, 6); // 6
	f.push(Op::Return { value: 1 }, loc(7)).unwrap(); // 7 (unreached)
	f.push(Op::Return { value: 1 }, loc(8)).unwrap(); // 8
	build(vec![f])
}

pub(super) fn ints(rows: &[&[i64]]) -> Vec<Vec<Arg>> {
	rows.iter()
		.map(|r| r.iter().map(|&v| Arg::I(v)).collect())
		.collect()
}

pub(super) fn corpus_cases() -> Vec<Case> {
	let mut cases = vec![
		rune(
			"kernel_while",
			"pub fn main(a) { let n = 0; while n < a { n += 1; } n }",
			ints(&[&[25]]),
			1,
			"CF",
		),
		rune(
			"kernel_numeric",
			"pub fn main(a) { let n = 0; let total = 0; while n < a { total += n % 17; n += 1; } total }",
			ints(&[&[40]]),
			1,
			"CF",
		),
	];

	for (name, text) in [
		(
			"overflow_add",
			"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += b; i += 1; } x }",
		),
		(
			"overflow_sub",
			"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x -= b; i += 1; } x }",
		),
		(
			"overflow_mul",
			"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x *= b; i += 1; } x }",
		),
		(
			"alias_x_plus_x",
			"pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += x; i += b; } x }",
		),
		(
			"faulting_live_in_destination",
			"pub fn main(a, b) { let i = 0; while i < 4 { a += b; b += 1; i += 1; } a + b }",
		),
	] {
		cases.push(rune(name, text, arithmetic_inputs(), 1, "CFR"));
	}

	cases.push(rune(
		"nested_callers",
		"fn inner(a, b) { let x = a; let i = 0; while i < 5 { x += b; i += 1; } x }
		fn mid(a, b) { let y = inner(a, b); y - 1 }
		pub fn main(a, b) { let z = mid(a, b); z + 1 }",
		ints(&[
			&[1, 1],
			&[i64::MAX - 2, 1],
			&[i64::MAX, 1],
			&[i64::MIN + 3, -1],
		]),
		1,
		"CFR",
	));

	for (k, (text, inputs)) in [
		(
			"pub fn main(a) { let i = 0; let s = 0; while i < a { if i % 3 == 0 { s += i; } else { s -= 1; } i += 1; } s }",
			vec![vec![Arg::I(20)]],
		),
		(
			"pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } else { s += 1; } i += 1; } s }",
			vec![vec![Arg::I(9), Arg::B(true)], vec![Arg::I(9), Arg::B(false)]],
		),
		(
			"pub fn main(a) { let i = 0; let s = 0; while i < a { if i * 2 > 6 { s += i * 2; } i += 1; } s }",
			vec![vec![Arg::I(12)]],
		),
		(
			"pub fn main(a) { let i = 0; let s = 0; while i < a { if i > 3 { if i < 9 { s += 1; } } i += 1; } s }",
			vec![vec![Arg::I(12)]],
		),
	]
	.into_iter()
	.enumerate()
	{
		cases.push(rune(&format!("branches_{k}"), text, inputs, 1, "CF"));
	}

	cases.push(rune(
		"inner_loop_in_refused_outer",
		"fn id(x) { x } pub fn main(a) { let i = 0; let s = 0; while i < a { let j = 0; while j < 3 { s += j; j += 1; } s = id(s); i += 1; } s }",
		ints(&[&[6]]),
		1,
		"CF",
	));
	cases.push(rune(
		"nested_admitted_loops",
		"pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
		ints(&[&[6]]),
		2,
		"CF",
	));

	// Expected entry refusals: wrong live-in tags never enter.
	let mut tags = rune(
		"wrong_live_in_tags",
		"pub fn main(a) { let n = 0; while n < a { n += 1; } n }",
		vec![vec![Arg::U(7)], vec![Arg::F(7.0)]],
		1,
		"",
	);
	tags.entry_refusals = true;
	cases.push(tags);

	let mut bool_as_int = rune(
		"bool_live_in_given_i64",
		"pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } i += 1; } s }",
		vec![vec![Arg::I(5), Arg::I(1)]],
		1,
		"",
	);
	bool_as_int.entry_refusals = true;
	cases.push(bool_as_int);

	// A dead-but-owned heap slot refuses the first arrival; later arrivals enter.
	cases.push(Case {
		name: "heap_slot_first_arrival".into(),
		src: Src::Code(heap_slot()),
		inputs: ints(&[&[5]]),
		loaded: 1,
		refused: [0; 3],
		targets: "CF",
		entry_refusals: true,
	});
	cases
}

pub(super) fn new_cases() -> Vec<Case> {
	let mut cases = vec![
		Case {
			name: "alias_out_lhs_rhs_and_out_rhs".into(),
			src: Src::Code(alias_square()),
			inputs: ints(&[
				&[12, 3],
				&[10, 2],
				&[3, i64::MAX],
				&[5, -3_037_000_500],
				&[4, 0],
				&[12, -2],
			]),
			loaded: 1,
			refused: [0; 3],
			targets: "CFR",
			entry_refusals: false,
		},
		Case {
			name: "bool_i64_reused_temp".into(),
			src: Src::Code(reused_temp()),
			inputs: ints(&[&[0], &[1], &[2], &[3], &[4], &[7]]),
			loaded: 1,
			refused: [0; 3],
			targets: "CF",
			entry_refusals: false,
		},
		rune(
			"path_dependent_write_rem",
			"pub fn main(a) { let i = 0; let w = 0; while i < a { if i % 3 == 1 { w += i; } i += 1; } w }",
			ints(&[&[10], &[1], &[2]]),
			1,
			"CF",
		),
		rune(
			"path_dependent_write_flag",
			"pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } i += 1; } s }",
			vec![vec![Arg::I(6), Arg::B(true)], vec![Arg::I(6), Arg::B(false)]],
			1,
			"CF",
		),
		rune(
			"region_in_function_called_from_loop",
			"fn inner(n, x) { let s = 0; let i = 0; while i < n { s += x; i += 1; } s }
			pub fn main(a, x) { let t = 0; let k = 0; while k < a { t = inner(k, x); k += 1; } t }",
			ints(&[&[6, 1], &[7, i64::MAX / 4], &[4, -9]]),
			1,
			"CFR",
		),
		rune(
			"div_const_rune",
			"pub fn main(a) { let n = 0; let q = 0; while n < a { q += n / 7; n += 1; } q }",
			ints(&[&[30]]),
			1,
			"CF",
		),
	];

	for d in [17, -17, 7, 2] {
		for op in [Binary::Div, Binary::Rem] {
			cases.push(Case {
				name: format!("const_{op:?}_{d}").to_lowercase(),
				src: Src::Code(divided(op, Some(d))),
				inputs: divided_inputs(),
				loaded: 1,
				refused: [0; 3],
				targets: "CFR",
				entry_refusals: false,
			});
		}
	}
	cases
}

pub(super) fn refusal_cases() -> Vec<Case> {
	let mut cases = Vec::new();
	for (d, refused) in [(0, [0, 1, 0]), (-1, [0, 0, 1])] {
		for op in [Binary::Div, Binary::Rem] {
			cases.push(Case {
				name: format!("refused_const_{op:?}_{d}").to_lowercase(),
				src: Src::Code(divided(op, Some(d))),
				inputs: divided_inputs(),
				loaded: 0,
				refused,
				targets: "",
				entry_refusals: false,
			});
		}
	}
	for op in [Binary::Div, Binary::Rem] {
		cases.push(Case {
			name: format!("refused_runtime_{op:?}_handbuilt").to_lowercase(),
			src: Src::Code(divided(op, None)),
			inputs: vec![
				vec![Arg::I(3), Arg::I(100)],
				vec![Arg::I(0), Arg::I(5)],
				vec![Arg::I(-1), Arg::I(i64::MIN)],
			],
			loaded: 0,
			refused: [1, 0, 0],
			targets: "",
			entry_refusals: false,
		});
	}
	for (name, text) in [
		(
			"refused_runtime_div_rune",
			"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a / b; i += 1; } t }",
		),
		(
			"refused_runtime_rem_rune",
			"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a % b; i += 1; } t }",
		),
	] {
		let mut case = rune(name, text, arithmetic_inputs(), 0, "");
		case.refused = [1, 0, 0];
		cases.push(case);
	}
	cases
}

// ---------------------------------------------------------------- the differential

fn dump_dir(case: &str) -> Option<std::path::PathBuf> {
	std::env::var_os("RUNE_P1_DUMP_DIR").map(|d| std::path::PathBuf::from(d).join(case))
}

fn has(targets: &str, c: char) -> bool {
	targets.contains(c)
}

fn run_case(case: &Case) {
	let code = case.src.code();
	let main = case.src.main();
	let dump = dump_dir(&case.name);

	hook::uninstall();
	let install: NativeInstall = hook::install_native(code, dump.as_deref());
	let refused = [
		install.division_runtime,
		install.division_const0,
		install.division_constneg1,
	];

	assert_eq!(install.loaded, case.loaded, "{}: loaded regions {install:?}", case.name);
	assert_eq!(refused, case.refused, "{}: refusals {install:?}", case.name);
	assert_eq!(
		(
			install.size,
			install.codegen,
			install.audit_other,
			install.other,
			install.load_refused
		),
		(0, 0, 0, 0, 0),
		"{}: unexpected refusal {install:?}",
		case.name
	);

	let mut all = Stats::default();
	let mut covered = Stats::default();
	let mut fuel_lo = usize::MAX;
	let mut fuel_hi = 0;
	let mut points = 0usize;

	for args in &case.inputs {
		// Total charge of the plain interpreter, measured with no install at all.
		hook::suspend();
		let reference = invoke(code, main, args, BIG_FUEL);
		hook::resume();
		if let Outcome::Failure { error, .. } = &reference.outcome {
			assert_ne!(error.kind, ErrorKind::FuelExhausted, "{}: {args:?}", case.name);
		}
		let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
		assert!(
			total + 1 <= MAX_SWEEP,
			"{}: {args:?} charges {total}: reduce the case",
			case.name
		);

		for fuel in 0..=total + 1 {
			hook::suspend();
			let plain = invoke(code, main, args, fuel);
			hook::resume();

			hook::set_mode(Mode::Simulate);
			hook::take_stats();
			let simulated = invoke(code, main, args, fuel);
			let sim_stats = hook::take_stats().unwrap();

			hook::set_mode(Mode::Native);
			let native = invoke(code, main, args, fuel);
			let native_stats = hook::take_stats().unwrap();

			if let Some(mismatch) = hook::take_mismatch() {
				panic!(
					"NATIVE MISMATCH case {} fuel {fuel} args {args:?}\n{mismatch}\ndisassembly: {:?}",
					case.name, dump
				);
			}

			assert_eq!(plain, simulated, "{}: simulator, fuel {fuel}, {args:?}", case.name);
			assert_eq!(plain, native, "{}: native, fuel {fuel}, {args:?}", case.name);
			assert!(plain.idle && plain.live_entries == 0, "{}", case.name);

			// The simulator and native installs enter and exit identically.
			if case.loaded == install.regions {
				assert_eq!(
					(
						sim_stats.entries,
						sim_stats.continue_exits,
						sim_stats.fuel_exits,
						sim_stats.retry_exits
					),
					(
						native_stats.native_entries,
						native_stats.native_continue,
						native_stats.native_fuel,
						native_stats.native_retry
					),
					"{}: fuel {fuel}, {args:?}",
					case.name
				);
			}
			assert_eq!(native_stats.mismatches, 0);

			add(&mut all, &native_stats);
			if fuel >= 1 {
				add(&mut covered, &native_stats);
			}
			fuel_lo = fuel_lo.min(fuel);
			fuel_hi = fuel_hi.max(fuel);
			points += 1;
		}
	}
	hook::uninstall();

	let entry_refused = all.refused_tag + all.refused_store + all.refused_cancel;
	println!(
		"TABLE\t{}\tregions={}\tloaded={}\trefused[runtime,const0,constneg1]={:?}\tinputs={}\tfuel={}..={}\tpoints={}\tnative_entries={}\tcontinue={}\tfuel_exits={}\tretry={}\tentry_refused={}(tag {} store {} cancel {})\trefused_call={}\tscratch_diffs={}\ttargets={}",
		case.name,
		install.regions,
		install.loaded,
		refused,
		case.inputs.len(),
		fuel_lo,
		fuel_hi,
		points,
		all.native_entries,
		all.native_continue,
		all.native_fuel,
		all.native_retry,
		entry_refused,
		all.refused_tag,
		all.refused_store,
		all.refused_cancel,
		all.refused_call,
		all.scratch_differences,
		if case.targets.is_empty() { "-" } else { case.targets }
	);

	// Coverage, over the fuel >= 1 part of the sweep.
	assert_eq!(all.refused_call, 0, "{}: refused_call", case.name);
	if case.loaded == 0 {
		assert_eq!(all.native_entries, 0, "{}", case.name);
	} else if !case.entry_refusals || !case.targets.is_empty() {
		assert!(covered.native_entries > 0, "{}: no native entry", case.name);
	}
	if !case.entry_refusals {
		assert_eq!(entry_refused, 0, "{}: unexpected entry fallback", case.name);
	} else {
		assert!(entry_refused > 0, "{}: expected entry refusals", case.name);
	}
	if has(case.targets, 'C') {
		assert!(covered.native_continue > 0, "{}: no continue exit", case.name);
	}
	if has(case.targets, 'F') {
		assert!(covered.native_fuel > 0, "{}: no fuel exit", case.name);
	}
	if has(case.targets, 'R') {
		assert!(covered.native_retry > 0, "{}: no retry exit", case.name);
	}
}

#[test]
#[ignore = "P1 native worker: run by p1_native_differential_corpus in a child process"]
fn child_corpus() {
	require_child();
	for case in corpus_cases() {
		run_case(&case);
	}
}

#[test]
#[ignore = "P1 native worker: run by p1_native_differential_new_cases in a child process"]
fn child_new_cases() {
	require_child();
	for case in new_cases() {
		run_case(&case);
	}
}

#[test]
#[ignore = "P1 native worker: run by p1_native_refusals in a child process"]
fn child_refusals() {
	require_child();
	for case in refusal_cases() {
		run_case(&case);
	}
}

/// Harness pre-check, no native execution: every case's analysed regions compile (or are
/// refused by the expected kind) without loading anything, and every input's total charge
/// fits the sweep bound.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn p1_case_expectations_compile_only() {
	let mut cases = corpus_cases();
	cases.extend(new_cases());
	cases.extend(refusal_cases());
	hook::uninstall();

	for case in &cases {
		let code = case.src.code();
		let mut loaded = 0;
		let mut refused = [0usize; 3];
		for id in 0..code.functions() as u32 {
			for (_, region) in analyze(code.function(id).unwrap(), id).unwrap() {
				let Ok(region) = region else { continue };
				match compile_region(&region, 0) {
					Ok(_) => loaded += 1,
					Err(NativeRefusal::Audit(super::audit::Finding::Division(_))) => {
						match hook::division_kind(&region) {
							Some(hook::DivisionKind::Runtime) => refused[0] += 1,
							Some(hook::DivisionKind::Const0) => refused[1] += 1,
							Some(hook::DivisionKind::ConstNeg1) => refused[2] += 1,
							None => panic!("{}: division finding without a division", case.name),
						}
					}
					Err(other) => panic!("{}: {other:?}", case.name),
				}
			}
		}
		assert_eq!((loaded, refused), (case.loaded, case.refused), "{}", case.name);

		for args in &case.inputs {
			let reference = invoke(code, case.src.main(), args, BIG_FUEL);
			let total = BIG_FUEL - reference.fuel_left.expect("final fuel");
			assert!(total + 1 <= MAX_SWEEP, "{}: {args:?} charges {total}", case.name);
			if let Outcome::Failure { error, .. } = &reference.outcome {
				assert_ne!(error.kind, ErrorKind::FuelExhausted, "{}: {args:?}", case.name);
			}
		}
	}
}

// --------------------------------------------------------- lifecycle and contract

/// The coarse leak screen, fixed before the first lifecycle run: growth of the process's
/// VmRSS and VmData from cycle 100 to cycle 10,000 of each phase. A leak of one 4 KiB page
/// per cycle would be 38.7 MiB over 9,900 cycles, far above both.
const CYCLES: usize = 10_000;
const BASELINE_CYCLE: usize = 100;
const RSS_TOLERANCE_KIB: u64 = 8 * 1024;
const DATA_TOLERANCE_KIB: u64 = 16 * 1024;

pub(super) fn status_kib(field: &str) -> u64 {
	let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
	status
		.lines()
		.find_map(|l| l.strip_prefix(field))
		.and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
		.expect("status field")
}

fn sample_cycle(cycle: usize) -> bool {
	cycle == 0 || cycle == BASELINE_CYCLE || cycle % 1000 == 0 || cycle == CYCLES
}

/// Runs `step` for cycles 1..=CYCLES, sampling VmRSS/VmData, and applies the screen.
fn screened(phase: &str, mut step: impl FnMut(usize)) {
	let mut baseline = None;
	let mut end = (0, 0);
	let first = (status_kib("VmRSS:"), status_kib("VmData:"));
	println!("RSS\t{phase}\tcycle=0\tVmRSS={} kB\tVmData={} kB", first.0, first.1);

	for cycle in 1..=CYCLES {
		step(cycle);
		if sample_cycle(cycle) {
			let now = (status_kib("VmRSS:"), status_kib("VmData:"));
			println!("RSS\t{phase}\tcycle={cycle}\tVmRSS={} kB\tVmData={} kB", now.0, now.1);
			if cycle == BASELINE_CYCLE {
				baseline = Some(now);
			}
			end = now;
		}
	}

	let baseline = baseline.unwrap();
	let rss = end.0 as i64 - baseline.0 as i64;
	let data = end.1 as i64 - baseline.1 as i64;
	println!(
		"RSS\t{phase}\tgrowth cycle {BASELINE_CYCLE}..{CYCLES}: VmRSS {rss:+} kB (tolerance {RSS_TOLERANCE_KIB}), VmData {data:+} kB (tolerance {DATA_TOLERANCE_KIB})"
	);
	assert!(rss <= RSS_TOLERANCE_KIB as i64, "{phase}: VmRSS grew {rss} kB");
	assert!(data <= DATA_TOLERANCE_KIB as i64, "{phase}: VmData grew {data} kB");
}

fn lifecycle_region() -> Region {
	let program = compile_source(
		"pub fn main(a) { let n = 0; let total = 0; while n < a { total += n % 17; n += 1; } total }",
	);
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut found = analyze(program.code.function(main).unwrap(), main)
		.unwrap()
		.into_iter();
	found.next().unwrap().1.unwrap()
}

/// The entry buffer of `region` with the parameter (frame slot 0) at `limit`.
fn entry_buffer(region: &Region, limit: i64) -> Vec<i64> {
	region
		.slots
		.iter()
		.zip(region.entry.iter())
		.map(|(&slot, class)| match class {
			EntryClass::Guard(Scalar::I64) if slot == 0 => limit,
			_ => 0,
		})
		.collect()
}

#[test]
#[ignore = "P1 native worker: run by p1_native_lifecycle_and_contract in a child process"]
fn child_lifecycle() {
	require_child();
	let region = lifecycle_region();
	let compiled = compile_region(&region, 0).unwrap();
	let n = region.slots.len();
	println!(
		"LIFECYCLE\tregion slots={n} exits={} code_bytes={} tolerance VmRSS {RSS_TOLERANCE_KIB} kB VmData {DATA_TOLERANCE_KIB} kB from cycle {BASELINE_CYCLE} to {CYCLES}",
		region.exits.len(),
		compiled.bytes().len()
	);

	// Contract: a wrong buffer length or a nonzero `reserved` returns None and runs nothing
	// (the native code writes the record on every exit; it stays exactly as given).
	{
		let native = NativeRegion::load(&compiled).unwrap();
		for len in [n - 1, n + 1, 0] {
			let mut buf = vec![0x5a5a_5a5a_i64; len];
			let mut record = ExitRecord::new(50);
			let before = record;
			assert_eq!(native.call(&mut buf, &mut record), None, "len {len}");
			assert_eq!(record, before);
			assert!(buf.iter().all(|&v| v == 0x5a5a_5a5a));
			println!("CONTRACT\tbuffer length {len} (slots {n}): None, record and buffer unchanged");
		}

		let mut buf = entry_buffer(&region, 30);
		let original = buf.clone();
		let mut record = ExitRecord::new(50);
		record.reserved = 1;
		let before = record;
		assert_eq!(native.call(&mut buf, &mut record), None);
		assert_eq!((record, &buf), (before, &original));
		println!("CONTRACT\trecord.reserved = 1: None, record and buffer unchanged");

		// The same region then runs normally.
		record.reserved = 0;
		assert!(native.call(&mut buf, &mut record).is_some());
		assert_ne!(record.status, 0);
	}

	// Oversized at the program bound: refused by compile, before any load. This is
	// compile-refusal coverage only; it does not reach NativeRegion::load.
	assert_eq!(
		compile_region(&region, MAX_PROGRAM_CODE_BYTES).err(),
		Some(NativeRefusal::Size)
	);
	println!("COMPILE-REFUSAL\tprogram bound: Size before NativeRegion::load (no loader coverage)");

	// Phase 1: load -> call -> drop, checked against the simulator on each call.
	let mut exits = [0usize; 4];
	screened("load_call_drop", |cycle| {
		let native = NativeRegion::load(&compiled).expect("load");
		let fuel = (cycle % 47) as u64;
		let mut buf = entry_buffer(&region, 30);
		let mut sim = buf.clone();
		let mut record = ExitRecord::new(fuel);
		let mut sim_record = record;
		let status = native.call(&mut buf, &mut record).expect("call");
		let sim_status = simulate(&region, &mut sim, &mut sim_record);
		assert_eq!((status, record), (sim_status, sim_record), "cycle {cycle}");
		for l in 0..n {
			if record.written & (1 << l) != 0 {
				assert_eq!(buf[l], sim[l], "cycle {cycle} slot {l}");
			}
		}
		exits[status as usize] += 1;
		drop(native);
	});
	println!(
		"LIFECYCLE\tload_call_drop exits: continue={} fuel={} retry={}",
		exits[Status::Continue as usize],
		exits[Status::Fuel as usize],
		exits[Status::Retry as usize]
	);
	assert!(exits[Status::Continue as usize] > 0 && exits[Status::Fuel as usize] > 0);

	// Phases 2 and 3: an injected error after allocation goes through load's own cleanup.
	for (phase, stage) in [
		("inject_after_define", InjectLoadError::AfterDefine),
		("inject_after_finalize", InjectLoadError::AfterFinalize),
	] {
		let frees = load_error_frees_for_test();
		inject_load_error_for_test(Some(stage));
		screened(phase, |cycle| match NativeRegion::load(&compiled) {
			Err(NativeRefusal::Codegen(text)) => assert!(text.contains("injected"), "{text}"),
			Err(other) => panic!("{phase} cycle {cycle}: {other:?}"),
			Ok(_) => panic!("{phase} cycle {cycle}: load succeeded despite the injection"),
		});
		inject_load_error_for_test(None);
		let taken = load_error_frees_for_test() - frees;
		println!("LIFECYCLE\t{phase}: error-path frees {taken}");
		assert_eq!(taken, CYCLES);
	}

	// Disarmed, load works again.
	let native = NativeRegion::load(&compiled).unwrap();
	let mut buf = entry_buffer(&region, 3);
	let mut record = ExitRecord::new(100);
	assert_eq!(native.call(&mut buf, &mut record), Some(Status::Continue as u32));
}

// ------------------------------------- VmSize and executable-mapping lifecycle screen

/// A second, separate screen over the same three lifecycle phases, fixed before its first
/// run. It watches address space, not resident memory: VmSize (with VmRSS and VmData
/// recorded but not bounded here), the number of mappings in /proc/self/maps and the total
/// size of mappings with execute permission. Growth from cycle 100 to cycle 10,000:
///
/// - One leaked 4 KiB executable page per cycle would add 9,900 x 4 KiB = 39,600 KiB of
///   executable mappings and of VmSize. The exec bound (1,024 KiB, 256 pages) is about 39x
///   below that; the VmSize bound (16,384 KiB) about 2.4x below. VmSize also moves with
///   allocator caching (heap growth, mmap'd chunks), so it is a looser signal; executable
///   bytes are the specific one, since nothing but loaded regions maps code after startup.
/// - The kernel merges adjacent anonymous mappings with equal permissions, so a page leak
///   need not add mappings; the mapping count is a secondary signal (a leak of separate,
///   non-adjacent mappings), bounded at 64.
/// - A leak after `define` but before `finalize` would be read+write, not executable: it
///   shows in VmSize and the mapping count, not in the executable bytes.
const VM_SIZE_TOLERANCE_KIB: i64 = 16 * 1024;
const EXEC_TOLERANCE_KIB: i64 = 1024;
const MAPS_TOLERANCE: i64 = 64;

/// One address-space sample of this process.
struct VmSample {
	vm_size: u64,
	vm_rss: u64,
	vm_data: u64,
	maps: u64,
	exec_kib: u64,
	/// Executable mappings without a path (anonymous: loaded regions land here).
	anon_exec_kib: u64,
	/// Executable mappings by their exact permission field: (count, KiB).
	exec_perms: std::collections::BTreeMap<String, (u64, u64)>,
}

fn vm_sample() -> VmSample {
	let maps = std::fs::read_to_string("/proc/self/maps").expect("/proc/self/maps");
	let mut sample = VmSample {
		vm_size: status_kib("VmSize:"),
		vm_rss: status_kib("VmRSS:"),
		vm_data: status_kib("VmData:"),
		maps: 0,
		exec_kib: 0,
		anon_exec_kib: 0,
		exec_perms: std::collections::BTreeMap::new(),
	};

	for line in maps.lines() {
		let mut fields = line.split_whitespace();
		let range = fields.next().expect("maps range");
		let perms = fields.next().expect("maps perms");
		let path = fields.nth(3);
		let (start, end) = range.split_once('-').expect("maps range");
		let start = u64::from_str_radix(start, 16).expect("maps start");
		let end = u64::from_str_radix(end, 16).expect("maps end");
		let kib = (end - start) / 1024;
		sample.maps += 1;

		if perms.as_bytes().get(2) == Some(&b'x') {
			sample.exec_kib += kib;
			if path.is_none() {
				sample.anon_exec_kib += kib;
			}
			let entry = sample.exec_perms.entry(perms.into()).or_insert((0, 0));
			entry.0 += 1;
			entry.1 += kib;
		}
	}

	sample
}

fn print_vm(phase: &str, cycle: usize, s: &VmSample) {
	let perms: Vec<String> = s
		.exec_perms
		.iter()
		.map(|(p, (count, kib))| format!("{p}:{count}/{kib}kB"))
		.collect();
	println!(
		"VM\t{phase}\tcycle={cycle}\tVmSize={} kB\tVmRSS={} kB\tVmData={} kB\tmaps={}\texec={} kB\tanon_exec={} kB\texec_perms={}",
		s.vm_size,
		s.vm_rss,
		s.vm_data,
		s.maps,
		s.exec_kib,
		s.anon_exec_kib,
		perms.join(",")
	);
}

/// Runs `step` for cycles 1..=CYCLES, sampling the address space, and applies the screen.
fn vm_screened(phase: &str, mut step: impl FnMut(usize)) {
	print_vm(phase, 0, &vm_sample());
	let mut baseline = None;
	let mut end = None;

	for cycle in 1..=CYCLES {
		step(cycle);
		if sample_cycle(cycle) {
			let now = vm_sample();
			print_vm(phase, cycle, &now);
			if cycle == BASELINE_CYCLE {
				baseline = Some((now.vm_size, now.exec_kib, now.maps, now.vm_rss, now.vm_data));
			}
			end = Some((now.vm_size, now.exec_kib, now.maps, now.vm_rss, now.vm_data));
		}
	}

	let (b, e) = (baseline.unwrap(), end.unwrap());
	let size = e.0 as i64 - b.0 as i64;
	let exec = e.1 as i64 - b.1 as i64;
	let maps = e.2 as i64 - b.2 as i64;
	let rss = e.3 as i64 - b.3 as i64;
	let data = e.4 as i64 - b.4 as i64;
	println!(
		"VM\t{phase}\tgrowth cycle {BASELINE_CYCLE}..{CYCLES}: VmSize {size:+} kB (tolerance {VM_SIZE_TOLERANCE_KIB}), exec {exec:+} kB (tolerance {EXEC_TOLERANCE_KIB}), maps {maps:+} (tolerance {MAPS_TOLERANCE}), VmRSS {rss:+} kB, VmData {data:+} kB (recorded only)"
	);
	assert!(size <= VM_SIZE_TOLERANCE_KIB, "{phase}: VmSize grew {size} kB");
	assert!(exec <= EXEC_TOLERANCE_KIB, "{phase}: executable mappings grew {exec} kB");
	assert!(maps <= MAPS_TOLERANCE, "{phase}: mapping count grew {maps}");
}

#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated machine code in a child process")]
fn p1_native_vm_exec_screen() {
	run_child("child_vm_exec_screen");
}

#[test]
#[ignore = "P1 native worker: run by p1_native_vm_exec_screen in a child process"]
fn child_vm_exec_screen() {
	require_child();
	let region = lifecycle_region();
	let compiled = compile_region(&region, 0).unwrap();
	let n = region.slots.len();
	println!(
		"VMSCREEN\tregion slots={n} exits={} code_bytes={} tolerance VmSize {VM_SIZE_TOLERANCE_KIB} kB exec {EXEC_TOLERANCE_KIB} kB maps {MAPS_TOLERANCE} from cycle {BASELINE_CYCLE} to {CYCLES}",
		region.exits.len(),
		compiled.bytes().len()
	);

	// Phase 1: load -> call -> drop, checked against the simulator on each call.
	let mut exits = [0usize; 4];
	vm_screened("load_call_drop", |cycle| {
		let native = NativeRegion::load(&compiled).expect("load");
		let fuel = (cycle % 47) as u64;
		let mut buf = entry_buffer(&region, 30);
		let mut sim = buf.clone();
		let mut record = ExitRecord::new(fuel);
		let mut sim_record = record;
		let status = native.call(&mut buf, &mut record).expect("call");
		let sim_status = simulate(&region, &mut sim, &mut sim_record);
		assert_eq!((status, record), (sim_status, sim_record), "cycle {cycle}");
		for l in 0..n {
			if record.written & (1 << l) != 0 {
				assert_eq!(buf[l], sim[l], "cycle {cycle} slot {l}");
			}
		}
		exits[status as usize] += 1;
		drop(native);
	});
	println!(
		"VMSCREEN\tload_call_drop exits: continue={} fuel={} retry={}",
		exits[Status::Continue as usize],
		exits[Status::Fuel as usize],
		exits[Status::Retry as usize]
	);
	assert!(exits[Status::Continue as usize] > 0 && exits[Status::Fuel as usize] > 0);

	// Phases 2 and 3: an injected error after allocation goes through load's own cleanup.
	for (phase, stage) in [
		("inject_after_define", InjectLoadError::AfterDefine),
		("inject_after_finalize", InjectLoadError::AfterFinalize),
	] {
		let frees = load_error_frees_for_test();
		inject_load_error_for_test(Some(stage));
		vm_screened(phase, |cycle| match NativeRegion::load(&compiled) {
			Err(NativeRefusal::Codegen(text)) => assert!(text.contains("injected"), "{text}"),
			Err(other) => panic!("{phase} cycle {cycle}: {other:?}"),
			Ok(_) => panic!("{phase} cycle {cycle}: load succeeded despite the injection"),
		});
		inject_load_error_for_test(None);
		let taken = load_error_frees_for_test() - frees;
		println!("VMSCREEN\t{phase}: error-path frees {taken}");
		assert_eq!(taken, CYCLES);
	}

	// Disarmed, load works again.
	let native = NativeRegion::load(&compiled).unwrap();
	let mut buf = entry_buffer(&region, 3);
	let mut record = ExitRecord::new(100);
	assert_eq!(native.call(&mut buf, &mut record), Some(Status::Continue as u32));
}

// ------------------------------------------ live executable-mapping positive control

pub(super) fn read_maps() -> String {
	std::fs::read_to_string("/proc/self/maps").expect("/proc/self/maps")
}

pub(super) fn anon_exec_kib(maps: &str) -> u64 {
	maps_overlapping(maps, 0, u64::MAX)
		.iter()
		.filter(|m| m.executable() && m.path.is_none())
		.map(|m| (m.end - m.start) / 1024)
		.sum()
}

fn print_located(phase: &str, located: &[MapInfo]) {
	println!("P1C\t{phase}\tlocated={}", located.len());
	for m in located {
		println!("P1C\t{phase}\tLINE\t{}", m.line);
	}
}

/// P1c: the positive control the VmSize screen lacked. It samples /proc/self/maps before a
/// load, while one `NativeRegion` is alive and after its drop, and locates the mapping that
/// contains the region's private entry address (through the test-only
/// `NativeRegion::entry_mapping`; the pointer does not leave `boundary`):
///
/// - alive: that mapping is executable and not writable (W^X on the actual mapping);
/// - baseline: no executable mapping overlapped that address range before the load;
/// - after drop: no mapping overlapping that range has execute permission. Whether the
///   range is unmapped entirely is recorded, not asserted.
///
/// The anonymous executable KiB delta against the baseline is printed as corroboration only.
#[test]
#[cfg_attr(miri, ignore = "Miri: loads generated machine code in a child process")]
fn p1c_native_live_mapping_positive_control() {
	run_child("child_live_mapping");
}

#[test]
#[ignore = "P1c native worker: run by p1c_native_live_mapping_positive_control in a child process"]
fn child_live_mapping() {
	require_child();
	let region = lifecycle_region();
	let compiled = compile_region(&region, 0).unwrap();

	// Baseline, then load and keep the region alive.
	let baseline = read_maps();
	let native = NativeRegion::load(&compiled).expect("load");
	let alive = read_maps();

	let mapping = native
		.entry_mapping(&alive)
		.expect("the entry address lies in no mapping of /proc/self/maps");
	let (start, end) = (mapping.start, mapping.end);
	println!(
		"P1C\tentry mapping {start:x}-{end:x} ({} KiB) perms={} offset={:x} path={:?} code_bytes={}",
		(end - start) / 1024,
		mapping.perms,
		mapping.offset,
		mapping.path,
		compiled.bytes().len()
	);

	// The region still runs from that mapping while sampled.
	let mut buf = entry_buffer(&region, 3);
	let mut record = ExitRecord::new(100);
	assert_eq!(native.call(&mut buf, &mut record), Some(Status::Continue as u32));

	drop(native);
	let after = read_maps();

	let before = maps_overlapping(&baseline, start, end);
	let live = maps_overlapping(&alive, start, end);
	let freed = maps_overlapping(&after, start, end);
	print_located("baseline", &before);
	print_located("alive", &live);
	print_located("after_drop", &freed);

	let (b, a, d) = (anon_exec_kib(&baseline), anon_exec_kib(&alive), anon_exec_kib(&after));
	println!(
		"P1C\tanon_exec KiB (corroboration only): baseline={b} alive={a} after_drop={d} delta_alive={:+} delta_after_drop={:+}",
		a as i64 - b as i64,
		d as i64 - b as i64
	);
	println!(
		"P1C\tafter_drop: range {start:x}-{end:x} {}",
		if freed.is_empty() { "unmapped" } else { "still (partly) mapped" }
	);

	assert!(mapping.executable(), "alive: entry mapping not executable: {}", mapping.line);
	assert!(!mapping.writable(), "alive: entry mapping writable (W^X): {}", mapping.line);
	assert!(
		before.iter().all(|m| !m.executable()),
		"baseline: an executable mapping already overlapped {start:x}-{end:x}"
	);
	assert!(
		freed.iter().all(|m| !m.executable()),
		"after drop: {start:x}-{end:x} is still mapped executable"
	);
}

/// Synthetic maps lines: the parser alone, no file system (runs in process and under Miri).
#[test]
fn p1c_maps_parser_synthetic_lines() {
	let anon = parse_maps_line("7f1c2a400000-7f1c2a401000 r-xp 00000000 00:00 0").unwrap();
	assert_eq!((anon.start, anon.end), (0x7f1c2a400000, 0x7f1c2a401000));
	assert_eq!((anon.perms.as_str(), anon.offset, anon.path.as_deref()), ("r-xp", 0, None));
	assert!(anon.executable() && !anon.writable());

	let heap = parse_maps_line(
		"55d0c0a3e000-55d0c0a5f000 rw-p 00000000 00:00 0                          [heap]",
	)
	.unwrap();
	assert_eq!(heap.path.as_deref(), Some("[heap]"));
	assert!(heap.writable() && !heap.executable());

	let spaced = "7f1c2b000000-7f1c2b022000 r-xp 0001a000 fd:01 1234567                    /home/u/dir with spaces/lib x.so (deleted)";
	let spaced = parse_maps_line(spaced).unwrap();
	assert_eq!(spaced.path.as_deref(), Some("/home/u/dir with spaces/lib x.so (deleted)"));
	assert_eq!((spaced.offset, spaced.perms.as_str()), (0x1a000, "r-xp"));

	let wx = parse_maps_line("7f0000000000-7f0000002000 rwxp 00000000 00:00 0").unwrap();
	assert!(wx.writable() && wx.executable());

	for bad in [
		"",
		"7f1c2a400000 r-xp 00000000 00:00 0",
		"7f1c2a401000-7f1c2a400000 r-xp 00000000 00:00 0",
		"7f1c2a400000-7f1c2a401000 r-xp 00000000 00:00",
		"zz-7f1c2a401000 r-xp 00000000 00:00 0",
	] {
		assert_eq!(parse_maps_line(bad), None, "{bad:?}");
	}

	let text = "1000-3000 r-xp 00000000 00:00 0\n3000-4000 rw-p 00000000 00:00 0\n8000-9000 r--p 00000000 fd:01 7 /lib/a b";
	let hit: Vec<u64> = maps_overlapping(text, 0x2fff, 0x3001).iter().map(|m| m.start).collect();
	assert_eq!(hit, [0x1000, 0x3000]);
	assert!(maps_overlapping(text, 0x4000, 0x8000).is_empty());
	assert_eq!(maps_overlapping(text, 0, u64::MAX).len(), 3);
}
