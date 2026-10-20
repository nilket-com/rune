//! C17b Object method and `is_empty` calls. Operands were evaluated in source order.
use super::*;
use crate::function::fresh_object::Method;

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn object_method_call(
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
		let mut values = [Value::Unit; 3];
		for (value, &slot) in values.iter_mut().zip(args) {
			if slot >= function.slots {
				return Err(invalid());
			}
			*value = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let result = store
			.object_method(method, &values[..count as usize])
			.map_err(|e| {
				// A wrong receiver needs the compile-time absence proof.
				match e.detail {
					Detail::MissingInstanceFunction { hash, .. } if !code.absent_method(hash) => {
						invalid()
					}
					_ => e,
				}
			})?;
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}
