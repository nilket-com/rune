//! D2 catalog: compile both engines first, then compare owned diagnostics.
//!
//! Both: the 50 message/span rows compared against live stock, all ten
//! assignment symbols, and the mutation/read-lease case. Runtime classes remain
//! coarse; no source semantics or failure precedence changes here.
//! Retained: iterator and method hashes, macro Display spans, tuple-set past-end
//! classification, and nominal names pending C. D1b wording changes are out of scope.
//! S-field converges named object insertion and Option iteration; Function cloning converges; its bytecode-offset Debug observation stays Retained.
//! S-index converges dynamic object indexing and Vec tuple-index access.
//! Refused: closures, explicit iterator methods, and inclusive ranges. Compile
//! errors compare frontend diagnostics, never VmError text.
//! Host misuse/resources stay generic: foreign tokens have dedicated detail;
//! InvalidCode/InvalidHandle/Allocation/capacity/fuel/cancellation are not enriched.
//! B1 owns stdout errors. Formatting has local resource-first pending-budget and
//! retained-scratch preflights: they can precede later child errors, without
//! globally outranking earlier errors. S owns dynamic missing-key storage.
//! Existing hand-built-code oracle tests retain all pre-D2 metadata assertions;
//! this catalog and pure rendering tests check the newly added detail field.
use crate::experimental_core::{
	error::ErrorKind,
	execute::Executor,
	frontend,
	heap::{Limits, Store},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::string::ToString;

fn compile(context: &Context, text: &str) -> (crate::runtime::Vm, frontend::Program) {
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let stock = crate::prepare(&mut sources)
		.with_context(context)
		.build_vm()
		.unwrap_or_else(|e| panic!("stock must compile {text}: {e:?}"));
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let execution = frontend::compile(&mut sources, context, &mut Diagnostics::new(), 73)
		.unwrap_or_else(|e| panic!("execution must admit {text}: {e:?}"));
	(stock, execution)
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn diagnostic_catalog_both_engines() {
	use ErrorKind::*;
	let context = Context::with_default_modules().unwrap();
	// Every row below is Both: no silent stock-only or new-only escape hatch.
	let rows = [
		("binary", "1 + true", Type, "primitive operation"),
		("multiline", "1 +\n true", Type, "primitive operation"),
		("binary-float", "1 + 1.0", Type, "primitive operation"),
		("char", "'a' + 1", Type, "primitive operation"),
		("unit", "() + 1", Type, "primitive operation"),
		("type", "i64 + 1", Type, "primitive operation"),
		("function", "let f = g; f + 1", Type, "primitive operation"),
		(
			"assign",
			"let x = 1; x += true",
			Type,
			"primitive operation",
		),
		("assign-string", "let x = \"a\"; x += 1", Type, "store"),
		("string", "\"a\" + 1", Type, "primitive operation"),
		("string-compare", "\"a\" < 1", Type, "primitive operation"),
		(
			"string-unsupported",
			"\"a\" - \"b\"",
			Type,
			"primitive operation",
		),
		(
			"overflow",
			"let x = 9223372036854775807; x + 1",
			Overflow,
			"primitive operation",
		),
		(
			"underflow",
			"let x = -9223372036854775808; x - 1",
			Overflow,
			"primitive operation",
		),
		(
			"unsigned-underflow",
			"1u64 - 2u64",
			Overflow,
			"primitive operation",
		),
		("divide", "1 / 0", DivisionByZero, "primitive operation"),
		("remainder", "1 % 0", DivisionByZero, "primitive operation"),
		(
			"coerce-signed",
			"1 + 18446744073709551615u64",
			IntegerCoercion,
			"primitive operation",
		),
		(
			"coerce-unsigned",
			"1u64 + -1",
			IntegerCoercion,
			"primitive operation",
		),
		(
			"shift-negative",
			"1 << -1",
			IntegerCoercion,
			"primitive operation",
		),
		(
			"shift-wide",
			"1 << 4294967296",
			IntegerCoercion,
			"primitive operation",
		),
		("shift-width", "1 << 64", Overflow, "primitive operation"),
		(
			"shift-unsigned",
			"1 << 18446744073709551615u64",
			IntegerCoercion,
			"primitive operation",
		),
		("neg", "-true", Type, "primitive operation"),
		("not", "!\"a\"", Type, "primitive operation"),
		(
			"cast-source",
			"true as i64",
			InvalidCast,
			"primitive operation",
		),
		(
			"cast-target",
			"1 as bool",
			InvalidCast,
			"primitive operation",
		),
		("condition", "if 1 {}", Type, "execute"),
		("type-condition", "if i64 {}", Type, "execute"),
		("function-condition", "let f = g; if f {}", Type, "execute"),
		// S-index: runtime ValueToIntegerCoercionError remains a coercion class.
		("negative-index", "[1][-1]", IntegerCoercion, "execute"),
		("non-number-index", "[1][1.5]", Type, "execute"),
		("bounds", "[1, 2][5]", IndexOutOfBounds, "store"),
		(
			"wide-bounds",
			"[1][18446744073709551615u64]",
			IndexOutOfBounds,
			"store",
		),
		("tuple-bounds", "(1, 2).5", IndexOutOfBounds, "store"),
		(
			"set-bounds",
			"let v = [1]; v[1] = 2",
			IndexOutOfBounds,
			"store",
		),
		("get-target", "let x = 1; x[0]", Type, "execute"),
		("set-target", "let x = 1; x[0] = 2", Type, "store"),
		("compound-get", "let x = 1; x[0] += 1", Type, "execute"),
		("tuple-get", "let x = 1; x.0", Type, "execute"),
		("tuple-set", "let x = 1; x.0 = 2", Type, "execute"),
		("tuple-compound", "let x = 1; x.0 += 1", Type, "execute"),
		(
			"missing-field",
			"let o = #{a:1}; o.b",
			IndexOutOfBounds,
			"store",
		),
		("field-target", "let x = 1; x.b", Type, "store"),
		(
			"field-set-target",
			"let x = 1; x.b = 2",
			IndexOutOfBounds,
			"store",
		),
		("call-target", "let x = 1; x()", Type, "execute"),
		("arity-zero", "let f = g; f()", ArgumentCount, "execute"),
		("arity-two", "let f = g; f(1,2)", ArgumentCount, "execute"),
		("try", "1?", Type, "store"),
		// Endpoint validation is deferred until iteration, matching stock.
		("range", "for i in 0..1.5 {}", Type, "store"),
	];
	let mut failures = std::vec::Vec::new();
	for (id, body, kind, operation) in rows {
		// Failures occur on line 3 or later. The helper also makes named-function
		// values available without admitting closures.
		let source =
			std::format!("pub fn target() {{\n\tlet unused = ();\n\t{body}\n}}\nfn g(a) {{ a }}");
		let (mut stock, program) = compile(&context, &source);
		let runtime_error = stock.call(["target"], ()).unwrap_err();
		let at = runtime_error.first_location().unwrap();
		let inst = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
		let legacy_span = inst.span.range();
		let source_id = inst.source_id.into_index();
		let legacy_text = runtime_error.to_string();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		let text = failure.message(&program).to_string();
		let at = failure.error.location.unwrap();
		if (
			text.as_str(),
			failure.error.kind,
			failure.error.operation,
			at.start as usize..at.end as usize,
			at.source as usize,
			at.revision,
		) != (
			legacy_text.as_str(),
			kind,
			operation,
			legacy_span.clone(),
			source_id,
			73,
		) {
			failures.push(std::format!(
				"{id}: stock={legacy_text:?} {legacy_span:?}; execution={text:?} {:?} {at:?} op={}",
				failure.error.kind,
				failure.error.operation
			));
		}
		assert!(failure.call_chain.is_empty(), "{id}");
		assert!(failure.cleanup_error.is_none(), "{id}");
		assert!(executor.is_idle(), "{id}");
		assert_eq!(store.live_entries(), 0, "{id}");
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn retained_diagnostics_and_semantic_gaps_are_explicit() {
	let context = Context::with_default_modules().unwrap();
	// len/push wording now converges. D1b pins the local DISPLAY_FMT argument site.
	for (id, body, expected) in [
		(
			"len",
			"let x = 1; x.len()",
			"Missing instance function `0x201f597bd58fafa8` for `std::i64`",
		),
		(
			"push",
			"let x = 1; x.push(2)",
			"Missing instance function `0x3fd16e7ebc3afb1a` for `std::i64`",
		),
		(
			"display",
			"format!(\"{}\", [1])",
			"Missing protocol function `DISPLAY_FMT` for `std::vec::Vec`",
		),
	] {
		let source = std::format!("pub fn target() {{\n\n{body}\n}}");
		let (mut stock, program) = compile(&context, &source);
		let stock = stock.call(["target"], ()).unwrap_err();
		let at = stock.first_location().unwrap();
		let legacy_actual_span = at
			.unit
			.debug_info()
			.unwrap()
			.instruction_at(at.ip)
			.unwrap()
			.span
			.range();
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = Executor::new()
			.unwrap()
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		// D1 preserves the original hash text above and declares only the new rendering.
		let d1_expected = match id {
			"len" => "Type `std::i64` has no method named `len`",
			"push" => "Type `std::i64` has no method named `push`",
			"display" => "`std::vec::Vec` cannot be formatted with `{}`",
			_ => expected,
		};
		assert_eq!(
			failure.error.message(&program).to_string(),
			d1_expected,
			"{id}"
		);
		let (legacy_text, legacy_span, span, reason) = match id {
			"iter" => ("Missing instance function `0x57a2b2f073daaf28` for `std::i64`",28..29,19..32,"stock points at iterable; execution points at the for expression; hash wording retained"),
			"len" => ("Missing instance function `0x201f597bd58fafa8` for `std::i64`",30..37,30..37,"instance-function hash converged exactly"),
			"push" => ("Missing instance function `0x3fd16e7ebc3afb1a` for `std::i64`",30..39,30..39,"instance-function hash converged exactly"),
			"display" => ("Missing protocol function `DISPLAY_FMT` for `std::vec::Vec`",33..36,33..36,"D1b maps the failing expression inside format!"),
			_ => unreachable!(),
		};
		assert!(!reason.is_empty());
		let d1_stock = match id {
			"len" | "push" => "Type `std::i64` has no matching method",
			"iter" => "Cannot iterate over `std::i64`",
			"display" => "`std::vec::Vec` cannot be formatted with `{}`",
			_ => legacy_text,
		};
		assert_eq!(stock.into_kind().to_string(), d1_stock, "{id}");
		assert_eq!(legacy_actual_span, legacy_span, "{id}");
		let at = failure.error.location.unwrap();
		#[cfg(not(miri))]
		if id == "display" {
			let legacy_site = std::format!("0:{}:{}", legacy_span.start, legacy_span.end);
			let site = std::format!("0:{}:{}", at.start, at.end);
			assert!(super::gap_hunt::d1b_diagnostics::intentional_site_before(
				"diagnostics",
				"display",
				&source,
				"stock",
				&legacy_site,
				"0:0:36",
			)
			.is_some());
			assert!(super::gap_hunt::d1b_diagnostics::intentional_site_before(
				"diagnostics",
				"display",
				&source,
				"fresh",
				&site,
				"0:19:37",
			)
			.is_some());
		}
		assert_eq!(at.start as usize..at.end as usize, span, "{id}");
		assert_eq!((at.source, at.revision), (0, 73));
	}
	// Function clone succeeds. Stock Debug prints an instruction offset; new
	// prints the function index instead, as its closure form does.
	let source="fn g(a) {a} pub fn target() { let f=g; f.clone() } pub fn call_clone() { let f=g.clone(); f(7) }";
	let (mut stock, program) = compile(&context, source);
	assert_eq!(
		stock.call(["target"], ()).unwrap().type_info().to_string(),
		"::std::ops::Function"
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let function = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["target"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap();
	let super::super::value::Value::Function { id, .. } = function.value else {
		panic!("function value");
	};
	assert_eq!(
		store.root_format(&function, true).unwrap().as_str(),
		std::format!("immediate function (function: {id})")
	);
	store.root_release(function).unwrap();
	let answer = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["call_clone"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap();
	assert_eq!(
		store.root_scalar(&answer).unwrap(),
		Some(super::super::value::Scalar::I64(7))
	);
	store.root_release(answer).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn diagnostics_in_a_callee_keep_full_spans_and_call_chain_after_cleanup() {
	let context = Context::with_default_modules().unwrap();
	let source = "pub fn target() {\n\n outer()\n}\nfn outer() {\n inner()\n}\nfn inner() {\n let o = #{a:1};\n o.missing\n}";
	let (mut stock, program) = compile(&context, source);
	let error = stock.call(["target"], ()).unwrap_err();
	let at = error.first_location().unwrap();
	let info = at.unit.debug_info().unwrap();
	let span = info.instruction_at(at.ip).unwrap().span.range();
	let legacy_chain: std::vec::Vec<_> = at
		.frames
		.iter()
		.rev()
		.map(|frame| {
			// Stock saves the RESUME IP; select the preceding actual instruction,
			// not ip-1 (encoded instructions have variable lengths).
			let (ip, inst) = at
				.unit
				.iter_instructions()
				.take_while(|(ip, _)| *ip < frame.ip)
				.last()
				.unwrap();
			assert!(matches!(
				inst.kind,
				crate::runtime::inst::Kind::Call { .. }
					| crate::runtime::inst::Kind::CallFn { .. }
					| crate::runtime::inst::Kind::CallOffset { .. }
			));
			info.instruction_at(ip).unwrap().span.range()
		})
		.collect();
	let expected = error.into_kind().to_string();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let failure = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["target"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap_err();
	assert_eq!(failure.error.message(&program).to_string(), expected);
	let at = failure.error.location.unwrap();
	assert_eq!(at.start as usize..at.end as usize, span);
	let chain: std::vec::Vec<_> = failure
		.call_chain
		.iter()
		.map(|at| {
			assert_eq!((at.source, at.revision), (0, 73));
			at.start as usize..at.end as usize
		})
		.collect();
	assert_eq!(chain, legacy_chain);
	assert_eq!(chain.len(), 2);
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	store.close().unwrap();
	assert_eq!(failure.error.message(&program).to_string(), expected);
}

#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
#[test]
#[cfg_attr(miri, ignore = "native generated execution and frontend compilation")]
fn native_replay_preserves_diagnostic_detail() {
	use crate::experimental_core::region::cache;
	struct Reset;
	impl Drop for Reset {
		fn drop(&mut self) {
			cache::enable_for_test(false);
		}
	}
	let _reset = Reset;
	let context = Context::with_default_modules().unwrap();
	let mut retries = 0;
	for body in [
		"let x = 9223372036854775807; x + i",
		"let x = -9223372036854775808; x - i",
		"let z = i - 1; i / z",
		"i << 64",
		"i + true",
	] {
		let source =
			std::format!("pub fn target() {{ let i = 1; while i < 100 {{ {body}; i += 1; }} }}");
		let (_, program) = compile(&context, &source);
		let mut store = Store::new(Limits::default()).unwrap();
		let f = program.function(Hash::type_hash(["target"])).unwrap();
		cache::enable_for_test(false);
		let interpreted = Executor::new()
			.unwrap()
			.invoke(&program, &mut store, f, &[], 100_000)
			.unwrap_err();
		cache::enable_for_test(true);
		let native = Executor::new()
			.unwrap()
			.invoke(&program, &mut store, f, &[], 100_000)
			.unwrap_err();
		if let Some(regions) = cache::regions_for_test(program.code.id()) {
			retries += regions.iter().map(|r| r.exits[2]).sum::<u64>();
		}
		assert_eq!(interpreted.error, native.error, "{body}");
		assert_eq!(interpreted.call_chain, native.call_chain, "{body}");
		assert_eq!(interpreted.cleanup_error, native.cleanup_error, "{body}");
		assert_eq!(
			native.error.message(&program).to_string(),
			interpreted.error.message(&program).to_string()
		);
	}
	assert!(
		retries > 0,
		"at least one real native Retry exit must be exercised"
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn compile_time_refusals_are_cataloged_before_execution() {
	use crate::experimental_core::error::Refusal;
	let context = Context::with_default_modules().unwrap();
	let (source, refusal) = ("pub fn target() { async { 1 } }", Refusal::AsyncBlock);
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	assert!(
		crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.is_ok(),
		"stock compile: {source}"
	);
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let error = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 73)
		.err()
		.expect("execution refusal");
	assert_eq!(error.refusal, Some(refusal), "{source}");
	assert_eq!(error.kind, ErrorKind::Unsupported);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn access_conflict_wording_distinguishes_mutation_and_invalidated_payload() {
	let context = Context::with_default_modules().unwrap();
	let (mut stock, program) = compile(
		&context,
		"pub fn target() { let v = [1]; for x in v { v.push(2); } }",
	);
	let stock = stock.call(["target"], ()).unwrap_err();
	let legacy_span = stock.first_location().unwrap();
	let legacy_span = legacy_span
		.unit
		.debug_info()
		.unwrap()
		.instruction_at(legacy_span.ip)
		.unwrap()
		.span
		.range();
	let mut store = Store::new(Limits::default()).unwrap();
	let failure = Executor::new()
		.unwrap()
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["target"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap_err();
	assert_eq!(
		failure.error.detail,
		crate::experimental_core::error::Detail::SharedArgument { arg: 0, readers: 1 }
	);
	assert_eq!(
		failure.error.message(&program).to_string(),
		stock.to_string()
	);
	let at = failure.error.location.unwrap();
	assert_eq!(at.start as usize..at.end as usize, legacy_span);
	let value = store.string("invalidated").unwrap();
	store.invalidate(value).unwrap();
	let error = store.text(value).unwrap_err();
	assert_eq!(error.kind, ErrorKind::AccessConflict);
	assert_eq!(
		error.detail,
		crate::experimental_core::error::Detail::Moved {
			access: crate::experimental_core::error::MovedAccess::Read
		}
	);
	assert_eq!(
		error.message(&program).to_string(),
		"Cannot read the value: it has been moved"
	);
	let take = crate::experimental_core::error::Error::new(ErrorKind::AccessConflict, "take")
		.with_detail(crate::experimental_core::error::Detail::TakeBorrowed);
	assert_eq!(
		take.message(&program).to_string(),
		"Cannot take the value: it is being read"
	);
	let receiver = crate::experimental_core::error::Error::new(ErrorKind::AccessConflict, "write")
		.with_detail(crate::experimental_core::error::Detail::ExclusiveArgument {
			access: crate::experimental_core::error::MovedAccess::Write,
			arg: 0,
		});
	assert_eq!(
		receiver.message(&program).to_string(),
		"Cannot write the receiver: it is being written"
	);
	store.release(value).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn all_assignment_symbols_are_preserved_in_owned_details() {
	let context = Context::with_default_modules().unwrap();
	for op in ["+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="] {
		let source = std::format!("pub fn target() {{\n\nlet x = 1; x {op} true\n}}");
		let (mut stock, program) = compile(&context, &source);
		let error = stock.call(["target"], ()).unwrap_err();
		let at = error.first_location().unwrap();
		let span = at
			.unit
			.debug_info()
			.unwrap()
			.instruction_at(at.ip)
			.unwrap()
			.span
			.range();
		let expected = std::format!("Cannot apply `{op}` to `i64` and `bool`");
		assert_eq!(error.into_kind().to_string(), expected);
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = Executor::new()
			.unwrap()
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		assert_eq!(failure.error.message(&program).to_string(), expected);
		let at = failure.error.location.unwrap();
		assert_eq!(at.start as usize..at.end as usize, span);
		assert_eq!((at.source, at.revision), (0, 73));
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn a_compile_error_is_not_compared_as_a_runtime_error() {
	use crate::diagnostics::Diagnostic;
	use crate::experimental_core::error::Refusal;
	let context = Context::with_default_modules().unwrap();
	let source = "pub fn target() { let x = missing; x }";
	let start = source.find("missing").unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	assert!(crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.is_err());
	let Diagnostic::Fatal(fatal) = &diagnostics.diagnostics()[0] else {
		panic!("fatal compile diagnostic");
	};
	assert_eq!(fatal.to_string(), "No local variable `missing`");
	assert_eq!(fatal.span().unwrap().range(), start..start + 7);
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let error = frontend::compile(&mut sources, &context, &mut diagnostics, 73)
		.err()
		.expect("frontend refusal");
	assert_eq!(error.refusal, Some(Refusal::FrontendDiagnostic));
	let Diagnostic::Fatal(execution) = &diagnostics.diagnostics()[0] else {
		panic!("execution frontend fatal");
	};
	assert_eq!(execution.to_string(), fatal.to_string());
	assert_eq!(execution.span().unwrap(), fatal.span().unwrap());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn nested_comparison_leaf_messages_keep_the_whole_expression_span() {
	let context = Context::with_default_modules().unwrap();
	for (setup, expression) in [
		("let a=[\"a\"]; let b=[1];", "a == b"),
		("let a=[1.0]; let b=[1];", "a != b"),
		("let a=Some([1]); let b=Some([1.0]);", "a <= b"),
		("let a=Ok((\"a\",)); let b=Ok((1,));", "a > b"),
		("let a=#{x:[\"a\"]}; let b=#{x:[1]};", "a == b"),
		("let a=P{x:[1]}; let b=P{x:[1.0]};", "a >= b"),
	] {
		let source = std::format!("struct P {{ x }} pub fn target() {{ {setup} {expression} }}");
		let (mut vm, program) = compile(&context, &source);
		let stock = vm.call(["target"], ()).unwrap_err();
		let at = stock.first_location().unwrap();
		let span = at
			.unit
			.debug_info()
			.unwrap()
			.instruction_at(at.ip)
			.unwrap()
			.span;
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		assert_eq!(
			failure.error.message(&program).to_string(),
			stock.into_kind().to_string(),
			"{expression}"
		);
		let location = failure.error.location.unwrap();
		assert_eq!(location.start as usize..location.end as usize, span.range());
		let start = source.rfind(expression).unwrap();
		assert_eq!(
			location.start as usize..location.end as usize,
			start..start + expression.len()
		);
		assert!(failure.cleanup_error.is_none());
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn comparison_type_values_use_stock_hash_order_including_nominals() {
	let context = Context::with_default_modules().unwrap();
	for (a, b) in [
		("P", "P"),
		("P", "Q"),
		("Q", "P"),
		("P", "i64"),
		("i64", "u64"),
		("i64", "i64"),
	] {
		for op in ["==", "!=", "<", "<=", ">", ">="] {
			let text = std::format!(
				"struct P {{ x }} struct Q {{ x }} pub fn target() {{ {a} {op} {b} }}"
			);
			let (mut stock, program) = compile(&context, &text);
			let expected = stock.call(["target"], ()).unwrap().as_bool().unwrap();
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = Executor::new().unwrap();
			let root = executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["target"])).unwrap(),
					&[],
					100_000,
				)
				.unwrap();
			assert_eq!(
				store.root_scalar(&root).unwrap(),
				Some(crate::experimental_core::value::Scalar::Bool(expected)),
				"{text}"
			);
			store.root_release(root).unwrap();
			assert!(executor.is_idle());
			assert_eq!(store.live_entries(), 0);
		}
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn dynamic_missing_keys_are_owned_and_copy_failure_preserves_the_original_failure() {
	use crate::experimental_core::error::Detail;
	let context = Context::with_default_modules().unwrap();
	for key in [
		std::string::String::from("missing"),
		std::format!("{}終", "key".repeat(400)),
	] {
		let source = std::format!("fn inner(k) {{ let x=#{{present:1}}; x[k] }} fn outer(k) {{ inner(k) }} pub fn main() {{ outer(\"{key}\") }}");
		let (mut stock, program) = compile(&context, &source);
		let expected = stock
			.call(["main"], ())
			.unwrap_err()
			.into_kind()
			.to_string();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let entry = program.function(Hash::type_hash(["main"])).unwrap();
		let full = executor
			.invoke(&program, &mut store, entry, &[], 10_000)
			.unwrap_err();
		assert_eq!(full.missing_key.as_deref(), Some(key.as_str()));
		assert_eq!(full.message(&program).to_string(), expected);
		assert_eq!(full.call_chain.len(), 2);
		assert_eq!(store.live_entries(), 0);
		executor.test_fail_key_copy = true;
		let fallback = executor
			.invoke(&program, &mut store, entry, &[], 10_000)
			.unwrap_err();
		assert!(fallback.missing_key.is_none());
		assert_eq!(fallback.error, full.error);
		assert_eq!(fallback.call_chain, full.call_chain);
		assert_eq!(fallback.cleanup_error, full.cleanup_error);
		assert!(matches!(fallback.error.detail, Detail::MissingKey { .. }));
		assert_eq!(
			fallback.message(&program).to_string(),
			"Missing field on `std::object::Object`"
		);
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn compound_tuple_string_append_preserves_aliases_and_container_borrow_order() {
	let context = Context::with_default_modules().unwrap();
	let (mut stock, program) = compile(
		&context,
		"fn update(x) { x.0 += \"b\"; } pub fn main() { let s=\"a\"; let x=(s,); update(x); s }",
	);
	let expected = stock.call(["main"], ()).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let result = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			10_000,
		)
		.unwrap();
	assert_eq!(store.root_text(&result).unwrap(), "ab");
	assert_eq!(
		expected
			.borrow_ref::<crate::alloc::String>()
			.unwrap()
			.as_str(),
		"ab"
	);
	store.root_release(result).unwrap();
	assert_eq!(store.live_entries(), 0);
	let (mut stock, program) = compile(
		&context,
		"pub fn main() { let x=[1]; for v in x { x.0 += \"wrong\"; } }",
	);
	let expected = stock
		.call(["main"], ())
		.unwrap_err()
		.into_kind()
		.to_string();
	let failure = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			10_000,
		)
		.unwrap_err();
	assert_eq!(failure.message(&program).to_string(), expected);
	assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn vector_dynamic_set_borrows_before_converting_the_index() {
	let context = Context::with_default_modules().unwrap();
	for index in ["-1", "0.0"] {
		let source =
			std::format!("pub fn main() {{ let x=[1]; for v in x {{ x[{index}] = 2; }} }}");
		let (mut stock, program) = compile(&context, &source);
		let expected = stock.call(["main"], ()).unwrap_err().to_string();
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = Executor::new()
			.unwrap()
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				10_000,
			)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
		assert_eq!(failure.message(&program).to_string(), expected);
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "stock/execution frontend witness; pure ownership checks cover Miri"
)]
fn field_compound_leased_nonrecords_and_string_alias_match_stock() {
	let context = Context::with_default_modules().unwrap();
	for body in [
		"let x=(1,2); for v in x { x.f += 1; }",
		"let x=[1,2]; for v in x { x.f += 1; }",
	] {
		let source = std::format!("pub fn target() {{ {body} }}");
		let (mut stock, program) = compile(&context, &source);
		let runtime_error = stock.call(["target"], ()).unwrap_err();
		let expected = runtime_error.into_kind().to_string();
		println!("stock {body}: {expected}");
		assert!(expected.starts_with("Field `f` not available on `std::"));
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let execution = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				1000,
			)
			.unwrap_err();
		assert_eq!(execution.error.kind, ErrorKind::Type);
		assert_eq!(execution.message(&program).to_string(), expected);
		assert_eq!(store.live_entries(), 0);
	}
	let source = "pub fn target() { let x=#{f:\"a\"}; let alias=x.f; x.f += \"b\"; alias }";
	let (mut stock, program) = compile(&context, source);
	let expected =
		crate::from_value::<crate::alloc::String>(stock.call(["target"], ()).unwrap()).unwrap();
	assert_eq!(expected.as_str(), "ab");
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["target"])).unwrap(),
			&[],
			1000,
		)
		.unwrap();
	assert_eq!(store.text(root.value).unwrap(), expected.as_str());
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(miri, ignore = "stock/execution frontend identity witness")]
fn iteration_nominal_and_function_hashes_match_lowered_stock_values() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"struct P { x } pub fn target() { P {x:1} }",
		"pub fn target() { target }",
	] {
		let (mut stock, program) = compile(&context, source);
		let legacy_value = stock.call(["target"], ()).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		let name = store.diagnostic_type(root.value).unwrap();
		let hash = match name {
			crate::experimental_core::error::TypeName::Struct(n) => {
				Hash::new(crate::experimental_core::nominal::hash_copy(n).unwrap())
			}
			_ => name.builtin_hash().unwrap(),
		};
		assert_eq!(hash, legacy_value.type_hash());
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
#[cfg(not(miri))]
fn leased_indexed_write_keeps_the_proven_receiver_position() {
	let context = Context::with_default_modules().unwrap();
	let (mut stock, program) = compile(
		&context,
		"pub fn target() { let v=[1]; for x in v { v[0]=2; } }",
	);
	let stock = stock.call(["target"], ()).unwrap_err();
	assert!(stock.chain().iter().any(|entry| matches!(
		entry.kind(),
		crate::runtime::VmErrorKind::BadArgument { arg: 0 }
	)));
	let mut store = Store::new(Limits::default()).unwrap();
	let failure = Executor::new()
		.unwrap()
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["target"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap_err();
	assert_eq!(
		failure.error.detail,
		crate::experimental_core::error::Detail::SharedArgument { arg: 0, readers: 1 }
	);
	assert_eq!(
		failure.message(&program).to_string(),
		"Cannot write the receiver: it is being read"
	);
	assert_eq!(stock.to_string(), failure.message(&program).to_string());
}

#[test]
#[cfg(not(miri))]
fn d1d_numeric_context_slice_both_engines() {
	use crate::runtime::{VmError, VmErrorKind};
	let context = Context::with_default_modules().unwrap();
	let rows = [
		("!a", "\"x\"", "0", "Cannot apply `!` to `String`"),
		("a & b", "7", "true", "Cannot apply `&` to `i64` and `bool`"),
		("a | b", "7", "true", "Cannot apply `|` to `i64` and `bool`"),
		("a ^ b", "7", "true", "Cannot apply `^` to `i64` and `bool`"),
		(
			"a << b",
			"7",
			"true",
			"Cannot apply `<<` to `i64` and `bool`",
		),
		(
			"a >> b",
			"7",
			"true",
			"Cannot apply `>>` to `i64` and `bool`",
		),
		(
			"a << b",
			"1",
			"-1",
			"Cannot convert `i64` value `-1` to `u32` for `<<`",
		),
		(
			"a >> b",
			"1",
			"-1",
			"Cannot convert `i64` value `-1` to `u32` for `>>`",
		),
		(
			"a << b",
			"1",
			"64",
			"Numerical overflow in `i64` left shift (`<<`)",
		),
		(
			"a << b",
			"1",
			"65",
			"Numerical overflow in `i64` left shift (`<<`)",
		),
		(
			"a >> b",
			"1",
			"64",
			"Numerical underflow in `i64` right shift (`>>`)",
		),
		(
			"a >> b",
			"1",
			"65",
			"Numerical underflow in `i64` right shift (`>>`)",
		),
		(
			"a / b",
			"1",
			"0",
			"Division by zero in `i64` division (`/`)",
		),
		(
			"a % b",
			"1",
			"0",
			"Division by zero in `i64` remainder (`%`)",
		),
		(
			"a + b",
			"9223372036854775807",
			"1",
			"Numerical overflow in `i64` addition (`+`)",
		),
		(
			"a - b",
			"-9223372036854775808",
			"1",
			"Numerical underflow in `i64` subtraction (`-`)",
		),
		(
			"a + b",
			"18446744073709551615u64",
			"1u64",
			"Numerical overflow in `u64` addition (`+`)",
		),
		(
			"a - b",
			"0u64",
			"1u64",
			"Numerical underflow in `u64` subtraction (`-`)",
		),
	];
	for (body, a, b, expected) in rows {
		let source = std::format!("fn f(a,b){{{body}}}pub fn main(){{f({a},{b})}}");
		let (mut stock, program) = compile(&context, &source);
		let error = stock.call(["main"], ()).unwrap_err();
		assert_eq!(error.to_string(), expected, "stock {source}");
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		assert_eq!(
			failure.message(&program).to_string(),
			expected,
			"execution {source}"
		);
		assert!(matches!(
			failure.error.kind,
			ErrorKind::Type
				| ErrorKind::Overflow
				| ErrorKind::DivisionByZero
				| ErrorKind::IntegerCoercion
		));
		assert_eq!(store.live_entries(), 0);
	}
	for (kind, expected) in [
		(VmErrorKind::Overflow, "Numerical overflow"),
		(VmErrorKind::Underflow, "Numerical underflow"),
		(VmErrorKind::DivideByZero, "Division by zero"),
	] {
		assert_eq!(VmError::from(kind).to_string(), expected);
	}
	let (_, program) = compile(&context, "pub fn main(){0}");
	for (kind, expected) in [
		(ErrorKind::Overflow, "Numerical overflow"),
		(ErrorKind::DivisionByZero, "Division by zero"),
	] {
		let error = crate::experimental_core::error::Error::new(kind, "native helper");
		assert_eq!(error.message(&program).to_string(), expected);
	}
}

#[test]
#[cfg_attr(miri, ignore = "frontend compilation; bounded D1e slice")]
fn d1e_logical_frames_and_rendering_slice() {
	use crate::runtime::{InvocationContext, LogicalFrameRole, VmError};
	let context = Context::with_default_modules().unwrap();
	let examples = [
		"pub fn target(){outer()} fn outer(){inner()} fn inner(){panic!(\"nested\")}",
		"pub fn target(){recur(3)} fn recur(n){if n == 0 {panic!(\"repeat\")} else {recur(n-1)}}",
		"pub fn target(){let a=Some([1]);let b=a;drop(a);format!(\"{:>7?}\",b)}",
		"pub fn target(){let a=[1];let b=a;drop(a);b.len()}",
		"pub fn target(){let é=1; panic!(\"unicode\")}",
	];
	for source in examples {
		let (mut stock, program) = compile(&context, source);
		let stock = stock.call(["target"], ()).unwrap_err();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		let project = |frame: crate::runtime::LogicalFrame<'_>| {
			(
				frame.role,
				frame
					.location
					.map(|at| (at.source_id.into_index(), at.span.range())),
			)
		};
		let legacy_frames = stock
			.logical_frames()
			.map(project)
			.collect::<std::vec::Vec<_>>();
		let frames = failure
			.logical_frames()
			.map(project)
			.collect::<std::vec::Vec<_>>();
		assert_eq!(legacy_frames, frames, "{source}");
		assert_eq!(legacy_frames[0].0, LogicalFrameRole::Origin);
		if source.contains("b.len()") {
			// Existing unrelated method provenance is not manufactured by D1e.
			assert!(failure.invocation_context().next().is_none());
			assert_eq!(
				stock.invocation_context().collect::<std::vec::Vec<_>>(),
				[InvocationContext::BadArgument { arg: 0 }]
			);
		} else {
			assert_eq!(
				stock.invocation_context().collect::<std::vec::Vec<_>>(),
				failure.invocation_context().collect::<std::vec::Vec<_>>(),
				"{source}"
			);
		}
		if source.contains(":>7?") {
			assert_eq!(
				failure.invocation_context().collect::<std::vec::Vec<_>>(),
				[InvocationContext::BadArgument { arg: 0 }]
			);
		}
		let mut sources = Sources::new();
		sources.insert(Source::memory(source).unwrap()).unwrap();
		let legacy_text = stock.diagnostic(&sources).to_string();
		let text = failure.diagnostic(&program, &sources).to_string();
		assert_eq!(legacy_text, text, "{source}");
		assert!(legacy_text.contains("\n  at "));
		std::println!("D1E source={source}\n{legacy_text}");
		assert!(failure.cleanup_error.is_none());
		assert_eq!(store.live_entries(), 0);
		let again = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["target"])).unwrap(),
				&[],
				100_000,
			)
			.unwrap_err();
		assert_eq!(again.diagnostic(&program, &sources).to_string(), text);
		assert_eq!(store.live_entries(), 0);
	}
	assert_eq!(
		VmError::panic("contextless")
			.diagnostic(&Sources::new())
			.to_string(),
		"Panicked: contextless\n  at <unknown>"
	);
	assert_eq!(
		core::mem::size_of::<crate::experimental_core::error::Detail>(),
		32
	);
	assert_eq!(
		core::mem::size_of::<crate::experimental_core::error::Error>(),
		88
	);
	assert_eq!(
		core::mem::size_of::<crate::experimental_core::execute::Failure>(),
		272
	);
}
