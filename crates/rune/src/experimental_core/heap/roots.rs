//! The sealed host boundary: every value crosses as a [`Root`] stamped with
//! its store's identity, and every root a host supplies is checked (closed
//! policy, then owner, then generation) before anything is retained or read.
//! Inside the core, values carry no owner; this file is where the owner
//! check lives.
//!
//! Allocation discipline: every host method validates counts, byte sizes,
//! tags, keys and roots WITHOUT allocating, and only then copies into a
//! scratch buffer that is never larger than the payload it has just checked
//! against the store's byte cap (so scratch plus payload stay within twice
//! the cap for the duration of one call). Invocation arguments need no proportional scratch;
//! existing diagnostic and program allocation sites remain in the inventory.
use super::{err, Store};
use crate::alloc::{String, Vec};
use crate::experimental_core::error::{ErrorKind, Result};
use crate::experimental_core::nominal;
use crate::experimental_core::value::{Nominal, ReleaseError, Root, Scalar, Type, Value};
use core::mem::size_of;

impl Store {
	/// Owner check for one supplied root: closed store first, then the brand,
	/// then (for heap values) generation validity. Nothing is changed.
	pub(crate) fn import(&self, root: &Root) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if root.store != self.id {
			return Err(err(ErrorKind::InvalidHandle));
		}
		if let Value::Heap(h) = root.value {
			self.slot(h)?;
		}
		Ok(root.value)
	}
	/// Brand a value this store produced. Crate-private: the only way a `Root`
	/// comes into existence.
	pub(crate) fn export(&self, value: Value) -> Root {
		Root {
			store: self.id,
			value,
		}
	}
	/// Owner-check every root, allocating nothing; the first refusal wins.
	pub(crate) fn import_all<'r>(&self, roots: impl IntoIterator<Item = &'r Root>) -> Result<()> {
		for root in roots {
			self.import(root)?;
		}
		Ok(())
	}
	/// Would a payload of `bytes` be admitted now? The same arithmetic as
	/// `admit`, run before any scratch allocation.
	fn admission_room(&self, bytes: usize) -> Result<()> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let total = self
			.bytes
			.checked_add(bytes)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if total > self.limits.bytes
			|| self.live == self.limits.entries
			|| (self.free.is_empty() && self.slots.len() == self.limits.entries)
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		Ok(())
	}
	// Scalars occupy no entry; the brand still makes them owner-checked.
	/// A root of `()`.
	pub fn root_unit(&self) -> Root {
		self.export(Value::Unit)
	}
	/// A root of a `bool`.
	pub fn root_bool(&self, value: bool) -> Root {
		self.export(Value::Bool(value))
	}
	/// A root of an `i64`.
	pub fn root_i64(&self, value: i64) -> Root {
		self.export(Value::I64(value))
	}
	/// A root of a `u64`.
	pub fn root_u64(&self, value: u64) -> Root {
		self.export(Value::U64(value))
	}
	/// A root of an `f64`.
	pub fn root_f64(&self, value: f64) -> Root {
		self.export(Value::F64(value))
	}
	/// A root of a `char`.
	pub fn root_char(&self, value: char) -> Root {
		self.export(Value::Char(value))
	}
	/// `Some` for a scalar, `None` for a heap value, function or type token.
	pub fn root_scalar(&self, root: &Root) -> Result<Option<Scalar>> {
		Ok(Some(match self.import(root)? {
			Value::Unit => Scalar::Unit,
			Value::Bool(b) => Scalar::Bool(b),
			Value::I64(n) => Scalar::I64(n),
			Value::U64(n) => Scalar::U64(n),
			Value::F64(n) => Scalar::F64(n),
			Value::Char(c) => Scalar::Char(c),
			Value::Heap(_) | Value::Function { .. } | Value::Type(_) => return Ok(None),
		}))
	}
	/// The type of the value.
	pub fn root_tag(&self, root: &Root) -> Result<Type> {
		let value = self.import(root)?;
		self.tag(value)
	}
	/// Whether the value is a function token. A host can only pass a token back to an
	/// invocation of the program that made it.
	pub fn root_is_function(&self, root: &Root) -> Result<bool> {
		Ok(matches!(self.import(root)?, Value::Function { .. }))
	}
	/// A new string entry holding a copy of `text`.
	pub fn root_string(&mut self, text: &str) -> Result<Root> {
		let value = self.string(text)?;
		Ok(self.export(value))
	}
	/// A new vector (`tag` [`Type::Vec`]) or tuple ([`Type::Tuple`]) holding `values`.
	/// The list retains each element; the caller keeps its own roots. Every root is
	/// checked and capacity admitted before anything is retained.
	pub fn root_list(&mut self, tag: Type, values: &[&Root]) -> Result<Root> {
		if !matches!(tag, Type::Vec | Type::Tuple) {
			return Err(err(ErrorKind::Type));
		}
		let bytes = values
			.len()
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.admission_room(bytes)?;
		self.import_all(values.iter().copied())?;
		// Scratch of exactly the checked payload size; `list` admits its own copy.
		let mut copy = Vec::try_with_capacity(values.len())?;
		for root in values {
			copy.try_push(root.value)?;
		}
		let value = self.list(tag, &copy)?;
		Ok(self.export(value))
	}
	/// A new record holding `fields`, which it retains. `None` constructs an anonymous
	/// object; `Some` a struct whose nominal must be one the interner issued (from
	/// [`root_tag`](Self::root_tag) of a struct value). Duplicate keys are refused with
	/// [`ErrorKind::InvalidCode`].
	pub fn root_record(
		&mut self,
		nominal: Option<Nominal>,
		fields: &[(&str, &Root)],
	) -> Result<Root> {
		let tag = match nominal {
			None => Type::Object,
			Some(n) if nominal::contains(n) => Type::Struct(n),
			Some(_) => return Err(err(ErrorKind::Type)),
		};
		// The same preflight `record` runs (duplicate keys, payload bytes), here
		// without any allocation, then the roots.
		let mut bytes = fields
			.len()
			.checked_mul(size_of::<(String, Value)>())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		for (i, (key, _)) in fields.iter().enumerate() {
			if fields[..i].iter().any(|(prior, _)| prior == key) {
				return Err(err(ErrorKind::InvalidCode));
			}
			bytes = bytes
				.checked_add(key.len())
				.ok_or_else(|| err(ErrorKind::Overflow))?;
		}
		self.admission_room(bytes)?;
		self.import_all(fields.iter().map(|(_, root)| *root))?;
		// Scratch: one (&str, Value) per field, smaller than the checked payload.
		let mut copy = Vec::try_with_capacity(fields.len())?;
		for (key, root) in fields {
			copy.try_push((*key, root.value))?;
		}
		let value = self.record(tag, &copy)?;
		Ok(self.export(value))
	}
	/// A new `Option` or `Result` value. For `Option`, `success` means `Some` and requires
	/// a payload; `None` takes none. A `Result` (`Ok` when `success`) requires a payload.
	/// The payload is retained.
	pub fn root_variant(
		&mut self,
		tag: Type,
		success: bool,
		payload: Option<&Root>,
	) -> Result<Root> {
		let payload = match payload {
			Some(root) => Some(self.import(root)?),
			None => None,
		};
		let value = self.variant(tag, success, payload)?;
		Ok(self.export(value))
	}
	/// The type, the success flag and whether a payload is present, for an `Option` or
	/// `Result` value.
	pub fn root_variant_parts(&self, root: &Root) -> Result<(Type, bool, bool)> {
		let value = self.import(root)?;
		let (tag, success, payload) = self.variant_parts(value)?;
		Ok((tag, success, payload.is_some()))
	}
	/// The text of a string value.
	pub fn root_text(&self, root: &Root) -> Result<&str> {
		let value = self.import(root)?;
		self.text(value)
	}
	/// The length of a string (in bytes), vector or tuple.
	pub fn root_len(&self, root: &Root) -> Result<usize> {
		let value = self.import(root)?;
		self.len(value)
	}
	/// Retained read: the returned root is a new owner of the element.
	pub fn root_get(&mut self, root: &Root, index: usize) -> Result<Root> {
		let value = self.import(root)?;
		let element = self.get(value, index)?;
		Ok(self.export(element))
	}
	/// Replaces element `index` of a vector or tuple with `new`, which the container
	/// retains.
	pub fn root_set(&mut self, root: &Root, index: usize, new: &Root) -> Result<()> {
		let value = self.import(root)?;
		let new = self.import(new)?;
		self.set(value, index, new)
	}
	/// Appends `new` to a vector, which retains it.
	pub fn root_push(&mut self, root: &Root, new: &Root) -> Result<()> {
		let value = self.import(root)?;
		let new = self.import(new)?;
		self.push(value, new)
	}
	/// Retained read: the returned root is a new owner of the field value.
	pub fn root_field(&mut self, root: &Root, key: &str) -> Result<Root> {
		let value = self.import(root)?;
		let field = self.field(value, key)?;
		Ok(self.export(field))
	}
	/// Replaces the existing field `key` of a record with `new`, which the record retains.
	pub fn root_set_field(&mut self, root: &Root, key: &str, new: &Root) -> Result<()> {
		let value = self.import(root)?;
		let new = self.import(new)?;
		self.set_field(value, key, new)
	}
	/// The value formatted as `{}` (`debug` false) or `{:?}` (`debug` true) would format
	/// it in a script. A value without a display form is a [`ErrorKind::Type`] error.
	pub fn root_format(&self, root: &Root, debug: bool) -> Result<String> {
		let value = self.import(root)?;
		self.format_value(value, debug)
	}
	/// An independently counted owner (strong + 1 for a heap value).
	pub fn root_retain(&mut self, root: &Root) -> Result<Root> {
		let value = self.import(root)?;
		self.retain(value)?;
		Ok(self.export(value))
	}
	/// Consumes the root only once its edge is released. A refusal before
	/// that (foreign or closed store, retired generation, or a last release
	/// rejected by its retirement checks) hands the unchanged root back so the
	/// only owner cannot be lost; an error after the edge was released
	/// consumes it (first error, drained as far as possible).
	pub fn root_release(&mut self, root: Root) -> core::result::Result<(), ReleaseError> {
		let value = match self.import(&root) {
			Ok(value) => value,
			Err(error) => return Err(ReleaseError::Refused { root, error }),
		};
		match self.release_owned(value) {
			Ok(()) => Ok(()),
			Err((false, error)) => Err(ReleaseError::Refused { root, error }),
			Err((true, error)) => Err(ReleaseError::Failed(error)),
		}
	}
	/// Drops the payload of a heap value now, releasing its children, while its entry
	/// stays live for its owners. Every later read through any alias is refused with
	/// [`ErrorKind::AccessConflict`]. A no-op for scalars.
	pub fn root_invalidate(&mut self, root: &Root) -> Result<()> {
		let value = self.import(root)?;
		self.invalidate(value)
	}
}
