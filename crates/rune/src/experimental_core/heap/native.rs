//! Checked native-tier snapshots and commits. B1 snapshots a RangeIterator's next/end;
//! N2 copies a homogeneous scalar Vec into a private budgeted i64 buffer. Generated
//! code never receives any Store, Value or payload pointer. Safe Rust throughout
//! (the execution core is `forbid(unsafe_code)`).
//!
//! Every check of the setter runs before its single write, on the same exclusive borrow
//! of the store: a refusal mutates nothing.

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

use super::{Payload, Store};
use crate::experimental_core::{
	region::{
		budget::{BVec, Budget},
		Scalar,
	},
	value::{Type, Value},
};

impl Store {
	/// Available configured heap room also bounds temporary native copies. The native
	/// transient Budget charges the actual scratch capacity independently of heap bytes.
	pub(crate) fn native_snapshot_room(&self) -> usize {
		self.limits.bytes.saturating_sub(self.bytes)
	}

	fn scalar_collection(&self, value: Value, ty: Scalar) -> Option<&[Value]> {
		let Value::Heap(handle) = value else {
			return None;
		};
		let entry = self.entry(handle).ok()?;
		if entry.readers != 0 {
			return None;
		}
		let Payload::List {
			tag: Type::Vec,
			values,
		} = entry.payload.as_ref()?
		else {
			return None;
		};
		if !values.iter().all(|value| {
			matches!(
				(ty, value),
				(Scalar::I64, Value::I64(_)) | (Scalar::Bool, Value::Bool(_))
			)
		}) {
			return None;
		}
		Some(values)
	}

	/// No payload pointer leaves this safe module. One initialized sentinel keeps even
	/// an empty snapshot's clamped address inside a live allocation.
	pub(crate) fn snapshot_for_native<'b>(
		&self,
		value: Value,
		ty: Scalar,
		budget: &'b Budget,
	) -> Option<(BVec<'b, i64>, usize)> {
		let values = self.scalar_collection(value, ty)?;
		let mut snapshot = BVec::with_capacity(budget, values.len().max(1)).ok()?;
		snapshot.resize(values.len().max(1), 0).ok()?;
		for (out, value) in snapshot.iter_mut().zip(values.iter()) {
			*out = match *value {
				Value::I64(v) => v,
				Value::Bool(v) => i64::from(v),
				_ => return None,
			};
		}
		Some((snapshot, values.len()))
	}

	/// Validate BOTH payloads before either write. No allocation, releases, callbacks or
	/// fallible operations follow the first mutation. The two handles cannot alias:
	/// one has a List payload and the other a RangeIterator payload.
	pub(crate) fn commit_snapshot_for_native(
		&mut self,
		value: Value,
		ty: Scalar,
		snapshot: &[i64],
		iter: Option<(Value, (i64, i64), i64)>,
	) -> bool {
		if !self.scalar_release_is_noop()
			|| self
				.scalar_collection(value, ty)
				.is_none_or(|v| v.len() != snapshot.len())
			|| (ty == Scalar::Bool && snapshot.iter().any(|v| !matches!(v, 0 | 1)))
		{
			return false;
		}
		if let Some((value, expect, new)) = iter {
			if self.range_iterator_for_native(value) != Some(expect)
				|| !range_next_reachable(expect, new)
			{
				return false;
			}
		}
		if let Some((Value::Heap(h), _, new)) = iter {
			let Some(Payload::RangeIterator { next, .. }) = self.slots[h.slot as usize]
				.entry
				.as_mut()
				.and_then(|e| e.payload.as_mut())
			else {
				unreachable!()
			};
			*next = new;
		}
		let Value::Heap(h) = value else {
			unreachable!()
		};
		let Some(Payload::List { values, .. }) = self.slots[h.slot as usize]
			.entry
			.as_mut()
			.and_then(|e| e.payload.as_mut())
		else {
			unreachable!()
		};
		for (out, &value) in values.iter_mut().zip(snapshot) {
			*out = match ty {
				Scalar::Bool => Value::Bool(value != 0),
				Scalar::I64 => Value::I64(value),
				Scalar::Unit => unreachable!(),
			};
		}
		true
	}
}

#[cfg(test)]
mod tests;
