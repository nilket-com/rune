//! Cold text cursors. Adapter traversal is emitted as charged instructions.
use super::boxed_iterator::{Kind, State};
use super::*;
use crate::experimental_core::error::{Detail, HostCause, IntTarget, IntValue};
#[derive(Debug)]
pub(super) struct Chars {
	pub(super) offset: usize,
}
#[derive(Debug)]
pub(super) struct Adapter {
	pub(super) remaining: usize,
}
impl Store {
	#[cold]
	#[inline(never)]
	fn pipeline_admit(
		&mut self,
		source: Value,
		kind: Kind,
		extra: usize,
		lease: bool,
	) -> Result<Value> {
		let Value::Heap(h) = source else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let readers = if lease {
			Some(
				self.entry(h)?
					.readers
					.checked_add(1)
					.filter(|n| *n != sorting::EXCLUSIVE)
					.ok_or_else(|| err(ErrorKind::Overflow))?,
			)
		} else {
			None
		};
		let charge = size_of::<State>()
			.checked_add(extra)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if charge > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let node = Box::try_new(State {
			source: Some(h),
			kind,
		})
		.map_err(crate::alloc::Error::from)?;
		let result = self.admit(Payload::OtherRangeIterator(node))?;
		let Value::Heap(iter) = result else {
			return Err(err(ErrorKind::InvalidCode));
		};
		self.bytes = self
			.bytes
			.checked_add(extra)
			.filter(|n| *n <= self.limits.bytes)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		self.entry_mut(iter)?.set_charged_bytes(charge);
		if let Some(n) = readers {
			self.entry_mut(h)?.readers = n;
		}
		Ok(result)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_chars(&mut self, source: Value) -> Result<Value> {
		if self.tag(source)? != Type::String {
			return Err(self.text_missing(
				source,
				crate::function::fresh_pipeline::Method::Chars.hash(),
			)?);
		}
		self.text(source)?;
		self.pipeline_admit(
			source,
			Kind::Chars(Box::try_new(Chars { offset: 0 }).map_err(crate::alloc::Error::from)?),
			size_of::<Chars>(),
			true,
		)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_adapter(
		&mut self,
		source: Value,
		n: Value,
		take: bool,
	) -> Result<Value> {
		let n = match n {
			Value::I64(n) => usize::try_from(n).map_err(|_| {
				err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
					cause: HostCause::IntegerCoercion,
					arg: 1,
					value: IntValue {
						negative: n < 0,
						magnitude: n.unsigned_abs(),
					},
					to: IntTarget::Usize,
				})
			})?,
			Value::U64(n) => usize::try_from(n).map_err(|_| {
				err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
					cause: HostCause::IntegerCoercion,
					arg: 1,
					value: IntValue {
						negative: false,
						magnitude: n,
					},
					to: IntTarget::Usize,
				})
			})?,
			_ => {
				return Err(
					err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
						cause: HostCause::ExpectedNumber,
						arg: 1,
						found: self
							.diagnostic_type(n)
							.ok_or_else(|| err(ErrorKind::InvalidCode))?,
					}),
				)
			}
		};
		let Value::Heap(h) = source else {
			return Err(err(ErrorKind::InvalidCode));
		};
		if !matches!(self.payload(h)?,Payload::OtherRangeIterator(s) if s.is_split()) {
			return Err(err(ErrorKind::InvalidCode));
		}
		let state = Box::try_new(Adapter { remaining: n }).map_err(crate::alloc::Error::from)?;
		self.pipeline_admit(
			source,
			if take {
				Kind::Take(state)
			} else {
				Kind::Skip(state)
			},
			size_of::<Adapter>(),
			false,
		)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_child(&mut self, value: Value) -> Result<Value> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::OtherRangeIterator(state) = self.payload(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		if !matches!(state.kind, Kind::Skip(_) | Kind::Take(_)) {
			return Err(err(ErrorKind::InvalidCode));
		}
		let v = Value::Heap(state.source.ok_or_else(|| err(ErrorKind::InvalidCode))?);
		self.retain(v)?;
		Ok(v)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_remaining(&mut self, value: Value, take: bool) -> Result<Value> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::OtherRangeIterator(state) = self.mutable(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		match &mut state.kind {
			Kind::Skip(a) if !take => {
				let n = a.remaining;
				a.remaining = 0;
				Ok(Value::U64(n as u64))
			}
			Kind::Take(a) if take => {
				let yes = a.remaining > 0;
				if yes {
					a.remaining -= 1;
				}
				Ok(Value::Bool(yes))
			}
			_ => Err(err(ErrorKind::InvalidCode)),
		}
	}
	#[cold]
	#[inline(never)]
	pub(super) fn chars_next(&mut self, h: Handle) -> Result<Option<Value>> {
		let Payload::OtherRangeIterator(state) = self.payload(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Kind::Chars(chars) = &state.kind else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let source = state.source.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let c = self
			.text(Value::Heap(source))?
			.get(chars.offset..)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?
			.chars()
			.next();
		if let Some(c) = c {
			let Payload::OtherRangeIterator(state) = self.mutable(h)? else {
				return Err(err(ErrorKind::InvalidCode));
			};
			let Kind::Chars(chars) = &mut state.kind else {
				return Err(err(ErrorKind::InvalidCode));
			};
			chars.offset = chars
				.offset
				.checked_add(c.len_utf8())
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			return Ok(Some(Value::Char(c)));
		}
		Ok(None)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_unwrap(&mut self, value: Value, default: Value) -> Result<Value> {
		let Value::Heap(h) = value else {
			return Err(self.text_missing(
				value,
				crate::function::fresh_pipeline::Method::Unwrap.hash(),
			)?);
		};
		let selected = match self.payload(h)? {
			Payload::Variant {
				tag: Type::Option | Type::Result,
				success,
				payload,
			} => {
				if *success {
					payload.ok_or_else(|| err(ErrorKind::InvalidCode))?
				} else {
					default
				}
			}
			_ => {
				return Err(self.text_missing(
					value,
					crate::function::fresh_pipeline::Method::Unwrap.hash(),
				)?)
			}
		};
		self.retain(selected)?;
		Ok(selected)
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn pipeline_append(&mut self, target: Value, element: Value) -> Result<Value> {
		match element {
			Value::Char(c) => {
				let mut buf = [0u8; 4];
				let text = c.encode_utf8(&mut buf);
				let Value::Heap(h) = target else {
					return Err(err(ErrorKind::InvalidCode));
				};
				let (len, cap) = match self.mutable(h)? {
					Payload::String(s) => (s.len(), s.capacity()),
					_ => return Err(err(ErrorKind::InvalidCode)),
				};
				let new_len = len
					.checked_add(text.len())
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				if new_len > cap {
					let available =
						cap.saturating_add(self.limits.bytes.saturating_sub(self.bytes));
					let capacity = new_len.max(cap.saturating_mul(2).min(available));
					if capacity > available {
						return Err(err(ErrorKind::StoreCapacity));
					}
					let mut replacement = String::try_with_capacity(capacity)?;
					replacement.try_push_str(self.text(target)?)?;
					replacement.try_push_str(text)?;
					let charge = replacement.capacity();
					let bytes = self.replacement_bytes(h, charge)?;
					let e = self.entry_mut(h)?;
					e.payload = Some(Payload::String(replacement));
					e.set_charged_bytes(charge);
					self.bytes = bytes;
				} else {
					let Payload::String(s) = self.mutable(h)? else {
						return Err(err(ErrorKind::InvalidCode));
					};
					s.try_push_str(text)?;
				}
			}
			_ => {
				self.append_string(target, element)?;
			}
		}
		Ok(Value::Unit)
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn pipeline_layout_is_cold() {
		assert_eq!(size_of::<State>(), 56);
		assert_eq!(size_of::<Payload>(), 40);
		assert_eq!(size_of::<Entry>(), 72);
		assert_eq!(size_of::<Slot>(), 80);
	}
}

#[cfg(test)]
mod ownership_tests {
	use super::*;
	#[test]
	fn nested_adapters_keep_one_source_lease_and_release_all_edges() {
		let mut store = Store::new(super::super::Limits::default()).unwrap();
		let source = store.string("a🦀b").unwrap();
		let chars = store.pipeline_chars(source).unwrap();
		let skip = store.pipeline_adapter(chars, Value::I64(1), false).unwrap();
		let take = store.pipeline_adapter(skip, Value::I64(0), true).unwrap();
		assert_eq!(store.shared_readers(source), Some(1));
		assert_eq!(
			store.pipeline_remaining(take, true).unwrap(),
			Value::Bool(false)
		);
		let child = store.pipeline_child(take).unwrap();
		assert_eq!(child, skip);
		store.release(child).unwrap();
		assert_eq!(
			store.pipeline_remaining(skip, false).unwrap(),
			Value::U64(1)
		);
		assert_eq!(
			store.pipeline_remaining(skip, false).unwrap(),
			Value::U64(0)
		);
		assert_eq!(store.iterator_next(chars).unwrap(), Some(Value::Char('a')));
		assert_eq!(store.iterator_next(chars).unwrap(), Some(Value::Char('🦀')));
		assert_eq!(store.iterator_next(chars).unwrap(), Some(Value::Char('b')));
		assert_eq!(store.iterator_next(chars).unwrap(), None);
		assert_eq!(store.shared_readers(source), Some(1));
		assert!(store.push(source, Value::Char('x')).is_err());
		store.release(chars).unwrap();
		store.release(skip).unwrap();
		assert_eq!(store.shared_readers(source), Some(1));
		store.release(take).unwrap();
		assert_eq!(store.shared_readers(source), Some(0));
		store.push(source, Value::Char('x')).unwrap();
		store.release(source).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
	#[test]
	fn rejected_admissions_and_argument_conversion_leave_no_lease() {
		let mut store = Store::new(super::super::Limits {
			entries: 1,
			bytes: 1024,
		})
		.unwrap();
		let source = store.string("a").unwrap();
		let bytes = store.payload_bytes();
		assert!(store.pipeline_chars(source).is_err());
		assert_eq!(store.shared_readers(source), Some(0));
		assert_eq!(store.live_entries(), 1);
		assert_eq!(store.payload_bytes(), bytes);
		store.release(source).unwrap();
		let mut store = Store::new(super::super::Limits::default()).unwrap();
		let source = store.string("a").unwrap();
		let chars = store.pipeline_chars(source).unwrap();
		let bytes = store.payload_bytes();
		assert!(store
			.pipeline_adapter(chars, Value::I64(-1), false)
			.is_err());
		assert_eq!(store.shared_readers(source), Some(1));
		assert_eq!(store.payload_bytes(), bytes);
		store.release(chars).unwrap();
		store.release(source).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}
