//! Canonical range endpoints, snapshot iterators and transactional shallow slices.
use super::{err, Payload, Store};
use crate::alloc::{String, Vec};
use crate::experimental_core::{
	error::{Detail, ErrorKind, IntTarget, IntValue, Result, TypeName},
	value::{Handle, Type, Value},
};
use core::mem::size_of;

impl Payload {
	#[cfg(debug_assertions)]
	#[inline]
	pub(super) fn assert_canonical_range(&self) {
		if let Self::ValueRange { start, end } = self {
			debug_assert!(!matches!((start, end), (Value::I64(_), Value::I64(_))));
		}
	}

	pub(super) fn range_values(&self) -> Result<(Value, Value)> {
		match self {
			Self::Range { start, end } => Ok((Value::I64(*start), Value::I64(*end))),
			Self::ValueRange { start, end } => {
				debug_assert!(!matches!((start, end), (Value::I64(_), Value::I64(_))));
				Ok((*start, *end))
			}
			_ => Err(err(ErrorKind::InvalidCode)),
		}
	}
	fn canonical_range(start: Value, end: Value) -> Self {
		match (start, end) {
			(Value::I64(start), Value::I64(end)) => Self::Range { start, end },
			(start, end) => Self::ValueRange { start, end },
		}
	}
}

impl Store {
	pub(crate) fn range_values(&mut self, start: Value, end: Value) -> Result<Value> {
		match (start, end) {
			(Value::I64(start), Value::I64(end)) => self.range(start, end),
			(start, end) => self.admit(Payload::ValueRange { start, end }),
		}
	}

	pub(super) fn range_field(&mut self, handle: Handle, key: &str) -> Result<Value> {
		if !matches!(key, "start" | "end") {
			return Err(err(ErrorKind::Type));
		}
		let (start, end) = self.payload(handle)?.range_values()?;
		let value = if key == "start" { start } else { end };
		self.retain(value)?;
		Ok(value)
	}

	pub(super) fn set_range_field(&mut self, handle: Handle, key: &str, new: Value) -> Result<()> {
		if !matches!(key, "start" | "end") {
			return Err(err(ErrorKind::Type));
		}
		let (start, end) = self.mutable(handle)?.range_values()?;
		let old = if key == "start" { start } else { end };
		let payload = if key == "start" {
			Payload::canonical_range(new, end)
		} else {
			Payload::canonical_range(start, new)
		};
		let charge = payload.bytes()?;
		let bytes = self.replacement_bytes(handle, charge)?;
		let edges = self.replacement_edges(old, new)?;
		self.retain(new)?;
		// Admission checks are complete. No callbacks, allocation, or competing
		// store mutation can invalidate this entry between retain and publication.
		*self.mutable(handle)? = payload;
		self.entry_mut(handle)?.set_charged_bytes(charge);
		self.bytes = bytes;
		self.edges = edges;
		self.assert_work();
		self.release(old)
	}

	pub(super) fn range_index(&mut self, value: Value, index: Value) -> Result<Value> {
		let (Value::Heap(target), Value::Heap(range)) = (value, index) else {
			return Err(err(ErrorKind::InvalidCode));
		};
		// Stock's protocol borrows the receiver before borrowing/converting bounds.
		let payload = self.payload(target)?;
		let (start, end) = self.payload(range)?.range_values()?;
		let bound = |value| match super::super::execute::index_value(value) {
			Err(error) if error.detail == Detail::None => {
				Err(error.with_detail(Detail::ExpectedNumber {
					found: self
						.diagnostic_type(value)
						.ok_or_else(|| err(ErrorKind::InvalidCode))?,
				}))
			}
			result => result,
		};
		let start = bound(start)?;
		let end = bound(end)?;
		let copy = match payload {
			Payload::List {
				tag: Type::Vec,
				values,
			} => {
				let slice = values.get(start..end).ok_or_else(|| {
					err(ErrorKind::IndexOutOfBounds).with_detail(Detail::MissingIndex {
						target: TypeName::Vec,
					})
				})?;
				let bytes = slice
					.len()
					.checked_mul(size_of::<Value>())
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				if bytes > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				// The unpublished vector contains inert handles. admit retains them
				// transactionally, including repeated children and rollback failures.
				Payload::List {
					tag: Type::Vec,
					values: {
						let mut copy = Vec::try_with_capacity(slice.len())?;
						for value in slice {
							copy.try_push(*value)?;
						}
						copy
					},
				}
			}
			Payload::String(text) => {
				let slice = text
					.get(start..end)
					.ok_or_else(|| err(ErrorKind::Panic).with_detail(Detail::MissingStringSlice))?;
				if slice.len() > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				Payload::String(String::try_from(slice)?)
			}
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		self.admit(copy)
	}

	pub(super) fn range_iteration_bounds(
		&self,
		start: Value,
		end: Value,
	) -> Result<(Value, Value)> {
		let coerce = |to| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce {
				value: match IntValue::of(end) {
					Some(value) => value,
					None => return err(ErrorKind::InvalidCode),
				},
				to,
			})
		};
		let expected_number = || -> Result<_> {
			Ok(err(ErrorKind::Type).with_detail(Detail::ExpectedNumber {
				found: self
					.diagnostic_type(end)
					.ok_or_else(|| err(ErrorKind::InvalidCode))?,
			}))
		};
		let end = match (start, end) {
			(Value::I64(_), Value::I64(_))
			| (Value::U64(_), Value::U64(_))
			| (Value::Char(_), Value::Char(_)) => end,
			(Value::I64(_), Value::U64(n)) => {
				Value::I64(i64::try_from(n).map_err(|_| coerce(IntTarget::I64))?)
			}
			(Value::U64(_), Value::I64(n)) => {
				Value::U64(u64::try_from(n).map_err(|_| coerce(IntTarget::U64))?)
			}
			// Stock accepts inline integer starts with any inline end and then
			// asks for a number; a heap end takes UnsupportedIterRange instead.
			(
				Value::I64(_) | Value::U64(_),
				Value::Unit | Value::Bool(_) | Value::Char(_) | Value::F64(_) | Value::Type(_),
			) => return Err(expected_number()?),
			_ => {
				return Err(
					err(ErrorKind::Type).with_detail(Detail::UnsupportedIterRange {
						start: self
							.diagnostic_type(start)
							.ok_or_else(|| err(ErrorKind::InvalidCode))?,
						end: self
							.diagnostic_type(end)
							.ok_or_else(|| err(ErrorKind::InvalidCode))?,
					}),
				)
			}
		};
		Ok((start, end))
	}

	pub(super) fn value_range_next(
		&mut self,
		handle: Handle,
		next: Value,
		end: Value,
	) -> Result<Option<Value>> {
		let advanced = match (next, end) {
			(Value::I64(a), Value::I64(b)) if a < b => {
				Value::I64(a.checked_add(1).ok_or_else(|| err(ErrorKind::Overflow))?)
			}
			(Value::U64(a), Value::U64(b)) if a < b => {
				Value::U64(a.checked_add(1).ok_or_else(|| err(ErrorKind::Overflow))?)
			}
			(Value::Char(a), Value::Char(b)) if a < b => {
				let code = a as u32 + 1;
				Value::Char(
					char::from_u32(if code == 0xd800 { 0xe000 } else { code })
						.ok_or_else(|| err(ErrorKind::InvalidCode))?,
				)
			}
			(Value::I64(_), Value::I64(_))
			| (Value::U64(_), Value::U64(_))
			| (Value::Char(_), Value::Char(_)) => return Ok(None),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		let Payload::ValueRangeIterator { next: state, .. } = self.mutable(handle)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		*state = advanced;
		Ok(Some(next))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::experimental_core::heap::Limits;
	use rune_alloc::limit;

	fn store(bytes: usize) -> Store {
		Store::new(Limits { entries: 16, bytes }).unwrap()
	}

	#[test]
	fn endpoint_mutation_canonicalizes_and_balances_children_and_cycles() {
		let mut s = store(4096);
		let r = s.range(0, 3).unwrap();
		let text = s.string("held").unwrap();
		let initial = s.payload_bytes();
		s.set_field(r, "start", text).unwrap();
		assert_eq!(s.payload_bytes(), initial + 16);
		s.release(text).unwrap();
		let held = s.field(r, "start").unwrap();
		assert_eq!(s.text(held).unwrap(), "held");
		s.release(held).unwrap();
		s.set_field(r, "start", Value::I64(1)).unwrap();
		assert_eq!(s.payload_bytes(), 16);
		let Value::Heap(h) = r else { unreachable!() };
		assert!(matches!(
			s.payload(h).unwrap(),
			Payload::Range { start: 1, end: 3 }
		));
		s.set_field(r, "end", r).unwrap();
		s.release(r).unwrap();
		assert_eq!(s.live_entries(), 1);
		s.close().unwrap();
		assert_eq!(s.live_entries(), 0);
		assert_eq!(s.payload_bytes(), 0);
	}

	#[test]
	fn failed_endpoint_growth_leaves_the_original_range_intact() {
		let mut s = store(16);
		let r = s.range(0, 3).unwrap();
		assert_eq!(
			s.set_field(r, "start", Value::Unit).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert!(matches!(s.field(r, "start").unwrap(), Value::I64(0)));
		assert_eq!(s.payload_bytes(), 16);
		s.release(r).unwrap();
		assert_eq!(s.live_entries(), 0);
	}

	#[test]
	fn endpoint_and_slice_retain_overflow_roll_back_the_acquired_prefix() {
		let mut s = store(4096);
		let child = s.string("held").unwrap();
		let source = s.list(Type::Vec, &[child, child]).unwrap();
		let r = s.range(0, 2).unwrap();
		let original_strong = s.test_strong(child);
		let before = (s.live_entries(), s.payload_bytes(), s.edges);
		// The first duplicate retain succeeds; the second must undo it.
		s.test_set_strong(child, u32::MAX - 1);
		assert_eq!(
			s.range_values(child, child).unwrap_err().kind,
			ErrorKind::Overflow
		);
		assert_eq!(s.test_strong(child), u32::MAX - 1);
		assert_eq!(
			s.dynamic_get(source, r).unwrap_err().kind,
			ErrorKind::Overflow
		);
		assert_eq!(s.test_strong(child), u32::MAX - 1);
		assert_eq!((s.live_entries(), s.payload_bytes(), s.edges), before);
		s.test_set_strong(child, u32::MAX);
		assert_eq!(
			s.set_field(r, "start", child).unwrap_err().kind,
			ErrorKind::Overflow
		);
		assert!(matches!(s.field(r, "start").unwrap(), Value::I64(0)));
		assert_eq!((s.live_entries(), s.payload_bytes(), s.edges), before);
		s.test_set_strong(child, original_strong);
		s.release(source).unwrap();
		s.release(child).unwrap();
		s.release(r).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes(), s.edges), (0, 0, 0));
	}

	#[test]
	fn slice_allocation_refusal_preserves_sources_and_duplicate_children() {
		let mut s = store(4096);
		let child = s.string("child").unwrap();
		let source = s.list(Type::Vec, &[child, child]).unwrap();
		let r = s.range(0, 2).unwrap();
		let before = (s.live_entries(), s.payload_bytes(), s.edges);
		let error = limit::with(0, || s.dynamic_get(source, r).unwrap_err()).call();
		assert_eq!(error.kind, ErrorKind::Allocation);
		assert_eq!((s.live_entries(), s.payload_bytes(), s.edges), before);
		let copy = s.dynamic_get(source, r).unwrap();
		s.release(source).unwrap();
		s.release(child).unwrap();
		let value = s.dynamic_get(copy, Value::I64(1)).unwrap();
		assert_eq!(s.text(value).unwrap(), "child");
		s.release(value).unwrap();
		s.release(copy).unwrap();
		s.release(r).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes(), s.edges), (0, 0, 0));
	}

	#[test]
	fn slice_budget_refusal_and_utf8_error_publish_nothing() {
		let mut s = store(18);
		let text = s.string("é").unwrap();
		let r = s.range(0, 2).unwrap();
		assert_eq!(
			s.dynamic_get(text, r).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!((s.live_entries(), s.payload_bytes()), (2, 18));
		s.set_field(r, "end", Value::I64(1)).unwrap();
		let error = s.dynamic_get(text, r).unwrap_err();
		assert_eq!(error.kind, ErrorKind::Panic);
		assert_eq!(error.detail, Detail::MissingStringSlice);
		s.release(r).unwrap();
		s.release(text).unwrap();
		assert_eq!(s.live_entries(), 0);
	}

	#[test]
	fn mixed_iterator_snapshots_endpoints_and_stays_out_of_native_tier() {
		let mut s = store(4096);
		let r = s.range_values(Value::I64(0), Value::U64(2)).unwrap();
		let it = s.iterator(r).unwrap();
		#[cfg(feature = "fresh-core-native")]
		assert_eq!(s.range_iterator_for_native(it), None);
		s.set_field(r, "end", Value::I64(99)).unwrap();
		s.release(r).unwrap();
		assert!(matches!(s.iterator_next(it).unwrap(), Some(Value::I64(0))));
		assert!(matches!(s.iterator_next(it).unwrap(), Some(Value::I64(1))));
		assert!(s.iterator_next(it).unwrap().is_none());
		s.release(it).unwrap();
		let r = s.range(0, 2).unwrap();
		let it = s.iterator(r).unwrap();
		#[cfg(feature = "fresh-core-native")]
		assert_eq!(s.range_iterator_for_native(it), Some((0, 2)));
		s.release(it).unwrap();
		s.release(r).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
}
