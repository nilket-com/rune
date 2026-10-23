//! Certified Vec constructors use existing List storage and ordinary Call.
use super::*;

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn vec_constructor_call(
		&mut self,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		count: u32,
	) -> Result<()> {
		if id == super::super::code::VEC_RESIZE {
			return self.vec_resize_call(store, function, base, out, start, count);
		}
		let invalid = || error(ErrorKind::InvalidCode);
		let capacity = id == super::super::code::VEC_WITH_CAPACITY;
		if !matches!(
			id,
			super::super::code::VEC_NEW | super::super::code::VEC_WITH_CAPACITY
		) || count != u32::from(capacity)
			|| out >= function.slots
		{
			return Err(invalid());
		}
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		let end = start.checked_add(count).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let n = if capacity {
			let slot = args[0];
			if slot >= function.slots {
				return Err(invalid());
			}
			let value = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
			store.vec_capacity_argument(value)?
		} else {
			0
		};
		let value = store.vec_with_capacity(n)?;
		self.put(store, destination, value)
	}
}
