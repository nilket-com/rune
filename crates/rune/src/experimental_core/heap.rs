//! The fresh-core heap: a bounded, generational [`Store`].
//!
//! Values inside a store refer to each other by handle, never by Rust-owned child trees.
//! Cleanup uses storage reserved before child edges are published, and never grows
//! bookkeeping itself. Reference cycles stay charged until the store is closed or dropped. Hosts reach the store only through [`Root`](super::value::Root)s
//! (see the `root_*` methods).

use super::error::{Error, ErrorKind, Result};
use super::value::{Handle, Type, Value};
use crate::alloc::{Box, String, Vec};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

mod cloning;
mod comparison;
mod formatting;
mod indexing;
pub use formatting::FormatLimits;
mod boxed_iterator;
mod ranges;
mod roots;
mod sorting;
#[cfg(test)]
mod sorting_tests;
mod text_methods;
mod text_pipeline;

#[cfg(feature = "fresh-core-native")]
pub(crate) mod native;

#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod children_tests;
#[cfg(test)]
mod growth_tests;
#[cfg(test)]
mod range_iterator_tests;
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
	// Explicit drop retains dispatch identity, but no value or child storage.
	Moved(Type),
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
	ValueRange {
		start: Value,
		end: Value,
	},
	VectorIterator(Iter),
	OptionIterator {
		payload: Option<Value>,
	},
	RangeIterator {
		next: i64,
		end: i64,
	},
	// Interpreter-only, even when mixed bounds coerce to two signed values.
	ValueRangeIterator {
		next: Value,
		end: Value,
	},
	Record {
		tag: Type,
		fields: Vec<(String, Value)>,
	},
	List {
		tag: Type,
		values: Vec<Value>,
	},
	Closure {
		code: u32,
		function: u32,
		captures: Vec<Value>,
	},
	SortGuard(Box<sorting::SortState>),
	OtherRange(Box<ranges::RangeShape>),
	OtherRangeIterator(Box<boxed_iterator::State>),
}

// Child values are inert copies; this iterator never acquires ownership.
// Dispatch the payload once, then walk its selected child storage.
enum Children<'a> {
	Empty,
	One(Option<Value>),
	Two(Option<Value>, Option<Value>),
	Values(core::slice::Iter<'a, Value>),
	Fields(core::slice::Iter<'a, (String, Value)>),
	ParentValues(Option<Value>, core::slice::Iter<'a, Value>),
}

impl Iterator for Children<'_> {
	type Item = Value;

	fn next(&mut self) -> Option<Value> {
		match self {
			Self::Empty => None,
			Self::One(value) => value.take(),
			Self::Two(first, second) => first.take().or_else(|| second.take()),
			Self::Values(values) => values.next().copied(),
			Self::Fields(fields) => fields.next().map(|(_, value)| *value),
			Self::ParentValues(parent, values) => parent.take().or_else(|| values.next().copied()),
		}
	}
}

impl Payload {
	fn bytes(&self) -> Result<usize> {
		let bytes = match self {
			Self::Moved(_) => Some(0),
			Self::SortGuard(state) => Some(state.bytes()?),
			Self::OtherRange(_) => Some(size_of::<ranges::RangeShape>()),
			// Split reserves its separately boxed state in its cold constructor.
			Self::OtherRangeIterator(_) => Some(size_of::<boxed_iterator::State>()),
			Self::String(s) => Some(s.capacity()),
			Self::Range { .. } => Some(16),
			Self::ValueRange { .. } | Self::ValueRangeIterator { .. } => {
				Some(2 * size_of::<Value>())
			}
			Self::Variant { .. } => Some(size_of::<Value>()),
			Self::VectorIterator(..) | Self::OptionIterator { .. } | Self::RangeIterator { .. } => {
				Some(size_of::<Value>())
			}
			Self::List { values, .. } => values.capacity().checked_mul(size_of::<Value>()),
			Self::Closure { captures, .. } => captures.capacity().checked_mul(size_of::<Value>()),
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
	fn children(&self) -> Children<'_> {
		match self {
			Self::OtherRangeIterator(state) => Children::One(state.source.map(Value::Heap)),
			Self::Moved(_)
			| Self::String(_)
			| Self::Range { .. }
			| Self::RangeIterator { .. }
			| Self::ValueRangeIterator { .. } => Children::Empty,
			Self::Variant { payload, .. } | Self::OptionIterator { payload } => {
				Children::One(*payload)
			}
			Self::VectorIterator(iter) => Children::One(Some(Value::Heap(iter.handle))),
			Self::SortGuard(state) => {
				Children::ParentValues(Some(Value::Heap(state.parent)), state.values.iter())
			}
			Self::Record { fields, .. } => Children::Fields(fields.iter()),
			Self::List { values, .. } => Children::Values(values.iter()),
			Self::Closure { captures, .. } => Children::Values(captures.iter()),
			Self::ValueRange { start, end } => Children::Two(Some(*start), Some(*end)),
			Self::OtherRange(shape) => Children::Two(shape.start, shape.end),
		}
	}

	fn tag(&self) -> Type {
		match self {
			Self::Moved(tag) => *tag,
			Self::String(_) => Type::String,
			Self::Range { .. } => Type::Range,
			Self::OtherRange(shape) => shape.tag,
			Self::ValueRange { start, end } => {
				debug_assert!(!matches!((start, end), (Value::I64(_), Value::I64(_))));
				Type::Range
			}
			Self::Variant { tag, .. } => *tag,
			Self::OtherRangeIterator(_)
			| Self::SortGuard(..)
			| Self::VectorIterator(..)
			| Self::OptionIterator { .. }
			| Self::RangeIterator { .. }
			| Self::ValueRangeIterator { .. } => Type::Iterator,
			Self::List { tag, .. } | Self::Record { tag, .. } => *tag,
			Self::Closure { .. } => Type::Function,
		}
	}
}

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

impl core::fmt::Debug for Entry {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		f.debug_struct("Entry")
			.field("previous", &self.previous)
			.field("next", &self.next)
			.field("strong", &self.strong)
			.field("readers", &self.readers)
			.field("charged_bytes", &self.charged_bytes())
			.field("payload", &self.payload)
			.finish()
	}
}

const CLONE_PATH: usize = 1usize << (usize::BITS - 1);
const _: () = assert!(MAX_BYTES < CLONE_PATH);
#[cfg(target_pointer_width = "64")]
const _: () = {
	assert!(size_of::<Entry>() == 72);
	assert!(size_of::<Slot>() == 80);
	assert!(size_of::<Payload>() == 40);
};
impl Entry {
	fn tag(&self) -> Result<Type> {
		self.payload
			.as_ref()
			.map(Payload::tag)
			.ok_or_else(|| err(ErrorKind::InvalidCode))
	}
	fn charged_bytes(&self) -> usize {
		self.charged_bytes & !CLONE_PATH
	}
	fn set_charged_bytes(&mut self, bytes: usize) {
		debug_assert!(bytes <= MAX_BYTES);
		self.charged_bytes = (self.charged_bytes & CLONE_PATH) | bytes;
	}
	fn clone_path(&self) -> bool {
		self.charged_bytes & CLONE_PATH != 0
	}
	fn set_clone_path(&mut self, active: bool) {
		if active {
			self.charged_bytes |= CLONE_PATH;
		} else {
			self.charged_bytes &= !CLONE_PATH;
		}
	}
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

	// Temporary host input copies are bounded by the configured Store byte limit,
	// independently of live payload (they are not published Store entries).
	// Retained identity is not payload readability. Type checks may precede reads.
	pub(crate) fn host_tag(&self, value: Value) -> Result<(Type, bool)> {
		match value {
			Value::Heap(handle) => match self.entry(handle)?.payload.as_ref() {
				Some(Payload::Moved(tag)) => {
					Ok((*tag, self.entry(handle)?.readers == sorting::EXCLUSIVE))
				}
				Some(payload) => Ok((payload.tag(), true)),
				None => Err(err(ErrorKind::InvalidCode)),
			},
			_ => value
				.primitive_type()
				.map(|tag| (tag, true))
				.ok_or_else(|| err(ErrorKind::Type)),
		}
	}

	// Dynamic planning shares the remaining byte budget with its final payload.
	pub(crate) fn host_remaining_bytes(&self) -> Result<usize> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		Ok(self.limits.bytes.saturating_sub(self.bytes))
	}

	pub(crate) fn host_scratch_room(&self, bytes: usize) -> Result<()> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		if bytes > self.limits.bytes {
			return Err(err(ErrorKind::StoreCapacity));
		}
		Ok(())
	}
	// Both entry and payload limits precede fresh output allocation.
	pub(crate) fn host_room(&self, bytes: usize, entries: usize) -> Result<()> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let live = self
			.live
			.checked_add(entries)
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		let reusable = self
			.free
			.len()
			.checked_add(self.limits.entries.saturating_sub(self.slots.len()))
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if bytes > self.limits.bytes.saturating_sub(self.bytes)
			|| live > self.limits.entries
			|| entries > reusable
		{
			return Err(err(ErrorKind::StoreCapacity));
		}
		Ok(())
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
		match payload {
			Payload::Moved(_)
			| Payload::String(_)
			| Payload::Range { .. }
			| Payload::RangeIterator { .. }
			| Payload::ValueRangeIterator { .. } => 0,
			Payload::Variant { payload, .. } | Payload::OptionIterator { payload } => {
				payload.map_or(0, Self::heap_edge)
			}
			Payload::OtherRange(shape) => {
				shape.start.map_or(0, Self::heap_edge) + shape.end.map_or(0, Self::heap_edge)
			}
			Payload::VectorIterator(_) => 1,
			Payload::OtherRangeIterator(state) => usize::from(state.source.is_some()),
			Payload::SortGuard(state) => {
				1 + state
					.values
					.iter()
					.filter(|v| matches!(v, Value::Heap(_)))
					.count()
			}
			Payload::Record { fields, .. } => fields
				.iter()
				.filter(|(_, value)| matches!(value, Value::Heap(_)))
				.count(),
			Payload::List { values, .. } => values
				.iter()
				.filter(|value| matches!(value, Value::Heap(_)))
				.count(),
			Payload::Closure { captures, .. } => captures
				.iter()
				.filter(|value| matches!(value, Value::Heap(_)))
				.count(),
			Payload::ValueRange { start, end } => Self::heap_edge(*start) + Self::heap_edge(*end),
		}
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
		match self.entry(h)?.payload.as_ref() {
			Some(Payload::Moved(_)) => {
				Err(sorting::conflict(sorting::Access::Read, self.entry(h)?))
			}
			Some(payload) => {
				#[cfg(debug_assertions)]
				payload.assert_canonical_range();
				Ok(payload)
			}
			None => Err(err(ErrorKind::InvalidCode)),
		}
	}

	fn mutable(&mut self, h: Handle) -> Result<&mut Payload> {
		let e = self.entry_mut(h)?;
		if e.readers != 0 {
			return Err(sorting::conflict(sorting::Access::Write, e));
		}
		match e.payload.as_mut() {
			Some(Payload::Moved(_)) => Err(err(ErrorKind::AccessConflict).with_detail(
				super::error::Detail::Moved {
					access: super::error::MovedAccess::Write,
				},
			)),
			Some(payload) => {
				#[cfg(debug_assertions)]
				payload.assert_canonical_range();
				Ok(payload)
			}
			None => Err(err(ErrorKind::InvalidCode)),
		}
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
		if matches!(payload, Payload::Moved(_)) {
			return Err(err(ErrorKind::InvalidCode));
		}
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
					.filter(|n| *n != sorting::EXCLUSIVE)
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

	#[cfg(test)]
	pub(crate) fn host_observe_fields(&self, value: Value) -> Result<&[(String, Value)]> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		let Payload::Record { fields, .. } = self.payload(handle)? else {
			return Err(err(ErrorKind::Type));
		};
		Ok(fields)
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

	/// Checked String::from copy: refusal by the byte budget precedes allocation.
	pub(crate) fn string_from(&mut self, value: Value) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let text = self.text(value)?;
		if text.len() > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let text = String::try_from(text)?;
		self.string_owned(text)
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

	/// Construct an opaque function value from a compiler-owned tuple of captures.
	/// The tuple is only a construction operand; the closure owns its own edge list.
	pub(crate) fn closure(&mut self, code: u32, function: u32, env: Value) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let Value::Heap(handle) = env else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::List {
			tag: Type::Tuple,
			values,
		} = self.payload(handle)?
		else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let bytes = values
			.len()
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::Overflow))?;
		if bytes > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let mut captures = Vec::try_with_capacity(values.len())?;
		for &value in values {
			captures.try_push(value)?;
		}
		let payload = Payload::Closure {
			code,
			function,
			captures,
		};
		if payload.bytes()? > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		self.admit(payload)
	}

	pub(crate) fn closure_parts(&self, value: Value) -> Result<(u32, u32, &[Value])> {
		let Value::Heap(handle) = value else {
			return Err(err(ErrorKind::Type));
		};
		match self.payload(handle)? {
			Payload::Closure {
				code,
				function,
				captures,
			} => Ok((*code, *function, captures)),
			_ => Err(err(ErrorKind::Type)),
		}
	}

	pub(crate) fn closure_capture(&self, value: Value, index: usize) -> Result<Value> {
		let (_, _, captures) = self.closure_parts(value)?;
		captures
			.get(index)
			.copied()
			.ok_or_else(|| err(ErrorKind::InvalidCode))
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

		match self.entry(handle)?.tag()? {
			Type::Object | Type::Struct(_) => (),
			Type::Range => return self.range_field(handle, key),
			Type::RangeFrom
			| Type::RangeFull
			| Type::RangeInclusive
			| Type::RangeToInclusive
			| Type::RangeTo => return self.other_range_field(handle, key),
			_ => return Err(err(ErrorKind::Type)),
		}
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
		if !matches!(self.entry(handle)?.tag()?, Type::Object | Type::Struct(_)) {
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
		match self.entry(handle)?.tag()? {
			Type::Object | Type::Struct(_) => (),
			Type::Range => return self.set_range_field(handle, key, new),
			Type::RangeFrom
			| Type::RangeFull
			| Type::RangeInclusive
			| Type::RangeToInclusive
			| Type::RangeTo => return self.set_other_range_field(handle, key, new),
			_ => return Err(err(ErrorKind::Type)),
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
			debug_assert_eq!(entry.tag()?, Type::String);
			entry.payload = Some(Payload::String(replacement));
			entry.set_charged_bytes(charged_bytes);
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
			.checked_sub(self.entry(h)?.charged_bytes())
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

	// The binary heap-pair branch has already matched both handles. Keep its
	// generation-checked tag read at the callsite without changing tag() globally.
	#[inline(always)]
	pub(crate) fn binary_heap_tag(&self, h: Handle) -> Result<Type> {
		self.entry(h)?.tag()
	}
	pub(crate) fn tag(&self, value: Value) -> Result<Type> {
		match value {
			Value::Heap(h) => Ok(self.entry(h)?.tag()?),
			_ => value.primitive_type().ok_or_else(|| err(ErrorKind::Type)),
		}
	}

	/// Diagnostic classification only: never invoked on a successful execution path.
	pub(crate) fn diagnostic_type(&self, value: Value) -> Option<super::error::TypeName> {
		use super::error::TypeName;
		let Value::Heap(h) = value else {
			return TypeName::primitive(value);
		};
		Some(match self.entry(h).ok()?.payload.as_ref() {
			Some(Payload::OtherRangeIterator(cursor)) => cursor.diagnostic_type(),
			Some(Payload::RangeIterator { .. }) => TypeName::RangeIter,
			Some(Payload::ValueRangeIterator {
				next: Value::U64(_),
				..
			}) => TypeName::UnsignedRangeIter,
			Some(Payload::ValueRangeIterator {
				next: Value::Char(_),
				..
			}) => TypeName::CharRangeIter,
			Some(Payload::ValueRangeIterator { .. }) => TypeName::RangeIter,
			Some(Payload::Moved(tag)) => TypeName::from_type(*tag),
			Some(Payload::VectorIterator(_)) => TypeName::VecIter,
			Some(Payload::OptionIterator { .. }) => TypeName::OptionIter,
			_ => TypeName::from_type(self.entry(h).ok()?.tag().ok()?),
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
			Value::Unit => Ok(0),
			Value::Heap(h) => match self.payload(h)? {
				Payload::String(s) => Ok(s.len()),
				Payload::Record {
					tag: Type::Object,
					fields,
				} => Ok(fields.len()),
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

	fn push_char(&mut self, value: Value, new: Value) -> Result<()> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let (len, cap) = match self.mutable(h)? {
			Payload::String(text) => (text.len(), text.capacity()),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		let Value::Char(ch) = new else {
			return Err(
				err(ErrorKind::Type).with_detail(super::error::Detail::ExpectedChar {
					found: self
						.diagnostic_type(new)
						.ok_or_else(|| err(ErrorKind::InvalidCode))?,
				}),
			);
		};
		let needed = len
			.checked_add(ch.len_utf8())
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		if needed.saturating_sub(cap) > self.limits.bytes.saturating_sub(self.bytes) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		if needed > cap {
			let mut text = String::try_with_capacity(needed)?;
			text.try_push_str(self.text(value)?)?;
			text.try_push(ch)?;
			let charge = text.capacity();
			let bytes = self.replacement_bytes(h, charge)?;
			let entry = self.entry_mut(h)?;
			debug_assert_eq!(entry.tag()?, Type::String);
			entry.payload = Some(Payload::String(text));
			entry.set_charged_bytes(charge);
			self.bytes = bytes;
		} else {
			let Payload::String(text) = self.mutable(h)? else {
				return Err(err(ErrorKind::InvalidCode));
			};
			text.try_push(ch)?;
		}
		Ok(())
	}

	pub(crate) fn push(&mut self, value: Value, new: Value) -> Result<()> {
		if matches!(value, Value::Heap(_)) && self.tag(value)? == Type::String {
			return self.push_char(value, new);
		}
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
			entry.set_charged_bytes(charged_bytes);
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

	/// Read only the variant flag, not its child value or its readability.
	/// Moved, stale and closed parent errors retain the Store's existing domains.
	pub(crate) fn variant_success(&self, value: Value) -> Result<bool> {
		let Value::Heap(h) = value else {
			return Err(err(ErrorKind::InvalidCode));
		};
		match self.payload(h)? {
			Payload::Variant { success, .. } => Ok(*success),
			_ => Err(err(ErrorKind::InvalidCode)),
		}
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
			match self.payload(h)? {
				Payload::Variant {
					tag: Type::Option,
					payload,
					..
				} => {
					return self.admit(Payload::OptionIterator { payload: *payload });
				}
				Payload::ValueRange { start, end } => {
					let (next, end) = self.range_iteration_bounds(*start, *end)?;
					return self.admit(Payload::ValueRangeIterator { next, end });
				}
				Payload::Range { start, end } => {
					return self.admit(Payload::RangeIterator {
						next: *start,
						end: *end,
					});
				}
				Payload::OtherRange(shape) => return self.other_range_iterator(**shape),
				Payload::OtherRangeIterator(state) if state.is_split() => {
					self.retain(value)?;
					return Ok(value);
				}
				_ => (),
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
		let payload = self.mutable(h)?;
		if let Payload::RangeIterator { next, end } = payload {
			if *next >= *end {
				return Ok(None);
			}

			let value = *next;
			let new = value
				.checked_add(1)
				.ok_or_else(|| err(ErrorKind::Overflow))?;
			*next = new;
			return Ok(Some(Value::I64(value)));
		}

		match self.payload(h)? {
			Payload::OtherRangeIterator(_) => self.other_range_next(h),
			Payload::ValueRangeIterator { next, end } => self.value_range_next(h, *next, *end),
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
			.filter(|n| *n != sorting::EXCLUSIVE)
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
		let count = Self::payload_edges(payload);
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
	fn enqueue_children(&mut self, mut payload: Payload) -> Result<()> {
		// Transfer held values back before accounting this guard's detached edges.
		let mut failure = match &mut payload {
			// These adjacent cold discriminants share one cold boundary.
			Payload::SortGuard(_) | Payload::OtherRange(_) | Payload::OtherRangeIterator(_) => {
				self.prepare_boxed_detach(&mut payload)?
			}
			_ => None,
		};
		// The detached payload no longer holds live edges. Its children become
		// queued releases, without growing the queue or following native recursion.
		self.edges = self
			.edges
			.checked_sub(Self::payload_edges(&payload))
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		self.assert_work();

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
			.checked_sub(entry.charged_bytes())
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
	pub(crate) fn slot_capacity_for_test(&self) -> usize {
		self.slots.capacity()
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
			| Value::Ordering(_)
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
			| Value::Ordering(_)
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

		let entry = self.entry(h)?;
		if entry.readers != 0 {
			return Err(sorting::conflict(sorting::Access::Take, entry));
		}
		let entry = self.entry(h)?;
		let payload = entry.payload.as_ref().ok_or_else(|| {
			err(ErrorKind::AccessConflict).with_detail(super::error::Detail::Moved {
				access: super::error::MovedAccess::Take,
			})
		})?;
		if matches!(payload, Payload::Moved(_)) {
			return Err(
				err(ErrorKind::AccessConflict).with_detail(super::error::Detail::Moved {
					access: super::error::MovedAccess::Take,
				}),
			);
		}
		if matches!(payload, Payload::SortGuard(_)) {
			return Err(err(ErrorKind::InvalidCode));
		}
		self.children_room(payload)?;
		let bytes = self
			.bytes
			.checked_sub(entry.charged_bytes())
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		let entry = self.entry_mut(h)?;
		let tag = entry.tag()?;
		let payload = entry
			.payload
			.replace(Payload::Moved(tag))
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		entry.set_charged_bytes(0);
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

mod method_cmp;
