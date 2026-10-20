//! Cold C12 intrinsics. Ownership stays in Store and ordinary charged frames.
use super::super::{code, value::Type};
use super::*;
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn small_forms_call(
		&mut self,
		store: &mut Store,
		function: &code::Function,
		base: usize,
		out: u32,
		id: u32,
		args_start: u32,
		args_len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
		let args = function
			.args
			.get(args_start as usize..end as usize)
			.ok_or_else(invalid)?;
		let mut values = [Value::Unit; 2];
		if args.len() > values.len() || out >= function.slots {
			return Err(invalid());
		}
		for (dst, slot) in values.iter_mut().zip(args) {
			if *slot >= function.slots {
				return Err(invalid());
			}
			*dst = *self
				.slots
				.get(base.checked_add(*slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let result = match id {
			code::RANGE_FROM => store.other_range(Type::RangeFrom, &values[..args.len()])?,
			code::RANGE_FULL => store.other_range(Type::RangeFull, &values[..args.len()])?,
			code::RANGE_INCLUSIVE => {
				store.other_range(Type::RangeInclusive, &values[..args.len()])?
			}
			code::RANGE_TO_INCLUSIVE => {
				store.other_range(Type::RangeToInclusive, &values[..args.len()])?
			}
			code::RANGE_TO => store.other_range(Type::RangeTo, &values[..args.len()])?,
			code::OBJECT_HAS_FIELD => Value::Bool(store.object_has_field(values[0], values[1])?),
			code::ITER_METHOD => match store.tag(values[0])? {
				Type::Vec
				| Type::Tuple
				| Type::Object
				| Type::Option
				| Type::Range
				| Type::RangeInclusive
				| Type::RangeFrom => store.iterator(values[0])?,
				Type::Unit => store.unit_iterator()?,
				_ => return Err(missing_iterator_method(store, values[0], "iter")?),
			},
			code::ANY_READY => {
				if store.tag(values[0])? != Type::Iterator {
					return Err(missing_iterator_method(store, values[0], "any")?);
				}
				store.retain(values[0])?;
				values[0]
			}
			code::NEXT_METHOD => {
				if store.tag(values[0])? != Type::Iterator {
					return Err(missing_iterator_method(store, values[0], "next")?);
				}
				let next = store.iterator_next(values[0])?;
				let result = store.variant(Type::Option, next.is_some(), next);
				if let Some(value) = next {
					store.release(value)?;
				}
				result?
			}
			_ => return Err(invalid()),
		};
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}

fn missing_iterator_method(store: &Store, receiver: Value, name: &str) -> Result<Error> {
	let target = store
		.diagnostic_type(receiver)
		.ok_or_else(|| error(ErrorKind::InvalidCode))?;
	let instance = match target {
		TypeName::Struct(n) => super::super::nominal::hash_copy(n).map(crate::Hash::new),
		_ => target.builtin_hash(),
	}
	.ok_or_else(|| error(ErrorKind::InvalidCode))?;
	Ok(
		error(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
			target,
			instance,
			hash: crate::Hash::associated_function(instance, crate::Hash::ident(name)),
		}),
	)
}
