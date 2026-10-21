//! Stock String/char Split state machine, borrowing its source and copying its pattern.
use super::super::error::{Detail, HostCause, TypeName};
use super::*;
use crate::function::fresh_text::Method;
#[derive(Debug)]
pub(super) enum Pattern {
	String(String),
	Char(char),
}
#[derive(Debug)]
pub(super) struct Split {
	pub pattern: Pattern,
	pub from: usize,
	pub to: usize,
}
impl Split {
	pub fn bytes(&self) -> Result<usize> {
		size_of::<Self>()
			.checked_add(match &self.pattern {
				Pattern::String(s) => s.capacity(),
				_ => 0,
			})
			.ok_or_else(|| err(ErrorKind::Overflow))
	}
	pub fn kind(&self) -> TypeName {
		match self.pattern {
			Pattern::String(_) => TypeName::StringSplit,
			Pattern::Char(_) => TypeName::CharSplit,
		}
	}
}
impl Store {
	fn text_argument(&self, value: Value, arg: u8) -> Result<&str> {
		self.text(value).map_err(|e| {
			if matches!(
				e.detail,
				Detail::Moved {
					access: crate::experimental_core::error::MovedAccess::Read
				}
			) {
				e.with_detail(Detail::CmpArgumentAccess {
					arg,
					access: crate::experimental_core::error::MovedAccess::Read,
				})
			} else {
				e
			}
		})
	}
	#[cold]
	#[inline(never)]
	pub(crate) fn text_method(&mut self, m: Method, args: &[Value]) -> Result<Value> {
		let receiver = args[0];
		if self.tag(receiver)? != Type::String {
			return Err(self.text_missing(receiver, m.hash())?);
		}
		self.text_argument(receiver, 0)?; // Native borrows the receiver before converting explicit arguments.
		for (i, &v) in args.iter().enumerate().skip(1) {
			if m == Method::Split && matches!(v, Value::Char(_)) {
				continue;
			}
			let is_string = match self.tag(v) {
				Ok(tag) => tag == Type::String,
				// Inline function and type values have no heap tag, but still
				// need the native argument diagnostic below.
				Err(error) if error.kind == ErrorKind::Type => false,
				Err(error) => return Err(error),
			};
			if !is_string {
				let found = self
					.diagnostic_type(v)
					.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				return Err(
					err(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
						cause: HostCause::ExpectedType,
						arg: i as u8,
						expected: TypeName::String,
						found,
					}),
				);
			}
			self.text_argument(v, i as u8)?;
		}
		if m == Method::Split {
			return self.text_split(receiver, args[1]);
		}
		let text = self.text(receiver)?;
		let result = match m {
			Method::StartsWith => return Ok(Value::Bool(text.starts_with(self.text(args[1])?))),
			Method::EndsWith => return Ok(Value::Bool(text.ends_with(self.text(args[1])?))),
			Method::Contains => return Ok(Value::Bool(text.contains(self.text(args[1])?))),
			Method::Trim => {
				let t = text.trim();
				if t.len() > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				String::try_from(t)?
			}
			Method::Replace => {
				let from = self.text(args[1])?;
				let to = self.text(args[2])?;
				let n = text.matches(from).count();
				let len = text
					.len()
					.checked_sub(
						n.checked_mul(from.len())
							.ok_or_else(|| err(ErrorKind::Overflow))?,
					)
					.and_then(|l| n.checked_mul(to.len()).and_then(|a| l.checked_add(a)))
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				if len > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				crate::alloc::str::replace(text, from, to)?
			}
			Method::Split => return Err(err(ErrorKind::InvalidCode)),
		};
		self.string_owned(result)
	}
	pub(crate) fn text_missing(&self, value: Value, method: crate::Hash) -> Result<Error> {
		let target = self
			.diagnostic_type(value)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let instance = match target {
			TypeName::Struct(n) => super::super::nominal::hash_copy(n).map(crate::Hash::new),
			_ => target.builtin_hash(),
		}
		.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		Ok(
			err(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
				target,
				instance,
				hash: crate::Hash::associated_function(instance, method),
			}),
		)
	}
	#[cold]
	#[inline(never)]
	fn text_split(&mut self, source: Value, delimiter: Value) -> Result<Value> {
		let Value::Heap(h) = source else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let readers = self
			.entry(h)?
			.readers
			.checked_add(1)
			.filter(|n| *n != sorting::EXCLUSIVE)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		let len = if matches!(delimiter, Value::Char(_)) {
			0
		} else {
			self.text(delimiter)?.len()
		};
		if size_of::<super::boxed_iterator::State>()
			.saturating_add(size_of::<Split>())
			.saturating_add(len)
			> self.limits.bytes.saturating_sub(self.bytes)
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		let pattern = if let Value::Char(c) = delimiter {
			Pattern::Char(c)
		} else {
			Pattern::String(String::try_from(self.text(delimiter)?)?)
		};
		let state = Box::try_new(Split {
			pattern,
			from: 0,
			to: 0,
		})
		.map_err(crate::alloc::Error::from)?;
		// The top-level cold box keeps its fixed base charge. Preflight the
		// separately allocated Split and copied pattern before publishing roots.
		let extra = state.bytes()?;
		let charge = size_of::<super::boxed_iterator::State>()
			.checked_add(extra)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		let total = self
			.bytes
			.checked_add(charge)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if total > self.limits.bytes {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let state = Box::try_new(super::boxed_iterator::State {
			source: Some(h),
			kind: super::boxed_iterator::Kind::Split(state),
		})
		.map_err(crate::alloc::Error::from)?;
		let value = self.admit(Payload::OtherRangeIterator(state))?;
		// admit charges the fixed outer box. Add the preflighted extra to its
		// post-admission count; invariant failures report InvalidCode, not panic.
		let Value::Heap(iter) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let bytes = self
			.bytes
			.checked_add(extra)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		if bytes > self.limits.bytes {
			return Err(err(ErrorKind::InvalidCode));
		}
		self.entry_mut(iter)
			.map_err(|_| err(ErrorKind::InvalidCode))?
			.set_charged_bytes(charge);
		self.bytes = bytes;
		self.entry_mut(h)
			.map_err(|_| err(ErrorKind::InvalidCode))?
			.readers = readers;
		Ok(value)
	}
	#[cold]
	#[inline(never)]
	pub(super) fn split_next(&mut self, h: Handle) -> Result<Option<Value>> {
		let (copy, from, to, finish) = {
			let Payload::OtherRangeIterator(boxed) = self.payload(h)? else {
				return Err(err(ErrorKind::InvalidCode));
			};
			let super::boxed_iterator::Kind::Split(state) = &boxed.kind else {
				return Err(err(ErrorKind::InvalidCode));
			};
			let Some(source) = boxed.source else {
				return Ok(None);
			};
			let text = self.text(Value::Heap(source))?;
			let mut from = state.from;
			let mut to = state.to;
			let (a, b, finish) = if from == text.len() && from == to {
				(from, to, true)
			} else {
				let mut found = None;
				while to < text.len() {
					let tail = text.get(to..).ok_or_else(|| err(ErrorKind::InvalidCode))?;
					let (yes, len) = match &state.pattern {
						Pattern::String(s) => (
							tail.starts_with(s.as_str()),
							if tail.starts_with(s.as_str()) {
								s.len()
							} else {
								tail.chars().next().map_or(0, char::len_utf8)
							},
						),
						Pattern::Char(c) => {
							let x = tail
								.chars()
								.next()
								.ok_or_else(|| err(ErrorKind::InvalidCode))?;
							(x == *c, x.len_utf8())
						}
					};
					if yes {
						let a = from;
						let b = to;
						if len == 0 {
							from = to;
							to += tail.chars().next().map_or(0, char::len_utf8);
						} else {
							to += len;
							from = to;
						}
						found = Some((a, b, false));
						break;
					}
					to += len;
				}
				match found {
					Some(x) => x,
					None => {
						let a = from;
						from = to;
						(
							a,
							to,
							!matches!(&state.pattern,Pattern::String(s) if s.is_empty()),
						)
					}
				}
			};
			let text = text.get(a..b).ok_or_else(|| err(ErrorKind::InvalidCode))?;
			if text.len() > self.limits.bytes.saturating_sub(self.bytes) {
				return Err(err(ErrorKind::StoreCapacity));
			}
			(String::try_from(text)?, from, to, finish)
		};
		let result = self.string_owned(copy)?;
		let Payload::OtherRangeIterator(boxed) = self.mutable(h)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let super::boxed_iterator::Kind::Split(state) = &mut boxed.kind else {
			return Err(err(ErrorKind::InvalidCode));
		};
		state.from = from;
		state.to = to;
		let released = if finish { boxed.source.take() } else { None };
		if let Some(source) = released {
			self.edges = self
				.edges
				.checked_sub(1)
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			let e = self.entry_mut(source)?;
			e.readers = e
				.readers
				.checked_sub(1)
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			self.release(Value::Heap(source))?;
		}
		Ok(Some(result))
	}
}

impl Store {
	pub(crate) fn shared_readers(&self, value: Value) -> Option<u32> {
		let Value::Heap(h) = value else {
			return None;
		};
		let e = self.entry(h).ok()?;
		Some(e.readers)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn split(s: &mut Store, a: Value, b: Value) -> Value {
		s.text_method(Method::Split, &[a, b]).unwrap()
	}
	#[test]
	fn source_lease_pattern_copy_exhaustion_and_drop() {
		let mut s = Store::new(Limits {
			entries: 32,
			bytes: 4096,
		})
		.unwrap();
		let source = s.string("a,b").unwrap();
		let pattern = s.string(",").unwrap();
		let iter = split(&mut s, source, pattern);
		let suffix = s.string("b").unwrap();
		s.append_string(pattern, suffix).unwrap();
		assert_eq!(
			s.append_string(source, suffix).unwrap_err().detail,
			Detail::WriteBorrowed
		);
		let a = s.iterator_next(iter).unwrap().unwrap();
		assert_eq!(s.text(a).unwrap(), "a");
		s.release(a).unwrap();
		let b = s.iterator_next(iter).unwrap().unwrap();
		assert_eq!(s.text(b).unwrap(), "b");
		s.release(b).unwrap();
		s.append_string(source, suffix).unwrap();
		assert_eq!(s.text(source).unwrap(), "a,bb");
		s.release(iter).unwrap();
		let iter = split(&mut s, source, pattern);
		s.release(iter).unwrap();
		s.append_string(source, suffix).unwrap();
		for v in [source, pattern, suffix] {
			s.release(v).unwrap();
		}
		assert_eq!(s.live_entries(), 0);
		assert_eq!(s.payload_bytes(), 0);
	}
	#[test]
	fn failed_split_admission_does_not_lease_or_retain() {
		let mut s = Store::new(Limits {
			entries: 2,
			bytes: 4096,
		})
		.unwrap();
		let source = s.string("a").unwrap();
		let pattern = s.string(",").unwrap();
		assert_eq!(
			s.text_method(Method::Split, &[source, pattern])
				.unwrap_err()
				.kind,
			ErrorKind::StoreCapacity
		);
		s.append_string(source, pattern).unwrap();
		s.release(source).unwrap();
		s.release(pattern).unwrap();
		assert_eq!(s.live_entries(), 0);
		assert_eq!(s.payload_bytes(), 0);
	}
}

#[cfg(test)]
mod access_order_tests {
	use super::*;
	#[test]
	fn moved_receiver_precedes_argument_conversion_and_never_creates_iterator() {
		let mut s = Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap();
		let a = s.string("a").unwrap();
		let delimiter = s.string(",").unwrap();
		s.invalidate(a).unwrap();
		let e = s.text_method(Method::Split, &[a, delimiter]).unwrap_err();
		assert!(matches!(e.detail, Detail::CmpArgumentAccess { arg: 0, .. }));
		let e = s
			.text_method(Method::Replace, &[a, Value::I64(7), delimiter])
			.unwrap_err();
		assert!(matches!(e.detail, Detail::CmpArgumentAccess { arg: 0, .. }));
		s.release(a).unwrap();
		s.release(delimiter).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
}

#[cfg(test)]
mod budget_tests {
	use super::*;
	#[test]
	fn split_extra_allocation_is_preflighted_and_charged_until_drop() {
		let charge = size_of::<super::super::boxed_iterator::State>() + size_of::<Split>();
		for extra in [0usize, 1] {
			let mut store = Store::new(Limits {
				entries: 16,
				bytes: 3 + charge - extra,
			})
			.unwrap();
			let source = store.string("a,b").unwrap();
			let result = store.text_method(Method::Split, &[source, Value::Char(',')]);
			if extra == 0 {
				let iter = result.unwrap();
				assert_eq!(store.payload_bytes(), 3 + charge);
				assert_eq!(
					store.iterator_next(iter).unwrap_err().kind,
					ErrorKind::StoreCapacity
				);
				store.release(iter).unwrap();
			} else {
				assert_eq!(result.unwrap_err().kind, ErrorKind::StoreCapacity);
			}
			assert_eq!(store.payload_bytes(), 3);
			assert_eq!(store.shared_readers(source), Some(0));
			store.release(source).unwrap();
			assert_eq!(store.payload_bytes(), 0);
			assert_eq!(store.live_entries(), 0);
		}
	}
}
