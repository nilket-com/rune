//! The exit record shared by the native region function and safe Rust (DESIGN; the native
//! side is compiled but never called in P0).
//!
//! Native signature (x86-64 System V): `unsafe extern "C" fn(buf: *mut i64, record: *mut
//! ExitRecord) -> u32`.
//!
//! - `buf` points at exactly `region.slots.len()` (at most 64) initialized `i64`s, one per
//!   local slot: a guarded live-in's payload (`Bool` as exactly 0 or 1), zero otherwise.
//!   Native code reads and writes only `buf[0..n]`, at compile-time constant offsets
//!   `8 * i`, `i < n` (audited on the CLIF).
//! - `record` is fully initialized by safe Rust before the call: `fuel` is the entry fuel,
//!   `written` 0, `bools` 0, `status` 0, `exit` `u32::MAX`, `pc` `u32::MAX`, `reserved` 0.
//! - The return value equals the `status` the native code stored. Safe Rust validates the
//!   record against the region's static exit table before it mutates anything; an invalid
//!   record is an interpreter fallback at the header with the original state.

/// The `#[repr(C)]` exit record. Offsets: fuel 0, written 8, bools 16, status 24, exit 28,
/// pc 32, reserved 36; size 40, align 8. Field widths are fixed (no `usize`): fuel is a `u64`
/// (the feature's only target is x86-64, where `usize` is 64 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct ExitRecord {
	/// In: the entry fuel. Out: `Continue`: remaining fuel; `Fuel`: 0; `Retry`: the fuel
	/// before the faulting instruction's own charge.
	pub(crate) fuel: u64,
	/// Out: bit `i` set iff the region wrote local slot `i` on the path taken.
	pub(crate) written: u64,
	/// Out: for the region's `tracked` slots only, bit `i` set iff the value last written
	/// to local slot `i` is a `Bool`; every other bit is 0.
	pub(crate) bools: u64,
	/// Out: a [`Status`](super::Status) discriminant (1, 2, 3); 0 means "not written".
	pub(crate) status: u32,
	/// Out: the index of the exit site in the region's exit table.
	pub(crate) exit: u32,
	/// Out: the interpreter pc to resume at; must equal the table entry's pc.
	pub(crate) pc: u32,
	/// Always 0; keeps the size a multiple of the alignment.
	pub(crate) reserved: u32,
}

impl ExitRecord {
	pub(crate) const FUEL: i32 = 0;
	pub(crate) const WRITTEN: i32 = 8;
	pub(crate) const BOOLS: i32 = 16;
	pub(crate) const STATUS: i32 = 24;
	pub(crate) const EXIT: i32 = 28;
	pub(crate) const PC: i32 = 32;
	pub(crate) const SIZE: usize = 40;
	pub(crate) const ALIGN: usize = 8;

	pub(crate) fn new(fuel: u64) -> Self {
		Self {
			fuel,
			written: 0,
			bools: 0,
			status: 0,
			exit: u32::MAX,
			pc: u32::MAX,
			reserved: 0,
		}
	}
}

const _: () = {
	assert!(core::mem::offset_of!(ExitRecord, fuel) == ExitRecord::FUEL as usize);
	assert!(core::mem::offset_of!(ExitRecord, written) == ExitRecord::WRITTEN as usize);
	assert!(core::mem::offset_of!(ExitRecord, bools) == ExitRecord::BOOLS as usize);
	assert!(core::mem::offset_of!(ExitRecord, status) == ExitRecord::STATUS as usize);
	assert!(core::mem::offset_of!(ExitRecord, exit) == ExitRecord::EXIT as usize);
	assert!(core::mem::offset_of!(ExitRecord, pc) == ExitRecord::PC as usize);
	assert!(core::mem::size_of::<ExitRecord>() == ExitRecord::SIZE);
	assert!(core::mem::align_of::<ExitRecord>() == ExitRecord::ALIGN);
	assert!(super::MAX_REGION_SLOTS <= 64);
};
