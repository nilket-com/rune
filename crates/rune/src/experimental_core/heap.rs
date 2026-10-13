//! The fresh-core heap: a bounded, generational [`Store`].
//!
//! Values inside a store refer to each other by handle, never by Rust-owned child trees.
//! Cleanup uses storage reserved before child edges are published, and never grows
//! bookkeeping itself. Reference cycles stay charged until the store is closed or dropped. Hosts reach the store only through [`Root`](super::value::Root)s
//! (see the `root_*` methods).

use super::error::{Error, ErrorKind, Result};
use super::value::{Handle, Type, Value};
use crate::alloc::{String, Vec};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

mod comparison;
mod formatting;
mod indexing;
pub use formatting::FormatLimits;
mod roots;

#[cfg(feature = "fresh-core-native")]
pub(crate) mod native;

#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod growth_tests;
#[cfg(test)]
mod retain_inline_tests;
#[cfg(test)]
mod scalar_release_tests;

static NEXT_STORE: AtomicU64 = AtomicU64::new(1);

const DEFAULT_ENTRIES: usize = 65_536;
const MAX_ENTRIES: usize = (u32::MAX - 1) as usize;
const INITIAL_ENTRIES: usize = 64;
const INITIAL_WORK: usize = 64;
const DEFAULT_BYTES: usize = 16 << 20;
const MAX_BYTES: usize = isize::MAX as usize;

/// Capacity limits of a [`Store`].
///
/// Defaults are 65,536 entries and 16 MiB of payload. Bookkeeping is
/// separate and grows with use; limits never cause proportional preallocation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
	/// Maximum entry slots, including permanently retired generations (1..=u32::MAX-1).
	pub entries: usize,
	/// Maximum payload bytes charged to live entries (at most isize::MAX).
	pub bytes: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: DEFAULT_ENTRIES,
			bytes: DEFAULT_BYTES,
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
	OptionIterator {
		payload: Option<Value>,
	},
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
			Self::VectorIterator(..) | Self::OptionIterator { .. } | Self::RangeIterator { .. } => {
				Some(size_of::<Value>())
			}
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
				Self::Variant { payload, .. } | Self::OptionIterator { payload } if index == 0 => {
					*payload
				}
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
			Self::VectorIterator(..) | Self::OptionIterator { .. } | Self::RangeIterator { .. } => {
				Type::Iterator
			}
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
	/// Heap child edges held by all admitted payloads (scalars do not count).
	edges: usize,
	first: Option<u32>,
	last: Option<u32>,
	live: usize,
	bytes: usize,
	#[cfg(test)]
	format_capacity_extra: usize,
	// The experiment is local to one runtime. No accidental cross-thread host use.
	_local: core::marker::PhantomData<std::rc::Rc<()>>,
}

fn err(kind: ErrorKind) -> Error {
	Error::new(kind, "store")
}

impl Store {
	/// A new, empty store with small initial bookkeeping reservations. Logical
	/// limits are validated without allocating their maxima; any reservation can
	/// fail with Allocation. Payload bytes exclude metadata and allocator overhead.
	pub fn new(limits: Limits) -> Result<Self> {
		if limits.entries == 0
			|| limits.entries > MAX_ENTRIES
			|| limits.bytes > MAX_BYTES
			|| limits
				.entries
				.checked_mul(size_of::<Slot>())
				.is_none_or(|n| n > isize::MAX as usize)
			|| limits
				.entries
				.checked_mul(size_of::<u32>())
				.is_none_or(|n| n > isize::MAX as usize)
		{
			return Err(err(ErrorKind::StoreCapacity));
		}

		let id = NEXT_STORE
			.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
			.map_err(|_| err(ErrorKind::StoreCapacity))?;
		Ok(Self {
			id,
			closed: false,
			limits,
			slots: Vec::try_with_capacity(limits.entries.min(INITIAL_ENTRIES))?,
			free: Vec::try_with_capacity(limits.entries.min(INITIAL_ENTRIES))?,
			work: Vec::try_with_capacity(INITIAL_WORK)?,
			edges: 0,
			first: None,
			last: None,
			live: 0,
			bytes: 0,
			#[cfg(test)]
			format_capacity_extra: 0,
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

	/// Retained store bookkeeping modeled from actual capacities. Payload bytes,
	/// allocator overhead, executor buffers and formatting are separate. This
	/// is not a transient allocation peak or RSS ceiling.
	pub fn bookkeeping_bytes(&self) -> Result<usize> {
		self.slots
			.capacity()
			.checked_mul(size_of::<Slot>())
			.and_then(|n| n.checked_add(self.free.capacity().checked_mul(size_of::<u32>())?))
			.and_then(|n| n.checked_add(self.work.capacity().checked_mul(size_of::<Value>())?))
			.ok_or_else(|| err(ErrorKind::Overflow))
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn reserved_metadata_bytes(&self) -> usize {
		self.slots.capacity() * size_of::<Slot>()
			+ self.free.capacity() * size_of::<u32>()
			+ self.work.capacity() * size_of::<Value>()
	}

	fn heap_edge(value: Value) -> usize {
		usize::from(matches!(value, Value::Heap(_)))
	}

	fn payload_edges(payload: &Payload) -> usize {
		payload
			.children()
			.filter(|v| matches!(v, Value::Heap(_)))
			.count()
	}

	/// Reserve conservative peak ownership before retaining/publishing anything.
	/// W: work.capacity >= pending releases + live child edges + one root.
	fn reserve_work(&mut self, new_edges: usize, queued: usize) -> Result<()> {
		let needed = self
			.edges
			.checked_add(new_edges)
			.and_then(|n| n.checked_add(self.work.len()))
			.and_then(|n| n.checked_add(queued))
			.and_then(|n| n.checked_add(1))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.work.try_reserve(needed - self.work.len())?;
		self.assert_work();
		Ok(())
	}

	fn assert_work(&self) {
		debug_assert!(self
			.edges
			.checked_add(self.work.len())
			.and_then(|n| n.checked_add(1))
			.is_some_and(|n| n <= self.work.capacity()));
		debug_assert!(self.free.capacity() >= self.slots.len());
	}

	fn replacement_edges(&mut self, old: Value, new: Value) -> Result<usize> {
		let next = self
			.edges
			.checked_sub(Self::heap_edge(old))
			.and_then(|n| n.checked_add(Self::heap_edge(new)))
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		self.reserve_work(Self::heap_edge(new), Self::heap_edge(old))?;
		Ok(next)
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
			return Err(
				err(ErrorKind::AccessConflict).with_detail(super::error::Detail::WriteBorrowed)
			);
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
			if !matches!(
				self.payload(iter.handle)?.tag(),
				Type::Vec | Type::Tuple | Type::Object
			) {
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

		let added_edges = Self::payload_edges(&payload);
		let edges = self
			.edges
			.checked_add(added_edges)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.reserve_work(added_edges, 0)?;
		if self.free.is_empty() {
			let needed = self
				.slots
				.len()
				.checked_add(1)
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
			self.free.try_reserve(needed - self.free.len())?;
			self.slots.try_reserve(1)?;
		}
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
		self.edges = edges;
		self.assert_work();
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

	pub(crate) fn field_get_mut(&mut self, value: Value, key: &str) -> Result<Value> {
		let Value::Heap(handle) = value else {
			return Err(err(if matches!(value, Value::Function { .. }) {
				ErrorKind::Type
			} else {
				ErrorKind::IndexOutOfBounds
			}));
		};
		if !matches!(self.payload(handle)?, Payload::Record { .. }) {
			return Err(err(ErrorKind::Type));
		}
		self.mutable(handle)?;
		let Payload::Record { tag, fields } = self.payload(handle)? else {
			return Err(err(ErrorKind::Type));
		};
		let old = fields
			.iter()
			.find(|(name, _)| name.as_str() == key)
			.map(|(_, value)| *value);
		let old = old.ok_or_else(|| {
			err(if *tag == Type::Object {
				ErrorKind::IndexOutOfBounds
			} else {
				ErrorKind::Type
			})
		})?;
		self.retain(old)?;
		Ok(old)
	}

	pub(crate) fn field_set_mut(&mut self, value: Value, key: &str, new: Value) -> Result<()> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::Record { fields, .. } = self.mutable(handle)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let index = fields
			.iter()
			.position(|(name, _)| name.as_str() == key)
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let old = fields[index].1;
		if old == new {
			return Ok(());
		}
		let edges = self.replacement_edges(old, new)?;
		self.retain(new)?;
		let Payload::Record { fields, .. } = self.mutable(handle)? else {
			unreachable!()
		};
		fields[index].1 = new;
		self.edges = edges;
		self.assert_work();
		self.release(old)
	}

	pub(crate) fn set_field(&mut self, value: Value, key: &str, new: Value) -> Result<()> {
		let Value::Heap(handle) = value else {
			return Err(err(if matches!(value, Value::Function { .. }) {
				ErrorKind::Type
			} else {
				ErrorKind::IndexOutOfBounds
			}));
		};
		if !matches!(self.payload(handle)?, Payload::Record { .. }) {
			return Err(err(ErrorKind::Type));
		}
		self.set_key(value, indexing::RecordKey::Static(key), new)
			.map_err(|error| {
				if matches!(error.detail, super::error::Detail::MissingKey { .. }) {
					// The instruction boundary supplies the static FieldRef. This is
					// never a dynamic missing-key diagnostic owned by Failure.
					err(ErrorKind::IndexOutOfBounds)
				} else {
					error
				}
			})
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

	/// Diagnostic classification only: never invoked on a successful execution path.
	pub(crate) fn diagnostic_type(&self, value: Value) -> Option<super::error::TypeName> {
		use super::error::TypeName;
		let Value::Heap(h) = value else {
			return TypeName::primitive(value);
		};
		Some(match self.payload(h).ok()? {
			Payload::RangeIterator { .. } => TypeName::RangeIter,
			// Internal loop iterators cannot escape. Option/Unit snapshots need
			// distinct diagnostic types before iterator-valued methods are admitted.
			Payload::VectorIterator(_) | Payload::OptionIterator { .. } => TypeName::VecIter,
			payload => TypeName::from_type(payload.tag()),
		})
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

		let old = match self.mutable(h)? {
			Payload::List { values, .. } if index < values.len() => values[index],
			Payload::List { .. } => return Err(err(ErrorKind::IndexOutOfBounds)),
			_ => return Err(err(ErrorKind::Type)),
		};

		let edges = self.replacement_edges(old, new)?;
		self.retain(new)?;
		let old = match self.mutable(h)? {
			Payload::List { values, .. } => core::mem::replace(&mut values[index], new),
			_ => unreachable!(),
		};

		self.edges = edges;
		self.assert_work();
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

		let edges = self
			.edges
			.checked_add(Self::heap_edge(new))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		self.reserve_work(Self::heap_edge(new), 0)?;

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
			self.edges = edges;
			self.assert_work();
			return Ok(());
		}

		self.retain(new)?;
		match self.mutable(h)? {
			Payload::List { values, .. } => values.try_push(new)?,
			_ => unreachable!(),
		};

		self.edges = edges;
		self.assert_work();
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
		// Unit is stock's empty tuple and has an empty iterator.
		if value == Value::Unit {
			return self.admit(Payload::OptionIterator { payload: None });
		}
		if let Value::Heap(h) = value {
			if let Payload::Variant {
				tag: Type::Option,
				payload,
				..
			} = self.payload(h)?
			{
				return self.admit(Payload::OptionIterator { payload: *payload });
			}
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

		// A failed advance must not allocate or consume any payload.
		self.mutable(h)?;
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

			Payload::OptionIterator { payload } => {
				let value = *payload;
				let Payload::OptionIterator { payload } = self.mutable(h)? else {
					unreachable!()
				};
				*payload = None;
				// Transfer the snapshot's retained root to the caller, without an
				// extra retain or a lease on the mutable Option wrapper.
				self.edges = self
					.edges
					.checked_sub(value.map_or(0, Self::heap_edge))
					.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				self.assert_work();
				Ok(value)
			}
			Payload::VectorIterator(iter) => {
				let root = Value::Heap(iter.handle);
				let index = iter.index;

				let length = match self.payload(iter.handle)? {
					Payload::Record {
						fields,
						tag: Type::Object,
					} => fields.len(),
					Payload::List { values, .. } => values.len(),
					_ => return Err(err(ErrorKind::InvalidCode)),
				};
				if index == length {
					return Ok(None);
				}

				let value = if let Payload::Record {
					fields,
					tag: Type::Object,
				} = self.payload(iter.handle)?
				{
					let (key, child) = fields
						.get(index)
						.ok_or_else(|| err(ErrorKind::InvalidCode))?;
					let child = *child;
					if key.len() > self.limits.bytes.saturating_sub(self.bytes) {
						return Err(err(ErrorKind::StoreCapacity));
					}
					let copy = String::try_from(key.as_str())?;
					let key = self.admit(Payload::String(copy))?;
					let pair = self.list(Type::Tuple, &[key, child]);
					let released = self.release(key);
					match pair {
						Ok(pair) => {
							if let Err(e) = released {
								let _ = self.release(pair);
								return Err(e);
							}
							pair
						}
						Err(e) => {
							released?;
							return Err(e);
						}
					}
				} else {
					self.get(root, index)?
				};
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
		if count > self.edges || count > self.work.capacity().saturating_sub(self.work.len()) {
			// W reserved room before these edges were admitted. Failure here is an
			// invariant error, not permission to allocate during cleanup.
			return Err(err(ErrorKind::InvalidCode));
		}

		Ok(())
	}

	/// Queue every owned heap edge without allocation. The capacity was checked
	/// before taking the payload. A malformed iterator lease reports an error
	/// but must still queue its retained owner root for drainage.
	fn enqueue_children(&mut self, payload: Payload) -> Result<()> {
		// The detached payload no longer holds live edges. Its children become
		// queued releases, without growing the queue or following native recursion.
		self.edges = self
			.edges
			.checked_sub(Self::payload_edges(&payload))
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		self.assert_work();
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
				self.assert_work();
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
			self.assert_work();
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
	/// Test hook: set an entry's reader count (the lease `mutable` refuses).
	#[cfg(test)]
	pub(crate) fn set_readers_for_test(&mut self, value: Value, readers: u32) {
		let Value::Heap(h) = value else {
			panic!("heap value expected");
		};
		self.entry_mut(h).unwrap().readers = readers;
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

		// Process the initial root directly: enqueueing it would consume W's
		// spare root cell while still requiring that spare in the assertion.
		// Root first, then its children, then prior work preserves LIFO order.
		let failure = if let Value::Heap(h) = value {
			if self.work.len() == self.work.capacity() {
				Some(err(ErrorKind::InvalidCode))
			} else {
				self.release_one(h).err()
			}
		} else {
			None
		};
		self.drain_work(failure)
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
		self.edges = 0;
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

#[cfg(test)]
mod field_iter_tests;
