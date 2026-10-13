//! B1: the native tier's read of a RangeIterator at region entry and validation, and its
//! one Store mutation, the commit of `next`. Safe Rust (the fresh core is
//! `forbid(unsafe_code)`).
//!
//! Every check of the setter runs before its single write, on the same exclusive borrow
//! of the store: a refusal mutates nothing.
use super::{Payload, Store};
use crate::experimental_core::value::Value;

/// The setter refused; nothing was mutated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RangeCommitRefused;

/// `new` is a value of `next` that `iterator_next` could reach from `(from, end)`.
/// Pure comparisons: no arithmetic, so no overflow at any i64 boundary.
pub(crate) fn range_next_reachable((from, end): (i64, i64), new: i64) -> bool {
	if from >= end {
		new == from
	} else {
		from <= new && new <= end
	}
}

impl Store {
	/// B1 entry and validation read. `Some((next, end))` iff `value` is a handle to a
	/// live entry of this open store, of the handle's generation, whose payload is
	/// present, is a `RangeIterator`, and whose `readers` is 0, so `mutable` would succeed.
	/// Never mutates (`&self`).
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

#[cfg(test)]
mod tests;
