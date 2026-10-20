//! C17b Object methods and `is_empty`, with stock native argument order.
use super::super::error::{Detail, HostCause, MovedAccess, TypeName};
use super::*;
use crate::function::fresh_object::Method;

/// Attaches the native argument index to a store access failure.
fn argument_access(error: Error, arg: u8, readers: u32) -> Error {
	let detail = match error.detail {
		Detail::Moved { access } => Detail::MovedArgument { arg, access },
		Detail::WriteBorrowed => Detail::SharedArgument { arg, readers },
		Detail::TakeBorrowed => Detail::TakeSharedArgument { arg, readers },
		Detail::ReadExclusive => Detail::ExclusiveArgument {
			access: MovedAccess::Read,
			arg,
		},
		Detail::WriteExclusive => Detail::ExclusiveArgument {
			access: MovedAccess::Write,
			arg,
		},
		Detail::TakeExclusive => Detail::ExclusiveArgument {
			access: MovedAccess::Take,
			arg,
		},
		_ => return error,
	};
	error.with_detail(detail)
}

impl Store {
	fn readers(&self, value: Value) -> u32 {
		match value {
			Value::Heap(h) => self.entry(h).map_or(0, |entry| entry.readers),
			_ => 0,
		}
	}

	/// Checks the access a native argument conversion makes. Every argument has
	/// already passed its type check.
	fn argument(&mut self, value: Value, arg: u8, access: MovedAccess) -> Result<()> {
		let Value::Heap(h) = value else {
			return Ok(());
		};
		let readers = self.readers(value);
		let result = match access {
			MovedAccess::Read => self.payload(h).map(|_| ()),
			MovedAccess::Write => self.mutable(h).map(|_| ()),
			MovedAccess::Take => {
				let entry = self.entry(h)?;
				if entry.readers != 0 {
					Err(sorting::conflict(sorting::Access::Take, entry))
				} else {
					self.payload(h).map(|_| ()).map_err(|e| match e.detail {
						Detail::Moved { .. } => e.with_detail(Detail::Moved { access }),
						_ => e,
					})
				}
			}
		};
		result.map_err(|e| argument_access(e, arg, readers))
	}

	/// The key argument: a `String`, type-checked before its access.
	fn key_argument(&mut self, key: Value, access: MovedAccess) -> Result<()> {
		if self.tag(key).ok() != Some(Type::String) {
			let found = self
				.diagnostic_type(key)
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			return Err(
				err(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
					cause: HostCause::ExpectedType,
					arg: 1,
					expected: TypeName::String,
					found,
				}),
			);
		}
		self.argument(key, 1, access)
	}

	pub(super) fn object_fields(&self, receiver: Value) -> Result<&Vec<(String, Value)>> {
		let Value::Heap(h) = receiver else {
			return Err(err(ErrorKind::InvalidCode));
		};
		match self.payload(h)? {
			Payload::Record {
				tag: Type::Object,
				fields,
			} => Ok(fields),
			_ => Err(err(ErrorKind::InvalidCode)),
		}
	}

	fn key_index(&self, receiver: Value, key: Value) -> Result<Option<usize>> {
		let key = self.text(key)?;
		Ok(self
			.object_fields(receiver)?
			.iter()
			.position(|(name, _)| name.as_str() == key))
	}

	fn some(&mut self, value: Value) -> Result<Value> {
		self.variant(Type::Option, true, Some(value))
	}

	/// Runs a certified method on evaluated operands, receiver first. Returned
	/// values are the stored handles themselves, never copies.
	#[cold]
	#[inline(never)]
	pub(crate) fn object_method(&mut self, method: Method, args: &[Value]) -> Result<Value> {
		let receiver = args[0];
		let tag = match receiver {
			Value::Heap(h) => Some(self.entry(h)?.tag()?),
			_ => receiver.primitive_type(),
		};
		let provided = match tag {
			Some(Type::Object) => true,
			Some(Type::Vec | Type::Tuple | Type::String | Type::Unit) => method == Method::IsEmpty,
			_ => false,
		};
		if !provided {
			return Err(self.text_missing(receiver, method.hash())?);
		}
		match method {
			Method::IsEmpty => {
				self.argument(receiver, 0, MovedAccess::Read)?;
				Ok(Value::Bool(self.len(receiver)? == 0))
			}
			Method::ContainsKey | Method::Get => {
				self.argument(receiver, 0, MovedAccess::Read)?;
				self.key_argument(args[1], MovedAccess::Read)?;
				let index = self.key_index(receiver, args[1])?;
				if method == Method::ContainsKey {
					return Ok(Value::Bool(index.is_some()));
				}
				match index {
					Some(i) => {
						let value = self.object_fields(receiver)?[i].1;
						self.some(value)
					}
					None => self.variant(Type::Option, false, None),
				}
			}
			Method::Remove => {
				self.argument(receiver, 0, MovedAccess::Write)?;
				self.key_argument(args[1], MovedAccess::Read)?;
				match self.key_index(receiver, args[1])? {
					Some(i) => self.remove_field(receiver, i),
					None => self.variant(Type::Option, false, None),
				}
			}
			Method::Insert => {
				self.argument(receiver, 0, MovedAccess::Write)?;
				self.key_argument(args[1], MovedAccess::Read)?;
				self.insert_field(receiver, args[1], args[2])
			}
		}
	}

	/// Shift-removes field `i`, keeping the order of the rest, and returns its
	/// value as `Some`.
	fn remove_field(&mut self, receiver: Value, i: usize) -> Result<Value> {
		let Value::Heap(h) = receiver else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let (old, key_bytes) = {
			let (key, old) = &self.object_fields(receiver)?[i];
			(*old, key.capacity())
		};
		let result = self.some(old)?;
		let charged = self.entry(h)?.charged_bytes().checked_sub(key_bytes);
		let staged = charged
			.ok_or_else(|| err(ErrorKind::InvalidCode))
			.and_then(|charged| Ok((charged, self.replacement_bytes(h, charged)?)))
			.and_then(|bytes| Ok((bytes, self.replacement_edges(old, Value::Unit)?)));
		let ((charged, bytes), edges) = match staged {
			Ok(staged) => staged,
			Err(e) => {
				self.release(result)?;
				return Err(e);
			}
		};
		let Payload::Record { fields, .. } = self.mutable(h)? else {
			unreachable!()
		};
		let (key, _) = fields.remove(i);
		drop(key);
		self.entry_mut(h)?.set_charged_bytes(charged);
		self.bytes = bytes;
		self.edges = edges;
		self.assert_work();
		self.release(old)?;
		Ok(result)
	}

	/// Stores a copy of `key` with `new` and returns the previous value; the key
	/// handle stays readable.
	fn insert_field(&mut self, receiver: Value, key: Value, new: Value) -> Result<Value> {
		let Value::Heap(h) = receiver else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let result = match self.key_index(receiver, key)? {
			Some(i) => {
				let old = self.object_fields(receiver)?[i].1;
				self.some(old)?
			}
			None => self.variant(Type::Option, false, None)?,
		};
		if let Err(e) = self.set_key(receiver, indexing::RecordKey::Dynamic(key), new) {
			self.release(result)?;
			return Err(e);
		}
		debug_assert!(matches!(self.entry(h)?.tag()?, Type::Object));
		Ok(result)
	}
}
