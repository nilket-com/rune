//! C17c numeric providers: stock `FromValue` operand conversion and the stock
//! result for every value.
use super::super::error::{Detail, HostCause, IntTarget, IntValue, TypeName};
use super::*;
use crate::function::number::{Method, Operand, Pair};

/// A converted operand.
#[derive(Clone, Copy)]
enum Number {
	I64(i64),
	U64(u64),
	F64(f64),
	Char(char),
	U32(u32),
	I32(i32),
}

/// The longest `f64` Display is a negative subnormal: `-0.`, 323 zeros and
/// at most 17 digits (343 bytes); integers need at most 20.
const TEXT: usize = 352;

struct Text {
	bytes: [u8; TEXT],
	len: usize,
}

impl core::fmt::Write for Text {
	fn write_str(&mut self, s: &str) -> core::fmt::Result {
		let end = self.len.checked_add(s.len()).ok_or(core::fmt::Error)?;
		self.bytes
			.get_mut(self.len..end)
			.ok_or(core::fmt::Error)?
			.copy_from_slice(s.as_bytes());
		self.len = end;
		Ok(())
	}
}

impl Store {
	fn number_type(&self, value: Value) -> Result<TypeName> {
		self.diagnostic_type(value)
			.ok_or_else(|| err(ErrorKind::InvalidCode))
	}

	/// Converts operand `arg` as the stock native parameter of type `operand`.
	fn number_operand(&self, value: Value, operand: Operand, arg: u8) -> Result<Number> {
		let expected = match operand {
			Operand::F64 => match value {
				Value::F64(v) => return Ok(Number::F64(v)),
				_ => TypeName::F64,
			},
			Operand::Char => match value {
				Value::Char(v) => return Ok(Number::Char(v)),
				_ => TypeName::Char,
			},
			Operand::I64 | Operand::U64 | Operand::U32 | Operand::I32 => {
				let Some(int) = IntValue::of(value) else {
					return Err(
						err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
							cause: HostCause::ExpectedNumber,
							arg,
							found: self.number_type(value)?,
						}),
					);
				};
				let wide = int.wide();
				let (converted, to) = match operand {
					Operand::I64 => (i64::try_from(wide).ok().map(Number::I64), IntTarget::I64),
					Operand::U64 => (u64::try_from(wide).ok().map(Number::U64), IntTarget::U64),
					Operand::U32 => (u32::try_from(wide).ok().map(Number::U32), IntTarget::U32),
					_ => (i32::try_from(wide).ok().map(Number::I32), IntTarget::I32),
				};
				return converted.ok_or_else(|| {
					err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
						cause: HostCause::IntegerCoercion,
						arg,
						value: int,
						to,
					})
				});
			}
		};
		Err(
			err(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
				cause: HostCause::ExpectedType,
				arg,
				expected,
				found: self.number_type(value)?,
			}),
		)
	}

	fn number_option(&mut self, value: Option<Value>) -> Result<Value> {
		self.variant(Type::Option, value.is_some(), value)
	}

	fn number_text(&mut self, value: &dyn core::fmt::Display) -> Result<Value> {
		let mut text = Text {
			bytes: [0; TEXT],
			len: 0,
		};
		core::fmt::write(&mut text, format_args!("{value}"))
			.map_err(|_| err(ErrorKind::InvalidCode))?;
		let text = core::str::from_utf8(&text.bytes[..text.len])
			.map_err(|_| err(ErrorKind::InvalidCode))?;
		self.string(text)
	}

	/// Calls the stock provider `pair` with `args`, receiver first.
	#[cold]
	#[inline(never)]
	pub(crate) fn number(&mut self, pair: Pair, args: &[Value]) -> Result<Value> {
		use Method as M;
		use Number as N;
		let invalid = || err(ErrorKind::InvalidCode);
		if args.len() != pair.method().arity() as usize {
			return Err(invalid());
		}
		let (own, other) = pair.operands();
		let a = self.number_operand(*args.first().ok_or_else(invalid)?, own, 0)?;
		let b = match other {
			Some(operand) => {
				Some(self.number_operand(*args.get(1).ok_or_else(invalid)?, operand, 1)?)
			}
			None => None,
		};
		let overflow = || err(ErrorKind::Overflow);
		let zero = || err(ErrorKind::DivisionByZero);
		macro_rules! option {
			($value:expr, $wrap:path) => {
				return self.number_option($value.map($wrap))
			};
		}
		Ok(match (pair.method(), a, b) {
			(M::Sqrt, N::F64(x), None) => Value::F64(x.sqrt()),
			(M::Floor, N::F64(x), None) => Value::F64(x.floor()),
			(M::Ceil, N::F64(x), None) => Value::F64(x.ceil()),
			(M::Exp, N::F64(x), None) => Value::F64(x.exp()),
			(M::Ln, N::F64(x), None) => Value::F64(x.ln()),
			(M::Powi, N::F64(x), Some(N::I32(n))) => Value::F64(x.powi(n)),
			(M::Min, N::F64(x), Some(N::F64(y))) => Value::F64(x.min(y)),
			(M::Max, N::F64(x), Some(N::F64(y))) => Value::F64(x.max(y)),
			(M::Min, N::I64(x), Some(N::I64(y))) => Value::I64(Ord::min(x, y)),
			(M::Max, N::I64(x), Some(N::I64(y))) => Value::I64(Ord::max(x, y)),
			(M::Min, N::U64(x), Some(N::U64(y))) => Value::U64(Ord::min(x, y)),
			(M::Max, N::U64(x), Some(N::U64(y))) => Value::U64(Ord::max(x, y)),
			(M::PartialCmp, N::F64(x), Some(N::F64(y))) => {
				option!(x.partial_cmp(&y), Value::Ordering)
			}
			(M::ToString, N::I64(x), None) => return self.number_text(&x),
			(M::ToString, N::U64(x), None) => return self.number_text(&x),
			(M::ToString, N::F64(x), None) => return self.number_text(&x),
			(M::Pow, N::I64(x), Some(N::U32(n))) => {
				Value::I64(x.checked_pow(n).ok_or_else(overflow)?)
			}
			(M::Pow, N::U64(x), Some(N::U32(n))) => {
				Value::U64(x.checked_pow(n).ok_or_else(overflow)?)
			}
			(M::CheckedPow, N::I64(x), Some(N::U32(n))) => option!(x.checked_pow(n), Value::I64),
			(M::CheckedPow, N::U64(x), Some(N::U32(n))) => option!(x.checked_pow(n), Value::U64),
			(M::WrappingPow, N::I64(x), Some(N::U32(n))) => Value::I64(x.wrapping_pow(n)),
			(M::WrappingPow, N::U64(x), Some(N::U32(n))) => Value::U64(x.wrapping_pow(n)),
			(M::SaturatingPow, N::I64(x), Some(N::U32(n))) => Value::I64(x.saturating_pow(n)),
			(M::SaturatingPow, N::U64(x), Some(N::U32(n))) => Value::U64(x.saturating_pow(n)),
			(M::CheckedAdd, N::I64(x), Some(N::I64(y))) => option!(x.checked_add(y), Value::I64),
			(M::CheckedSub, N::I64(x), Some(N::I64(y))) => option!(x.checked_sub(y), Value::I64),
			(M::CheckedMul, N::I64(x), Some(N::I64(y))) => option!(x.checked_mul(y), Value::I64),
			(M::CheckedDiv, N::I64(x), Some(N::I64(y))) => option!(x.checked_div(y), Value::I64),
			(M::CheckedRem, N::I64(x), Some(N::I64(y))) => option!(x.checked_rem(y), Value::I64),
			(M::CheckedAdd, N::U64(x), Some(N::U64(y))) => option!(x.checked_add(y), Value::U64),
			(M::CheckedSub, N::U64(x), Some(N::U64(y))) => option!(x.checked_sub(y), Value::U64),
			(M::CheckedMul, N::U64(x), Some(N::U64(y))) => option!(x.checked_mul(y), Value::U64),
			(M::CheckedDiv, N::U64(x), Some(N::U64(y))) => option!(x.checked_div(y), Value::U64),
			(M::CheckedRem, N::U64(x), Some(N::U64(y))) => option!(x.checked_rem(y), Value::U64),
			(M::WrappingAdd, N::I64(x), Some(N::I64(y))) => Value::I64(x.wrapping_add(y)),
			(M::WrappingSub, N::I64(x), Some(N::I64(y))) => Value::I64(x.wrapping_sub(y)),
			(M::WrappingMul, N::I64(x), Some(N::I64(y))) => Value::I64(x.wrapping_mul(y)),
			(M::WrappingAdd, N::U64(x), Some(N::U64(y))) => Value::U64(x.wrapping_add(y)),
			(M::WrappingSub, N::U64(x), Some(N::U64(y))) => Value::U64(x.wrapping_sub(y)),
			(M::WrappingMul, N::U64(x), Some(N::U64(y))) => Value::U64(x.wrapping_mul(y)),
			(M::WrappingDiv | M::WrappingRem, N::I64(_), Some(N::I64(0)))
			| (M::WrappingDiv | M::WrappingRem, N::U64(_), Some(N::U64(0))) => return Err(zero()),
			(M::WrappingDiv, N::I64(x), Some(N::I64(y))) => Value::I64(x.wrapping_div(y)),
			(M::WrappingRem, N::I64(x), Some(N::I64(y))) => Value::I64(x.wrapping_rem(y)),
			(M::WrappingDiv, N::U64(x), Some(N::U64(y))) => Value::U64(x.wrapping_div(y)),
			(M::WrappingRem, N::U64(x), Some(N::U64(y))) => Value::U64(x.wrapping_rem(y)),
			(M::SaturatingAdd, N::I64(x), Some(N::I64(y))) => Value::I64(x.saturating_add(y)),
			(M::SaturatingSub, N::I64(x), Some(N::I64(y))) => Value::I64(x.saturating_sub(y)),
			(M::SaturatingMul, N::I64(x), Some(N::I64(y))) => Value::I64(x.saturating_mul(y)),
			(M::SaturatingAdd, N::U64(x), Some(N::U64(y))) => Value::U64(x.saturating_add(y)),
			(M::SaturatingSub, N::U64(x), Some(N::U64(y))) => Value::U64(x.saturating_sub(y)),
			(M::SaturatingMul, N::U64(x), Some(N::U64(y))) => Value::U64(x.saturating_mul(y)),
			(M::CheckedAbs, N::I64(x), None) => option!(x.checked_abs(), Value::I64),
			(M::WrappingAbs, N::I64(x), None) => Value::I64(x.wrapping_abs()),
			(M::SaturatingAbs, N::I64(x), None) => Value::I64(x.saturating_abs()),
			(M::Signum, N::I64(x), None) => Value::I64(x.signum()),
			(M::ToI64, N::Char(c), None) => Value::I64(i64::from(u32::from(c))),
			(M::FromI64, N::I64(v), None) => {
				if v < 0 {
					return Err(overflow().with_detail(Detail::Underflow));
				}
				let v = u32::try_from(v).map_err(|_| overflow())?;
				option!(char::from_u32(v), Value::Char)
			}
			_ => return Err(invalid()),
		})
	}
}
