//! Numeric builtins on the existing cold certified Call miss path only.
use super::super::error::HostCause;
use super::*;
use crate::function::fresh_numeric::{Call, Method};
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn numeric_call(
		&mut self,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		call: Call,
		start: u32,
		len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if len != call.name().arity() {
			return Err(invalid());
		}
		let end = start.checked_add(len).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let get = |i: usize| -> Result<Value> {
			let slot = *args.get(i).ok_or_else(invalid)?;
			if slot >= function.slots {
				return Err(invalid());
			}
			self.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.copied()
				.ok_or_else(invalid)
		};
		if out >= function.slots {
			return Err(invalid());
		}
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		let receiver = get(0)?;
		if call == Call::MethodCmp {
			let rhs = get(1)?;
			let value = store.method_cmp(receiver, rhs)?;
			return self.put(store, destination, value);
		}
		if call.is_method() {
			let admitted = matches!(receiver, Value::F64(_))
				|| call == Call::MethodAbs && matches!(receiver, Value::I64(_));
			if !admitted {
				let target = store.diagnostic_type(receiver).ok_or_else(invalid)?;
				let instance = match target {
					TypeName::Struct(n) => {
						super::super::nominal::hash_copy(n).map(crate::Hash::new)
					}
					_ => target.builtin_hash(),
				}
				.ok_or_else(invalid)?;
				return Err(
					error(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
						target,
						instance,
						hash: crate::Hash::associated_function(
							instance,
							crate::Hash::ident(call.name().name()),
						),
					}),
				);
			}
		}
		let result = if call == Call::I64Abs
			|| call == Call::MethodAbs && matches!(receiver, Value::I64(_))
		{
			let value = match receiver {
				Value::I64(v) => v,
				Value::U64(v) => i64::try_from(v).map_err(|_| {
					error(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
						cause: HostCause::IntegerCoercion,
						arg: 0,
						value: IntValue::Unsigned(v),
						to: IntTarget::I64,
					})
				})?,
				_ => {
					return Err(
						error(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
							cause: HostCause::ExpectedNumber,
							arg: 0,
							found: store.diagnostic_type(receiver).ok_or_else(invalid)?,
						}),
					)
				}
			};
			Value::I64(
				value
					.checked_abs()
					.ok_or_else(|| error(ErrorKind::Overflow))?,
			)
		} else {
			let convert = |v: Value, arg: u8| -> Result<f64> {
				if let Value::F64(v) = v {
					Ok(v)
				} else {
					Err(
						error(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
							cause: HostCause::ExpectedType,
							arg,
							expected: TypeName::F64,
							found: store.diagnostic_type(v).ok_or_else(invalid)?,
						}),
					)
				}
			};
			let x = convert(receiver, 0)?;
			Value::F64(match call.name() {
				Method::Abs => x.abs(),
				Method::Round => x.round(),
				Method::Powf => x.powf(convert(get(1)?, 1)?),
				Method::Cmp => return Err(invalid()),
			})
		};
		self.put(store, destination, result)?;
		Ok(())
	}
}
