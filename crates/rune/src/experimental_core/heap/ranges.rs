//! Canonical range endpoints, snapshot iterators and transactional shallow slices.
use super::{err, Payload, Store};
use crate::alloc::{Box, String, Vec};
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
		let (start, end, inclusive) = match self.payload(range)? {
			Payload::OtherRange(shape) => (
				shape.start,
				shape.end,
				matches!(shape.tag, Type::RangeInclusive | Type::RangeToInclusive),
			),
			payload => {
				let (start, end) = payload.range_values()?;
				(Some(start), Some(end), false)
			}
		};
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
		let start = start.map(&bound).transpose()?.unwrap_or(0);
		let len = match payload {
			Payload::List {
				tag: Type::Vec,
				values,
			} => values.len(),
			Payload::String(text) => text.len(),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		let end = match end {
			Some(value) => {
				let end = bound(value)?;
				if inclusive {
					end.checked_add(1).ok_or_else(|| match payload {
						Payload::List { tag: Type::Vec, .. } => err(ErrorKind::IndexOutOfBounds)
							.with_detail(Detail::MissingIndex {
								target: TypeName::Vec,
							}),
						Payload::String(_) => {
							err(ErrorKind::Panic).with_detail(Detail::MissingStringSlice)
						}
						_ => err(ErrorKind::InvalidCode),
					})?
				} else {
					end
				}
			}
			None => len,
		};
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
		#[cfg(feature = "native-code")]
		assert_eq!(s.range_iterator_for_native(it), None);
		s.set_field(r, "end", Value::I64(99)).unwrap();
		s.release(r).unwrap();
		assert!(matches!(s.iterator_next(it).unwrap(), Some(Value::I64(0))));
		assert!(matches!(s.iterator_next(it).unwrap(), Some(Value::I64(1))));
		assert!(s.iterator_next(it).unwrap().is_none());
		s.release(it).unwrap();
		let r = s.range(0, 2).unwrap();
		let it = s.iterator(r).unwrap();
		#[cfg(feature = "native-code")]
		assert_eq!(s.range_iterator_for_native(it), Some((0, 2)));
		s.release(it).unwrap();
		s.release(r).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
}

/// Cold shapes own inert endpoint handles until Store::admit retains them.
#[derive(Clone, Copy, Debug)]
pub(super) struct RangeShape {
	pub(super) tag: Type,
	pub(super) start: Option<Value>,
	pub(super) end: Option<Value>,
}

/// Only validated inline scalars enter a cursor. An inclusive upper endpoint
/// has a separate exhausted bit, so yielding MAX never requires MAX + 1.
#[derive(Debug)]
pub(super) struct RangeCursor {
	pub(super) next: Value,
	pub(super) end: Option<Value>,
	pub(super) done: bool,
}

impl Store {
	#[cold]
	pub(crate) fn other_range(&mut self, tag: Type, args: &[Value]) -> Result<Value> {
		let (start, end) = match (tag, args) {
			(Type::RangeFrom, [start]) => (Some(*start), None),
			(Type::RangeFull, []) => (None, None),
			(Type::RangeInclusive, [start, end]) => (Some(*start), Some(*end)),
			(Type::RangeToInclusive | Type::RangeTo, [end]) => (None, Some(*end)),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		self.admit(Payload::OtherRange(
			Box::try_new(RangeShape { tag, start, end }).map_err(|_| err(ErrorKind::Allocation))?,
		))
	}

	#[cold]
	pub(super) fn other_range_iterator(&mut self, shape: RangeShape) -> Result<Value> {
		let next = shape.start.ok_or_else(|| err(ErrorKind::Type))?;
		let end = match shape.tag {
			Type::RangeInclusive => {
				let end = shape.end.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				match self.range_iteration_bounds(next, end) {
					Ok((_, end)) => Some(end),
					Err(mut error) => {
						if let Detail::UnsupportedIterRange { start, end } = error.detail {
							error.detail = Detail::UnsupportedIterRangeInclusive { start, end };
						}
						return Err(error);
					}
				}
			}
			Type::RangeFrom if matches!(next, Value::I64(_) | Value::U64(_) | Value::Char(_)) => {
				None
			}
			Type::RangeFrom => {
				return Err(
					err(ErrorKind::Type).with_detail(Detail::UnsupportedIterRangeFrom {
						start: self
							.diagnostic_type(next)
							.ok_or_else(|| err(ErrorKind::InvalidCode))?,
					}),
				)
			}
			_ => return Err(err(ErrorKind::Type)),
		};
		self.admit(Payload::OtherRangeIterator(
			Box::try_new(super::boxed_iterator::State {
				source: None,
				kind: super::boxed_iterator::Kind::Range(RangeCursor {
					next,
					end,
					done: false,
				}),
			})
			.map_err(|_| err(ErrorKind::Allocation))?,
		))
	}

	#[cold]
	pub(super) fn other_range_next(&mut self, handle: Handle) -> Result<Option<Value>> {
		if matches!(self.payload(handle)?, Payload::OtherRangeIterator(state) if matches!(state.kind,super::boxed_iterator::Kind::Chars(_)))
		{
			return self.chars_next(handle);
		}
		if matches!(self.payload(handle)?, Payload::OtherRangeIterator(state) if matches!(state.kind,super::boxed_iterator::Kind::Split(_)))
		{
			return self.split_next(handle);
		}
		let Payload::OtherRangeIterator(state) = self.mutable(handle)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let super::boxed_iterator::Kind::Range(cursor) = &mut state.kind else {
			return Err(err(ErrorKind::InvalidCode));
		};
		if cursor.done {
			return Ok(None);
		}
		let value = cursor.next;
		let (past, last) = match (value, cursor.end) {
			(Value::I64(a), Some(Value::I64(b))) => (a > b, a == b),
			(Value::U64(a), Some(Value::U64(b))) => (a > b, a == b),
			(Value::Char(a), Some(Value::Char(b))) => (a > b, a == b),
			(_, None) => (false, false),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		if past {
			cursor.done = true;
			return Ok(None);
		}
		if last {
			cursor.done = true;
			return Ok(Some(value));
		}
		let next = match value {
			Value::I64(n) => Value::I64(n.checked_add(1).ok_or_else(|| err(ErrorKind::Overflow))?),
			Value::U64(n) => Value::U64(n.checked_add(1).ok_or_else(|| err(ErrorKind::Overflow))?),
			Value::Char(c) => {
				let mut n = c as u32 + 1;
				if n == 0xd800 {
					n = 0xe000;
				}
				Value::Char(char::from_u32(n).ok_or_else(|| err(ErrorKind::Overflow))?)
			}
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		cursor.next = next;
		Ok(Some(value))
	}
}

impl RangeCursor {
	pub(super) fn diagnostic_type(&self) -> TypeName {
		match (self.end.is_some(), self.next) {
			(true, Value::I64(_)) => TypeName::RangeInclusiveIter,
			(true, Value::U64(_)) => TypeName::UnsignedRangeInclusiveIter,
			(true, Value::Char(_)) => TypeName::CharRangeInclusiveIter,
			(false, Value::I64(_)) => TypeName::RangeFromIter,
			(false, Value::U64(_)) => TypeName::UnsignedRangeFromIter,
			(false, Value::Char(_)) => TypeName::CharRangeFromIter,
			_ => unreachable!("validated range cursor"),
		}
	}
}

impl Store {
	#[cold]
	pub(crate) fn object_has_field(&self, object: Value, key: Value) -> Result<bool> {
		let Value::Heap(handle) = object else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let key = self.text(key)?;
		let Payload::Record {
			tag: Type::Object,
			fields,
		} = self.payload(handle)?
		else {
			return Err(err(ErrorKind::InvalidCode));
		};
		Ok(fields.iter().any(|(field, _)| field.as_str() == key))
	}
}

impl Store {
	#[cold]
	pub(super) fn other_range_field(&mut self, handle: Handle, key: &str) -> Result<Value> {
		let Payload::OtherRange(shape) = self.payload(handle)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let value = match key {
			"start" => shape.start,
			"end" => shape.end,
			_ => None,
		}
		.ok_or_else(|| err(ErrorKind::Type))?;
		self.retain(value)?;
		Ok(value)
	}

	#[cold]
	pub(super) fn set_other_range_field(
		&mut self,
		handle: Handle,
		key: &str,
		new: Value,
	) -> Result<()> {
		let Payload::OtherRange(shape) = self.mutable(handle)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let old = match key {
			"start" => shape.start,
			"end" => shape.end,
			_ => None,
		}
		.ok_or_else(|| err(ErrorKind::Type))?;
		let edges = self.replacement_edges(old, new)?;
		self.retain(new)?;
		let Payload::OtherRange(shape) = self.mutable(handle)? else {
			unreachable!()
		};
		match key {
			"start" => shape.start = Some(new),
			"end" => shape.end = Some(new),
			_ => unreachable!(),
		}
		self.edges = edges;
		self.release(old)
	}

	#[cfg(test)]
	pub(crate) fn host_observe_other_range(
		&self,
		value: Value,
	) -> Result<(Type, Option<Value>, Option<Value>)> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		let Payload::OtherRange(shape) = self.payload(handle)? else {
			return Err(err(ErrorKind::Type));
		};
		Ok((shape.tag, shape.start, shape.end))
	}
}

#[cfg(test)]
mod cold_range_tests {
	use super::super::Limits;
	use super::*;

	#[test]
	fn inclusive_max_is_yielded_once_then_exhausted() {
		let mut store = Store::new(Limits::default()).unwrap();
		for (start, end) in [
			(Value::I64(i64::MAX - 1), Value::I64(i64::MAX)),
			(Value::U64(u64::MAX - 1), Value::U64(u64::MAX)),
			(Value::Char('\u{d7ff}'), Value::Char('\u{e000}')),
		] {
			let range = store
				.other_range(Type::RangeInclusive, &[start, end])
				.unwrap();
			let iter = store.iterator(range).unwrap();
			store.release(range).unwrap();
			assert_eq!(store.iterator_next(iter).unwrap(), Some(start));
			assert_eq!(store.iterator_next(iter).unwrap(), Some(end));
			assert_eq!(store.iterator_next(iter).unwrap(), None);
			assert_eq!(store.iterator_next(iter).unwrap(), None);
			store.release(iter).unwrap();
		}
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}

	#[test]
	fn cold_shape_retains_aliases_and_replacement_edges() {
		let mut store = Store::new(Limits::default()).unwrap();
		let child = store.list(Type::Vec, &[]).unwrap();
		let range = store
			.other_range(Type::RangeInclusive, &[child, child])
			.unwrap();
		store.release(child).unwrap();
		let Value::Heap(handle) = range else {
			unreachable!()
		};
		store
			.set_other_range_field(handle, "start", Value::I64(1))
			.unwrap();
		assert_eq!(store.other_range_field(handle, "end").unwrap(), child);
		store.release(child).unwrap();
		store.release(range).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}

	#[test]
	fn rejected_shape_admission_does_not_take_endpoint_ownership() {
		let mut store = Store::new(Limits {
			entries: 1,
			bytes: 4096,
		})
		.unwrap();
		let child = store.list(Type::Vec, &[]).unwrap();
		assert!(store.other_range(Type::RangeFrom, &[child]).is_err());
		assert_eq!(store.len(child).unwrap(), 0);
		store.release(child).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}
