//! Certified resize uses the cold intrinsic call path and existing Store values.
use super::*;
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn vec_resize_call(
		&mut self,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if count != 3 || out >= function.slots {
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
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		store
			.vec_resize(values[0], values[1], values[2])
			.map_err(|error| {
				if error.detail == Detail::ReadExclusive {
					error.with_detail(Detail::ExclusiveArgument {
						access: super::super::error::MovedAccess::Read,
						arg: 0,
					})
				} else {
					error
				}
			})?;
		self.put(store, destination, Value::Unit)
	}
}
