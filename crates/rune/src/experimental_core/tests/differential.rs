//! Bounded stock-VM versus execution differential smoke test.
//!
//! Case data: `differential/differential_cases.tsv` and `differential/programs/*.rn`
//! (origin in `differential/PROVENANCE.txt`).
//!
//! Each function is compiled on its own on both engines, as the reference runners did
//! (one unit per `pub fn` line); see [`unit_source`].
//!
//! Each row compares current stock and execution outcomes and witness effects.
//! Unknown stock error classes and unencodable values fail closed. The corpus runs
//! 675 rows, including a 20-row Miri tier.
//!
//! `host::mark(x)` and `host::t(n, x)` of the original receipts are re-expressed with a
//! caller-owned witness vector `w`: `fn mark(w, x) { w.push(0); x }` and
//! `fn t(w, n, x) { w.push(n); x }`. The witness vectors are compared in order.

use crate::experimental_core::execute::Executor;
use crate::experimental_core::frontend;
use crate::experimental_core::heap::{Limits, Store};
use crate::experimental_core::value::{Root, Scalar, Type};
use crate::runtime::{Inline, Value, VmErrorKind};
use crate::{Context, Diagnostics, Hash, Source, Sources};

const CASES: &str = include_str!("differential/differential_cases.tsv");
const PROGRAMS: &[(&str, &str)] = &[
	(
		"operators",
		include_str!("differential/programs/operators.rn"),
	),
	(
		"addendum",
		include_str!("differential/programs/addendum.rn"),
	),
	("rebind", include_str!("differential/programs/rebind.rn")),
	(
		"supplement",
		include_str!("differential/programs/supplement.rn"),
	),
	("objfmt", include_str!("differential/programs/objfmt.rn")),
];
const HEADER: &str = "tier\tbatch\tid\tfunction\toperands\trule";
const FULL_ROWS: usize = 675;
const MIRI_ROWS: usize = 20;
// Only these frozen receipt rows have deliberately converged in F1. Every
// identity and rule is checked before execution, including the Miri subset.
const CONVERGED: &[(&str, &str, &str, &str)] = &[
	// S-index: negative-index class converged to IntegerCoercion.
	("operators", "088710", "index_get", "-"),
	("operators", "088725", "index_set", "-"),
	(
		"operators",
		"044944",
		"and",
		"strict-bool-lazy (chatd 1932)",
	),
	(
		"operators",
		"044945",
		"and",
		"strict-bool-lazy (chatd 1932)",
	),
	("operators", "047753", "or", "strict-bool-lazy (chatd 1932)"),
	("operators", "047754", "or", "strict-bool-lazy (chatd 1932)"),
	(
		"operators",
		"050562",
		"and_effect",
		"strict-bool-lazy (chatd 1932)",
	),
	(
		"operators",
		"050563",
		"and_effect",
		"strict-bool-lazy (chatd 1932)",
	),
	(
		"operators",
		"053371",
		"or_effect",
		"strict-bool-lazy (chatd 1932)",
	),
	(
		"operators",
		"053372",
		"or_effect",
		"strict-bool-lazy (chatd 1932)",
	),
	("objfmt", "000000", "objdbg_yx", "-"),
	(
		"objfmt",
		"000001",
		"objdbg_xy",
		"insertion-order-object-debug (chatd 2100); stock side compared by content",
	),
	(
		"objfmt",
		"000002",
		"objdbg_six_down",
		"insertion-order-object-debug (chatd 2100); stock side compared by content",
	),
	(
		"objfmt",
		"000003",
		"objdbg_six_up",
		"insertion-order-object-debug (chatd 2100); stock side compared by content",
	),
	("objfmt", "000005", "objdbg_insert_built", "-"),
	("objfmt", "000006", "objdbg_overwrite", "-"),
	("addendum", "000370", "debug_object", "-"),
	("addendum", "000371", "debug_object", "-"),
	("addendum", "000372", "debug_object", "-"),
	("addendum", "000373", "debug_object", "-"),
	("addendum", "000374", "debug_object", "-"),
	("addendum", "000375", "debug_object", "-"),
	("addendum", "000376", "debug_object", "-"),
	("addendum", "000377", "debug_object", "-"),
	("addendum", "000378", "debug_object", "-"),
	("addendum", "000379", "debug_object", "-"),
	("addendum", "000380", "debug_object", "-"),
	("addendum", "000381", "debug_object", "-"),
	("addendum", "000382", "debug_object", "-"),
];

struct Case<'a> {
	tier: &'a str,
	batch: &'a str,
	id: &'a str,
	function: &'a str,
	operands: std::vec::Vec<&'a str>,
	rule: &'a str,
}

fn cases() -> std::vec::Vec<Case<'static>> {
	let mut lines = CASES.lines();
	assert_eq!(lines.next(), Some(HEADER), "case table header");
	lines
		.map(|line| {
			let f: std::vec::Vec<&str> = line.split('\t').collect();
			assert_eq!(f.len(), 6, "malformed row {line}");
			assert!(matches!(f[0], "full" | "miri+full"), "tier {}", f[0]);
			Case {
				tier: f[0],
				batch: f[1],
				id: f[2],
				function: f[3],
				operands: if f[4].is_empty() {
					std::vec::Vec::new()
				} else {
					f[4].split(';').collect()
				},
				rule: f[5],
			}
		})
		.collect()
}

/// The stock error kind's class at one site, or `None` when that site has no entry.
///
/// Operators: the site tables of OPERATOR_AMENDMENT.md (`operators_amended.py` `SITE`
/// plus `COMMON`), keyed by the function's site. Every other batch: the explicit
/// (batch, function) entries of CANDIDATE_REPLAY.md R1; nothing is borrowed between sites.
fn runtime_error_class(batch: &str, function: &str, kind: &VmErrorKind) -> Option<&'static str> {
	use VmErrorKind as K;
	if batch == "operators" {
		let base = match function.split_once("_assign") {
			Some((base, _)) => base,
			None => function.strip_suffix("_effect").unwrap_or(function),
		};

		let site = if matches!(base, "shl" | "shr") {
			"shift"
		} else if function.starts_with("index_") {
			"index"
		} else if function.starts_with("as_") {
			"cast"
		} else if matches!(function, "neg" | "bit_not") || function.starts_with("is_") {
			"unary"
		} else {
			"binary"
		};

		let sited = match (site, kind) {
			("shift", K::Overflow | K::Underflow) => Some("Overflow"),
			("shift", K::ValueToIntegerCoercionError { .. }) => Some("IntegerCoercion"),

			("index", K::MissingIndexInteger { .. } | K::OutOfRange { .. }) => {
				Some("IndexOutOfBounds")
			}
			("index", K::ValueToIntegerCoercionError { .. }) => Some("IntegerCoercion"),
			("index", K::UnsupportedIndexGet { .. } | K::UnsupportedIndexSet { .. }) => {
				Some("Type")
			}

			("cast", K::UnsupportedAs { .. } | K::UnsupportedAsTarget { .. }) => {
				Some("InvalidCast")
			}
			("binary", K::Overflow | K::Underflow) => Some("Overflow"),
			("binary", K::DivideByZero) => Some("DivisionByZero"),
			("binary", K::ValueToIntegerCoercionError { .. }) => Some("IntegerCoercion"),
			("unary", K::Overflow) => Some("Overflow"),
			_ => None,
		};

		return sited.or(match kind {
			K::UnsupportedBinaryOperation { .. }
			| K::UnsupportedUnaryOperation { .. }
			| K::ExpectedNumber { .. }
			| K::ExpectedType { .. }
			| K::UnsupportedIs { .. } => Some("Type"),
			_ => None,
		});
	}

	match (batch, function, kind) {
		(
			"addendum",
			"display" | "template" | "display_two" | "display_vec",
			K::MissingProtocolFunction { .. },
		) => Some("Type"),
		(
			"addendum",
			"order_binary" | "order_compare" | "order_index_compound" | "order_field_compound",
			K::ExpectedType { .. },
		) => Some("Type"),
		("addendum", "order_range", K::UnsupportedIterRange { .. }) => Some("Type"),
		("supplement", "order_supp_field_read_missing", K::MissingField { .. }) => {
			Some("IndexOutOfBounds")
		}

		("supplement", "order_supp_field_compound_missing", K::MissingField { .. }) => {
			Some("IndexOutOfBounds")
		}
		_ => None,
	}
}

/// Operand grammar of the replay: `i:` `u:` `f:`(64-bit hex) `b:0|1` `c:`(hex) `s:`(hex|-) `n`.
fn runtime_operand(text: &str) -> Value {
	if text == "n" {
		return crate::to_value(()).unwrap();
	}

	let (tag, body) = text.split_once(':').unwrap();
	match tag {
		"i" => crate::to_value(body.parse::<i64>().unwrap()).unwrap(),
		"u" => crate::to_value(body.parse::<u64>().unwrap()).unwrap(),
		"f" => crate::to_value(f64::from_bits(u64::from_str_radix(body, 16).unwrap())).unwrap(),
		"b" => crate::to_value(bool_operand(body)).unwrap(),
		"c" => crate::to_value(char_operand(body)).unwrap(),
		"s" => crate::to_value(crate::alloc::String::try_from(hex_text(body).as_str()).unwrap())
			.unwrap(),
		other => panic!("operand tag {other}"),
	}
}

fn operand(store: &mut Store, text: &str) -> Root {
	if text == "n" {
		return store.root_unit();
	}

	let (tag, body) = text.split_once(':').unwrap();
	match tag {
		"i" => store.root_i64(body.parse().unwrap()),
		"u" => store.root_u64(body.parse().unwrap()),
		"f" => store.root_f64(f64::from_bits(u64::from_str_radix(body, 16).unwrap())),
		"b" => store.root_bool(bool_operand(body)),
		"c" => store.root_char(char_operand(body)),
		"s" => store.root_string(&hex_text(body)).unwrap(),
		other => panic!("operand tag {other}"),
	}
}

fn bool_operand(body: &str) -> bool {
	match body {
		"0" => false,
		"1" => true,
		other => panic!("bool operand {other}"),
	}
}

fn char_operand(body: &str) -> char {
	char::from_u32(u32::from_str_radix(body, 16).unwrap()).unwrap()
}

fn hex_text(body: &str) -> std::string::String {
	if body == "-" {
		return std::string::String::new();
	}

	assert!(body.len().is_multiple_of(2), "hex text {body}");
	let bytes: std::vec::Vec<u8> = (0..body.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&body[i..i + 2], 16).unwrap())
		.collect();
	std::string::String::from_utf8(bytes).unwrap()
}

fn enc_str(s: &str) -> std::string::String {
	if s.is_empty() {
		return "s:-".into();
	}

	let mut out = std::string::String::from("s:");
	for b in s.bytes() {
		out.push_str(&std::format!("{b:02x}"));
	}

	out
}

/// Encodes a new result in the replay grammar by its own tag (scalars, strings and
/// flat vectors); anything else is reported as unencodable and fails the row.
fn encode(store: &mut Store, root: &Root) -> std::string::String {
	match store.root_tag(root).unwrap() {
		Type::String => enc_str(store.root_text(root).unwrap()),
		Type::Vec => {
			let n = store.root_len(root).unwrap();
			let mut parts = std::vec::Vec::new();

			for i in 0..n {
				let item = store.root_get(root, i).unwrap();
				parts.push(encode(store, &item));
				store.root_release(item).unwrap();
			}

			std::format!("v[{}]", parts.join(","))
		}

		tag => match store.root_scalar(root).unwrap() {
			Some(Scalar::I64(n)) => std::format!("i:{n}"),
			Some(Scalar::U64(n)) => std::format!("u:{n}"),
			Some(Scalar::F64(x)) => std::format!("f:{:016x}", x.to_bits()),
			Some(Scalar::Bool(b)) => std::format!("b:{}", b as u8),
			Some(Scalar::Char(c)) => std::format!("c:{:x}", c as u32),
			Some(Scalar::Ordering(v)) => std::format!("ordering:{v:?}"),
			other => std::format!("unencodable:{tag:?}:{other:?}"),
		},
	}
}

/// Encodes a stock result by its own runtime type, never by the expected encoding, so a
/// tag mismatch cannot be hidden by a conversion.
fn legacy_encode(value: &Value) -> std::string::String {
	if let Some(inline) = value.as_inline() {
		return match *inline {
			Inline::Signed(n) => std::format!("i:{n}"),
			Inline::Unsigned(n) => std::format!("u:{n}"),
			Inline::Float(x) => std::format!("f:{:016x}", x.to_bits()),
			Inline::Bool(b) => std::format!("b:{}", b as u8),
			Inline::Char(c) => std::format!("c:{:x}", c as u32),
			ref other => std::format!("unencodable:{other:?}"),
		};
	}

	if let Ok(text) = value.borrow_string_ref() {
		return enc_str(&text);
	}

	if let Ok(vec) = value.borrow_ref::<crate::runtime::Vec>() {
		let parts: std::vec::Vec<_> = vec.iter().map(legacy_encode).collect();
		return std::format!("v[{}]", parts.join(","));
	}

	std::format!("unencodable:{:?}", value.type_hash())
}

/// Whether `pub fn <name>(…)` in `text` takes the witness `w` as its last parameter.
fn takes_witness(text: &str, name: &str) -> bool {
	let start = std::format!("pub fn {name}(");
	let at = text
		.find(&start)
		.unwrap_or_else(|| panic!("no function {name}"));
	let params = &text[at + start.len()..];
	let params = &params[..params.find(')').unwrap()];
	params.rsplit(',').map(str::trim).next() == Some("w")
}

/// The source of one function, compiled on its own as the reference runners did: its
/// `pub fn` line (which carries any helper or constant it declares) plus the shared
/// non-`pub` helper lines of the batch (the witness functions `mark` and `t`).
fn unit_source(text: &str, function: &str) -> std::string::String {
	let start = std::format!("pub fn {function}(");
	let mut lines = text.lines().filter(|l| l.starts_with(&start));
	let line = lines
		.next()
		.unwrap_or_else(|| panic!("no function {function}"));
	assert!(lines.next().is_none(), "duplicate function {function}");
	let mut out = std::string::String::from(line);

	for helper in text.lines().filter(|l| !l.starts_with("pub fn ")) {
		out.push('\n');
		out.push_str(helper);
	}

	out
}

/// Builds one function's unit on both engines; a build failure on either side panics.
pub(super) fn compile_unit(
	context: &Context,
	batch: &str,
	text: &str,
	function: &str,
) -> (crate::runtime::Vm, frontend::Program) {
	let source = unit_source(text, function);
	let mut sources = Sources::new();
	sources.insert(Source::memory(&source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let vm = crate::prepare(&mut sources)
		.with_context(context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let vm = vm.unwrap_or_else(|e| {
		panic!(
			"stock build of {batch} {function}: {e:?}: {:?}",
			diagnostics.diagnostics()
		)
	});
	let mut sources = Sources::new();
	sources.insert(Source::memory(&source).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1)
		.unwrap_or_else(|e| panic!("execution compile of {batch} {function}: {e:?}"));
	(vm, program)
}

fn run(miri_only: bool) {
	let all = cases();
	assert_eq!(all.len(), FULL_ROWS, "corpus rows");
	assert_eq!(CONVERGED.len(), 29);
	assert_eq!(
		CONVERGED.iter().filter(|row| row.0 == "operators").count(),
		10
	);
	assert_eq!(CONVERGED.iter().filter(|row| row.0 == "objfmt").count(), 6);
	assert_eq!(
		CONVERGED.iter().filter(|row| row.0 == "addendum").count(),
		13
	);
	for &(batch, id, function, rule) in CONVERGED {
		let rows: std::vec::Vec<_> = all
			.iter()
			.filter(|c| c.batch == batch && c.id == id)
			.collect();
		assert_eq!(rows.len(), 1, "converged identity {batch}/{id}");
		assert_eq!((rows[0].function, rows[0].rule), (function, rule));
	}
	for case in &all {
		if case.rule == "strict-bool-lazy (chatd 1932)"
			|| case.batch == "objfmt"
			|| (case.batch == "addendum" && case.function == "debug_object")
		{
			assert!(CONVERGED
				.iter()
				.any(|row| (row.0, row.1) == (case.batch, case.id)));
		}
	}

	assert_eq!(
		all.iter().filter(|c| c.tier == "miri+full").count(),
		MIRI_ROWS,
		"miri tier rows"
	);
	let context = Context::with_default_modules().unwrap();
	let mut mismatches = std::vec::Vec::new();
	let mut ran = 0;

	for (batch, text) in PROGRAMS {
		let rows: std::vec::Vec<&Case<'_>> = all
			.iter()
			.filter(|c| c.batch == *batch && (!miri_only || c.tier == "miri+full"))
			.collect();
		if rows.is_empty() {
			continue;
		}

		let mut units = std::collections::BTreeMap::new();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();

		for case in rows {
			ran += 1;
			let at = std::format!("{} {} {}", case.batch, case.id, case.function);
			let witness = takes_witness(text, case.function);
			let (vm, program) = units
				.entry(case.function)
				.or_insert_with(|| compile_unit(&context, batch, text, case.function));

			// ---- stock side ----
			let mut args: crate::alloc::Vec<Value> = crate::alloc::Vec::new();
			for op in &case.operands {
				args.try_push(runtime_operand(op)).unwrap();
			}

			let legacy_w = crate::to_value(crate::runtime::Vec::new()).unwrap();
			if witness {
				args.try_push(legacy_w.clone()).unwrap();
			}

			let got_stock = match vm.call(Hash::type_hash([case.function]), args) {
				Ok(v) => ("ok", legacy_encode(&v)),
				Err(e) => {
					let kind = e.into_kind();
					match runtime_error_class(case.batch, case.function, &kind) {
						Some(class) => ("err", class.into()),
						None => ("err", std::format!("unmapped:{kind:?}")),
					}
				}
			};

			assert!(
				!got_stock.1.starts_with("unmapped:") && !got_stock.1.contains("unencodable:"),
				"stock {at}: {got_stock:?}"
			);
			let stock_effects = if witness {
				let w = legacy_w.borrow_ref::<crate::runtime::Vec>().unwrap();
				w.iter()
					.map(|v| match v.as_inline() {
						Some(Inline::Signed(n)) => *n,
						other => panic!("{at}: stock witness item {other:?}"),
					})
					.collect::<std::vec::Vec<_>>()
			} else {
				std::vec::Vec::new()
			};

			// ---- new side ----
			let roots: std::vec::Vec<Root> = case
				.operands
				.iter()
				.map(|op| operand(&mut store, op))
				.collect();
			let w = store.root_list(Type::Vec, &[]).unwrap();
			let mut refs: std::vec::Vec<&Root> = roots.iter().collect();

			if witness {
				refs.push(&w);
			}

			let function = program
				.function(Hash::type_hash([case.function]))
				.unwrap_or_else(|| panic!("{at}: no execution function"));
			let got = match executor.invoke(program, &mut store, function, &refs, 10_000_000) {
				Ok(root) => {
					let s = encode(&mut store, &root);
					store.root_release(root).unwrap();
					("ok", s)
				}
				Err(failure) => ("err", std::format!("{:?}", failure.error.kind)),
			};

			assert!(!got.1.contains("unencodable:"), "execution {at}: {got:?}");
			if got != got_stock {
				mismatches.push(std::format!("{at}: execution {got:?}, stock {got_stock:?}"));
			}

			if witness {
				let mut got = std::vec::Vec::new();
				for i in 0..store.root_len(&w).unwrap() {
					let item = store.root_get(&w, i).unwrap();
					match store.root_scalar(&item).unwrap() {
						Some(Scalar::I64(x)) => got.push(x),
						other => panic!("{at}: execution witness item {other:?}"),
					}

					store.root_release(item).unwrap();
				}

				if got != stock_effects {
					mismatches.push(std::format!(
						"effects {at}: execution {got:?}, stock {stock_effects:?}"
					));
				}
			}

			store.root_release(w).unwrap();
			for root in roots {
				store.root_release(root).unwrap();
			}

			assert_eq!(store.live_entries(), 0, "leak after {at}");
			assert!(executor.is_idle(), "executor not idle after {at}");
		}
	}

	assert_eq!(
		ran,
		if miri_only { MIRI_ROWS } else { FULL_ROWS },
		"rows run"
	);
	assert!(
		mismatches.is_empty(),
		"{} of {ran} rows disagree:\n{}",
		mismatches.len(),
		mismatches.join("\n")
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full 675-row differential corpus: native CI only, far too slow under Miri; its 20-row Miri tier is opt-in (--ignored differential_miri_tier)"
)]
fn differential_full_corpus() {
	run(false);
	#[cfg(all(feature = "std", not(miri)))]
	super::gap_hunt::contract_completed("both", FULL_ROWS);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: 20-row stock-vs-execution differential tier builds a std Context and compiles each case on both engines (~20 min); opt in with --ignored"
)]
fn differential_miri_tier() {
	run(true);
}
