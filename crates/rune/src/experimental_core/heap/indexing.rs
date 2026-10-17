//! Stock index dispatch, with separate read, write and mutable-place rules.
use super::{err, Payload, Store};
use crate::alloc::{String, Vec};
use crate::experimental_core::{
	error::{Detail, Error, ErrorKind, Result, TypeName},
	value::{Type, Value},
};
use core::mem::size_of;

/// Static keys borrow the Program; dynamic keys are resolved only while the
/// Store is immutably borrowed, before the insertion transaction mutates it.
pub(super) enum RecordKey<'p> {
	Static(&'p str),
	Dynamic(Value),
}

impl RecordKey<'_> {
	fn borrow<'a>(&'a self, store: &'a Store) -> Result<&'a str> {
		match self {
			Self::Static(key) => Ok(key),
			Self::Dynamic(key) => store.text(*key),
		}
	}
}

// Function values had no store tag before protocol dispatch was expanded.
// Preserve their public error operation metadata on unsupported reads/writes.
fn unsupported(value: Value) -> Error {
	Error::new(
		ErrorKind::Type,
		if matches!(value, Value::Function { .. }) {
			"store"
		} else {
			"execute"
		},
	)
}

impl Store {
	fn index_tag(&self, value: Value) -> Result<Option<Type>> {
		Ok(match value {
			Value::Heap(h) => Some(self.entry(h)?.tag()?),
			_ => value.primitive_type(),
		})
	}

	fn index_type(&self, value: Value) -> Result<TypeName> {
		if let Value::Heap(h) = value {
			self.entry(h)?;
		}
		self.diagnostic_type(value)
			.ok_or_else(|| err(ErrorKind::InvalidCode))
	}
	fn missing_integer(&self, value: Value, index: usize) -> Result<Error> {
		Ok(
			err(ErrorKind::IndexOutOfBounds).with_detail(Detail::MissingInteger {
				target: self.index_type(value)?,
				index,
			}),
		)
	}
	fn tuple_get_error(&self, value: Value, index: usize) -> Result<Error> {
		Ok(unsupported(value).with_detail(Detail::TupleIndexGet {
			target: self.index_type(value)?,
			index,
		}))
	}
	pub(crate) fn tuple_get(&mut self, value: Value, index: usize) -> Result<Value> {
		let old = match value {
			Value::Unit => return Err(self.missing_integer(value, index)?),
			Value::Heap(h) => {
				match self.entry(h)?.tag()? {
					Type::Vec | Type::Tuple | Type::Option | Type::Result | Type::String => (),
					Type::Struct(_) => return self.tuple_struct_get(value, h, index),
					_ => return Err(self.tuple_get_error(value, index)?),
				}
				match self.payload(h)? {
					Payload::List { values, .. } => *values.get(index).ok_or_else(|| {
						err(ErrorKind::IndexOutOfBounds).with_detail(Detail::OutOfRange {
							index,
							length: values.len(),
						})
					})?,
					Payload::Variant { payload, .. } => match (index, payload) {
						(0, Some(old)) => *old,
						_ => return Err(self.missing_integer(value, index)?),
					},
					Payload::Record {
						tag: Type::Struct(_),
						..
					} => return Err(self.missing_integer(value, index)?),
					_ => return Err(self.tuple_get_error(value, index)?),
				}
			}
			_ => return Err(self.tuple_get_error(value, index)?),
		};
		self.retain(old)?;
		Ok(old)
	}

	#[cold]
	#[inline(never)]
	fn tuple_struct_get(
		&mut self,
		value: Value,
		handle: super::Handle,
		index: usize,
	) -> Result<Value> {
		let payload = match self.payload(handle) {
			Ok(payload) => payload,
			Err(error) => {
				// Nominal types without integer fields fail dispatch before any
				// moved/leased payload access, as the stock index path does.
				if error.kind == ErrorKind::AccessConflict {
					if let Type::Struct(nominal) = self.entry(handle)?.tag()? {
						let missing = match super::super::nominal::shape_copy(nominal) {
							Some(
								super::super::nominal::Shape::Named
								| super::super::nominal::Shape::Empty,
							) => true,
							Some(super::super::nominal::Shape::Tuple(arity)) => {
								index >= arity as usize
							}
							None => false,
						};
						if missing {
							return Err(self.missing_integer(value, index)?);
						}
					}
				}
				return Err(error);
			}
		};
		let old = match payload {
			Payload::List {
				tag: Type::Struct(_),
				values,
			} => values.get(index).copied(),
			_ => None,
		};
		let Some(old) = old else {
			return Err(self.missing_integer(value, index)?);
		};
		self.retain(old)?;
		Ok(old)
	}
	/// Resolve mutable access BEFORE looking up the element or doing arithmetic.
	pub(crate) fn tuple_get_mut(&mut self, value: Value, index: usize) -> Result<Value> {
		let Value::Heap(h) = value else {
			return Err(self.tuple_get_error(value, index)?);
		};
		self.mutable(h)?;
		let old = match self.payload(h)? {
			Payload::List { values, .. } => *values.get(index).ok_or_else(|| {
				err(ErrorKind::IndexOutOfBounds).with_detail(Detail::OutOfRange {
					index,
					length: values.len(),
				})
			})?,
			Payload::Variant { payload, .. } => match (index, payload) {
				(0, Some(old)) => *old,
				_ => return Err(self.missing_integer(value, index)?),
			},
			Payload::Record {
				tag: Type::Struct(_),
				..
			} => return Err(self.missing_integer(value, index)?),
			_ => return Err(self.tuple_get_error(value, index)?),
		};
		self.retain(old)?;
		Ok(old)
	}
	pub(crate) fn tuple_set(&mut self, value: Value, index: usize, new: Value) -> Result<()> {
		let unsupported = |this: &Self| -> Result<Error> {
			Ok(unsupported(value).with_detail(Detail::TupleIndexSet {
				target: this.index_type(value)?,
			}))
		};
		let Value::Heap(h) = value else {
			return Err(unsupported(self)?);
		};
		let old = match self.mutable(h)? {
			Payload::List { values, .. } => values.get(index).copied(),
			Payload::Variant {
				tag,
				success,
				payload,
			} => {
				let accepted = match *tag {
					Type::Option => index == 0,
					Type::Result => index == usize::from(!*success),
					_ => false,
				};
				if accepted {
					*payload
				} else {
					None
				}
			}
			_ => None,
		};
		let Some(old) = old else {
			return Err(unsupported(self)?);
		};
		self.replace_tuple_place(value, index, old, new)
	}
	pub(crate) fn tuple_set_mut(&mut self, value: Value, index: usize, new: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let old = match self.mutable(h)? {
			Payload::List { values, .. } => *values
				.get(index)
				.ok_or_else(|| err(ErrorKind::InvalidCode))?,
			Payload::Variant {
				payload: Some(old), ..
			} if index == 0 => *old,
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		self.replace_tuple_place(value, index, old, new)
	}
	fn replace_tuple_place(
		&mut self,
		value: Value,
		index: usize,
		old: Value,
		new: Value,
	) -> Result<()> {
		// An in-place string append already updated this same value.
		if old == new {
			return Ok(());
		}
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let edges = self.replacement_edges(old, new)?;
		self.retain(new)?;
		match self.mutable(h)? {
			Payload::List { values, .. } => values[index] = new,
			Payload::Variant { payload, .. } => *payload = Some(new),
			_ => unreachable!(),
		}
		self.edges = edges;
		self.assert_work();
		self.release(old)
	}
	pub(crate) fn dynamic_get(&mut self, value: Value, index: Value) -> Result<Value> {
		if matches!(
			self.index_tag(index)?,
			Some(
				Type::Range
					| Type::RangeFrom
					| Type::RangeFull
					| Type::RangeInclusive
					| Type::RangeToInclusive
					| Type::RangeTo
			)
		) && matches!(self.index_tag(value)?, Some(Type::Vec | Type::String))
		{
			return self.range_index(value, index);
		}
		if matches!(
			index,
			Value::Unit
				| Value::Bool(_)
				| Value::Char(_)
				| Value::I64(_)
				| Value::U64(_)
				| Value::F64(_)
				| Value::Type(_)
		) {
			let i = super::super::execute::index_value(index).map_err(|e| {
				if e.detail == Detail::None {
					e.with_detail(Detail::ExpectedNumber {
						found: self.index_type(index).unwrap_or(TypeName::Tuple),
					})
				} else {
					e
				}
			})?;
			return self.tuple_get(value, i).map_err(|e| {
				if matches!(e.detail, Detail::TupleIndexGet { .. }) {
					e.with_detail(Detail::IndexTarget {
						target: self.diagnostic_type(value).expect("validated target"),
						index: self.diagnostic_type(index).expect("inline index"),
					})
				} else {
					e
				}
			});
		}
		if self.index_tag(index)? == Some(Type::String)
			&& match self.index_tag(value)? {
				Some(Type::Object) => true,
				Some(Type::Struct(n)) => {
					matches!(
						super::super::nominal::shape_copy(n),
						Some(super::super::nominal::Shape::Named)
					)
				}
				_ => false,
			} {
			let Value::Heap(h) = value else {
				unreachable!()
			};
			let key = self.text(index)?;
			let Payload::Record { fields, .. } = self.payload(h)? else {
				unreachable!()
			};
			let Some(old) = fields
				.iter()
				.find(|(name, _)| name.as_str() == key)
				.map(|(_, old)| *old)
			else {
				return Err(
					err(ErrorKind::IndexOutOfBounds).with_detail(Detail::MissingKey {
						target: self.index_type(value)?,
					}),
				);
			};
			self.retain(old)?;
			return Ok(old);
		}
		if self.index_tag(value)? == Some(Type::Vec) {
			return Err(Error::new(ErrorKind::Type, "execute").with_detail(
				Detail::ExpectedNumber {
					found: self.index_type(index)?,
				},
			));
		}
		Err(unsupported(value).with_detail(Detail::IndexTarget {
			target: self.index_type(value)?,
			index: self.index_type(index)?,
		}))
	}
	pub(crate) fn dynamic_set(&mut self, value: Value, index: Value, new: Value) -> Result<()> {
		if self.index_tag(index)? == Some(Type::String)
			&& matches!(
				value,
				Value::Unit
					| Value::Bool(_)
					| Value::Char(_)
					| Value::I64(_) | Value::U64(_)
					| Value::F64(_) | Value::Type(_)
			) {
			return Err(
				err(ErrorKind::IndexOutOfBounds).with_detail(Detail::MissingKey {
					target: self.index_type(value)?,
				}),
			);
		}
		if self.index_tag(index)? == Some(Type::String)
			&& match self.index_tag(value)? {
				Some(Type::Object) => true,
				Some(Type::Struct(n)) => {
					matches!(
						super::super::nominal::shape_copy(n),
						Some(super::super::nominal::Shape::Named)
					)
				}
				_ => false,
			} {
			return self.set_key(value, RecordKey::Dynamic(index), new);
		}
		if self.index_tag(value)? == Some(Type::Vec) {
			let Value::Heap(h) = value else {
				unreachable!()
			};
			self.mutable(h)?;
			let i = super::super::execute::index_value(index).map_err(|e| {
				if e.detail == Detail::None {
					e.with_detail(Detail::ExpectedNumber {
						found: self.diagnostic_type(index).expect("validated index"),
					})
				} else {
					e
				}
			})?;
			let length = self.len(value)?;
			return self.set(value, i, new).map_err(|e| {
				if e.kind == ErrorKind::IndexOutOfBounds {
					e.with_detail(Detail::OutOfRange { index: i, length })
				} else {
					e
				}
			});
		}
		Err(err(ErrorKind::Type).with_detail(Detail::IndexSet {
			target: self.index_type(value)?,
			index: self.index_type(index)?,
			value: self.index_type(new)?,
		}))
	}
	pub(super) fn set_key(&mut self, value: Value, key: RecordKey<'_>, new: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			unreachable!()
		};
		self.mutable(h)?;
		let text = key.borrow(self)?;
		let Payload::Record { tag, fields } = self.payload(h)? else {
			unreachable!()
		};
		if let Some(i) = fields.iter().position(|(name, _)| name.as_str() == text) {
			let old = fields[i].1;
			let edges = self.replacement_edges(old, new)?;
			self.retain(new)?;
			let Payload::Record { fields, .. } = self.mutable(h)? else {
				unreachable!()
			};
			fields[i].1 = new;
			self.edges = edges;
			self.assert_work();
			return self.release(old);
		}
		if *tag != Type::Object {
			return Err(
				err(ErrorKind::IndexOutOfBounds).with_detail(Detail::MissingKey {
					target: self.index_type(value)?,
				}),
			);
		}
		let len = fields.len();
		let cap = fields.capacity();
		let key_bytes = fields
			.iter()
			.try_fold(0usize, |n, (key, _)| n.checked_add(key.capacity()))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		let next = if len == cap {
			cap.checked_mul(2)
				.ok_or_else(|| err(ErrorKind::Overflow))?
				.max(1)
		} else {
			cap
		};
		let planned = next
			.checked_mul(size_of::<(String, Value)>())
			.and_then(|n| n.checked_add(key_bytes))
			.and_then(|n| n.checked_add(text.len()))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.replacement_bytes(h, planned)?;
		let copy = String::try_from(text)?;
		let mut replacement = if len == cap {
			Some(Vec::try_with_capacity(next)?)
		} else {
			None
		};
		let actual_cap = replacement.as_ref().map_or(cap, |v| v.capacity());
		let charged = actual_cap
			.checked_mul(size_of::<(String, Value)>())
			.and_then(|n| n.checked_add(key_bytes))
			.and_then(|n| n.checked_add(copy.capacity()))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		let bytes = self.replacement_bytes(h, charged)?;
		let edges = self
			.edges
			.checked_add(Self::heap_edge(new))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.reserve_work(Self::heap_edge(new), 0)?;
		// Validate the entire capacity proof before retaining or moving any key.
		let actual_room = replacement.as_ref().map_or(cap, |next| next.capacity());
		if actual_room < len.checked_add(1).ok_or_else(|| err(ErrorKind::Overflow))? {
			return Err(Error::new(ErrorKind::InvalidCode, "field capacity"));
		}
		self.retain(new)?;
		let staged = (|| -> Result<()> {
			let Payload::Record { fields, .. } = self.mutable(h)? else {
				unreachable!()
			};
			if let Some(mut next) = replacement.take() {
				let mut failure = None;
				while let Some(pair) = fields.pop() {
					if let Err(pair) = next.push_within_capacity(pair) {
						// Restore the original ordering and ownership on a violated
						// capacity proof; neither vector ever owns extra heap roots.
						fields.push_within_capacity(pair).map_err(|_| {
							Error::new(ErrorKind::InvalidCode, "field rollback capacity")
						})?;
						while let Some(pair) = next.pop() {
							fields.push_within_capacity(pair).map_err(|_| {
								Error::new(ErrorKind::InvalidCode, "field rollback capacity")
							})?;
						}
						failure = Some(Error::new(ErrorKind::InvalidCode, "field capacity"));
						break;
					}
				}
				if let Some(e) = failure {
					Err(e)
				} else {
					next.reverse();
					match next.push_within_capacity((copy, new)) {
						Ok(_) => {
							core::mem::swap(fields, &mut next);
							Ok(())
						}
						Err(_) => {
							next.reverse();
							while let Some(pair) = next.pop() {
								fields.push_within_capacity(pair).map_err(|_| {
									Error::new(ErrorKind::InvalidCode, "field rollback capacity")
								})?;
							}
							Err(Error::new(ErrorKind::InvalidCode, "field capacity"))
						}
					}
				}
			} else {
				fields
					.push_within_capacity((copy, new))
					.map(|_| ())
					.map_err(|_| Error::new(ErrorKind::InvalidCode, "field capacity"))
			}
		})();
		if let Err(e) = staged {
			self.release(new)?;
			return Err(e);
		}
		self.entry_mut(h)?.set_charged_bytes(charged);
		self.bytes = bytes;
		self.edges = edges;
		self.assert_work();
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rune_alloc::limit;

	fn state(s: &Store) -> (usize, usize, usize, std::string::String) {
		(s.live, s.bytes, s.edges, std::format!("{:?}", s.slots))
	}
	fn check(s: &Store) {
		let actual: usize = s
			.slots
			.iter()
			.filter_map(|slot| slot.entry.as_ref())
			.filter_map(|entry| entry.payload.as_ref())
			.map(Store::payload_edges)
			.sum();
		assert_eq!(s.edges, actual);
		s.assert_work();
		assert!(s.work.is_empty());
	}
	#[test]
	fn object_insert_allocation_failures_preserve_payload_and_ownership() {
		let mut successes = 0;
		let mut failures = 0;
		for budget in 0..160 {
			let mut s = Store::new(super::super::Limits::default()).unwrap();
			let object = s.record(Type::Object, &[]).unwrap();
			let key = s.string("new-key").unwrap();
			let child = s.string("owned-child").unwrap();
			let before = state(&s);
			let result = limit::with(budget, || s.dynamic_set(object, key, child)).call();
			match result {
				Ok(()) => {
					successes += 1;
					let found = s.dynamic_get(object, key).unwrap();
					assert_eq!(found, child);
					s.release(found).unwrap();
				}
				Err(e) => {
					failures += 1;
					assert_eq!(e.kind, ErrorKind::Allocation);
					assert_eq!(state(&s), before);
				}
			}
			check(&s);
			s.release(child).unwrap();
			s.release(key).unwrap();
			limit::with(0, || s.release(object).unwrap()).call();
			assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
		}
		assert!(successes > 0 && failures > 0);
	}
	#[test]
	fn object_insert_growth_and_cycles_keep_the_release_work_invariant() {
		let mut s = Store::new(super::super::Limits::default()).unwrap();
		let object = s.record(Type::Object, &[]).unwrap();
		for n in 0..100 {
			let key = s.string(&std::format!("key{n}")).unwrap();
			let child = s.string("child").unwrap();
			s.dynamic_set(object, key, child).unwrap();
			s.release(key).unwrap();
			s.release(child).unwrap();
			check(&s);
		}
		assert_eq!(s.edges, 100);
		limit::with(0, || s.release(object).unwrap()).call();
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
		let object = s.record(Type::Object, &[]).unwrap();
		let key = s.string("cycle").unwrap();
		s.dynamic_set(object, key, object).unwrap();
		s.release(key).unwrap();
		s.release(object).unwrap();
		check(&s);
		limit::with(0, || s.close().unwrap()).call();
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}
	#[test]
	fn object_insert_byte_limit_failure_preserves_existing_fields() {
		let mut s = Store::new(super::super::Limits::default()).unwrap();
		let object = s.record(Type::Object, &[("x", Value::I64(1))]).unwrap();
		let key = s.string("new").unwrap();
		s.limits.bytes = s.bytes;
		let before = state(&s);
		assert_eq!(
			s.dynamic_set(object, key, object).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!(state(&s), before);
		check(&s);
		s.release(key).unwrap();
		s.release(object).unwrap();
	}
	#[test]
	fn result_plain_write_and_compound_place_have_different_indices() {
		let mut s = Store::new(super::super::Limits::default()).unwrap();
		let value = s
			.variant(Type::Result, false, Some(Value::I64(10)))
			.unwrap();
		assert_eq!(s.tuple_get(value, 0).unwrap(), Value::I64(10));
		assert_eq!(
			s.tuple_set(value, 0, Value::I64(3)).unwrap_err().kind,
			ErrorKind::Type
		);
		s.tuple_set(value, 1, Value::I64(3)).unwrap();
		assert_eq!(s.tuple_get_mut(value, 0).unwrap(), Value::I64(3));
		s.tuple_set_mut(value, 0, Value::I64(13)).unwrap();
		assert_eq!(s.tuple_get(value, 0).unwrap(), Value::I64(13));
		assert!(matches!(
			s.tuple_get_mut(value, 1).unwrap_err().detail,
			Detail::MissingInteger { index: 1, .. }
		));
		s.release(value).unwrap();
		check(&s);
	}
	#[test]
	fn mutable_tuple_lookup_checks_reader_lease_before_bounds_or_arithmetic() {
		let mut s = Store::new(super::super::Limits::default()).unwrap();
		let value = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		s.set_readers_for_test(value, 1);
		let before = state(&s);
		for index in [0, 99] {
			assert_eq!(
				s.tuple_get_mut(value, index).unwrap_err().detail,
				Detail::WriteBorrowed
			);
		}
		assert_eq!(state(&s), before);
		s.set_readers_for_test(value, 0);
		s.release(value).unwrap();
		check(&s);
	}
}
