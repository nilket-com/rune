//! Managed sort scratch owns one parent edge and temporarily holds its list buffer.
//! The parent cannot be invalidated or mutated until this guard is released.
use super::super::error::Detail;
use super::*;
pub(super) const EXCLUSIVE: u32 = u32::MAX;
pub(super) enum Access {
	Read,
	Write,
	Take,
}
#[inline(always)]
pub(super) fn conflict(kind: Access, entry: &Entry) -> Error {
	match kind {
		Access::Read => {
			if entry.readers == EXCLUSIVE {
				access(Access::Read)
			} else {
				err(ErrorKind::AccessConflict).with_detail(Detail::Moved {
					access: super::super::error::MovedAccess::Read,
				})
			}
		}
		Access::Write | Access::Take => {
			// Only Detail crosses the opaque call. Construct Error afterwards.
			err(ErrorKind::AccessConflict).with_detail(borrowed_detail(kind, entry))
		}
	}
}
#[cold]
#[inline(never)]
fn borrowed_detail(kind: Access, entry: &Entry) -> Detail {
	if entry.readers == EXCLUSIVE {
		match kind {
			Access::Read => Detail::ReadExclusive,
			Access::Write => Detail::WriteExclusive,
			Access::Take => Detail::TakeExclusive,
		}
	} else {
		match kind {
			Access::Read => Detail::ReadExclusive,
			Access::Write => Detail::WriteBorrowed,
			Access::Take => Detail::TakeBorrowed,
		}
	}
}
pub(super) fn access(access: Access) -> Error {
	err(ErrorKind::AccessConflict).with_detail(match access {
		Access::Read => Detail::ReadExclusive,
		Access::Write => Detail::WriteExclusive,
		Access::Take => Detail::TakeExclusive,
	})
}
#[derive(Debug)]
pub(super) struct SortState {
	pub(super) parent: Handle,
	pub(super) active: bool,
	banks: [Vec<usize>; 2],
	// Parent keeps the byte charge; this guard owns the held child edges.
	pub(super) values: Vec<Value>,
}
impl SortState {
	pub(super) fn bytes(&self) -> Result<usize> {
		self.banks[0]
			.capacity()
			.checked_add(self.banks[1].capacity())
			.and_then(|n| n.checked_mul(size_of::<usize>()))
			.and_then(|n| n.checked_add(size_of::<Self>()))
			.ok_or_else(|| err(ErrorKind::StoreCapacity))
	}
}
impl Store {
	/// All allocation and owner admission precede publishing the sentinel.
	pub(crate) fn sort_begin(&mut self, value: Value) -> Result<Value> {
		self.sort_receiver(value)?;
		let Value::Heap(parent) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::List { values, .. } = self.payload(parent)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let len = values.len();
		if len < 2 {
			return Ok(Value::I64(len as i64));
		}
		let needed = len
			.checked_mul(2 * size_of::<usize>())
			.and_then(|n| n.checked_add(size_of::<SortState>()))
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		if needed > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let first = Vec::try_with_capacity(len)?;
		let first_charge = first
			.capacity()
			.checked_mul(size_of::<usize>())
			.and_then(|n| n.checked_add(size_of::<SortState>()))
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		if first_charge
			.checked_add(
				len.checked_mul(size_of::<usize>())
					.ok_or_else(|| err(ErrorKind::StoreCapacity))?,
			)
			.is_none_or(|n| n > self.limits.bytes.saturating_sub(self.bytes))
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		let second = Vec::try_with_capacity(len)?;
		let mut banks = [first, second];
		for bank in &mut banks {
			for index in 0..len {
				bank.try_push(index)?;
			}
		}
		let state = SortState {
			parent,
			active: true,
			banks,
			values: Vec::new(),
		};
		if state.bytes()? > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let guard = self.admit(Payload::SortGuard(
			Box::try_new(state).map_err(|_| Error::new(ErrorKind::Allocation, "reserve"))?,
		))?;
		// Admission completed all allocation, edge reservation and fallible checks.
		// Publish without user code: the old Moved branch is the cold read gate.
		let entry = self.entry_mut(parent).expect("admitted parent edge");
		let Some(Payload::List {
			tag: Type::Vec,
			values,
		}) = entry.payload.replace(Payload::Moved(Type::Vec))
		else {
			unreachable!()
		};
		entry.readers = EXCLUSIVE;
		let Value::Heap(h) = guard else {
			unreachable!()
		};
		let Some(Payload::SortGuard(state)) =
			self.entry_mut(h).expect("admitted guard").payload.as_mut()
		else {
			unreachable!()
		};
		state.values = values;
		Ok(guard)
	}
	pub(crate) fn sort_receiver(&mut self, value: Value) -> Result<()> {
		if !matches!(value, Value::Heap(_)) || self.tag(value)? != Type::Vec {
			let target = self
				.diagnostic_type(value)
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			let instance = match target {
				crate::experimental_core::error::TypeName::Struct(n) => {
					crate::experimental_core::nominal::hash_copy(n).map(crate::Hash::new)
				}
				_ => target.builtin_hash(),
			}
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			return Err(
				err(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
					target,
					instance,
					hash: crate::Hash::associated_function(instance, crate::Hash::ident("sort_by")),
				}),
			);
		}
		let Value::Heap(parent) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		self.mutable(parent)?;
		Ok(())
	}
	fn sort_state(&self, guard: Value) -> Result<&SortState> {
		let Value::Heap(h) = guard else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::SortGuard(state) = self.payload(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		if !state.active || self.entry(state.parent)?.readers != EXCLUSIVE {
			return Err(err(ErrorKind::InvalidCode));
		}
		Ok(state)
	}
	pub(crate) fn sort_len(&self, guard: Value) -> Result<usize> {
		if let Value::I64(n @ 0..=1) = guard {
			return Ok(n as usize);
		}
		Ok(self.sort_state(guard)?.banks[0].len())
	}
	pub(crate) fn sort_load(&self, guard: Value, bank: usize, index: usize) -> Result<usize> {
		self.sort_state(guard)?
			.banks
			.get(bank)
			.and_then(|b| b.get(index))
			.copied()
			.ok_or_else(|| err(ErrorKind::InvalidCode))
	}
	pub(crate) fn sort_value(&mut self, guard: Value, index: usize) -> Result<Value> {
		let value = *self
			.sort_state(guard)?
			.values
			.get(index)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		self.retain(value)?;
		Ok(value)
	}
	pub(crate) fn sort_store(
		&mut self,
		guard: Value,
		bank: usize,
		index: usize,
		element: usize,
	) -> Result<()> {
		if element >= self.sort_len(guard)? {
			return Err(err(ErrorKind::InvalidCode));
		}
		let Value::Heap(h) = guard else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::SortGuard(state) = self.mutable(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		*state
			.banks
			.get_mut(bank)
			.and_then(|b| b.get_mut(index))
			.ok_or_else(|| err(ErrorKind::InvalidCode))? = element;
		Ok(())
	}
	/// Validate a complete permutation before the first parent write. Use the
	/// other bank as its inverse; swaps then move ownership without edge changes.
	pub(crate) fn sort_commit(&mut self, guard: Value, bank: usize) -> Result<()> {
		if matches!(guard, Value::I64(0..=1)) {
			return if bank <= 1 {
				Ok(())
			} else {
				Err(err(ErrorKind::InvalidCode))
			};
		}
		self.sort_state(guard)?;
		if bank > 1 {
			return Err(err(ErrorKind::InvalidCode));
		}
		let Value::Heap(h) = guard else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let len = self.sort_len(guard)?;
		let Payload::SortGuard(state) = self.mutable(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		state.banks[1 - bank].fill(usize::MAX);
		for dst in 0..len {
			let src = state.banks[bank][dst];
			if src >= len || state.banks[1 - bank][src] != usize::MAX {
				return Err(err(ErrorKind::InvalidCode));
			}
			state.banks[1 - bank][src] = dst;
		}
		if state.values.len() != len {
			return Err(err(ErrorKind::InvalidCode));
		}
		// Permutation is complete before any write. No allocation or user code.
		for dst in 0..len {
			loop {
				let next = state.banks[1 - bank][dst];
				if next == dst {
					break;
				}
				state.values.swap(dst, next);
				state.banks[1 - bank].swap(dst, next);
			}
		}
		// Detach the buffer before reborrowing the parent. These operations cannot fail.
		let parent = state.parent;
		let values = core::mem::take(&mut state.values);
		state.active = false;
		let entry = self
			.entry_mut(parent)
			.expect("leased parent retained by guard");
		entry.payload = Some(Payload::List {
			tag: Type::Vec,
			values,
		});
		entry.readers = 0;
		Ok(())
	}
	// Called before enqueueing the guard's retained parent edge, even on abort.
	pub(super) fn sort_restore(&mut self, state: &mut SortState) -> Result<()> {
		let entry = self.entry_mut(state.parent)?;
		if entry.readers != EXCLUSIVE || !matches!(entry.payload, Some(Payload::Moved(Type::Vec))) {
			return Err(err(ErrorKind::InvalidCode));
		}
		entry.payload = Some(Payload::List {
			tag: Type::Vec,
			values: core::mem::take(&mut state.values),
		});
		entry.readers = 0;
		state.active = false;
		Ok(())
	}
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SortState>() == 88);
