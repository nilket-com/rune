//! The fresh-core heap: a bounded, generational [`Store`].
//!
//! Values inside a store refer to each other by handle, never by Rust-owned child trees.
//! Cleanup uses storage reserved up front; reference cycles stay charged until the store
//! is closed or dropped. Hosts reach the store only through [`Root`](super::value::Root)s
//! (see the `root_*` methods).
use super::error::{Error, ErrorKind, Result};
use super::value::{Handle, Type, Value};
use crate::alloc::{String, Vec};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

mod formatting;
mod roots;

#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod retain_inline_tests;
#[cfg(test)]
mod scalar_release_tests;

static NEXT_STORE: AtomicU64 = AtomicU64::new(1);
const MAX_ENTRIES: usize = 65_536;
const MAX_BYTES: usize = 16 << 20;
/// Capacity limits of a [`Store`].
///
/// The default is the maximum: 65,536 entries and 16 MiB of payload.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
	/// The maximum number of live heap entries (1 to 65,536).
	pub entries: usize,
	/// The maximum payload bytes charged to live entries (at most 16 MiB).
	pub bytes: usize,
}
impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: MAX_ENTRIES,
			bytes: MAX_BYTES,
		}
	}
}
#[derive(Debug)]
enum Payload {
	String(String),
	Variant {
		tag: Type,
		success: bool,
		payload: Option<Value>,
	},
	Range {
		start: i64,
		end: i64,
	},
	VectorIterator(Iter),
	RangeIterator {
		next: i64,
		end: i64,
	},
	Record {
		tag: Type,
		fields: Vec<(String, Value)>,
	},
	List {
		tag: Type,
		values: Vec<Value>,
	},
}
impl Payload {
	fn bytes(&self) -> Result<usize> {
		let bytes = match self {
			Self::String(s) => Some(s.capacity()),
			Self::Range { .. } => Some(16),
			Self::Variant { .. } => Some(size_of::<Value>()),
			Self::VectorIterator(..) | Self::RangeIterator { .. } => Some(size_of::<Value>()),
			Self::List { values, .. } => values.capacity().checked_mul(size_of::<Value>()),
			Self::Record { fields, .. } => fields
				.capacity()
				.checked_mul(size_of::<(String, Value)>())
				.and_then(|n| {
					fields
						.iter()
						.try_fold(n, |n, (key, _)| n.checked_add(key.capacity()))
				}),
		};
		bytes.ok_or_else(|| err(ErrorKind::Overflow))
	}
	/// Inert child handles in construction order. They acquire ownership only
	/// when admission retains them; dropping an unpublished payload owns none.
	fn children(&self) -> impl Iterator<Item = Value> + '_ {
		let mut index = 0;
		core::iter::from_fn(move || {
			let value = match self {
				Self::Variant { payload, .. } if index == 0 => *payload,
				Self::VectorIterator(iter) if index == 0 => Some(Value::Heap(iter.handle)),
				Self::Record { fields, .. } => fields.get(index).map(|(_, value)| *value),
				Self::List { values, .. } => values.get(index).copied(),
				_ => None,
			};
			index += usize::from(value.is_some());
			value
		})
	}
	fn tag(&self) -> Type {
		match self {
			Self::String(_) => Type::String,
			Self::Range { .. } => Type::Range,
			Self::Variant { tag, .. } => *tag,
			Self::VectorIterator(..) | Self::RangeIterator { .. } => Type::Iterator,
			Self::List { tag, .. } | Self::Record { tag, .. } => *tag,
		}
	}
}
#[derive(Debug)]
struct Entry {
	previous: Option<u32>,
	next: Option<u32>,
	strong: u32,
	readers: u32,
	// Actual admitted buffer capacity; released exactly once, never recomputed
	// from a payload after mutation or invalidation.
	charged_bytes: usize,
	payload: Option<Payload>,
}
#[derive(Debug)]
struct Slot {
	generation: u32,
	entry: Option<Entry>,
}
/// An iteration token owns a retained root and holds a logical shared lease.
/// It exposes no Rust borrow and must be closed by the executor on all exits.
#[derive(Debug)]
pub(crate) struct Iter {
	handle: Handle,
	index: usize,
}
/// A bounded heap for one runtime (not `Send`), with a process-unique identity.
///
/// Scalars occupy no entry. Strings, vectors, tuples, objects, structs and variants are
/// reference-counted entries, charged against the store's [`Limits`]. Every root a host
/// passes in is checked against the store's identity before it is read or retained.
pub struct Store {
	id: u64,
	closed: bool,
	limits: Limits,
	slots: Vec<Slot>,
	free: Vec<u32>,
	work: Vec<Value>,
	first: Option<u32>,
	last: Option<u32>,
	live: usize,
	bytes: usize,
	// The experiment is local to one runtime. No accidental cross-thread host use.
	_local: core::marker::PhantomData<std::rc::Rc<()>>,
}
fn err(kind: ErrorKind) -> Error {
	Error::new(kind, "store")
}
impl Store {
	/// A new, empty store. Reserves its bookkeeping for `limits` up front; limits of zero
	/// entries or above the maximum are refused with [`ErrorKind::StoreCapacity`].
	pub fn new(limits: Limits) -> Result<Self> {
		if limits.entries == 0 || limits.entries > MAX_ENTRIES || limits.bytes > MAX_BYTES {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let id = NEXT_STORE
			.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
			.map_err(|_| err(ErrorKind::StoreCapacity))?;
		// Every pending release came from an admitted edge, plus the initial root.
		// Reserve this maximum before any payload exists, so destruction cannot grow it.
		let edges = limits.bytes / size_of::<Value>();
		let work_capacity = edges
			.checked_add(1)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		Ok(Self {
			id,
			closed: false,
			limits,
			slots: Vec::try_with_capacity(limits.entries)?,
			free: Vec::try_with_capacity(limits.entries)?,
			work: Vec::try_with_capacity(work_capacity)?,
			first: None,
			last: None,
			live: 0,
			bytes: 0,
			_local: core::marker::PhantomData,
		})
	}
	/// Whether [`close`](Self::close) has run.
	pub fn is_closed(&self) -> bool {
		self.closed
	}
	/// The number of live heap entries.
	pub fn live_entries(&self) -> usize {
		self.live
	}
	/// The payload bytes charged to live entries.
	pub fn payload_bytes(&self) -> usize {
		self.bytes
	}
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn reserved_metadata_bytes(&self) -> usize {
		self.slots.capacity() * size_of::<Slot>()
			+ self.free.capacity() * size_of::<u32>()
			+ self.work.capacity() * size_of::<Value>()
	}
	fn slot(&self, h: Handle) -> Result<&Slot> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		self.slots
			.get(h.slot as usize)
			.filter(|s| s.generation == h.generation && s.entry.is_some())
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}
	fn entry(&self, h: Handle) -> Result<&Entry> {
		self.slot(h)?
			.entry
			.as_ref()
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}
	fn entry_mut(&mut self, h: Handle) -> Result<&mut Entry> {
		self.slot(h)?;
		self.slots[h.slot as usize]
			.entry
			.as_mut()
			.ok_or_else(|| err(ErrorKind::InvalidHandle))
	}
	fn payload(&self, h: Handle) -> Result<&Payload> {
		self.entry(h)?
			.payload
			.as_ref()
			.ok_or_else(|| err(ErrorKind::AccessConflict))
	}
	fn mutable(&mut self, h: Handle) -> Result<&mut Payload> {
		let e = self.entry_mut(h)?;
		if e.readers != 0 {
			return Err(err(ErrorKind::AccessConflict));
		}
		e.payload
			.as_mut()
			.ok_or_else(|| err(ErrorKind::AccessConflict))
	}
	/// Undo only references acquired for an unpublished payload. Each acquired
	/// heap edge still has its original owner, so this cannot destroy a value,
	/// enqueue cleanup, allocate, or invoke user code (duplicates included).
	fn undo_admission_edges(&mut self, payload: &Payload, retained: usize) {
		for value in payload.children().take(retained) {
			if let Value::Heap(h) = value {
				let entry = self.slots[h.slot as usize]
					.entry
					.as_mut()
					.expect("successfully retained admission edge stays live");
				debug_assert!(entry.strong >= 2);
				entry.strong -= 1;
			}
		}
	}
	fn admit(&mut self, payload: Payload) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let charged_bytes = payload.bytes()?;
		let bytes = self
			.bytes
			.checked_add(charged_bytes)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if bytes > self.limits.bytes
			|| self.live == self.limits.entries
			|| (self.free.is_empty() && self.slots.len() == self.limits.entries)
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		// Validate the links and the reusable slot before acquiring any edges.
		if let Some(last) = self.last {
			if self
				.slots
				.get(last as usize)
				.and_then(|s| s.entry.as_ref())
				.is_none_or(|e| e.next.is_some())
			{
				return Err(err(ErrorKind::InvalidHandle));
			}
		}
		let readers = if let Payload::VectorIterator(iter) = &payload {
			if self.payload(iter.handle)?.tag() != Type::Vec {
				return Err(err(ErrorKind::Type));
			}
			Some(
				self.entry(iter.handle)?
					.readers
					.checked_add(1)
					.ok_or_else(|| err(ErrorKind::Overflow))?,
			)
		} else {
			None
		};
		let reused = self.free.last().copied();
		let index = if let Some(index) = reused {
			if self
				.slots
				.get(index as usize)
				.is_none_or(|slot| slot.entry.is_some())
			{
				return Err(err(ErrorKind::InvalidHandle));
			}
			index
		} else {
			let index =
				u32::try_from(self.slots.len()).map_err(|_| err(ErrorKind::StoreCapacity))?;
			// Reserve the empty metadata slot before retaining a child. On failure
			// it is removed; neither live count nor allocation-order links changed.
			self.slots.try_push(Slot {
				generation: 1,
				entry: None,
			})?;
			index
		};
		// No release may be called in this loop: rollback relies on original owners staying live.
		for (retained, value) in payload.children().enumerate() {
			if let Err(error) = self.retain(value) {
				self.undo_admission_edges(&payload, retained);
				if reused.is_none() {
					self.slots.pop();
				}
				return Err(error);
			}
		}
		// No fallible operation remains. No external code ran during preparation,
		// so the validated links, slot, child roots and lease are unchanged.
		if let Payload::VectorIterator(iter) = &payload {
			self.slots[iter.handle.slot as usize]
				.entry
				.as_mut()
				.expect("validated iterator owner stays live")
				.readers = readers.expect("iterator readers checked");
		}
		if reused.is_some() {
			self.free.pop();
		}
		self.slots[index as usize].entry = Some(Entry {
			previous: self.last,
			next: None,
			strong: 1,
			readers: 0,
			charged_bytes,
			payload: Some(payload),
		});
		if let Some(last) = self.last {
			self.slots[last as usize]
				.entry
				.as_mut()
				.expect("validated allocation-order tail stays live")
				.next = Some(index);
		} else {
			self.first = Some(index);
		}
		self.last = Some(index);
		self.live += 1;
		self.bytes = bytes;
		Ok(Value::Heap(Handle {
			slot: index,
			generation: self.slots[index as usize].generation,
		}))
	}
	pub(crate) fn string(&mut self, text: &str) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if text.len() > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let mut s = String::try_with_capacity(text.len())?;
		s.try_push_str(text)?;
		self.admit(Payload::String(s))
	}
	/// Admit the renderer's already bounded buffer without a second text copy.
	pub(crate) fn string_owned(&mut self, text: String) -> Result<Value> {
		self.admit(Payload::String(text))
	}
	pub(crate) fn list(&mut self, tag: Type, values: &[Value]) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if !matches!(tag, Type::Vec | Type::Tuple) {
			return Err(err(ErrorKind::Type));
		}
		let bytes = values
			.len()
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if bytes > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let mut copy = Vec::try_with_capacity(values.len())?;
		for &v in values {
			copy.try_push(v)?;
		}
		self.admit(Payload::List { tag, values: copy })
	}

	/// Construct only declared fields. No key insertion or implicit protocol call.
	pub(crate) fn record(&mut self, tag: Type, fields: &[(&str, Value)]) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if !matches!(tag, Type::Object | Type::Struct(_)) {
			return Err(err(ErrorKind::Type));
		}
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
		if bytes > self.limits.bytes.saturating_sub(self.bytes)
			|| self.live == self.limits.entries
			|| (self.free.is_empty() && self.slots.len() == self.limits.entries)
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		// Allocate every key before acquiring any child root. Later retain failures
		// release only the successfully retained prefix, without recursive cleanup.
		let mut copy = Vec::try_with_capacity(fields.len())?;
		for (key, value) in fields {
			copy.try_push((String::try_from(*key)?, *value))?;
		}
		self.record_owned(tag, copy)
	}
	fn record_owned(&mut self, tag: Type, copy: Vec<(String, Value)>) -> Result<Value> {
		self.admit(Payload::Record { tag, fields: copy })
	}
	pub(crate) fn field(&mut self, value: Value, key: &str) -> Result<Value> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		let Payload::Record { fields, .. } = self.payload(handle)? else {
			return Err(err(ErrorKind::Type));
		};
		let value = fields
			.iter()
			.find(|(name, _)| name.as_str() == key)
			.map(|(_, value)| *value)
			.ok_or_else(|| err(ErrorKind::IndexOutOfBounds))?;
		self.retain(value)?;
		Ok(value)
	}
	pub(crate) fn set_field(&mut self, value: Value, key: &str, new: Value) -> Result<()> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		let Payload::Record { fields, .. } = self.mutable(handle)? else {
			return Err(err(ErrorKind::Type));
		};
		let index = fields
			.iter()
			.position(|(name, _)| name.as_str() == key)
			.ok_or_else(|| err(ErrorKind::IndexOutOfBounds))?;
		self.retain(new)?;
		let Payload::Record { fields, .. } = self.mutable(handle)? else {
			unreachable!()
		};
		let old = core::mem::replace(&mut fields[index].1, new);
		self.release(old)
	}

	pub(crate) fn clone_outer(&mut self, value: Value) -> Result<Value> {
		let Value::Heap(h) = value else {
			self.retain(value)?;
			return Ok(value);
		};
		match self.payload(h)? {
			Payload::String(text) => {
				if text.len() > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				let copy = String::try_from(text.as_str())?;
				self.admit(Payload::String(copy))
			}
			Payload::Record { tag, fields } => {
				let tag = *tag;
				let bytes = fields
					.len()
					.checked_mul(size_of::<(String, Value)>())
					.and_then(|n| {
						fields
							.iter()
							.try_fold(n, |n, (key, _)| n.checked_add(key.len()))
					})
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				if bytes > self.limits.bytes.saturating_sub(self.bytes)
					|| self.live == self.limits.entries
					|| (self.free.is_empty() && self.slots.len() == self.limits.entries)
				{
					return Err(err(ErrorKind::StoreCapacity));
				}
				let mut keys = Vec::try_with_capacity(fields.len())?;
				for (key, value) in fields {
					keys.try_push((String::try_from(key.as_str())?, *value))?;
				}
				self.record_owned(tag, keys)
			}
			Payload::List { tag, values } => {
				let tag = *tag;
				let bytes = values
					.len()
					.checked_mul(size_of::<Value>())
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				if bytes > self.limits.bytes.saturating_sub(self.bytes) {
					return Err(err(ErrorKind::StoreCapacity));
				}
				let mut copy = Vec::try_with_capacity(values.len())?;
				for &v in values {
					copy.try_push(v)?;
				}
				self.list(tag, &copy)
			}
			_ => Err(err(ErrorKind::Type)),
		}
	}
	pub(crate) fn append_string(&mut self, target: Value, rhs: Value) -> Result<()> {
		let Value::Heap(h) = target else {
			return Err(err(ErrorKind::Type));
		};
		let (len, cap) = match self.mutable(h)? {
			Payload::String(s) => (s.len(), s.capacity()),
			_ => return Err(err(ErrorKind::Type)),
		};
		let suffix = self.text(rhs)?;
		let new_len = len
			.checked_add(suffix.len())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if new_len.saturating_sub(cap) > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		if new_len > cap {
			// Grow off-store. An allocator may round capacity up: account the
			// returned capacity before publishing, rather than a guessed delta.
			let mut replacement = String::try_with_capacity(new_len)?;
			replacement.try_push_str(self.text(target)?)?;
			replacement.try_push_str(self.text(rhs)?)?;
			let charged_bytes = replacement.capacity();
			let bytes = self.replacement_bytes(h, charged_bytes)?;
			let entry = self.entry_mut(h)?;
			entry.payload = Some(Payload::String(replacement));
			entry.charged_bytes = charged_bytes;
			self.bytes = bytes;
			return Ok(());
		}
		// Bounded temporary is necessary when rhs aliases target. No borrowed
		// script reference crosses mutation; the store contains only owned buffers.
		let copy = String::try_from(suffix)?;
		match self.mutable(h)? {
			Payload::String(s) => {
				s.try_push_str(&copy)?;
			}
			_ => unreachable!(),
		}
		Ok(())
	}
	fn replacement_bytes(&self, h: Handle, charged_bytes: usize) -> Result<usize> {
		let bytes = self
			.bytes
			.checked_sub(self.entry(h)?.charged_bytes)
			.and_then(|n| n.checked_add(charged_bytes))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if bytes > self.limits.bytes {
			return Err(err(ErrorKind::StoreCapacity));
		}
		Ok(bytes)
	}
	pub(crate) fn concat_string(&mut self, a: Value, b: Value) -> Result<Value> {
		let left = self.text(a)?;
		let right = self.text(b)?;
		let len = left
			.len()
			.checked_add(right.len())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if len > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let mut text = String::try_with_capacity(len)?;
		text.try_push_str(left)?;
		text.try_push_str(right)?;
		self.admit(Payload::String(text))
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
	pub(crate) fn tag(&self, value: Value) -> Result<Type> {
		match value {
			Value::Heap(h) => Ok(self.payload(h)?.tag()),
			_ => value.primitive_type().ok_or_else(|| err(ErrorKind::Type)),
		}
	}
	pub(crate) fn text(&self, value: Value) -> Result<&str> {
		match value {
			Value::Heap(h) => match self.payload(h)? {
				Payload::String(s) => Ok(s.as_str()),
				_ => Err(err(ErrorKind::Type)),
			},
			_ => Err(err(ErrorKind::Type)),
		}
	}
	pub(crate) fn len(&self, value: Value) -> Result<usize> {
		match value {
			Value::Heap(h) => match self.payload(h)? {
				Payload::String(s) => Ok(s.len()),
				Payload::List { values, .. } => Ok(values.len()),
				_ => Err(err(ErrorKind::Type)),
			},
			_ => Err(err(ErrorKind::Type)),
		}
	}
	pub(crate) fn get(&mut self, value: Value, index: usize) -> Result<Value> {
		let v = match value {
			Value::Heap(h) => match self.payload(h)? {
				Payload::List { values, .. } => *values
					.get(index)
					.ok_or_else(|| err(ErrorKind::IndexOutOfBounds))?,
				_ => return Err(err(ErrorKind::Type)),
			},
			_ => return Err(err(ErrorKind::Type)),
		};
		self.retain(v)?;
		Ok(v)
	}
	pub(crate) fn set(&mut self, value: Value, index: usize, new: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};
		match self.mutable(h)? {
			Payload::List { values, .. } if index < values.len() => (),
			Payload::List { .. } => return Err(err(ErrorKind::IndexOutOfBounds)),
			_ => return Err(err(ErrorKind::Type)),
		};
		self.retain(new)?;
		let old = match self.mutable(h)? {
			Payload::List { values, .. } => core::mem::replace(&mut values[index], new),
			_ => unreachable!(),
		};
		self.release(old)
	}
	pub(crate) fn push(&mut self, value: Value, new: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};
		let (len, cap) = match self.mutable(h)? {
			Payload::List {
				tag: Type::Vec,
				values,
			} => (values.len(), values.capacity()),
			_ => return Err(err(ErrorKind::Type)),
		};
		if len == cap {
			let next = cap
				.checked_mul(2)
				.ok_or_else(|| err(ErrorKind::Overflow))?
				.max(1);
			let planned = next
				.checked_mul(size_of::<Value>())
				.ok_or_else(|| err(ErrorKind::Overflow))?;
			self.replacement_bytes(h, planned)?;
			let mut replacement = Vec::try_with_capacity(next)?;
			let Payload::List { values, .. } = self.payload(h)? else {
				unreachable!()
			};
			for &value in values {
				replacement.try_push(value)?;
			}
			replacement.try_push(new)?;
			let charged_bytes = replacement
				.capacity()
				.checked_mul(size_of::<Value>())
				.ok_or_else(|| err(ErrorKind::Overflow))?;
			let bytes = self.replacement_bytes(h, charged_bytes)?;
			// Existing edges transfer to the replacement without another retain.
			// All allocations and accounting checks precede the new edge retain.
			self.retain(new)?;
			let entry = self.entry_mut(h)?;
			entry.payload = Some(Payload::List {
				tag: Type::Vec,
				values: replacement,
			});
			entry.charged_bytes = charged_bytes;
			self.bytes = bytes;
			return Ok(());
		}
		self.retain(new)?;
		match self.mutable(h)? {
			Payload::List { values, .. } => values.try_push(new)?,
			_ => unreachable!(),
		};
		Ok(())
	}
	pub(crate) fn variant(
		&mut self,
		tag: Type,
		success: bool,
		payload: Option<Value>,
	) -> Result<Value> {
		if !matches!(tag, Type::Option | Type::Result)
			|| tag == Type::Result && payload.is_none()
			|| tag == Type::Option && success != payload.is_some()
		{
			return Err(err(ErrorKind::Type));
		}
		self.admit(Payload::Variant {
			tag,
			success,
			payload,
		})
	}
	pub(crate) fn variant_parts(&self, value: Value) -> Result<(Type, bool, Option<Value>)> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};
		match self.payload(h)? {
			Payload::Variant {
				tag,
				success,
				payload,
			} => Ok((*tag, *success, *payload)),
			_ => Err(err(ErrorKind::Type)),
		}
	}
	pub(crate) fn range(&mut self, start: i64, end: i64) -> Result<Value> {
		self.admit(Payload::Range { start, end })
	}
	pub(crate) fn iterator(&mut self, value: Value) -> Result<Value> {
		if let Value::Heap(h) = value {
			if let Payload::Range { start, end } = self.payload(h)? {
				return self.admit(Payload::RangeIterator {
					next: *start,
					end: *end,
				});
			}
		}
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		// Admission acquires the retained root and shared lease together only
		// after every capacity and metadata check; no temporary lease escapes.
		self.admit(Payload::VectorIterator(Iter { handle, index: 0 }))
	}
	pub(crate) fn iterator_next(&mut self, value: Value) -> Result<Option<Value>> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};
		match self.payload(h)? {
			Payload::RangeIterator { next, end } => {
				if next >= end {
					return Ok(None);
				}
				let value = *next;
				let new = value
					.checked_add(1)
					.ok_or_else(|| err(ErrorKind::Overflow))?;
				let Payload::RangeIterator { next, .. } = self.mutable(h)? else {
					return Err(err(ErrorKind::Type));
				};
				*next = new;
				Ok(Some(Value::I64(value)))
			}
			Payload::VectorIterator(iter) => {
				let root = Value::Heap(iter.handle);
				let index = iter.index;
				if index == self.len(root)? {
					return Ok(None);
				}
				let value = self.get(root, index)?;
				let Payload::VectorIterator(iter) = self.mutable(h)? else {
					return Err(err(ErrorKind::Type));
				};
				iter.index += 1;
				Ok(Some(value))
			}
			_ => Err(err(ErrorKind::Type)),
		}
	}
	// Lease-token iteration: exercised by the crate's own tests, not yet by the executor.
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn begin_iter(&mut self, value: Value) -> Result<Iter> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::Type));
		};
		if self.payload(h)?.tag() != Type::Vec {
			return Err(err(ErrorKind::Type));
		}
		let readers = self
			.entry(h)?
			.readers
			.checked_add(1)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.retain(value)?;
		self.entry_mut(h)?.readers = readers;
		Ok(Iter {
			handle: h,
			index: 0,
		})
	}
	// Lease-token iteration: exercised by the crate's own tests, not yet by the executor.
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn next(&mut self, iter: &mut Iter) -> Result<Option<Value>> {
		let len = self.len(Value::Heap(iter.handle))?;
		if iter.index == len {
			return Ok(None);
		}
		let v = self.get(Value::Heap(iter.handle), iter.index)?;
		iter.index += 1;
		Ok(Some(v))
	}
	// Lease-token iteration: exercised by the crate's own tests, not yet by the executor.
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn end_iter(&mut self, iter: Iter) -> Result<()> {
		let e = self.entry_mut(iter.handle)?;
		e.readers = e
			.readers
			.checked_sub(1)
			.ok_or_else(|| err(ErrorKind::InvalidHandle))?;
		self.release(Value::Heap(iter.handle))
	}
	fn children_room(&self, payload: &Payload) -> Result<()> {
		let count = payload
			.children()
			.filter(|v| matches!(v, Value::Heap(_)))
			.count();
		if count > self.work.capacity().saturating_sub(self.work.len()) {
			// Every admitted edge paid at least sizeof(Value). Failure here is an
			// internal invariant error, not permission to allocate during cleanup.
			return Err(err(ErrorKind::InvalidCode));
		}
		Ok(())
	}
	/// Queue every owned heap edge without allocation. The capacity was checked
	/// before taking the payload. A malformed iterator lease reports an error
	/// but must still queue its retained owner root for drainage.
	fn enqueue_children(&mut self, payload: Payload) -> Result<()> {
		let mut failure = None;
		if let Payload::VectorIterator(iter) = &payload {
			let result = (|| {
				let entry = self.entry_mut(iter.handle)?;
				entry.readers = entry
					.readers
					.checked_sub(1)
					.ok_or_else(|| err(ErrorKind::InvalidHandle))?;
				Ok(())
			})();
			if let Err(error) = result {
				failure = Some(error);
			}
		}
		for value in payload.children() {
			if matches!(value, Value::Heap(_)) {
				// Preflight and the store-wide edge bound prove spare capacity.
				self.work
					.try_push(value)
					.expect("release work was pre-reserved and checked");
			}
		}
		failure.map_or(Ok(()), Err)
	}
	/// Process one release. Validate retirement bookkeeping before changing a
	/// last reference; malformed handles/links leave this entry untouched.
	fn release_one(&mut self, h: Handle) -> Result<()> {
		let entry = self.entry(h)?;
		let next = entry
			.strong
			.checked_sub(1)
			.ok_or_else(|| err(ErrorKind::InvalidHandle))?;
		if next == 0 && entry.readers != 0 {
			return Err(err(ErrorKind::AccessConflict));
		}
		if next != 0 {
			self.entry_mut(h)?.strong = next;
			return Ok(());
		}
		let previous = entry.previous;
		let following = entry.next;
		if let Some(previous) = previous {
			if self
				.slots
				.get(previous as usize)
				.and_then(|s| s.entry.as_ref())
				.is_none_or(|e| e.next != Some(h.slot))
			{
				return Err(err(ErrorKind::InvalidHandle));
			}
		} else if self.first != Some(h.slot) {
			return Err(err(ErrorKind::InvalidHandle));
		}
		if let Some(following) = following {
			if self
				.slots
				.get(following as usize)
				.and_then(|s| s.entry.as_ref())
				.is_none_or(|e| e.previous != Some(h.slot))
			{
				return Err(err(ErrorKind::InvalidHandle));
			}
		} else if self.last != Some(h.slot) {
			return Err(err(ErrorKind::InvalidHandle));
		}
		let bytes = self
			.bytes
			.checked_sub(entry.charged_bytes)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let live = self
			.live
			.checked_sub(1)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let generation = self.slots[h.slot as usize].generation.checked_add(1);
		if generation.is_some() && self.free.len() == self.free.capacity() {
			return Err(err(ErrorKind::InvalidCode));
		}
		if let Some(payload) = &entry.payload {
			self.children_room(payload)?;
		}
		// All retirement checks passed. No fallible structural mutation remains.
		let mut entry = self.slots[h.slot as usize]
			.entry
			.take()
			.expect("validated retiring entry");
		if let Some(previous) = previous {
			self.slots[previous as usize]
				.entry
				.as_mut()
				.expect("validated previous link")
				.next = following;
		} else {
			self.first = following;
		}
		if let Some(following) = following {
			self.slots[following as usize]
				.entry
				.as_mut()
				.expect("validated next link")
				.previous = previous;
		} else {
			self.last = previous;
		}
		self.bytes = bytes;
		self.live = live;
		if let Some(generation) = generation {
			self.slots[h.slot as usize].generation = generation;
			self.free
				.try_push(h.slot)
				.expect("retired slot fits reserved free list");
		} // Exhausted generations retire their slot permanently.
		if let Some(payload) = entry.payload.take() {
			self.enqueue_children(payload)?;
		}
		Ok(())
	}
	/// Always finish the queue. An invalid edge must not strand unrelated owned
	/// edges or hide the first error behind a later one; no recursion/allocation.
	fn drain_work(&mut self, mut failure: Option<Error>) -> Result<()> {
		while let Some(value) = self.work.pop() {
			if let Value::Heap(h) = value {
				if let Err(error) = self.release_one(h) {
					if failure.is_none() {
						failure = Some(error);
					}
				}
			}
		}
		failure.map_or(Ok(()), Err)
	}
	/// A host root's release, reporting whether the root's own edge was consumed:
	/// `Err((false, e))` left the entry untouched (closed store, work queue
	/// full, or a last release refused by its retirement checks); `Err((true,
	/// e))` consumed the edge and a later drainage step failed (first error,
	/// everything drainable drained).
	pub(crate) fn release_owned(
		&mut self,
		value: Value,
	) -> core::result::Result<(), (bool, Error)> {
		if self.closed {
			return Err((false, err(ErrorKind::ClosedStore)));
		}
		let Value::Heap(h) = value else {
			return Ok(());
		};
		if self.work.len() == self.work.capacity() {
			return Err((false, err(ErrorKind::InvalidCode)));
		}
		if let Err(error) = self.release_one(h) {
			// release_one validates before changing a last reference and a
			// non-last decrement cannot fail; but a last release can fail AFTER
			// retiring the entry (a malformed lease found while enqueuing its
			// children, whose retained owner root is already queued). Whether
			// the edge was consumed is read off the store, not assumed: a
			// still-live entry means nothing changed; a retired one means the
			// queued edges must drain, keeping this first error.
			if self.slot(h).is_ok() {
				return Err((false, error));
			}
			return self.drain_work(Some(error)).map_err(|error| (true, error));
		}
		self.drain_work(None).map_err(|error| (true, error))
	}
	#[cfg(test)]
	pub(crate) fn zero_readers_for_test(&mut self, value: Value) {
		let Value::Heap(h) = value else {
			panic!("heap value expected");
		};
		self.entry_mut(h).unwrap().readers = 0;
	}
	#[cfg(test)]
	pub(crate) fn queued_work_for_test(&self) -> usize {
		self.work.len()
	}
	/// Test hook: move a live slot's generation near exhaustion so retirement
	/// at `u32::MAX` is reachable without `u32::MAX` releases.
	#[cfg(test)]
	pub(crate) fn set_generation_for_test(&mut self, value: Value, generation: u32) -> Value {
		let Value::Heap(h) = value else {
			panic!("heap value expected");
		};
		self.slots[h.slot as usize].generation = generation;
		Value::Heap(Handle {
			slot: h.slot,
			generation,
		})
	}
	#[cfg(test)]
	pub(crate) fn free_slots_for_test(&self) -> usize {
		self.free.len()
	}
	/// D13 test hook: queue a pending release without draining it.
	#[cfg(test)]
	pub(crate) fn push_work_for_test(&mut self, value: Value) {
		self.work.try_push(value).unwrap();
	}
	/// D13 test hook: the store's mutable state and accounting (everything but
	/// the process-unique id, the constant limits and the thread marker).
	#[cfg(test)]
	pub(crate) fn state_for_test(&self) -> std::string::String {
		std::format!(
			"{:?}",
			(
				self.closed,
				&self.slots,
				&self.free,
				&self.work,
				self.first,
				self.last,
				self.live,
				self.bytes,
			)
		)
	}
	/// D13: `release_slot` of a scalar is a no-op exactly while the store is
	/// open and no release work is queued.
	#[inline]
	pub(crate) fn scalar_release_is_noop(&self) -> bool {
		!self.closed && self.work.is_empty()
	}
	/// D13: `release_slot`'s scalar classification. Exhaustive with no
	/// wildcard, so a new `Value` variant must be classified here explicitly.
	#[inline]
	pub(crate) fn is_slot_scalar(value: &Value) -> bool {
		match value {
			Value::Unit
			| Value::Bool(_)
			| Value::I64(_)
			| Value::U64(_)
			| Value::F64(_)
			| Value::Char(_)
			| Value::Type(_)
			| Value::Function { .. } => true,
			Value::Heap(_) => false,
		}
	}
	/// Executor slot release: skip only the empty scalar case. Closed stores
	/// and queued work still go through the original error/drainage path.
	#[inline]
	pub(crate) fn release_slot(&mut self, value: Value) -> Result<()> {
		let scalar = match value {
			Value::Unit
			| Value::Bool(_)
			| Value::I64(_)
			| Value::U64(_)
			| Value::F64(_)
			| Value::Char(_)
			| Value::Type(_)
			| Value::Function { .. } => true,
			Value::Heap(_) => false,
		};
		if scalar && !self.closed && self.work.is_empty() {
			Ok(())
		} else {
			self.release(value)
		}
	}
	pub(crate) fn release(&mut self, value: Value) -> Result<()> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if matches!(value, Value::Heap(_)) {
			if self.work.len() == self.work.capacity() {
				return self.drain_work(Some(err(ErrorKind::InvalidCode)));
			}
			self.work
				.try_push(value)
				.expect("initial release root fits reserved work");
		}
		self.drain_work(None)
	}
	pub(crate) fn invalidate(&mut self, value: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			return Ok(());
		};
		self.mutable(h)?;
		let entry = self.entry(h)?;
		let payload = entry
			.payload
			.as_ref()
			.ok_or_else(|| err(ErrorKind::AccessConflict))?;
		self.children_room(payload)?;
		let bytes = self
			.bytes
			.checked_sub(entry.charged_bytes)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let entry = self.entry_mut(h)?;
		let payload = entry.payload.take().expect("validated live payload");
		entry.charged_bytes = 0;
		self.bytes = bytes;
		let failure = self.enqueue_children(payload).err();
		self.drain_work(failure)
	}
	/// Closes the store and frees every entry, including cycles. Closing twice is a
	/// no-op. Refused with [`ErrorKind::AccessConflict`] while an iteration holds an entry.
	/// Afterwards every root of this store is refused with [`ErrorKind::ClosedStore`].
	pub fn close(&mut self) -> Result<()> {
		if self.closed {
			return Ok(());
		}
		if self
			.slots
			.iter()
			.filter_map(|s| s.entry.as_ref())
			.any(|e| e.readers != 0)
		{
			return Err(err(ErrorKind::AccessConflict));
		}
		self.drain();
		Ok(())
	}
	fn drain(&mut self) {
		self.closed = true;
		// Handle edges are inert bits. Do not decrement/follow them while draining.
		while let Some(index) = self.last {
			let Some(entry) = self.slots[index as usize].entry.take() else {
				break;
			};
			self.last = entry.previous;
			// Dropping payload buffers follows no heap edge; Value::Heap is plain bits.
			drop(entry);
		}
		self.first = None;
		self.work.clear();
		self.free.clear();
		self.live = 0;
		self.bytes = 0;
	}
}
impl Drop for Store {
	fn drop(&mut self) {
		self.drain();
	}
}

// Synthetic ownership counts are available only to in-crate contract tests.
#[cfg(test)]
impl Store {
	pub(super) fn test_strong(&self, value: Value) -> u32 {
		let Value::Heap(h) = value else {
			panic!("heap expected")
		};
		self.entry(h).unwrap().strong
	}
	pub(super) fn test_set_strong(&mut self, value: Value, count: u32) {
		let Value::Heap(h) = value else {
			panic!("heap expected")
		};
		self.entry_mut(h).unwrap().strong = count;
	}
}
