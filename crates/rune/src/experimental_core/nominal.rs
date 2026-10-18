//! Process-wide, append-only interner of nominal type and variant descriptors,
//! indexed by [`Nominal`]. A descriptor includes its hash, parent enum and shape.
//! Descriptors survive every program and store for the life of the process; nothing depends on a
//! `Code` or `Store` lifetime.
//!
//! Compiler storage is two exactly reserved buffers (entry records and one name
//! arena). Bounded read slots own immutable metadata and text. Compilation stages
//! identities locally and prepares owned text before taking the writer lock.
//! Admission publishes one atomic batch. Appends under the lock never allocate.
//! Exhaustion and a failed construction are the named `TypeTable` refusal.
//!
//! Only the process-global table issues identities a host can use:
//! construction of a private table is crate-private (isolated tests), and
//! the global table is reachable only through the functions below, never as
//! a mutable value. The module itself is crate-private (see the module docs of
//! `experimental_core` for the sealing doctests).

use super::error::{Error, ErrorKind, Refusal, Result};
use super::value::Nominal;
use crate::alloc::{String, Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub(super) const PROVISIONAL: u32 = 1 << 31;
const MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
	pub entries: usize,
	pub name_bytes: usize,
	pub total_name_bytes: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: MAX_ENTRIES,
			name_bytes: 256,
			total_name_bytes: 256 << 10,
		}
	}
}

/// Storage shape and positional arity are cold metadata, separate from the type hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
	Named,
	Empty,
	Tuple(u32),
}

#[derive(Clone, Copy)]
struct Entry {
	hash: u64,
	enum_hash: u64,
	shape: Shape,
	start: u32,
	len: u32,
	ordinal: u32,
	total_len: u32,
}

struct ReadEntry {
	metadata: Entry,
	record: String,
}

impl ReadEntry {
	fn name(&self) -> &str {
		&self.record[..self.metadata.len as usize]
	}
}

// Variant order and persistent field names are cold metadata; heap layouts are unchanged.
const _: () = assert!(core::mem::size_of::<Entry>() == 40);

pub(crate) struct Interner {
	limits: Limits,
	entries: Vec<Entry>,
	names: String,
}

fn refused() -> Error {
	Error::refused(Refusal::TypeTable)
}

/// The named refusal with the failing operation's kind retained.
fn refused_by(kind: ErrorKind, operation: &'static str) -> Error {
	Error {
		kind,
		location: None,
		operation,
		refusal: Some(Refusal::TypeTable),
		detail: super::error::Detail::None,
	}
}

/// Formats into a fixed stack buffer; overflow is an error, never a reallocation.
struct StackWriter<'a> {
	buffer: &'a mut [u8],
	len: usize,
}

impl core::fmt::Write for StackWriter<'_> {
	fn write_str(&mut self, s: &str) -> core::fmt::Result {
		let end = self.len.checked_add(s.len()).ok_or(core::fmt::Error)?;
		if end > self.buffer.len() {
			return Err(core::fmt::Error);
		}

		self.buffer[self.len..end].copy_from_slice(s.as_bytes());
		self.len = end;
		Ok(())
	}
}

impl Interner {
	/// Reserve both buffers exactly, with checked arithmetic, before anything
	/// is published; a failed reservation is the named refusal with the
	/// allocation kind retained.
	pub(crate) fn with_limits(limits: Limits) -> Result<Self> {
		if limits.entries == 0
			|| limits.entries >= PROVISIONAL as usize
			|| limits.total_name_bytes > u32::MAX as usize
			|| limits.name_bytes > limits.total_name_bytes
			|| limits
				.entries
				.checked_mul(core::mem::size_of::<Entry>())
				.and_then(|n| n.checked_add(limits.total_name_bytes))
				.is_none()
		{
			return Err(refused());
		}

		let mut entries = Vec::new();
		entries
			.try_reserve_exact(limits.entries)
			.map_err(|_| refused_by(ErrorKind::Allocation, "nominal entries"))?;
		let mut names = String::new();
		names
			.try_reserve_exact(limits.total_name_bytes)
			.map_err(|_| refused_by(ErrorKind::Allocation, "nominal names"))?;
		Ok(Self {
			limits,
			entries,
			names,
		})
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn len(&self) -> usize {
		self.entries.len()
	}

	/// Exactly the two reserved buffers: `Vec::capacity` and `String::capacity`
	/// are the allocated element counts for these types (no hashing index).
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn reserved_bytes(&self) -> usize {
		self.entries
			.capacity()
			.checked_mul(core::mem::size_of::<Entry>())
			.and_then(|n| n.checked_add(self.names.capacity()))
			.expect("validated at construction")
	}

	pub(crate) fn lookup(&self, hash: u64) -> Option<Nominal> {
		self.lookup_shape(hash, Shape::Named, 0, 0)
	}

	fn lookup_shape(
		&self,
		hash: u64,
		shape: Shape,
		enum_hash: u64,
		ordinal: u32,
	) -> Option<Nominal> {
		self.entries
			.iter()
			.position(|e| {
				e.hash == hash
					&& e.shape == shape
					&& e.enum_hash == enum_hash
					&& e.ordinal == ordinal
			})
			.map(|i| Nominal(i as u32))
	}

	fn fields_text(&self, e: &Entry) -> &str {
		&self.names[e.start as usize + e.len as usize..e.start as usize + e.total_len as usize]
	}
	fn lookup_fields(
		&self,
		hash: u64,
		shape: Shape,
		parent: u64,
		ordinal: u32,
		fields: &[&str],
	) -> Option<Nominal> {
		self.entries
			.iter()
			.position(|e| {
				e.hash == hash
					&& e.shape == shape
					&& e.enum_hash == parent
					&& e.ordinal == ordinal
					&& self
						.fields_text(e)
						.split('\0')
						.skip(1)
						.eq(fields.iter().copied())
			})
			.map(|i| Nominal(i as u32))
	}
	fn lookup_staged(&self, staging: &Staging, entry: &Entry) -> Option<Nominal> {
		let fields = &staging.names[entry.start as usize + entry.len as usize
			..entry.start as usize + entry.total_len as usize];
		self.entries
			.iter()
			.position(|e| {
				e.hash == entry.hash
					&& e.shape == entry.shape
					&& e.enum_hash == entry.enum_hash
					&& e.ordinal == entry.ordinal
					&& self.fields_text(e) == fields
			})
			.map(|i| Nominal(i as u32))
	}

	/// Identity is the hash; the first name survives. Never allocates.
	pub(crate) fn intern(&mut self, hash: u64, name: &str) -> Result<Nominal> {
		if let Some(id) = self.lookup(hash) {
			return Ok(id);
		}

		self.check_room(1, name.len())?;
		if name.len() > self.limits.name_bytes {
			return Err(refused());
		}
		let id = Nominal(self.entries.len() as u32);
		let before = (self.entries.len(), self.names.len());
		if self
			.append(hash, name, Shape::Named, 0, 0, name.len() as u32)
			.is_err()
		{
			self.rollback(before);
			debug_assert!(false, "preflighted nominal append failed");
			return Err(refused());
		}
		Ok(id)
	}

	fn check_room(&self, entries: usize, bytes: usize) -> Result<()> {
		let count = self
			.entries
			.len()
			.checked_add(entries)
			.ok_or_else(refused)?;
		let end = self.names.len().checked_add(bytes).ok_or_else(refused)?;
		if count > self.limits.entries
			|| count > self.entries.capacity()
			|| end > self.limits.total_name_bytes
			|| end > self.names.capacity()
			|| u32::try_from(count).is_err()
			|| u32::try_from(end).is_err()
		{
			return Err(refused());
		}
		Ok(())
	}

	// Only called after the whole batch's capacity and conversion preflight.
	fn append(
		&mut self,
		hash: u64,
		name: &str,
		shape: Shape,
		enum_hash: u64,
		ordinal: u32,
		name_len: u32,
	) -> Result<()> {
		let entry = Entry {
			hash,
			enum_hash,
			shape,
			start: self.names.len() as u32,
			len: name_len,
			ordinal,
			total_len: name.len() as u32,
		};
		self.names.try_push_str(name).map_err(|_| refused())?;
		self.entries
			.push_within_capacity(entry)
			.map_err(|_| refused())
	}

	fn rollback(&mut self, before: (usize, usize)) {
		self.entries.truncate(before.0);
		self.names.truncate(before.1);
	}

	fn commit(&mut self, staging: &Staging, remap: &mut [Nominal]) -> Result<()> {
		if remap.len() != staging.len() {
			return Err(refused());
		}
		let mut count = 0usize;
		let mut bytes = 0usize;
		for (i, entry) in staging.entries.iter().enumerate() {
			if let Some(id) = self.lookup_staged(staging, entry) {
				remap[i] = id;
			} else {
				let name = staging.name(entry);
				if name.len() > self.limits.name_bytes || u32::try_from(name.len()).is_err() {
					return Err(refused());
				}
				count = count.checked_add(1).ok_or_else(refused)?;
				bytes = bytes
					.checked_add(entry.total_len as usize)
					.ok_or_else(refused)?;
			}
		}
		self.check_room(count, bytes)?;
		let before = (self.entries.len(), self.names.len());
		let capacities = (self.entries.capacity(), self.names.capacity());
		for (i, entry) in staging.entries.iter().enumerate() {
			if let Some(id) = self.lookup_staged(staging, entry) {
				remap[i] = id;
				continue;
			}
			remap[i] = Nominal(self.entries.len() as u32);
			if self
				.append(
					entry.hash,
					staging.record(entry),
					entry.shape,
					entry.enum_hash,
					entry.ordinal,
					entry.len,
				)
				.is_err()
			{
				self.rollback(before);
				debug_assert!(false, "preflighted nominal batch append failed");
				return Err(refused());
			}
		}
		debug_assert_eq!(capacities, (self.entries.capacity(), self.names.capacity()));
		Ok(())
	}

	/// `intern` with a formatted name, written to a stack buffer bounded by the
	/// name cap; the duplicate lookup happens before any formatting.
	#[cfg(test)]
	pub(crate) fn intern_fmt(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
	) -> Result<Nominal> {
		if let Some(id) = self.lookup(hash) {
			return Ok(id);
		}

		let mut buffer = [0u8; 256];
		let cap = self.limits.name_bytes.min(buffer.len());
		let mut writer = StackWriter {
			buffer: &mut buffer[..cap],
			len: 0,
		};

		core::fmt::Write::write_fmt(&mut writer, name).map_err(|_| refused())?;
		let len = writer.len;
		let text = core::str::from_utf8(&buffer[..len]).map_err(|_| refused())?;
		self.intern(hash, text)
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn hash(&self, nominal: Nominal) -> Option<u64> {
		self.entries.get(nominal.0 as usize).map(|e| {
			if e.enum_hash == 0 {
				e.hash
			} else {
				e.enum_hash
			}
		})
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn name(&self, nominal: Nominal) -> Option<&str> {
		self.entries
			.get(nominal.0 as usize)
			.map(|e| &self.names[e.start as usize..(e.start + e.len) as usize])
	}
}

/// Compile-local identities. Only CompileTransaction owns one; it cannot be
/// paired with an independently constructed Builder by any API.
#[derive(Default)]
pub(super) struct Staging {
	entries: Vec<Entry>,
	names: String,
}

impl Staging {
	pub(super) fn len(&self) -> usize {
		self.entries.len()
	}

	fn name(&self, entry: &Entry) -> &str {
		&self.names[entry.start as usize..entry.start as usize + entry.len as usize]
	}

	fn record(&self, entry: &Entry) -> &str {
		&self.names[entry.start as usize..entry.start as usize + entry.total_len as usize]
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(super) fn stage(&mut self, hash: u64, name: core::fmt::Arguments<'_>) -> Result<Nominal> {
		self.stage_shape(hash, name, Shape::Named)
	}

	pub(super) fn stage_shape(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
		shape: Shape,
	) -> Result<Nominal> {
		self.stage_variant(hash, name, shape, 0)
	}

	pub(super) fn stage_variant(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
		shape: Shape,
		enum_hash: u64,
	) -> Result<Nominal> {
		self.stage_variant_order(hash, name, shape, enum_hash, 0, &[])
	}

	#[allow(clippy::too_many_arguments)]
	pub(super) fn stage_variant_order(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
		shape: Shape,
		enum_hash: u64,
		ordinal: u32,
		fields: &[&str],
	) -> Result<Nominal> {
		let existing = {
			let guard = GLOBAL.lock().map_err(|_| refused())?;
			guard
				.as_ref()
				.and_then(|t| t.lookup_fields(hash, shape, enum_hash, ordinal, fields))
		};
		if let Some(id) = existing {
			return Ok(id);
		}
		if let Some(i) = self.entries.iter().position(|e| {
			e.hash == hash
				&& e.shape == shape
				&& e.enum_hash == enum_hash
				&& e.ordinal == ordinal
				&& self.names
					[e.start as usize + e.len as usize..e.start as usize + e.total_len as usize]
					.split('\0')
					.skip(1)
					.eq(fields.iter().copied())
		}) {
			return Ok(Nominal(PROVISIONAL | i as u32));
		}
		let limits = Limits::default();
		let mut buffer = [0; 256];
		let mut writer = StackWriter {
			buffer: &mut buffer,
			len: 0,
		};
		core::fmt::Write::write_fmt(&mut writer, name).map_err(|_| refused())?;
		let len = writer.len;
		if self.entries.len() >= limits.entries
			|| self
				.names
				.len()
				.checked_add(len)
				.is_none_or(|n| n > limits.total_name_bytes)
		{
			return Err(refused());
		}
		let text = core::str::from_utf8(&buffer[..len]).map_err(|_| refused())?;
		let mut required = len;
		for field in fields {
			required = required
				.checked_add(field.len())
				.and_then(|n| n.checked_add(1))
				.ok_or_else(refused)?;
		}
		if self
			.names
			.len()
			.checked_add(required)
			.is_none_or(|n| n > limits.total_name_bytes)
		{
			return Err(refused());
		}
		let record = if fields.is_empty() {
			None
		} else {
			let mut record = String::try_from(text)
				.map_err(|_| refused_by(ErrorKind::Allocation, "staged nominal fields"))?;
			for field in fields {
				record.try_push('\0').map_err(|_| refused())?;
				record.try_push_str(field).map_err(|_| refused())?;
			}
			Some(record)
		};
		let text = record.as_deref().unwrap_or(text);
		let total_len = text.len();
		if u32::try_from(total_len).is_err() {
			return Err(refused());
		}
		// Reserve both buffers before changing either logical length.
		self.entries
			.try_reserve(1)
			.map_err(|_| refused_by(ErrorKind::Allocation, "staged nominal entries"))?;
		self.names
			.try_reserve(total_len)
			.map_err(|_| refused_by(ErrorKind::Allocation, "staged nominal names"))?;
		let entry = Entry {
			hash,
			enum_hash,
			shape,
			start: self.names.len() as u32,
			len: len as u32,
			ordinal,
			total_len: total_len as u32,
		};
		self.names.try_push_str(text).map_err(|_| refused())?;
		self.entries
			.push_within_capacity(entry)
			.map_err(|_| refused())?;
		Ok(Nominal(PROVISIONAL | (self.entries.len() as u32 - 1)))
	}

	pub(super) fn valid(&self, nominal: Nominal) -> bool {
		if nominal.0 & PROVISIONAL != 0 {
			(nominal.0 & !PROVISIONAL) < self.entries.len() as u32
		} else {
			contains(nominal)
		}
	}

	pub(super) fn prepare_remap(&self) -> Result<Vec<Nominal>> {
		let mut remap = Vec::try_with_capacity(self.len())?;
		for _ in 0..self.len() {
			remap
				.push_within_capacity(Nominal(0))
				.map_err(|_| refused())?;
		}
		Ok(remap)
	}

	pub(super) fn commit(&self, remap: &mut [Nominal]) -> Result<()> {
		if self.entries.is_empty() {
			return Ok(());
		}
		let records = prepare_records(self.entries.iter().map(|entry| self.record(entry)))?;
		with_global(records, |table| table.commit(self, remap))
	}
}

static GLOBAL: Mutex<Option<Interner>> = Mutex::new(None);
static INITIALIZED: AtomicBool = AtomicBool::new(false);
// Fixed, append-only identity publication. Readers acquire the published length;
// all entries below it are immutable. Private interner tests never publish here.
static PUBLISHED: AtomicUsize = AtomicUsize::new(0);
static VARIANT_IDS: [AtomicU32; MAX_ENTRIES] = [const { AtomicU32::new(0) }; MAX_ENTRIES];
// Immutable copies retain metadata independently of the writer's Vec borrows.
// Only the writer under GLOBAL initializes slots; readers never take that lock.
static READ_ENTRIES: [OnceLock<ReadEntry>; MAX_ENTRIES] = [const { OnceLock::new() }; MAX_ENTRIES];

#[cfg(test)]
#[path = "nominal/construction_micro.rs"]
mod construction_micro;

#[cfg(test)]
fn published_entry<'a>(
	entries: &'a [OnceLock<Entry>],
	published: &AtomicUsize,
	n: Nominal,
) -> Option<&'a Entry> {
	if n.0 as usize >= published.load(Ordering::Acquire) {
		return None;
	}
	entries.get(n.0 as usize)?.get()
}

#[inline]
fn read_entry(n: Nominal) -> Option<&'static ReadEntry> {
	if n.0 as usize >= PUBLISHED.load(Ordering::Acquire) {
		return None;
	}
	READ_ENTRIES.get(n.0 as usize)?.get()
}

fn prepare_records<'a>(records: impl ExactSizeIterator<Item = &'a str>) -> Result<Vec<String>> {
	let allocation = || refused_by(ErrorKind::Allocation, "nominal read records");
	let mut prepared = Vec::try_with_capacity(records.len()).map_err(|_| allocation())?;
	for record in records {
		prepared
			.try_push(String::try_from(record).map_err(|_| allocation())?)
			.map_err(|_| allocation())?;
	}
	Ok(prepared)
}

/// Compare variant identity without the global metadata mutex or payload access.
#[inline]
pub(crate) fn same_variant(a: Nominal, b: Nominal) -> bool {
	let Some(a) = VARIANT_IDS.get(a.0 as usize) else {
		return false;
	};
	let Some(b) = VARIANT_IDS.get(b.0 as usize) else {
		return false;
	};
	let a = a.load(Ordering::Acquire);
	a != 0 && a == b.load(Ordering::Acquire)
}

/// Read the published, immutable variant identity; zero names a non-variant.
#[inline]
pub(super) fn variant_id(n: Nominal) -> u32 {
	VARIANT_IDS
		.get(n.0 as usize)
		.map_or(0, |id| id.load(Ordering::Acquire))
}
#[inline]
pub(super) fn same_variant_id(actual: Nominal, expected: u32) -> bool {
	expected != 0 && variant_id(actual) == expected
}

/// The atomic flag is only a monotonic allocation hint. The mutex is authoritative.
/// A losing candidate is dropped after the guard, including every error path.
fn with_global<T>(records: Vec<String>, f: impl FnOnce(&mut Interner) -> Result<T>) -> Result<T> {
	let candidate = if !INITIALIZED.load(Ordering::Acquire) {
		Some(Interner::with_limits(Limits::default())?)
	} else {
		None
	};
	with_prepared(candidate, records, f)
}

fn with_prepared<T>(
	mut candidate: Option<Interner>,
	mut records: Vec<String>,
	f: impl FnOnce(&mut Interner) -> Result<T>,
) -> Result<T> {
	let result = {
		let mut guard = GLOBAL.lock().map_err(|_| refused())?;
		if guard.is_none() {
			*guard = candidate.take();
			INITIALIZED.store(true, Ordering::Release);
		}
		let table = guard.as_mut().ok_or_else(refused)?;
		let before = PUBLISHED.load(Ordering::Relaxed);
		let before_names = table.names.len();
		let result = f(table);
		if result.is_ok() {
			let count = table.entries.len() - before;
			if table.entries.len() > MAX_ENTRIES {
				table.rollback((before, before_names));
				return Err(refused());
			}
			// Bind every owned record before publishing any slot. All storage was
			// prepared before the lock; a failed binding rolls back the whole batch.
			for offset in 0..count {
				let e = table.entries[before + offset];
				let text = &table.names[e.start as usize..e.start as usize + e.total_len as usize];
				let found = records
					.get(offset..)
					.and_then(|tail| tail.iter().position(|record| record.as_str() == text));
				let Some(found) = found else {
					table.rollback((before, before_names));
					return Err(refused());
				};
				records.swap(offset, offset + found);
			}
			for (i, record) in (before..table.entries.len()).zip(records.drain(..count)) {
				let e = table.entries[i];
				assert!(
					READ_ENTRIES[i]
						.set(ReadEntry {
							metadata: e,
							record
						})
						.is_ok(),
					"nominal slot already published"
				);
				let id = if e.enum_hash == 0 {
					0
				} else {
					table.entries[..=i]
						.iter()
						.position(|other| other.hash == e.hash && other.enum_hash == e.enum_hash)
						.expect("published descriptor") as u32
						+ 1
				};
				VARIANT_IDS[i].store(id, Ordering::Release);
			}
			PUBLISHED.store(table.entries.len(), Ordering::Release);
		}
		result
	};
	drop(candidate);
	result
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn intern(hash: u64, name: &str) -> Result<Nominal> {
	if let Some(nominal) = lookup(hash)? {
		return Ok(nominal);
	}
	if name.len() > Limits::default().name_bytes {
		return Err(refused());
	}
	let records = prepare_records(core::iter::once(name))?;
	with_global(records, |table| table.intern(hash, name))
}

/// Format before acquiring the publication lock; user Display never runs inside it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn intern_fmt(hash: u64, name: core::fmt::Arguments<'_>) -> Result<Nominal> {
	if let Some(id) = lookup(hash)? {
		return Ok(id);
	}
	let mut buffer = [0; 256];
	let mut writer = StackWriter {
		buffer: &mut buffer,
		len: 0,
	};
	core::fmt::Write::write_fmt(&mut writer, name).map_err(|_| refused())?;
	let len = writer.len;
	intern(
		hash,
		core::str::from_utf8(&buffer[..len]).map_err(|_| refused())?,
	)
}

pub(super) fn lookup(hash: u64) -> Result<Option<Nominal>> {
	let guard = GLOBAL.lock().map_err(|_| refused())?;
	Ok(guard.as_ref().and_then(|table| table.lookup(hash)))
}

/// Read-only validation never installs a cold table.
pub fn contains(nominal: Nominal) -> bool {
	read_entry(nominal).is_some()
}

/// Read the canonical hash for an admitted type value, without allocating.
pub(crate) fn hash_copy(nominal: Nominal) -> Option<u64> {
	let e = &read_entry(nominal)?.metadata;
	Some(if e.enum_hash == 0 {
		e.hash
	} else {
		e.enum_hash
	})
}

/// Copy a diagnostic name without initializing the table or holding its lock while formatting.
pub(crate) fn name_copy(nominal: Nominal, buffer: &mut [u8; 256]) -> Option<usize> {
	let name = read_entry(nominal)?.name();
	buffer
		.get_mut(..name.len())?
		.copy_from_slice(name.as_bytes());
	Some(name.len())
}

/// Persistent declaration metadata for diagnostics and enum ordering.
pub(crate) fn variant_info(n: Nominal) -> Option<(u64, u32)> {
	let e = &read_entry(n)?.metadata;
	Some((if e.enum_hash == 0 { 0 } else { e.hash }, e.ordinal))
}
pub(crate) fn fields_copy(n: Nominal) -> Option<Vec<String>> {
	let entry = read_entry(n)?;
	let e = &entry.metadata;
	let tail = entry.record.get(e.len as usize..e.total_len as usize)?;
	let mut fields = Vec::new();
	for name in tail.split('\0').skip(1) {
		fields.try_push(String::try_from(name).ok()?).ok()?;
	}
	Some(fields)
}
pub(crate) fn type_name_copy(n: Nominal, buffer: &mut [u8; 256]) -> Option<usize> {
	let entry = read_entry(n)?;
	let e = &entry.metadata;
	let name = entry.name();
	let name = if e.enum_hash == 0 {
		name
	} else {
		name.rsplit_once("::")?.0
	};
	buffer
		.get_mut(..name.len())?
		.copy_from_slice(name.as_bytes());
	Some(name.len())
}

pub(crate) fn variant_rtti(n: Nominal) -> Option<crate::sync::Arc<crate::runtime::Rtti>> {
	let (variant, ordinal) = variant_info(n)?;
	if variant == 0 {
		return None;
	}
	let mut buffer = [0; 256];
	let len = name_copy(n, &mut buffer)?;
	let name = core::str::from_utf8(&buffer[..len]).ok()?;
	let kind = match shape_copy(n)? {
		Shape::Empty => crate::runtime::RttiKind::Empty,
		Shape::Tuple(_) => crate::runtime::RttiKind::Tuple,
		Shape::Named => crate::runtime::RttiKind::Struct,
	};
	let mut fields = crate::runtime::new_field_map();
	for (i, name) in fields_copy(n)?.iter().enumerate() {
		fields
			.try_insert(crate::alloc::Box::<str>::try_from(name.as_str()).ok()?, i)
			.ok()?;
	}
	crate::sync::Arc::try_new(crate::runtime::Rtti {
		variant_index: ordinal,
		kind,
		hash: crate::Hash::new(hash_copy(n)?),
		variant_hash: crate::Hash::new(variant),
		item: crate::ItemBuf::with_item(name.trim_start_matches("::").split("::")).ok()?,
		fields,
	})
	.ok()
}

/// No allocation or compiler/query dependency at runtime.
#[inline]
pub(crate) fn shape_copy(n: Nominal) -> Option<Shape> {
	Some(read_entry(n)?.metadata.shape)
}
#[cold]
#[inline(never)]
pub(crate) fn same_type(a: Nominal, b: Nominal) -> bool {
	if a.0 == b.0 {
		return true;
	}
	match (hash_copy(a), hash_copy(b)) {
		(Some(a), Some(b)) => a == b,
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn variant_identity_is_lock_free_and_shape_independent() {
		let mut staging = super::Staging::default();
		let a = staging
			.stage_variant(
				0xc15b001,
				format_args!("C15b::T"),
				super::Shape::Tuple(1),
				0xc15b000,
			)
			.unwrap();
		let twin = staging
			.stage_variant(
				0xc15b001,
				format_args!("C15b::T"),
				super::Shape::Tuple(2),
				0xc15b000,
			)
			.unwrap();
		let miss = staging
			.stage_variant(
				0xc15b002,
				format_args!("C15b::U"),
				super::Shape::Empty,
				0xc15b000,
			)
			.unwrap();
		let foreign = staging
			.stage_variant(
				0xc15b001,
				format_args!("C15bOther::T"),
				super::Shape::Tuple(1),
				0xc15b100,
			)
			.unwrap();
		let named = staging
			.stage_variant_order(
				0xc15b003,
				format_args!("C15b::Named"),
				super::Shape::Named,
				0xc15b000,
				2,
				&["right", "left"],
			)
			.unwrap();
		let mut remap = staging.prepare_remap().unwrap();
		staging.commit(&mut remap).unwrap();
		let resolve = |n: super::Nominal| {
			if n.0 & super::PROVISIONAL == 0 {
				n
			} else {
				remap[(n.0 & !super::PROVISIONAL) as usize]
			}
		};
		let (a, twin, miss, foreign) = (resolve(a), resolve(twin), resolve(miss), resolve(foreign));
		let named = resolve(named);
		// Holding the metadata mutex proves both hits and misses avoid that mutex.
		let _guard = super::GLOBAL.lock().unwrap();
		assert_eq!(super::shape_copy(a), Some(super::Shape::Tuple(1)));
		assert_eq!(super::shape_copy(twin), Some(super::Shape::Tuple(2)));
		assert_eq!(super::shape_copy(miss), Some(super::Shape::Empty));
		assert_eq!(super::shape_copy(super::Nominal(super::PROVISIONAL)), None);
		assert!(super::same_variant(a, a));
		assert!(super::same_variant(a, twin));
		assert!(!super::same_variant(a, miss));
		assert!(!super::same_variant(a, foreign));
		let identity = super::variant_id(a);
		assert!(super::same_variant_id(a, identity));
		assert!(super::same_variant_id(twin, identity));
		assert!(!super::same_variant_id(miss, identity));
		assert!(!super::same_variant_id(foreign, identity));
		assert!(!super::same_variant_id(a, 0));
		assert!(super::contains(named));
		assert_eq!(super::hash_copy(named), Some(0xc15b000));
		assert_eq!(super::variant_info(named), Some((0xc15b003, 2)));
		let mut buffer = [0; 256];
		let len = super::name_copy(named, &mut buffer).unwrap();
		assert_eq!(&buffer[..len], b"C15b::Named");
		let len = super::type_name_copy(named, &mut buffer).unwrap();
		assert_eq!(&buffer[..len], b"C15b");
		assert!(super::fields_copy(named)
			.unwrap()
			.iter()
			.map(|name| name.as_str())
			.eq(["right", "left"]));
		assert!(super::same_type(a, named));
		assert!(!super::same_type(a, foreign));
		let rtti = super::variant_rtti(named).unwrap();
		assert_eq!(rtti.hash, crate::Hash::new(0xc15b000));
		assert_eq!(rtti.variant_hash, crate::Hash::new(0xc15b003));
		assert_eq!(rtti.variant_index, 2);
		let invalid = super::Nominal(super::PROVISIONAL);
		assert!(!super::contains(invalid));
		assert_eq!(super::hash_copy(invalid), None);
		assert_eq!(super::name_copy(invalid, &mut buffer), None);
		assert_eq!(super::type_name_copy(invalid, &mut buffer), None);
		assert!(super::fields_copy(invalid).is_none());
		assert!(super::variant_info(invalid).is_none());
		assert!(super::variant_rtti(invalid).is_none());
		assert!(!super::same_type(a, invalid));
		assert!(super::same_type(invalid, invalid));
	}
	use super::*;

	#[test]
	fn failed_record_binding_rolls_back_before_publication() {
		let first_hash = 0x4e31_ab01;
		let second_hash = 0x4e31_ab02;
		let mut first = None;
		let records = prepare_records(core::iter::once("N1AbortA")).unwrap();
		let result = with_global(records, |table| {
			first = Some(table.intern(first_hash, "N1AbortA")?);
			table.intern(second_hash, "N1AbortB")?;
			Ok(())
		});
		assert_eq!(result.unwrap_err().refusal, Some(Refusal::TypeTable));
		assert_eq!(lookup(first_hash).unwrap(), None);
		assert_eq!(lookup(second_hash).unwrap(), None);
		let first = first.unwrap();
		assert_ne!(hash_copy(first), Some(first_hash));
		assert!(READ_ENTRIES[first.0 as usize]
			.get()
			.is_none_or(|entry| entry.metadata.hash != first_hash));
		let retry = intern(first_hash, "N1AbortA").unwrap();
		assert_eq!(hash_copy(retry), Some(first_hash));
		let mut buffer = [0; 256];
		let len = name_copy(retry, &mut buffer).unwrap();
		assert_eq!(&buffer[..len], b"N1AbortA");
	}

	#[test]
	fn concurrent_publication_keeps_old_entries_readable() {
		let entries: [OnceLock<Entry>; 32] = [const { OnceLock::new() }; 32];
		let published = AtomicUsize::new(0);
		std::thread::scope(|scope| {
			for _ in 0..4 {
				let entries = &entries;
				let published = &published;
				scope.spawn(move || loop {
					let len = published.load(Ordering::Acquire);
					for i in 0..len {
						let e = published_entry(entries, published, Nominal(i as u32)).unwrap();
						assert_eq!(e.hash, i as u64 + 1);
						assert_eq!(e.shape, Shape::Tuple(i as u32));
						assert_eq!((e.start, e.len, e.total_len), (0, 0, 0));
					}
					assert!(published_entry(entries, published, Nominal(32)).is_none());
					if len == entries.len() {
						break;
					}
					std::thread::yield_now();
				});
			}
			for (i, slot) in entries.iter().enumerate() {
				assert!(slot
					.set(Entry {
						hash: i as u64 + 1,
						enum_hash: 0,
						shape: Shape::Tuple(i as u32),
						start: 0,
						len: 0,
						ordinal: 0,
						total_len: 0,
					})
					.is_ok());
				published.store(i + 1, Ordering::Release);
				std::thread::yield_now();
			}
		});
	}

	#[test]
	fn concurrent_interning_publishes_constructor_shapes() {
		fn check(i: usize, n: Nominal) {
			let shape = if i.is_multiple_of(2) {
				Shape::Named
			} else {
				Shape::Tuple(i as u32)
			};
			assert!(contains(n));
			assert_eq!(shape_copy(n), Some(shape));
			assert_eq!(hash_copy(n), Some(0x4e31_ffff));
			assert_eq!(variant_info(n), Some((0x4e31_0000 + i as u64, i as u32)));
			let mut buffer = [0; 256];
			let len = name_copy(n, &mut buffer).unwrap();
			assert_eq!(
				&buffer[..len],
				std::format!("N1Concurrent::V{i}").as_bytes()
			);
			let len = type_name_copy(n, &mut buffer).unwrap();
			assert_eq!(&buffer[..len], b"N1Concurrent");
			let fields = fields_copy(n).unwrap();
			if shape == Shape::Named {
				assert!(fields
					.iter()
					.map(|name| name.as_str())
					.eq(["first", "second"]));
			} else {
				assert!(fields.is_empty());
			}
		}
		let ids: [AtomicU32; 16] = [const { AtomicU32::new(0) }; 16];
		let start = std::sync::Barrier::new(5);
		let finish = std::sync::Barrier::new(5);
		std::thread::scope(|scope| {
			for _ in 0..4 {
				let ids = &ids;
				let start = &start;
				let finish = &finish;
				scope.spawn(move || {
					for round in 0..=ids.len() {
						start.wait();
						if round > 0 {
							let i = round - 1;
							let id = ids[i].load(Ordering::Acquire);
							assert_ne!(id, 0);
							check(i, Nominal(id - 1));
							let first = ids[0].load(Ordering::Acquire);
							check(0, Nominal(first - 1));
						}
						finish.wait();
					}
				});
			}
			for i in 0..=ids.len() {
				start.wait();
				if i < ids.len() {
					let mut staging = Staging::default();
					let shape = if i.is_multiple_of(2) {
						Shape::Named
					} else {
						Shape::Tuple(i as u32)
					};
					let fields: &[&str] = if shape == Shape::Named {
						&["first", "second"]
					} else {
						&[]
					};
					let n = staging
						.stage_variant_order(
							0x4e31_0000 + i as u64,
							format_args!("N1Concurrent::V{i}"),
							shape,
							0x4e31_ffff,
							i as u32,
							fields,
						)
						.unwrap();
					let mut remap = staging.prepare_remap().unwrap();
					staging.commit(&mut remap).unwrap();
					let n = if n.0 & PROVISIONAL == 0 {
						n
					} else {
						remap[(n.0 & !PROVISIONAL) as usize]
					};
					ids[i].store(n.0 + 1, Ordering::Release);
				}
				finish.wait();
			}
		});
	}

	fn small(entries: usize, name_bytes: usize, total_name_bytes: usize) -> Interner {
		Interner::with_limits(Limits {
			entries,
			name_bytes,
			total_name_bytes,
		})
		.unwrap()
	}

	#[test]
	fn variant_fields_count_toward_atomic_publication_capacity() {
		let mut staging = Staging::default();
		staging
			.stage_variant_order(
				0xc15b201,
				format_args!("V"),
				Shape::Named,
				0xc15b200,
				0,
				&["long"],
			)
			.unwrap();
		staging
			.stage_variant_order(
				0xc15b202,
				format_args!("W"),
				Shape::Named,
				0xc15b200,
				1,
				&["long"],
			)
			.unwrap();
		let mut table = small(4, 8, 10);
		let mut remap = staging.prepare_remap().unwrap();
		assert!(table.commit(&staging, &mut remap).is_err());
		assert_eq!(table.entries.len(), 0);
		assert_eq!(table.names.len(), 0);
	}

	#[test]
	fn limits_refuse_before_publishing() {
		let mut table = small(2, 8, 12);
		let a = table.intern(1, "A").unwrap();
		assert_eq!(
			table.intern(1, "A again").unwrap(),
			a,
			"duplicate hash: same identity, no entry"
		);
		assert_eq!(table.len(), 1);
		assert_eq!(table.name(a), Some("A"));
		assert_eq!(table.hash(a), Some(1));
		let long = table.intern(2, "123456789").unwrap_err();
		assert_eq!(long.refusal, Some(Refusal::TypeTable));
		assert_eq!(table.len(), 1, "a refused name is not published");
		let b = table.intern(2, "B").unwrap();
		assert_ne!(a, b);
		assert_eq!(table.name(b), Some("B"));
		let full = table.intern(3, "C").unwrap_err();
		assert_eq!(full.refusal, Some(Refusal::TypeTable));
		assert_eq!(table.len(), 2);
		assert_eq!(
			table.intern(2, "B").unwrap(),
			b,
			"lookup still works when full"
		);

		// Total name budget: two short names exceed 3 bytes in total.
		let mut tight = small(4, 2, 3);
		tight.intern(1, "ab").unwrap();
		assert_eq!(
			tight.intern(2, "cd").unwrap_err().refusal,
			Some(Refusal::TypeTable)
		);
		assert_eq!(tight.len(), 1);
	}

	#[test]
	fn storage_is_reserved_exactly_at_construction_and_never_grows() {
		let mut table = small(3, 4, 10);
		let reserved = table.reserved_bytes();
		assert_eq!(reserved, 3 * core::mem::size_of::<Entry>() + 10);

		for (i, name) in ["abc", "de", "f"].iter().enumerate() {
			table.intern(i as u64 + 10, name).unwrap();
			assert_eq!(table.reserved_bytes(), reserved);
		}

		assert_eq!(table.name(Nominal(1)), Some("de"));
		assert!(Interner::with_limits(Limits {
			entries: PROVISIONAL as usize,
			name_bytes: 1,
			total_name_bytes: 1
		})
		.is_err());
		assert!(Interner::with_limits(Limits {
			entries: 0,
			name_bytes: 1,
			total_name_bytes: 1
		})
		.is_err());
		assert!(Interner::with_limits(Limits {
			entries: 1,
			name_bytes: 2,
			total_name_bytes: 1
		})
		.is_err());
	}

	#[test]
	fn formatted_names_are_bounded_by_the_stack_buffer_and_the_cap() {
		let mut table = small(4, 4, 16);
		let n = table.intern_fmt(7, format_args!("{}", 1234)).unwrap();
		assert_eq!(table.name(n), Some("1234"));
		assert_eq!(
			table.intern_fmt(7, format_args!("{}", 99999)).unwrap(),
			n,
			"duplicate before formatting"
		);
		let over = table.intern_fmt(8, format_args!("{}", 12345)).unwrap_err();
		assert_eq!(over.refusal, Some(Refusal::TypeTable));
		assert_eq!(table.len(), 1);
	}

	#[test]
	fn private_table_indices_are_local_and_do_not_prove_global_admission() {
		let mut private = small(8, 8, 64);
		for i in 0..8u64 {
			let id = private.intern(0xdead_0000 + i, "p").unwrap();
			assert_eq!(id, Nominal(i as u32));
			assert_eq!(private.hash(id), Some(0xdead_0000 + i));
		}

		assert_eq!(private.len(), 8);

		// Private indices can coincide with global indices. External code cannot
		// construct this table: the separate visibility doctest proves the seal.
		// No global-length assertion here: other tests may intern concurrently.
	}
	fn staged(items: &[(u64, &str)]) -> Staging {
		let mut s = Staging::default();
		for &(hash, name) in items {
			s.stage(hash, format_args!("{name}")).unwrap();
		}
		s
	}

	#[test]
	fn batch_boundaries_are_atomic_and_deduplicated() {
		let mut table = small(3, 4, 6);
		let old = table.intern(10, "aa").unwrap();
		let s = staged(&[(10, "ignored"), (20, "bb"), (20, "again"), (30, "cc")]);
		assert_eq!(s.len(), 3);
		let mut remap = s.prepare_remap().unwrap();
		let capacities = table.reserved_bytes();
		table.commit(&s, &mut remap).unwrap();
		assert_eq!(remap, [old, Nominal(1), Nominal(2)]);
		assert_eq!(table.names.len(), 6);
		assert_eq!(table.reserved_bytes(), capacities);
		let extra = staged(&[(40, "d")]);
		assert!(table
			.commit(&extra, &mut extra.prepare_remap().unwrap())
			.is_err());
		assert_eq!((table.len(), table.names.len()), (3, 6));

		let mut bytes = small(8, 4, 5);
		bytes.intern(1, "aa").unwrap();
		let too_many = staged(&[(2, "bb"), (3, "cc")]);
		assert!(bytes
			.commit(&too_many, &mut too_many.prepare_remap().unwrap())
			.is_err());
		assert_eq!((bytes.len(), bytes.names.len()), (1, 2));
		let fits = staged(&[(2, "bbb")]);
		bytes
			.commit(&fits, &mut fits.prepare_remap().unwrap())
			.unwrap();
		assert_eq!((bytes.len(), bytes.names.len()), (2, 5));
		let plus_one = staged(&[(3, "c")]);
		assert!(bytes
			.commit(&plus_one, &mut plus_one.prepare_remap().unwrap())
			.is_err());
		assert_eq!((bytes.len(), bytes.names.len()), (2, 5));

		let mut entries = small(2, 4, 16);
		let three = staged(&[(1, "a"), (2, "b"), (3, "c")]);
		assert!(entries
			.commit(&three, &mut three.prepare_remap().unwrap())
			.is_err());
		assert_eq!((entries.len(), entries.names.len()), (0, 0));
	}

	fn global_len() -> usize {
		GLOBAL.lock().unwrap().as_ref().map_or(0, Interner::len)
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "isolated subprocess frontend and concurrency coverage; native tests run this"
	)]
	fn global_admission_isolated() {
		use std::process::Command;
		let Ok(mode) = std::env::var("RUNE_NOMINAL_CHILD") else {
			for mode in [
				"lifecycle",
				"cold-allocation",
				"cold-capacity",
				"poison",
				"concurrent",
			] {
				let output = Command::new(std::env::current_exe().unwrap())
					.args([
						"--exact",
						"experimental_core::nominal::tests::global_admission_isolated",
						"--nocapture",
					])
					.env("RUNE_NOMINAL_CHILD", mode)
					.output()
					.unwrap();
				assert!(
					output.status.success(),
					"{mode}: {}\n{}",
					std::string::String::from_utf8_lossy(&output.stdout),
					std::string::String::from_utf8_lossy(&output.stderr)
				);
			}
			return;
		};
		assert!(GLOBAL.lock().unwrap().is_none());
		assert!(!contains(Nominal(0)));
		assert_eq!(lookup(1).unwrap(), None);
		assert_eq!(name_copy(Nominal(0), &mut [0; 256]), None);
		assert!(GLOBAL.lock().unwrap().is_none(), "reads never initialize");
		match mode.as_str() {
			"cold-allocation" => {
				let s = staged(&[(1, "A")]);
				let mut remap = s.prepare_remap().unwrap();
				rune_alloc::limit::with(0, || {
					assert_eq!(
						s.commit(&mut remap).unwrap_err().kind,
						ErrorKind::Allocation
					);
				})
				.call();
				assert!(GLOBAL.lock().unwrap().is_none());
			}
			"cold-capacity" => {
				// A batch can fit staging yet exceed the destination's capacity.
				let s = staged(&[(1, "A"), (2, "B"), (3, "C")]);
				let mut remap = s.prepare_remap().unwrap();
				assert!(with_prepared(
					Some(small(2, 4, 8)),
					prepare_records(s.entries.iter().map(|e| s.record(e))).unwrap(),
					|table| table.commit(&s, &mut remap)
				)
				.is_err());
				assert_eq!(global_len(), 0);
			}
			"poison" => {
				let _ = std::thread::spawn(|| {
					let _guard = GLOBAL.lock().unwrap();
					panic!("poison seam");
				})
				.join();
				assert_eq!(
					intern(1, "A").unwrap_err().refusal,
					Some(Refusal::TypeTable)
				);
				assert_eq!(name_copy(Nominal(0), &mut [0; 256]), None);
				assert!(!contains(Nominal(0)));
			}
			"concurrent" => {
				let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
				let mut threads = std::vec::Vec::new();
				for (hash, name) in [(2, "First"), (3, "Second")] {
					let barrier = barrier.clone();
					threads.push(std::thread::spawn(move || {
						let s = staged(&[(1, name), (hash, name)]);
						let mut remap = s.prepare_remap().unwrap();
						let candidate = Interner::with_limits(Limits::default()).unwrap();
						barrier.wait();
						with_prepared(
							Some(candidate),
							prepare_records(s.entries.iter().map(|e| s.record(e))).unwrap(),
							|table| table.commit(&s, &mut remap),
						)
						.unwrap();
						(remap[0], name)
					}));
				}
				let a = threads.remove(0).join().unwrap();
				let b = threads.remove(0).join().unwrap();
				assert_eq!(a.0, b.0);
				assert_eq!(global_len(), 3);
				let mut buffer = [0; 256];
				let n = name_copy(a.0, &mut buffer).unwrap();
				assert!([a.1.as_bytes(), b.1.as_bytes()].contains(&&buffer[..n]));
				let again = intern(1, "later").unwrap();
				assert_eq!(again, a.0);
				assert_eq!(name_copy(a.0, &mut [0; 256]), Some(n));
				struct Reentrant;
				impl core::fmt::Display for Reentrant {
					fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
						assert!(lookup(1).unwrap().is_some());
						f.write_str("Unlocked")
					}
				}
				let formatted = intern_fmt(4, format_args!("{}", Reentrant)).unwrap();
				assert_eq!(name_copy(formatted, &mut [0; 256]), Some(8));
				assert_eq!(global_len(), 4);
			}
			"lifecycle" => {
				use super::super::frontend;
				use crate::{Context, Diagnostics, Source, Sources};
				let context = Context::with_default_modules().unwrap();
				let compile = |text: &str| {
					let mut sources = Sources::new();
					sources.insert(Source::memory(text).unwrap()).unwrap();
					frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1)
				};
				let text = "struct A { x } struct B { x } struct C { x } pub fn main() { let a = A { x: 1 }; let b = B { x: 2 }; let c = C { x: 3 }; a.x + b.x + c.x }";
				let _p = compile(text).unwrap();
				assert_eq!(global_len(), 3);
				let _p = compile(text).unwrap();
				assert_eq!(global_len(), 3);
				let _p = compile("struct A { x } struct D { x } pub fn main() { let a = A { x: 1 }; let d = D { x: 2 }; a.x + d.x }").unwrap();
				assert_eq!(global_len(), 4);
				assert!(compile(
					"struct Late { x } pub fn main() { let p = Late { x: 1 }; async { p.x } }"
				)
				.is_err());
				assert_eq!(global_len(), 4);
				assert!(compile(
					"struct Missing { x } pub fn main() { let p = Missing { x: 1 }; nowhere(p) }"
				)
				.is_err());
				assert_eq!(global_len(), 4);
				assert!(compile(
					"struct Bad { x } pub fn main() { let p = Bad { x: 1 }; let c = absent; p.x }"
				)
				.is_err());
				assert_eq!(global_len(), 4);
				// Finish allocation and remap allocation both precede commit.
				super::super::code::transaction_tests::allocation_refusals_publish_nothing();
				assert_eq!(global_len(), 4);
				let name = "N".repeat(256);
				compile(&std::format!(
					"struct {name} {{ x }} pub fn main() {{ {name} {{ x: 1 }} }}"
				))
				.unwrap();
				assert_eq!(global_len(), 5);
				let too_long = "L".repeat(257);
				let error = compile(&std::format!(
					"struct {too_long} {{ x }} pub fn main() {{ {too_long} {{ x: 1 }} }}"
				))
				.err()
				.unwrap();
				assert_eq!(error.refusal, Some(Refusal::TypeTable));
				assert_eq!(global_len(), 5, "overlong name never publishes");
			}
			_ => panic!("unknown child mode"),
		}
	}
}
