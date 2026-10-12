//! THE UNSAFE BOUNDARY of the P0 native tier. Compiled, never called in P0: no code path
//! constructs a [`NativeRegion`] or calls [`NativeRegion::call`] until the execution GO.
//!
//! Ownership and lifetime:
//!
//! - A [`NativeRegion`] owns its `JITModule` (and with it the executable memory) for its
//!   whole life. The entry pointer is private and never exposed; no IR, module or pointer
//!   leaves this type.
//! - The only way to obtain one is [`NativeRegion::load`] from a
//!   [`Compiled`](super::lower::Compiled), a sealed capability that only `lower::compile`
//!   constructs, after every gate passed: its fields are private, so safe code elsewhere in
//!   the crate cannot forge or alter one. The audited bytes are exactly the bytes copied
//!   into executable memory (`define_function_bytes`), with no relocations. The test-only
//!   machine-gate diagnostic returns a `Listing`, which cannot be loaded.
//! - `Drop` frees the module's memory with `JITModule::free_memory`. `call` borrows `&self`
//!   and cannot outlive the region, so no call can run after (or during) the free.
//! - The raw entry pointer makes the type `!Send` and `!Sync`: a region is used only on the
//!   thread that loaded it. Cranelift's `JITModule` is itself `!Sync` (it holds a
//!   `RefCell`). Sharing compiled regions across threads (a `Program` is `Send`) is an open
//!   design question, not solved here.
//!
//! Safety argument of the call (all must hold; each is established by safe code):
//!
//! 1. The code was produced by `lower::compile` for this region's slot count with the
//!    shared ISA flags, passed the IR and machine audits (no trap site, no call, no
//!    relocation, no divide instruction; every IR-level load/store is `buf[0..n]` or the
//!    record at a constant offset), and was finalized read+execute by
//!    `JITModule::finalize_definitions`. The machine code also touches memory the IR does
//!    not name: backend stack spills in its own frame and read-only constant pools in the
//!    same code allocation.
//!
//! Cranelift is a TRUSTED code generator here. The textual VCode checks and the external
//! fixture disassembly corroborate the contract; they are not a general proof that the
//! machine code is safe.
//! 2. The signature is `extern "C" fn(*mut i64, *mut ExitRecord) -> u32` under System V,
//!    which is the signature `lower::signature` declares to Cranelift.
//! 3. `buf` is a live, exclusive, initialized `&mut [i64]` of exactly `slots` elements,
//!    and `record` a live, exclusive, initialized `&mut ExitRecord` (`#[repr(C)]`, 40
//!    bytes, align 8): checked by `call` before the call.
//! 4. The function only loops while fuel lasts (every iteration passes the charged header,
//!    which exits at fuel 0) and never unwinds: there is no call, so no panic can cross it.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
// Not wired into execution in P0.
#![allow(dead_code)]
use std::prelude::rust_2021::*;

use super::lower::{self, Compiled};
use super::NativeRefusal;
use crate::experimental_core::region::abi::ExitRecord;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};

type Entry = unsafe extern "C" fn(*mut i64, *mut ExitRecord) -> u32;

/// One loaded region: owned executable memory plus its private entry point.
pub(crate) struct NativeRegion {
	/// `Some` until `Drop` frees it.
	module: Option<JITModule>,
	entry: *const u8,
	slots: usize,
}

impl NativeRegion {
	/// Copies audited bytes into fresh executable memory. Any error (allocation, `mmap` or
	/// `mprotect` failure) is an interpreter fallback; memory allocated before the failure
	/// is freed here, since nothing from it was finalized or called.
	pub(crate) fn load(compiled: &Compiled) -> Result<Self, NativeRefusal> {
		let isa = lower::isa()?;
		let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
		let result = (|| {
			let id = module
				.declare_function("region", Linkage::Local, &lower::signature())
				.map_err(|e| NativeRefusal::Codegen(e.to_string()))?;
			module
				.define_function_bytes(id, compiled.alignment(), compiled.bytes(), &[])
				.map_err(|e| NativeRefusal::Codegen(e.to_string()))?;

			// P1 tests only: an injected error after the code memory was allocated.
			#[cfg(test)]
			injected(InjectLoadError::AfterDefine)?;

			module
				.finalize_definitions()
				.map_err(|e| NativeRefusal::Codegen(e.to_string()))?;

			// P1 tests only: an injected error after the memory was made executable.
			#[cfg(test)]
			injected(InjectLoadError::AfterFinalize)?;

			Ok(module.get_finalized_function(id))
		})();
		match result {
			Ok(entry) => Ok(Self {
				module: Some(module),
				entry,
				slots: compiled.slots(),
			}),
			Err(refusal) => {
				#[cfg(test)]
				LOAD_ERROR_FREES.with(|c| c.set(c.get() + 1));

				// SAFETY: no pointer into this module's memory was handed out or called.
				unsafe { module.free_memory() };
				Err(refusal)
			}
		}
	}

	/// The one call site. Returns `None` (no call made) if the buffer or record does not
	/// satisfy the contract; the caller then falls back to the interpreter.
	pub(crate) fn call(&self, buf: &mut [i64], record: &mut ExitRecord) -> Option<u32> {
		if buf.len() != self.slots || self.module.is_none() || record.reserved != 0 {
			return None;
		}
		// SAFETY: see the module documentation, items 1 to 4. `entry` came from
		// `get_finalized_function` of the module `self` still owns (it is freed only in
		// `Drop`, which cannot run while `&self` is borrowed); `Entry` is the signature the
		// code was compiled for; both pointers come from live exclusive borrows checked
		// above, and the code touches only `buf[0..slots]` and the 40-byte record.
		let status = unsafe {
			let entry = core::mem::transmute::<*const u8, Entry>(self.entry);
			entry(buf.as_mut_ptr(), record as *mut ExitRecord)
		};
		Some(status)
	}
}

impl Drop for NativeRegion {
	fn drop(&mut self) {
		if let Some(module) = self.module.take() {
			// SAFETY: `&mut self` proves no `call` is running or can start, and the entry
			// pointer is private to this value, which is being dropped.
			unsafe { module.free_memory() };
		}
	}
}

/// P1 tests only: where `NativeRegion::load` returns an injected error. Only an `Err` can be
/// injected; the bytes loaded are always the audited `Compiled` bytes.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InjectLoadError {
	/// After `define_function_bytes` allocated and filled the code memory.
	AfterDefine,
	/// After `finalize_definitions` made it read+execute.
	AfterFinalize,
}

#[cfg(test)]
std::thread_local! {
	static INJECT_LOAD_ERROR: core::cell::Cell<Option<InjectLoadError>> =
		const { core::cell::Cell::new(None) };

	/// How many times this thread's `load` took its error cleanup path.
	static LOAD_ERROR_FREES: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Arms (or with `None` disarms) the injected load error for this thread.
#[cfg(test)]
pub(crate) fn inject_load_error_for_test(stage: Option<InjectLoadError>) {
	INJECT_LOAD_ERROR.with(|c| c.set(stage));
}

/// The number of error-path frees `load` performed on this thread.
#[cfg(test)]
pub(crate) fn load_error_frees_for_test() -> usize {
	LOAD_ERROR_FREES.with(|c| c.get())
}

#[cfg(test)]
fn injected(stage: InjectLoadError) -> Result<(), NativeRefusal> {
	if INJECT_LOAD_ERROR.with(|c| c.get()) == Some(stage) {
		return Err(NativeRefusal::Codegen("injected load error (test)".into()));
	}
	Ok(())
}

/// P1c tests only: one parsed line of `/proc/<pid>/maps`, kept verbatim in `line`.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapInfo {
	pub(crate) start: u64,
	pub(crate) end: u64,
	pub(crate) perms: String,
	pub(crate) offset: u64,
	/// `None` for an anonymous mapping; otherwise everything after the inode field, which
	/// may contain spaces.
	pub(crate) path: Option<String>,
	pub(crate) line: String,
}

#[cfg(test)]
impl MapInfo {
	pub(crate) fn writable(&self) -> bool {
		self.perms.as_bytes().get(1) == Some(&b'w')
	}

	pub(crate) fn executable(&self) -> bool {
		self.perms.as_bytes().get(2) == Some(&b'x')
	}

	/// Whether this mapping shares at least one byte with `[start, end)`.
	pub(crate) fn overlaps(&self, start: u64, end: u64) -> bool {
		self.start < end && start < self.end
	}
}

/// P1c tests only: parses one maps line (`start-end perms offset dev inode [path]`).
/// Returns `None` for a malformed line. Touches no file system.
#[cfg(test)]
pub(crate) fn parse_maps_line(line: &str) -> Option<MapInfo> {
	fn field<'a>(rest: &mut &'a str) -> Option<&'a str> {
		let text = rest.trim_start_matches(' ');
		let end = text.find(' ').unwrap_or(text.len());
		let (field, tail) = text.split_at(end);
		*rest = tail;
		(!field.is_empty()).then_some(field)
	}

	let mut rest = line;
	let range = field(&mut rest)?;
	let perms = field(&mut rest)?;
	let offset = field(&mut rest)?;
	let dev = field(&mut rest)?;
	let inode = field(&mut rest)?;

	let (start, end) = range.split_once('-')?;
	let start = u64::from_str_radix(start, 16).ok()?;
	let end = u64::from_str_radix(end, 16).ok()?;
	let offset = u64::from_str_radix(offset, 16).ok()?;

	if start >= end || perms.len() != 4 || !dev.contains(':') {
		return None;
	}

	inode.parse::<u64>().ok()?;
	let path = rest.trim_start_matches(' ');

	Some(MapInfo {
		start,
		end,
		perms: perms.into(),
		offset,
		path: (!path.is_empty()).then(|| path.into()),
		line: line.into(),
	})
}

/// P1c tests only: every mapping of a maps text that overlaps `[start, end)`. A line that
/// does not parse is a test failure, never skipped.
#[cfg(test)]
pub(crate) fn maps_overlapping(maps: &str, start: u64, end: u64) -> Vec<MapInfo> {
	maps.lines()
		.map(|line| parse_maps_line(line).unwrap_or_else(|| core::panic!("maps line: {line:?}")))
		.filter(|m| m.overlaps(start, end))
		.collect()
}

#[cfg(test)]
impl NativeRegion {
	/// P1c tests only: the mapping of `maps` (a `/proc/self/maps` snapshot taken by the
	/// caller) that contains this region's private entry address. Only the parsed mapping
	/// leaves this type; the pointer itself does not.
	pub(crate) fn entry_mapping(&self, maps: &str) -> Option<MapInfo> {
		self.module.as_ref()?;
		let address = self.entry as usize as u64;
		maps_overlapping(maps, address, address + 1).into_iter().next()
	}
}
