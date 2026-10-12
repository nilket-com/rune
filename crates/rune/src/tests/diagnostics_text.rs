//! Stock diagnostics: source operators, named fields and lossless bounds.
prelude!();
use crate::runtime::VmError;

fn fail(body: &str) -> VmError {
	let source = format!("pub fn main() {{ {body} }}");
	let context = Context::with_default_modules().unwrap();
	let mut sources = crate::tests::sources(&source);
	let mut vm = crate::tests::vm(&context, &mut sources, &mut Diagnostics::new(), false).unwrap();
	vm.call(["main"], ()).unwrap_err()
}

#[test]
fn missing_instance_identity_survives_real_source_execution() {
	let error = fail("[1].frobnicate()");
	let info = error.missing_instance_function().unwrap();
	let instance = <crate::runtime::Vec as crate::TypeHash>::HASH;
	assert_eq!(info.instance, instance);
	assert_eq!(
		info.hash,
		crate::Hash::associated_function(instance, "frobnicate")
	);
	assert_eq!(info.instance_name().to_string(), "std::vec::Vec");
}

#[test]
fn source_operator_symbols_and_inline_mismatches() {
	for symbol in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
		for (lhs, expected_lhs) in [
			("1", TypeInfo::named::<i64>()),
			("1u64", TypeInfo::named::<u64>()),
		] {
			for (rhs, expected_rhs) in [
				("true", TypeInfo::named::<bool>()),
				("1.0", TypeInfo::named::<f64>()),
				("'c'", TypeInfo::named::<char>()),
				("()", Value::unit().type_info()),
			] {
				for assign in [false, true] {
					let op = if assign {
						format!("{symbol}=")
					} else {
						symbol.into()
					};
					let body = if assign {
						format!("let x = {lhs}; x {op} {rhs}")
					} else {
						format!("{lhs} {op} {rhs}")
					};
					let error = fail(&body);
					let VmErrorKind::UnsupportedBinaryOperation {
						op: actual,
						lhs,
						rhs,
					} = error.error().kind()
					else {
						panic!("{body}: {error}")
					};
					assert_eq!(*actual, op, "{body}");
					assert_eq!(*lhs, expected_lhs, "{body}");
					assert_eq!(*rhs, expected_rhs, "{body}");
					assert_eq!(
						error.to_string(),
						format!(
							"Unsupported binary operation `{op}` on `{}` and `{}`",
							crate::runtime::messages::DiagnosticType(lhs),
							crate::runtime::messages::DiagnosticType(rhs)
						),
						"{body}"
					);
				}
			}
		}
	}
	for op in ["+", "<", "<=", ">", ">=", "==", "!="] {
		let error = fail(&format!("1 {op} \"a\""));
		assert!(
			matches!(error.error().kind(), VmErrorKind::UnsupportedBinaryOperation { op: actual, .. } if *actual == op)
		);
		assert_eq!(
			error.to_string(),
			format!("Unsupported binary operation `{op}` on `std::i64` and `std::string::String`")
		);
	}
	for (body, op, ty) in [
		("-(true)", "-", "std::bool"),
		("!\"a\"", "!", "std::string::String"),
	] {
		let error = fail(body);
		assert!(
			matches!(error.error().kind(), VmErrorKind::UnsupportedUnaryOperation { op: actual, .. } if *actual == op)
		);
		assert_eq!(
			error.to_string(),
			format!("Unsupported unary operation `{op}` on `{ty}`")
		);
	}
}

#[test]
fn fields_report_names_and_sequence_bounds_report_lengths() {
	for body in ["#{a:1}.b", "#{a:1}[\"b\"]"] {
		let error = fail(body);
		assert!(
			matches!(error.error().kind(), VmErrorKind::MissingField { field, .. } if field.as_str() == "b")
		);
		assert_eq!(
			error.to_string(),
			"Missing field `b` on `std::object::Object`"
		);
	}
	for (body, length, index) in [
		("[1,2][5]", 2, "5"),
		("(1,2).5", 2, "5"),
		("let v = [1]; v[3] += 1", 1, "3"),
		("let t = (1,2); t.3 += 1", 2, "3"),
		("let v = [1]; v[3] = 2", 1, "3"),
		("[1][u64::MAX]", 1, "18446744073709551615"),
	] {
		let error = fail(body);
		assert!(
			matches!(error.error().kind(), VmErrorKind::OutOfRange { .. }),
			"{body}: {error}"
		);
		assert_eq!(
			error.to_string(),
			format!("Index out of bounds, the length is `{length}` but the index is `{index}`"),
			"{body}"
		);
	}
}

#[test]
fn coercion_and_library_errors_keep_their_contract() {
	for (body, text) in [
		("1u64 + -1", "Failed to convert value `-1` to integer `u64`"),
		(
			"1 + u64::MAX",
			"Failed to convert value `18446744073709551615` to integer `i64`",
		),
		("1 << -1", "Failed to convert value `-1` to integer `u32`"),
		("[1][-1]", "Failed to convert value `-1` to integer `usize`"),
		("[1,2][1.5]", "Expected number type, but found `std::f64`"),
		(
			"Some(1).1",
			"Type `std::option::Option` missing integer index `1`",
		),
		(
			"\"a\" + 1",
			"Expected type `std::string::String` but found `std::i64`",
		),
	] {
		assert_eq!(fail(body).to_string(), text, "{body}");
	}
	let error = fail("let v = [1, \"a\"]; v.sort()");
	assert!(
		matches!(
			error.error().kind(),
			VmErrorKind::UnsupportedBinaryOperation { op: "CMP", .. }
		),
		"{error}"
	);
}

#[test]
fn operator_mapping_preserves_nested_locations_and_auxiliary_errors() {
	let error = fail("1 < \"a\"");
	let location = error.first_location().unwrap().ip;
	let error = error.with_operator("<", "OUTER");
	assert!(matches!(
		error.error().kind(),
		VmErrorKind::UnsupportedBinaryOperation { op: "<", .. }
	));
	assert_eq!(error.first_location().unwrap().ip, location);
	let error = VmError::new(VmErrorKind::UnsupportedBinaryOperation {
		op: "PARTIAL_CMP",
		lhs: TypeInfo::named::<i64>(),
		rhs: TypeInfo::named::<bool>(),
	})
	.with_error(VmErrorKind::Overflow)
	.with_operator("PARTIAL_CMP", "<=");
	assert!(matches!(
		error.error().kind(),
		VmErrorKind::UnsupportedBinaryOperation { op: "<=", .. }
	));
	assert!(matches!(error.chain()[0].kind(), VmErrorKind::Overflow));
	let error = VmError::new(VmErrorKind::UnsupportedBinaryOperation {
		op: "+",
		lhs: TypeInfo::named::<i64>(),
		rhs: TypeInfo::named::<bool>(),
	})
	.with_operator("PARTIAL_CMP", "<=");
	assert!(matches!(
		error.error().kind(),
		VmErrorKind::UnsupportedBinaryOperation { op: "+", .. }
	));
}

#[test]
fn compound_targets_and_heap_rhs_keep_source_symbols() {
	for symbol in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
		for body in [
			format!("let x = 1; x {symbol}= \"x\""),
			format!("let o = #{{a: 1}}; o.a {symbol}= \"x\""),
			format!("let v = [1]; v[0] {symbol}= \"x\""),
		] {
			let error = fail(&body);
			let op = format!("{symbol}=");
			assert!(
				matches!(error.error().kind(), VmErrorKind::UnsupportedBinaryOperation { op: actual, .. } if *actual == op),
				"{body}: {error}"
			);
			assert_eq!(
				error.to_string(),
				format!(
					"Unsupported binary operation `{op}` on `std::i64` and `std::string::String`"
				),
				"{body}"
			);
		}
	}
}
