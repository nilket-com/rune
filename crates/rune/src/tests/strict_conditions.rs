//! F1: evaluated conditions are bool, with exact operand locations and effects.
prelude!();
use crate::runtime::VmError;

fn execute(body: &str) -> (core::result::Result<Value, VmError>, Value, String) {
	let source = format!("pub fn main(w) {{ {body} }} fn mark(w, x) {{ w.push(0); x }}");
	let context = Context::with_default_modules().unwrap();
	let mut sources = crate::tests::sources(&source);
	let mut vm = crate::tests::vm(&context, &mut sources, &mut Diagnostics::new(), false).unwrap();
	let witness = crate::to_value(crate::runtime::Vec::new()).unwrap();
	let result = vm.call(["main"], (witness.clone(),));
	(result, witness, source)
}

fn type_error(body: &str, operand: &str, marks: usize, actual: TypeInfo) {
	let (result, witness, source) = execute(body);
	let error = result.unwrap_err();
	let location = error.first_location().unwrap();
	let span = location
		.unit
		.debug_info()
		.unwrap()
		.instruction_at(location.ip)
		.unwrap()
		.span;
	let start = source.find(operand).unwrap();
	assert_eq!(
		span.range(),
		start..start + operand.len(),
		"{body}: {error}"
	);
	assert_eq!(
		error.into_kind(),
		VmErrorKind::ExpectedType {
			expected: TypeInfo::named::<bool>(),
			actual,
		},
		"{body}"
	);
	assert_eq!(
		witness.borrow_ref::<crate::runtime::Vec>().unwrap().len(),
		marks,
		"{body}"
	);
}

#[test]
fn non_bool_conditions_report_the_operand_and_preserve_effects() {
	for (body, operand, marks) in [
		("1 && mark(w, true)", "1", 0),
		("1 || mark(w, true)", "1", 0),
		("true && 5", "5", 0),
		("false || 7", "7", 0),
		("true && mark(w, 1)", "mark(w, 1)", 1),
		("false || mark(w, 1)", "mark(w, 1)", 1),
		("true && mark(w, 1); ()", "mark(w, 1)", 1),
		("false || mark(w, 1); ()", "mark(w, 1)", 1),
		("(true && 1) || true", "1", 0),
		("true && (false || 2)", "2", 0),
		("!(1 && true)", "1", 0),
		("if 1 {}", "1", 0),
		("if false {} else if 1 {}", "1", 0),
		("while 0 {}", "0", 0),
		("match 3 { _ if 1 => 0, _ => 2 }", "1", 0),
	] {
		type_error(body, operand, marks, TypeInfo::named::<i64>());
	}
	type_error(
		"\"x\" || false",
		"\"x\"",
		0,
		TypeInfo::named::<crate::alloc::String>(),
	);
}

#[test]
fn short_circuit_and_diverging_rhs_never_execute_unreachable_work() {
	for (body, expected, marks) in [
		("false && mark(w, 1)", false, 0),
		("true || mark(w, 1)", true, 0),
		("false && panic(\"x\")", false, 0),
		("true || panic(\"x\")", true, 0),
		("true && mark(w, true)", true, 1),
		("false || mark(w, false)", false, 1),
		("true && { return false; }", false, 0),
		("false || { return true; }", true, 0),
	] {
		let (result, witness, _) = execute(body);
		assert_eq!(
			from_value::<bool>(result.unwrap()).unwrap(),
			expected,
			"{body}"
		);
		assert_eq!(
			witness.borrow_ref::<crate::runtime::Vec>().unwrap().len(),
			marks,
			"{body}"
		);
	}
}

#[test]
fn assertions_patterns_and_bitwise_not_keep_their_intended_meaning() {
	for body in ["assert!(1)", "assert!(0)"] {
		let (result, _, source) = execute(body);
		let error = result.unwrap_err();
		let at = error.first_location().unwrap();
		let span = at
			.unit
			.debug_info()
			.unwrap()
			.instruction_at(at.ip)
			.unwrap()
			.span;
		// Macro expansion currently attributes the generated condition to the invocation.
		let start = source.find(body).unwrap();
		assert_eq!(span.range(), start..start + body.len(), "{body}");
		assert_eq!(
			error.into_kind(),
			VmErrorKind::ExpectedType {
				expected: TypeInfo::named::<bool>(),
				actual: TypeInfo::named::<i64>(),
			}
		);
	}
	assert!(execute("assert!(true)").0.is_ok());
	assert!(execute("assert!(false)")
		.0
		.unwrap_err()
		.to_string()
		.contains("assertion failed: false"));
	assert_eq!(from_value::<i64>(execute("!1").0.unwrap()).unwrap(), -2);
	assert_eq!(
		from_value::<bool>(execute("!true").0.unwrap()).unwrap(),
		false
	);
	assert_eq!(
		from_value::<i64>(
			execute("if let Some(x) = Some(1) { x } else { 0 }")
				.0
				.unwrap()
		)
		.unwrap(),
		1
	);
	assert_eq!(
		from_value::<i64>(
			execute("let x = Some(1); let n = 0; while let Some(v) = x { n += v; x = None; } n")
				.0
				.unwrap()
		)
		.unwrap(),
		1
	);
}

#[test]
fn discarded_lazy_results_still_short_circuit_and_propagate_returns() {
	for (body, expected, marks) in [
		("false && mark(w, 1); true", true, 0),
		("true || mark(w, 1); false", false, 0),
		("true && mark(w, true); true", true, 1),
		("false || mark(w, false); true", true, 1),
		("true && { return false; }; true", false, 0),
		("(true && mark(w, true)) || mark(w, true); true", true, 1),
		("true && (false || mark(w, true)); true", true, 1),
		("if true { true && mark(w, true); } true", true, 1),
		("for _ in 0..3 { false || mark(w, false); } true", true, 3),
		("(return true) && mark(w, 1); false", true, 0),
		("false || { return true; }; false", true, 0),
	] {
		let (result, witness, _) = execute(body);
		assert_eq!(
			from_value::<bool>(result.unwrap()).unwrap(),
			expected,
			"{body}"
		);
		assert_eq!(
			witness.borrow_ref::<crate::runtime::Vec>().unwrap().len(),
			marks,
			"{body}"
		);
	}
}
