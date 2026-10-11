//! Process-wide, append-only interner of nominal type identities (hash plus a
//! name), indexed by [`Nominal`]. The same hash gets the same identity in
//! every program and store for the life of the process; nothing depends on a
//! `Code` or `Store` lifetime.
//!
//! Storage is two exactly reserved buffers (entry records and one name
//! arena), so the table's allocation is known at construction and admission
//! never allocates: it scans, checks the caps, and appends within capacity.
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
use std::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
	pub entries: usize,
	pub name_bytes: usize,
	pub total_name_bytes: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: 4096,
			name_bytes: 256,
			total_name_bytes: 256 << 10,
		}
	}
}

#[derive(Clone, Copy)]
struct Entry {
	hash: u64,
	start: u32,
	len: u32,
}

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
			|| limits.entries > u32::MAX as usize
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
		self.entries
			.iter()
			.position(|e| e.hash == hash)
			.map(|i| Nominal(i as u32))
	}

	/// Identity is the hash; the first name survives. Never allocates.
	pub(crate) fn intern(&mut self, hash: u64, name: &str) -> Result<Nominal> {
		if let Some(id) = self.lookup(hash) {
			return Ok(id);
		}

		// Caps, then capacity invariants, before any append.
		if self.entries.len() >= self.limits.entries || name.len() > self.limits.name_bytes {
			return Err(refused());
		}

		let start = self.names.len();
		let end = start.checked_add(name.len()).ok_or_else(refused)?;

		if end > self.limits.total_name_bytes
			|| end > self.names.capacity()
			|| self.entries.len() >= self.entries.capacity()
		{
			return Err(refused());
		}

		let id = u32::try_from(self.entries.len()).map_err(|_| refused())?;
		let (start, len) = (
			u32::try_from(start).map_err(|_| refused())?,
			u32::try_from(name.len()).map_err(|_| refused())?,
		);

		// Publish within reserved capacity: neither append can reallocate.
		self.names
			.try_push_str(name)
			.map_err(|_| refused_by(ErrorKind::Allocation, "nominal names"))?;
		self.entries
			.try_push(Entry { hash, start, len })
			.map_err(|_| refused_by(ErrorKind::Allocation, "nominal entries"))?;
		Ok(Nominal(id))
	}

	/// `intern` with a formatted name, written to a stack buffer bounded by the
	/// name cap; the duplicate lookup happens before any formatting.
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
		self.entries.get(nominal.0 as usize).map(|e| e.hash)
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn name(&self, nominal: Nominal) -> Option<&str> {
		self.entries
			.get(nominal.0 as usize)
			.map(|e| &self.names[e.start as usize..(e.start + e.len) as usize])
	}

	pub(crate) fn contains(&self, nominal: Nominal) -> bool {
		(nominal.0 as usize) < self.entries.len()
	}
}

static GLOBAL: Mutex<Option<Interner>> = Mutex::new(None);

/// Run `f` on the process-wide table, creating it with the default limits on
/// first use. Private: the global table is append-only and is never handed
/// out as a mutable value. A poisoned lock is the named refusal.
fn with_global<T>(f: impl FnOnce(&mut Interner) -> Result<T>) -> Result<T> {
	let mut guard = GLOBAL.lock().map_err(|_| refused())?;
	if guard.is_none() {
		*guard = Some(Interner::with_limits(Limits::default())?);
	}

	f(guard.as_mut().expect("initialised above"))
}

/// The one issuing route: the identity of `hash` in the process-wide table,
/// created on first use with `name`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn intern(hash: u64, name: &str) -> Result<Nominal> {
	with_global(|table| table.intern(hash, name))
}

pub(crate) fn intern_fmt(hash: u64, name: core::fmt::Arguments<'_>) -> Result<Nominal> {
	with_global(|table| table.intern_fmt(hash, name))
}

/// Whether an identity names an entry of the process-wide table (validation
/// of literals and host-supplied nominals).
pub fn contains(nominal: Nominal) -> bool {
	with_global(|table| Ok(table.contains(nominal))).unwrap_or(false)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn small(entries: usize, name_bytes: usize, total_name_bytes: usize) -> Interner {
		Interner::with_limits(Limits {
			entries,
			name_bytes,
			total_name_bytes,
		})
		.unwrap()
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
}
