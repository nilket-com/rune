//! Instance method lookup. The receiver and all explicit arguments were
//! already evaluated in source order; the lookup reads the saved receiver.
use super::*;
impl Executor {
	/// Writes the script instance function at `Hash::associated_function(type
	/// of the receiver, method)` to `out` as a function value, or fails with
	/// stock's `MissingInstanceFunction`.
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn instance_method_call(
		&mut self,
		code: &Code,
		store: &mut Store,
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
		if let Some(id) = code.method(hash) {
			let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
			return self.put(
				store,
				destination,
				Value::Function {
					code: code.id(),
					id,
				},
			);
		}
		// A miss: the compile proved no native function at every key of this
		// program's receiver types and of every context type. A struct from
		// another program sharing the store is outside that list; stock finds
		// no script function for it either, and only a type the runtime
		// context registers could carry a native function.
		let proven = code.absent_method(hash);
		let native = self
			.active_host_runtime
			.as_deref()
			.is_some_and(|runtime| runtime.function(&hash).is_some());
		if !proven && native {
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
