//! The receiver and all explicit arguments were already evaluated in source order.
use super::*;
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn missing_method_call(
		&self,
		code: &Code,
		store: &Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if count != 2 || out >= function.slots {
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
		let Value::U64(method) = read(args[0])? else {
			return Err(invalid());
		};
		let target = store.diagnostic_type(read(args[1])?).ok_or_else(invalid)?;
		let instance = match target {
			TypeName::Struct(n) => super::super::nominal::hash_copy(n).map(crate::Hash::new),
			_ => target.builtin_hash(),
		}
		.ok_or_else(invalid)?;
		let hash = crate::Hash::associated_function(instance, crate::Hash::new(method));
		// Missing proof means an invariant breach, never a user-facing method path.
		if !code.absent_method(hash) {
			return Err(invalid());
		}
		Err(
			error(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
				target,
				instance,
				hash,
			}),
		)
	}
}
