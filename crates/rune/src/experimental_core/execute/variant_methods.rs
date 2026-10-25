//! C17a Option and Result method calls. Operands were evaluated in source order.
use super::*;
use crate::experimental_core::heap::VariantOutcome;
use crate::function::variant::Method;

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn variant_method_call(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		method: Method,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if count != method.arity() || out >= function.slots {
			return Err(invalid());
		}
		let end = start.checked_add(count).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let read = |slot: u32| {
			if slot >= function.slots {
				return Err(invalid());
			}
			self.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.copied()
				.ok_or_else(invalid)
		};
		let receiver = read(args[0])?;
		let message = args.get(1).map(|slot| read(*slot)).transpose()?;
		let outcome = store
			.variant_method(method, receiver, message, &self.format_limits)
			.map_err(|e| {
				// A wrong receiver needs the compile-time absence proof.
				match e.detail {
					Detail::MissingInstanceFunction { hash, .. } if !code.absent_method(hash) => {
						invalid()
					}
					_ => e,
				}
			})?;
		match outcome {
			VariantOutcome::Value(value) => self.put(
				store,
				base.checked_add(out as usize).ok_or_else(invalid)?,
				value,
			),
			VariantOutcome::Panic(message) => {
				self.pending_host_message = Some(message);
				Err(error(ErrorKind::Panic).with_detail(Detail::PanicMessage))
			}
		}
	}
}
