//! Native tier backend for execution loop regions (x86-64 Linux, feature
//! `native-code`). UNSTABLE. Wired into execution (P2a) only through the thread-local
//! region cache in `experimental_core::region::cache`; there is no public API.
//!
//! - [`lower`]: safe translation of an analyzed [`Region`](crate::experimental_core::region::Region)
//!   into Cranelift IR with the exit protocol of
//!   [`ExitRecord`](crate::experimental_core::region::abi::ExitRecord), and compilation to
//!   machine code with a fixed baseline x86-64 ISA (no host feature detection).
//! - [`audit`]: the post-compile gate. A region whose IR or lowered machine code contains a
//!   trap site, a call, a relocation, a division instruction, or a memory access outside
//!   `buf[0..n]`, the exit record and N2's checked private scalar snapshot is refused:
//!   the interpreter keeps it.
//! - [`boundary`]: the only module allowed `unsafe` (executable memory ownership, the
//!   function-pointer conversion and the call).
//!
//! Everything except `boundary` is `deny(unsafe_code)`; the execution core itself stays
//! `forbid(unsafe_code)`.
#![deny(unsafe_code)]
// Some items (listings, test diagnostics) are used by tests only.
#![cfg_attr(not(test), allow(dead_code))]
use std::string::String;

pub(crate) mod audit;
pub(crate) mod boundary;
pub(crate) mod lower;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod exec_tests;

#[cfg(test)]
mod p2_tests;

#[cfg(test)]
mod p3a_tests;

#[cfg(test)]
mod p2b_tests;

#[cfg(test)]
mod b1_tests;

#[cfg(test)]
mod w13_tests;

#[cfg(test)]
mod r1_tests;

#[cfg(test)]
mod fuel_tests;

/// Most machine-code bytes for one region.
pub(crate) const MAX_REGION_CODE_BYTES: usize = 64 << 10;
/// Most machine-code bytes for all regions of one program (the code cache bound).
pub(crate) const MAX_PROGRAM_CODE_BYTES: usize = 1 << 20;

/// Why a region was not compiled. Every refusal is an interpreter fallback; nothing is
/// published for a refused region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NativeRefusal {
	/// The ISA or a setting could not be configured.
	Isa,
	/// The region exceeds a slot or exit bound of the ABI.
	Shape,
	/// The Cranelift verifier rejected the IR (a lowering bug).
	Verifier(String),
	/// Code generation failed.
	Codegen(String),
	/// The audit found something the P0 contract forbids.
	Audit(audit::Finding),
	/// The code-size bound per region or per program would be exceeded.
	Size,
}
