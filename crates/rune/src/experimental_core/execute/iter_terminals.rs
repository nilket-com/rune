//! C17d iterator terminal and C17e adapter intrinsics. Operands were evaluated in source order.
use super::*;
use crate::function::iter_terminal::Intrinsic;

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn terminal_call(
		&mut self,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		intrinsic: Intrinsic,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if count != intrinsic.arity() || out >= function.slots {
			return Err(invalid());
		}
		let end = start.checked_add(count).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let mut values = [Value::Unit; 2];
		for (value, &slot) in values.iter_mut().zip(args) {
			if slot >= function.slots {
				return Err(invalid());
			}
			*value = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let [a, b] = values;
		let result = match intrinsic {
			Intrinsic::AdapterCount => {
				let n = store.adapter_count(a)?;
				Value::U64(u64::try_from(n).map_err(|_| invalid())?)
			}
			Intrinsic::Range => store.range_iterator(a, b)?,
			Intrinsic::Seed(target) => store.number_element(a, target)?,
			Intrinsic::Sum => store.number_fold(a, b, false)?,
			Intrinsic::Product => store.number_fold(a, b, true)?,
			Intrinsic::Insert => {
				store.collect_insert(a, b)?;
				Value::Unit
			}
			Intrinsic::IntoTuple => store.collect_tuple(a)?,
			Intrinsic::Append => {
				store.collect_append(a, b)?;
				Value::Unit
			}
			Intrinsic::OptionSome => Value::Bool(store.option_some(a)?),
			Intrinsic::RevEmpty => Value::Bool(store.rev_empty(a)?),
			Intrinsic::RevLast => Value::U64(store.rev_last(a)?),
			Intrinsic::RevAt => store.rev_at(a, b)?,
			Intrinsic::IntoIter => store.protocol_iterator(a)?,
		};
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}
