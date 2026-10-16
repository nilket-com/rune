//! Frozen pre-optimization primitive binary oracle.
use super::*;

fn binary_reference(op: Binary, a: Value, b: Value) -> Result<Value> {
	binary_inner_reference(op, a, b).map_err(|error| {
		if error.kind == ErrorKind::Type {
			if let (Some(lhs), Some(rhs)) = (TypeName::primitive(a), TypeName::primitive(b)) {
				return error.with_detail(Detail::Binary {
					op: BinarySymbol::from_op(op, false),
					lhs,
					rhs,
				});
			}
		}
		error
	})
}

#[allow(clippy::bool_comparison)]
fn binary_inner_reference(op: Binary, a: Value, b: Value) -> Result<Value> {
	use Binary::*;
	use Value::*;

	// Shifts convert the count to u32 independently of the left integer's tag.
	// Other mixed integer operations convert the right operand to the left tag.
	let b = match (op, a, b) {
		(Shl | Shr, I64(_), U64(_)) => I64(i64::from(shift(b, op)?)),
		(Shl | Shr, U64(_), I64(_)) => U64(u64::from(shift(b, op)?)),
		(_, I64(_), U64(n)) => I64(i64::try_from(n).map_err(|_| {
			error(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce {
				value: IntValue {
					negative: false,
					magnitude: n,
				},
				to: IntTarget::I64,
			})
		})?),

		(_, U64(_), I64(n)) => U64(u64::try_from(n).map_err(|_| {
			error(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce {
				value: IntValue {
					negative: n < 0,
					magnitude: n.unsigned_abs(),
				},
				to: IntTarget::U64,
			})
		})?),
		_ => b,
	};

	match (a, b) {
		(I64(a), I64(b)) => Ok(match op {
			Add => I64(checked(a.checked_add(b))?),
			Sub => I64(a
				.checked_sub(b)
				.ok_or_else(|| error(ErrorKind::Overflow).with_detail(Detail::Underflow))?),
			Mul => I64(checked(a.checked_mul(b))?),
			Div | Rem if b == 0 => return Err(error(ErrorKind::DivisionByZero)),
			Div => I64(checked(a.checked_div(b))?),
			Rem => I64(checked(a.checked_rem(b))?),
			BitAnd => I64(a & b),
			BitOr => I64(a | b),
			BitXor => I64(a ^ b),
			Shl | Shr => {
				let n = shift(I64(b), op)?;
				if op == Shl {
					I64(a << n)
				} else {
					I64(a >> n)
				}
			}

			_ => return compare(op, a == b, a < b, a > b),
		}),
		(U64(a), U64(b)) => Ok(match op {
			Add => U64(checked(a.checked_add(b))?),
			Sub => U64(a
				.checked_sub(b)
				.ok_or_else(|| error(ErrorKind::Overflow).with_detail(Detail::Underflow))?),
			Mul => U64(checked(a.checked_mul(b))?),
			Div | Rem if b == 0 => return Err(error(ErrorKind::DivisionByZero)),
			Div => U64(a / b),
			Rem => U64(a % b),
			BitAnd => U64(a & b),
			BitOr => U64(a | b),
			BitXor => U64(a ^ b),
			Shl | Shr => {
				let n = shift(U64(b), op)?;
				if op == Shl {
					U64(a << n)
				} else {
					U64(a >> n)
				}
			}

			_ => return compare(op, a == b, a < b, a > b),
		}),
		(F64(a), F64(b)) => Ok(match op {
			Add => F64(a + b),
			Sub => F64(a - b),
			Mul => F64(a * b),
			Div => F64(a / b),
			Rem => F64(a % b),
			_ => return compare(op, a == b, a < b, a > b),
		}),
		(Bool(a), Bool(b)) => match op {
			BitAnd => Ok(Bool(a & b)),
			BitOr => Ok(Bool(a | b)),
			BitXor => Ok(Bool(a ^ b)),
			Eq | Ne | Lt | Le | Gt | Ge => compare(op, a == b, a < b, a > b),
			_ => Err(error(ErrorKind::Type)),
		},

		(Char(a), Char(b)) => compare(op, a == b, a < b, a > b),
		(Unit, Unit) => compare(op, true, false, false),
		_ => Err(error(ErrorKind::Type)),
	}
}

fn same_result(actual: Result<Value>, expected: Result<Value>, op: Binary, a: Value, b: Value) {
	match (actual, expected) {
		(Ok(Value::F64(actual)), Ok(Value::F64(expected))) => {
			if expected.is_nan() {
				assert!(actual.is_nan(), "{op:?} {a:?} {b:?}");
			} else {
				assert_eq!(actual.to_bits(), expected.to_bits(), "{op:?} {a:?} {b:?}");
			}
		}
		(actual, expected) => assert_eq!(actual, expected, "{op:?} {a:?} {b:?}"),
	}
}

#[test]
fn all_binary_operations_match_frozen_results_and_error_details() {
	let operands = [
		Value::Unit,
		Value::Bool(false),
		Value::Bool(true),
		Value::I64(i64::MIN),
		Value::I64(-1),
		Value::I64(0),
		Value::I64(1),
		Value::I64(63),
		Value::I64(64),
		Value::I64(i64::MAX),
		Value::U64(0),
		Value::U64(1),
		Value::U64(63),
		Value::U64(64),
		Value::U64(u32::MAX as u64 + 1),
		Value::U64(i64::MAX as u64 + 1),
		Value::U64(u64::MAX),
		Value::F64(f64::NAN),
		Value::F64(f64::from_bits(0xfff8_0000_0000_0001)),
		Value::F64(-0.0),
		Value::F64(0.0),
		Value::F64(-1.0),
		Value::F64(1.0),
		Value::F64(f64::INFINITY),
		Value::F64(f64::NEG_INFINITY),
		Value::Char('\0'),
		Value::Char('a'),
		Value::Char('\u{10ffff}'),
		Value::Type(ValueType::I64),
		Value::Type(ValueType::Vec),
		Value::Function { code: 7, id: 3 },
		Value::Heap(crate::experimental_core::value::Handle {
			slot: 17,
			generation: 42,
		}),
	];
	let mut rows = 0;
	for op in [
		Binary::Add,
		Binary::Sub,
		Binary::Mul,
		Binary::Div,
		Binary::Rem,
		Binary::Eq,
		Binary::Ne,
		Binary::Lt,
		Binary::Le,
		Binary::Gt,
		Binary::Ge,
		Binary::BitAnd,
		Binary::BitOr,
		Binary::BitXor,
		Binary::Shl,
		Binary::Shr,
	] {
		for a in operands {
			for b in operands {
				let actual = binary(
					std::hint::black_box(op),
					std::hint::black_box(a),
					std::hint::black_box(b),
				);
				let expected = binary_reference(op, a, b);
				same_result(actual, expected, op, a, b);
				rows += 1;
			}
		}
	}
	assert_eq!(rows, 16 * operands.len() * operands.len());
	std::println!(
		"BINARY_ORACLE rows={rows} operand_values={}",
		operands.len()
	);
}

#[test]
fn type_errors_from_comparison_and_shift_inputs_keep_original_operands() {
	for (op, a, b) in [
		(Binary::Add, Value::Char('a'), Value::Char('b')),
		(Binary::Mul, Value::Unit, Value::Unit),
		(Binary::BitAnd, Value::F64(1.0), Value::F64(2.0)),
		(Binary::Shl, Value::I64(1), Value::Bool(false)),
		(Binary::Shr, Value::Bool(true), Value::U64(1)),
	] {
		let expected =
			Error::new(ErrorKind::Type, "primitive operation").with_detail(Detail::Binary {
				op: BinarySymbol::from_op(op, false),
				lhs: TypeName::primitive(a).unwrap(),
				rhs: TypeName::primitive(b).unwrap(),
			});
		assert_eq!(binary(op, a, b), Err(expected));
	}
	let heap = Value::Heap(crate::experimental_core::value::Handle {
		slot: 1,
		generation: 1,
	});
	assert_eq!(
		binary(Binary::Add, heap, Value::I64(1)),
		Err(Error::new(ErrorKind::Type, "primitive operation"))
	);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; native tests cover constants"
)]
fn constant_folding_type_and_overflow_diagnostics_remain_exact() {
	use crate::experimental_core::{
		error::{Location, Refusal},
		frontend,
	};
	use crate::{Context, Diagnostics, Source, Sources};
	let context = Context::with_default_modules().unwrap();
	for text in [
		"const A = 1 + true; pub fn main() { A }",
		"const A = 9223372036854775807 + 1; pub fn main() { A }",
	] {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		let mut diagnostics = Diagnostics::new();
		let error = frontend::compile(&mut sources, &context, &mut diagnostics, 87)
			.err()
			.unwrap();
		let expected = Error::refused(Refusal::FrontendDiagnostic).at(Location {
			source: 0,
			revision: 87,
			start: 10,
			end: text.find(';').unwrap() as u32,
		});
		assert_eq!(error, expected);
		let messages: std::vec::Vec<_> = diagnostics
			.diagnostics()
			.iter()
			.filter_map(|diagnostic| {
				let crate::diagnostics::Diagnostic::Fatal(fatal) = diagnostic else {
					return None;
				};
				fatal
					.as_compile_error()
					.map(std::string::ToString::to_string)
			})
			.collect();
		assert_eq!(messages, ["primitive operation"]);
	}
}
