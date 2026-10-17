//! Cold certified sort operations. Comparators run only through CallValue frames.
use super::super::code::{
	SORT_BEGIN, SORT_COMMIT, SORT_LEN, SORT_LOAD, SORT_ORDERING, SORT_STORE, SORT_VALUE,
};
use super::*;
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn sort_call(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let expected = match id {
			SORT_BEGIN | SORT_VALUE | SORT_COMMIT => 2,
			SORT_LEN | SORT_ORDERING => 1,
			SORT_LOAD => 3,
			SORT_STORE => 4,
			_ => return Err(invalid()),
		};
		if len != expected || out >= function.slots {
			return Err(invalid());
		}
		let end = start.checked_add(len).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
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
		let index = |i| -> Result<usize> {
			match get(i)? {
				Value::I64(n) => usize::try_from(n).map_err(|_| invalid()),
				_ => Err(invalid()),
			}
		};
		let value = get(0)?;
		let result = match id {
			SORT_BEGIN => {
				// Stock resolves mutable receiver then &Function before checking len.
				store.sort_receiver(value).map_err(sort_argument_access)?;
				match get(1)? {
					Value::Function { code: owner, id }
						if owner == code.id() && code.function(id).is_some() => {}
					Value::Function { .. } => {
						return Err(error(ErrorKind::Type).with_detail(Detail::ForeignFunction))
					}
					closure @ Value::Heap(_) => {
						let (owner, id, _) = match store.closure_parts(closure) {
							Ok(parts) => parts,
							Err(_) => {
								return Err(error(ErrorKind::Type).with_detail(
									Detail::NumericArgumentType {
										cause: super::super::error::HostCause::ExpectedType,
										arg: 1,
										expected: TypeName::Function,
										found: store
											.diagnostic_type(closure)
											.ok_or_else(invalid)?,
									},
								));
							}
						};
						if owner != code.id() || code.function(id).is_none() {
							return Err(error(ErrorKind::Type).with_detail(Detail::ForeignFunction));
						}
					}
					other => {
						return Err(error(ErrorKind::Type).with_detail(
							Detail::NumericArgumentType {
								cause: super::super::error::HostCause::ExpectedType,
								arg: 1,
								expected: TypeName::Function,
								found: store.diagnostic_type(other).ok_or_else(invalid)?,
							},
						))
					}
				}
				store.sort_begin(value)?
			}
			SORT_LEN => Value::I64(i64::try_from(store.sort_len(value)?).map_err(|_| invalid())?),
			SORT_LOAD => Value::I64(
				i64::try_from(store.sort_load(value, index(1)?, index(2)?)?)
					.map_err(|_| invalid())?,
			),
			SORT_VALUE => store.sort_value(value, index(1)?)?,
			SORT_STORE => {
				store.sort_store(value, index(1)?, index(2)?, index(3)?)?;
				Value::Unit
			}
			SORT_COMMIT => {
				store.sort_commit(value, index(1)?)?;
				Value::Unit
			}
			SORT_ORDERING => {
				return Err(error(ErrorKind::Type).with_detail(Detail::ExpectedType {
					expected: TypeName::Ordering,
					found: store.diagnostic_type(value).ok_or_else(invalid)?,
				}))
			}
			_ => return Err(invalid()),
		};
		// Destination validation above precedes producing a newly owned result.
		// put publishes it before releasing the previous destination owner.
		self.put(store, destination, result)
	}
}

fn sort_argument_access(error: Error) -> Error {
	if error.detail == Detail::WriteExclusive {
		error.with_detail(Detail::ExclusiveArgument {
			access: super::super::error::MovedAccess::Write,
			arg: 0,
		})
	} else {
		error
	}
}
