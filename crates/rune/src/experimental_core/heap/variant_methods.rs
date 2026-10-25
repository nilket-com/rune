//! C17a Option and Result methods over owned variant payloads.
use super::{FormatLimits, Payload, Store};
use crate::alloc::{Box, String};
use crate::experimental_core::error::{Error, ErrorKind, Result};
use crate::experimental_core::value::{Type, Value};
use crate::function::variant::{Family, Method};

/// What a certified Option or Result method produced. A panic carries its
/// complete stock message; every other failure is an ordinary error.
pub(crate) enum VariantOutcome {
	Value(Value),
	Panic(Box<str>),
}

impl Store {
	/// Runs `method` on an evaluated receiver. `message` is the `expect` argument.
	/// Unwrapping retains and returns the payload handle itself, never a copy.
	#[cold]
	#[inline(never)]
	pub(crate) fn variant_method(
		&mut self,
		method: Method,
		receiver: Value,
		message: Option<Value>,
		limits: &FormatLimits,
	) -> Result<VariantOutcome> {
		let parts = match receiver {
			Value::Heap(h) => match self.payload(h)? {
				Payload::Variant {
					tag,
					success,
					payload,
				} => match tag {
					Type::Option => Some((Family::Option, *success, *payload)),
					Type::Result => Some((Family::Result, *success, *payload)),
					_ => None,
				},
				_ => None,
			},
			_ => None,
		};
		let Some((family, success, payload)) =
			parts.filter(|(family, ..)| method.provided_by(*family))
		else {
			return Err(self.text_missing(receiver, method.hash())?);
		};
		let flag = |value: bool| Ok(VariantOutcome::Value(Value::Bool(value)));
		match method {
			Method::IsSome | Method::IsOk => return flag(success),
			Method::IsNone | Method::IsErr => return flag(!success),
			Method::Unwrap | Method::Expect => (),
		}
		if success {
			let value = payload.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "variant"))?;
			self.retain(value)?;
			return Ok(VariantOutcome::Value(value));
		}
		let mut text = match (method, message) {
			(Method::Expect, Some(message)) => {
				self.format_value_with_limits(message, false, limits)?
			}
			(Method::Unwrap, None) => String::try_from(match family {
				Family::Option => "Called `Option::unwrap()` on a `None` value",
				Family::Result => "Called `Result::unwrap()` on an `Err` value: ",
			})?,
			_ => {
				return Err(Error::new(
					ErrorKind::InvalidCode,
					"variant method operands",
				))
			}
		};
		if family == Family::Result {
			let error = payload.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "variant"))?;
			let debug = self.format_value_with_limits(error, true, limits)?;
			if method == Method::Expect {
				text.try_push_str(": ")?;
			}
			text.try_push_str(&debug)?;
		}
		if text.len() > limits.bytes {
			return Err(Error::new(ErrorKind::FormatLimit, "variant panic message"));
		}
		Ok(VariantOutcome::Panic(Box::try_from(text.as_str())?))
	}
}
