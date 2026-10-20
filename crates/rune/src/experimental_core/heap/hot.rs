//! Heap execution roots and their immediate checked accessors.
//! Kept together so their existing fast checks can share a compilation neighborhood.
#[cfg(feature = "fresh-core-native")]
use super::native::{range_next_reachable, RangeCommitRefused};
use super::*;

impl Store {
	pub(super) fn slot(&self, h: Handle) -> Result<&Slot> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}

		self.slots
			.get(h.slot as usize)
			.filter(|s| s.generation == h.generation && s.entry.is_some())
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}

	pub(super) fn entry(&self, h: Handle) -> Result<&Entry> {
		self.slot(h)?
			.entry
			.as_ref()
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}

	pub(super) fn entry_mut(&mut self, h: Handle) -> Result<&mut Entry> {
		self.slot(h)?;
		self.slots[h.slot as usize]
			.entry
			.as_mut()
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}

	pub(super) fn payload(&self, h: Handle) -> Result<&Payload> {
		match self.entry(h)?.payload.as_ref() {
			Some(Payload::Moved(_)) => {
				Err(sorting::conflict(sorting::Access::Read, self.entry(h)?))
			}
			Some(payload) => {
				#[cfg(debug_assertions)]
				payload.assert_canonical_range();
				Ok(payload)
			}
			None => Err(err(ErrorKind::InvalidCode)),
		}
	}

	pub(super) fn mutable(&mut self, h: Handle) -> Result<&mut Payload> {
		let e = self.entry_mut(h)?;
		if e.readers != 0 {
			return Err(sorting::conflict(sorting::Access::Write, e));
		}
		match e.payload.as_mut() {
			Some(Payload::Moved(_)) => Err(err(ErrorKind::AccessConflict).with_detail(
				crate::experimental_core::error::Detail::Moved {
					access: crate::experimental_core::error::MovedAccess::Write,
				},
			)),
			Some(payload) => {
				#[cfg(debug_assertions)]
				payload.assert_canonical_range();
				Ok(payload)
			}
			None => Err(err(ErrorKind::InvalidCode)),
		}
	}

	pub(crate) fn retain(&mut self, value: Value) -> Result<()> {
		if let Value::Heap(h) = value {
			let e = self.entry_mut(h)?;
			e.strong = e
				.strong
				.checked_add(1)
				.ok_or_else(|| err(ErrorKind::Overflow))?;
		}

		Ok(())
	}

	pub(crate) fn iterator_next(&mut self, value: Value) -> Result<Option<Value>> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};

		// A failed advance must not allocate or consume any payload.
		let payload = self.mutable(h)?;
		if let Payload::RangeIterator { next, end } = payload {
			if *next >= *end {
				return Ok(None);
			}

			let value = *next;
			let new = value
				.checked_add(1)
				.ok_or_else(|| err(ErrorKind::Overflow))?;
			*next = new;
			return Ok(Some(Value::I64(value)));
		}

		if let Payload::VectorIterator(iter) = self.payload(h)? {
			let root = Value::Heap(iter.handle);
			let index = iter.index;

			let length = match self.payload(iter.handle)? {
				Payload::Record {
					fields,
					tag: Type::Object,
				} => fields.len(),
				Payload::List { values, .. } => values.len(),
				_ => return Err(err(ErrorKind::InvalidCode)),
			};
			if index == length {
				return Ok(None);
			}

			let value = if let Payload::Record {
				fields,
				tag: Type::Object,
			} = self.payload(iter.handle)?
			{
				let (key, child) = fields
					.get(index)
					.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				let child = *child;
				if key.len() > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				let copy = String::try_from(key.as_str())?;
				let key = self.admit(Payload::String(copy))?;
				let pair = self.list(Type::Tuple, &[key, child]);
				let released = self.release(key);
				match pair {
					Ok(pair) => {
						if let Err(e) = released {
							let _ = self.release(pair);
							return Err(e);
						}
						pair
					}
					Err(e) => {
						released?;
						return Err(e);
					}
				}
			} else {
				self.get(root, index)?
			};
			let Payload::VectorIterator(iter) = self.mutable(h)? else {
				return Err(err(ErrorKind::Type));
			};

			iter.index += 1;
			return Ok(Some(value));
		}

		self.iterator_next_cold(h)
	}

	#[cold]
	#[inline(never)]
	fn iterator_next_cold(&mut self, h: Handle) -> Result<Option<Value>> {
		match self.payload(h)? {
			Payload::OtherRangeIterator(_) => self.other_range_next(h),
			Payload::ValueRangeIterator { next, end } => self.value_range_next(h, *next, *end),
			Payload::OptionIterator { payload } => {
				let value = *payload;
				let Payload::OptionIterator { payload } = self.mutable(h)? else {
					unreachable!()
				};
				*payload = None;
				// Transfer the snapshot's retained root to the caller, without an
				// extra retain or a lease on the mutable Option wrapper.
				self.edges = self
					.edges
					.checked_sub(value.map_or(0, Self::heap_edge))
					.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				self.assert_work();
				Ok(value)
			}
			_ => Err(err(ErrorKind::Type)),
		}
	}
}

#[cfg(feature = "fresh-core-native")]
impl Store {
	/// B1 entry and validation read. `Some((next, end))` iff `value` is a handle to a
	/// live entry of this open store, of the handle's generation, whose payload is
	/// present, is a `RangeIterator`, and whose `readers` is 0, so `mutable` would succeed.
	/// Never mutates (`&self`).
	#[inline]
	pub(crate) fn range_iterator_for_native(&self, value: Value) -> Option<(i64, i64)> {
		// Kind of value: a heap handle. Any scalar is not an iterator.
		let Value::Heap(h) = value else {
			return None;
		};

		// Open store, slot in range, same generation, entry present.
		let entry = self.entry(h).ok()?;

		// `mutable`'s lease condition.
		if entry.readers != 0 {
			return None;
		}

		// A taken payload is an inert Moved tombstone; only a live RangeIterator is accepted.
		match entry.payload.as_ref()? {
			Payload::Moved(_) => None,
			Payload::RangeIterator { next, end } => Some((*next, *end)),
			_ => None,
		}
	}

	/// B1 commit (M1): sets the RangeIterator's `next` to `new`, only if the entry still
	/// passes every check of `range_iterator_for_native` with exactly `expect`, and `new`
	/// is reachable from it. Every `Err` returns before the single write.
	#[inline]
	pub(crate) fn commit_range_next_for_native(
		&mut self,
		value: Value,
		expect: (i64, i64),
		new: i64,
	) -> core::result::Result<(), RangeCommitRefused> {
		// Re-check on a shared borrow; it ends at the end of this statement (the tuple is Copy).
		if self.range_iterator_for_native(value) != Some(expect) {
			return Err(RangeCommitRefused);
		}

		if !range_next_reachable(expect, new) {
			return Err(RangeCommitRefused);
		}

		let Value::Heap(h) = value else {
			return Err(RangeCommitRefused);
		};

		// `mutable` re-checks closed, generation, presence, readers and payload. It returns a
		// reference and mutates nothing on any path.
		let Ok(Payload::RangeIterator { next, .. }) = self.mutable(h) else {
			return Err(RangeCommitRefused);
		};

		// The single mutation.
		*next = new;
		Ok(())
	}
}
