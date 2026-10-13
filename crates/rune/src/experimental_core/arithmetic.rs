//! Primitive rules from the explicitly post-observation 0184 operator amendment.
//! Strict-bool lazy operators remain a deliberate slice difference.
//! No old Value, protocol dispatcher, VM or native binding is consulted here.

use super::error::{
	BinarySymbol, Detail, Error, ErrorKind, IntTarget, IntValue, Result, TypeName, UnarySymbol,
};
use super::value::{Type as ValueType, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
	Add,
	Sub,
	Mul,
	Div,
	Rem,
	Eq,
	Ne,
	Lt,
	Le,
	Gt,
	Ge,
	BitAnd,
	BitOr,
	BitXor,
	Shl,
	Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
	Neg,
	Not,
}

fn error(kind: ErrorKind) -> Error {
	Error::new(kind, "primitive operation")
}

fn checked<T>(value: Option<T>) -> Result<T> {
	value.ok_or_else(|| error(ErrorKind::Overflow))
}

pub(crate) fn compare(op: Binary, eq: bool, lt: bool, gt: bool) -> Result<Value> {
	Ok(Value::Bool(match op {
		Binary::Eq => eq,
		Binary::Ne => !eq,
		Binary::Lt => lt,
		Binary::Le => lt || eq,
		Binary::Gt => gt,
		Binary::Ge => gt || eq,
		_ => return Err(error(ErrorKind::Type)),
	}))
}

#[cold]
#[inline(never)]
fn type_error(op: Binary, a: Value, b: Value) -> Error {
	let mut error = error(ErrorKind::Type);
	if let (Some(lhs), Some(rhs)) = (TypeName::primitive(a), TypeName::primitive(b)) {
		error.detail = Detail::Binary {
			op: BinarySymbol::from_op(op, false),
			lhs,
			rhs,
		};
	}
	error
}

// Binary's type errors retain their original operands. The shared compare
// helper also serves other callers and deliberately keeps its own contract.
fn compare_binary(op: Binary, a: Value, b: Value, eq: bool, lt: bool, gt: bool) -> Result<Value> {
	match op {
		Binary::Eq | Binary::Ne | Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge => {
			compare(op, eq, lt, gt)
		}
		_ => Err(type_error(op, a, b)),
	}
}

fn shift(value: Value) -> Result<u32> {
	let count = match value {
		Value::I64(n) => u32::try_from(n).map_err(|_| {
			error(ErrorKind::InvalidShift).with_detail(Detail::Coerce {
				value: IntValue {
					negative: n < 0,
					magnitude: n.unsigned_abs(),
				},
				to: IntTarget::U32,
			})
		}),
		Value::U64(n) => u32::try_from(n).map_err(|_| {
			error(ErrorKind::InvalidShift).with_detail(Detail::Coerce {
				value: IntValue {
					negative: false,
					magnitude: n,
				},
				to: IntTarget::U32,
			})
		}),
		_ => return Err(error(ErrorKind::Type)),
	}?;
	if count >= 64 {
		return Err(error(ErrorKind::InvalidShift).with_detail(Detail::ShiftWidth));
	}
	Ok(count)
}

// Bool ordering follows the script contract, as on the stock VM.
#[allow(clippy::bool_comparison)]
pub fn binary(op: Binary, a: Value, b: Value) -> Result<Value> {
	use Binary::*;
	use Value::*;
	let original_a = a;
	let original_b = b;

	// Shifts convert the count to u32 independently of the left integer's tag.
	// Other mixed integer operations convert the right operand to the left tag.
	let b = match (op, a, b) {
		(Shl | Shr, I64(_), U64(_)) => I64(i64::from(shift(b)?)),
		(Shl | Shr, U64(_), I64(_)) => U64(u64::from(shift(b)?)),
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
				let n = shift(I64(b))?;
				if op == Shl {
					I64(a << n)
				} else {
					I64(a >> n)
				}
			}

			_ => return compare_binary(op, original_a, original_b, a == b, a < b, a > b),
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
				let n = shift(U64(b))?;
				if op == Shl {
					U64(a << n)
				} else {
					U64(a >> n)
				}
			}

			_ => return compare_binary(op, original_a, original_b, a == b, a < b, a > b),
		}),
		(F64(a), F64(b)) => Ok(match op {
			Add => F64(a + b),
			Sub => F64(a - b),
			Mul => F64(a * b),
			Div => F64(a / b),
			Rem => F64(a % b),
			_ => return compare_binary(op, original_a, original_b, a == b, a < b, a > b),
		}),
		(Bool(a), Bool(b)) => match op {
			BitAnd => Ok(Bool(a & b)),
			BitOr => Ok(Bool(a | b)),
			BitXor => Ok(Bool(a ^ b)),
			Eq | Ne | Lt | Le | Gt | Ge => {
				compare_binary(op, original_a, original_b, a == b, a < b, a > b)
			}
			_ => Err(type_error(op, original_a, original_b)),
		},

		(Char(a), Char(b)) => compare_binary(op, original_a, original_b, a == b, a < b, a > b),
		(Unit, Unit) => compare_binary(op, original_a, original_b, true, false, false),
		_ => Err(type_error(op, original_a, original_b)),
	}
}

pub fn unary(op: Unary, value: Value) -> Result<Value> {
	Ok(match (op, value) {
		(Unary::Neg, Value::I64(a)) => Value::I64(checked(a.checked_neg())?),
		(Unary::Neg, Value::F64(a)) => Value::F64(-a),
		(Unary::Not, Value::Bool(a)) => Value::Bool(!a),
		(Unary::Not, Value::I64(a)) => Value::I64(!a),
		(Unary::Not, Value::U64(a)) => Value::U64(!a),
		_ => {
			let mut error = error(ErrorKind::Type);
			if let Some(operand) = TypeName::primitive(value) {
				error.detail = Detail::Unary {
					op: match op {
						Unary::Neg => UnarySymbol::Neg,
						Unary::Not => UnarySymbol::Not,
					},
					operand,
				};
			}
			return Err(error);
		}
	})
}

/// Numeric Rust `as`: integer wrapping, IEEE rounding and saturating float casts.
/// Identity casts are admitted only for the three numeric tags.
pub fn cast(value: Value, target: ValueType) -> Result<Value> {
	use Value::*;
	Ok(match (value, target) {
		(I64(a), ValueType::I64) => I64(a),
		(I64(a), ValueType::U64) => U64(a as u64),
		(I64(a), ValueType::F64) => F64(a as f64),
		(U64(a), ValueType::I64) => I64(a as i64),
		(U64(a), ValueType::U64) => U64(a),
		(U64(a), ValueType::F64) => F64(a as f64),
		(F64(a), ValueType::I64) => I64(a as i64),
		(F64(a), ValueType::U64) => U64(a as u64),
		(F64(a), ValueType::F64) => F64(a),
		_ => {
			let mut error = error(ErrorKind::InvalidCast);
			if let Some(found) = TypeName::primitive(value) {
				error.detail = Detail::Cast {
					found,
					to_numeric: matches!(target, ValueType::I64 | ValueType::U64 | ValueType::F64),
				};
			}
			return Err(error);
		}
	})
}

#[cfg(test)]
mod binary_tests;
