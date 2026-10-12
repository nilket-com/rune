//! Process-wide, append-only interner of nominal type identities (hash plus a
//! name), indexed by [`Nominal`]. The same hash gets the same identity in
//! every program and store for the life of the process; nothing depends on a
//! `Code` or `Store` lifetime.
//!
//! Storage is two exactly reserved buffers (entry records and one name
//! arena). Compilation stages identities locally; admission prepares all storage
//! before publishing one atomic batch. Appends under the lock never allocate.
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
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub(super) const PROVISIONAL: u32 = 1 << 31;

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

		self.check_room(1, name.len())?;
		if name.len() > self.limits.name_bytes {
			return Err(refused());
		}
		let id = Nominal(self.entries.len() as u32);
		let before = (self.entries.len(), self.names.len());
		if self.append(hash, name).is_err() {
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
	fn append(&mut self, hash: u64, name: &str) -> Result<()> {
		let entry = Entry {
			hash,
			start: self.names.len() as u32,
			len: name.len() as u32,
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
			if let Some(id) = self.lookup(entry.hash) {
				remap[i] = id;
			} else {
				let name = staging.name(entry);
				if name.len() > self.limits.name_bytes || u32::try_from(name.len()).is_err() {
					return Err(refused());
				}
				count = count.checked_add(1).ok_or_else(refused)?;
				bytes = bytes.checked_add(name.len()).ok_or_else(refused)?;
			}
		}
		self.check_room(count, bytes)?;
		let before = (self.entries.len(), self.names.len());
		let capacities = (self.entries.capacity(), self.names.capacity());
		for (i, entry) in staging.entries.iter().enumerate() {
			if let Some(id) = self.lookup(entry.hash) {
				remap[i] = id;
				continue;
			}
			remap[i] = Nominal(self.entries.len() as u32);
			if self.append(entry.hash, staging.name(entry)).is_err() {
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

	pub(super) fn stage(&mut self, hash: u64, name: core::fmt::Arguments<'_>) -> Result<Nominal> {
		if let Some(id) = lookup(hash)? {
			return Ok(id);
		}
		if let Some(i) = self.entries.iter().position(|e| e.hash == hash) {
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
		// Reserve both buffers before changing either logical length.
		self.entries
			.try_reserve(1)
			.map_err(|_| refused_by(ErrorKind::Allocation, "staged nominal entries"))?;
		self.names
			.try_reserve(len)
			.map_err(|_| refused_by(ErrorKind::Allocation, "staged nominal names"))?;
		let entry = Entry {
			hash,
			start: self.names.len() as u32,
			len: len as u32,
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
		with_global(|table| table.commit(self, remap))
	}
}

static GLOBAL: Mutex<Option<Interner>> = Mutex::new(None);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// The atomic flag is only a monotonic allocation hint. The mutex is authoritative.
/// A losing candidate is dropped after the guard, including every error path.
fn with_global<T>(f: impl FnOnce(&mut Interner) -> Result<T>) -> Result<T> {
	let candidate = if !INITIALIZED.load(Ordering::Acquire) {
		Some(Interner::with_limits(Limits::default())?)
	} else {
		None
	};
	with_prepared(candidate, f)
}

fn with_prepared<T>(
	mut candidate: Option<Interner>,
	f: impl FnOnce(&mut Interner) -> Result<T>,
) -> Result<T> {
	let result = {
		let mut guard = GLOBAL.lock().map_err(|_| refused())?;
		if guard.is_none() {
			*guard = candidate.take();
			INITIALIZED.store(true, Ordering::Release);
		}
		f(guard.as_mut().ok_or_else(refused)?)
	};
	drop(candidate);
	result
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn intern(hash: u64, name: &str) -> Result<Nominal> {
	with_global(|table| table.intern(hash, name))
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
	GLOBAL
		.lock()
		.ok()
		.and_then(|guard| guard.as_ref().map(|table| table.contains(nominal)))
		.unwrap_or(false)
}

/// Copy a diagnostic name without initializing the table or holding its lock while formatting.
pub(crate) fn name_copy(nominal: Nominal, buffer: &mut [u8; 256]) -> Option<usize> {
	let guard = GLOBAL.lock().ok()?;
	let name = guard.as_ref()?.name(nominal)?;
	buffer
		.get_mut(..name.len())?
		.copy_from_slice(name.as_bytes());
	Some(name.len())
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
				assert!(
					with_prepared(Some(small(2, 4, 8)), |table| table.commit(&s, &mut remap))
						.is_err()
				);
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
						with_prepared(Some(candidate), |table| table.commit(&s, &mut remap))
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
					"struct Late { x } pub fn main() { let p = Late { x: 1 }; let c = || 1; p.x }"
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
