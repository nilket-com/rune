//! Opt-in fresh execution core (feature `fresh-core`). UNSTABLE: no semver promise.
//!
//! The core runs an admitted subset of Rune. [`frontend::compile`](crate::experimental_core::frontend::compile) turns sources into a
//! [`frontend::Program`](crate::experimental_core::frontend::Program); an [`execute::Executor`](crate::experimental_core::execute::Executor) invokes its functions against a
//! [`heap::Store`](crate::experimental_core::heap::Store), with arguments and results passed as [`value::Root`](crate::experimental_core::value::Root)s.
//!
//! Everything outside the subset is a compile-time [`error::Refusal`](crate::experimental_core::error::Refusal), and the caller
//! decides whether to run the program on the stock `Vm` instead. Nothing here falls back
//! automatically.
//!
//! `println!` is admitted only when the context's `::std::io::println` is the std stdio
//! builtin; capture, disable, custom or absent printers are refused at compile time.
//!
//! Constant Display and Debug template specifications admit fill, alignment,
//! signs, zero padding, width and precision up to 65,535. Radix and pointer
//! specifications, and dynamic precision, remain compile-time refusals.
//! Width counts Unicode characters; formatting limits count UTF-8 bytes.
//! The executor's formatting limits still apply before any formatted output.
//!
//! # Declared differences from the stock VM
//!
//! - `&&` and `||` require `bool` operands; a non-`bool` operand is a
//!   [`error::ErrorKind::Type`](crate::experimental_core::error::ErrorKind::Type) error.
//! - Object debug text lists fields in insertion order.
//! - Run-time error classes ([`error::ErrorKind`](crate::experimental_core::error::ErrorKind)) are coarser than the stock error
//!   kinds.
//!
//! Template errors retain the source invocation's location rather than the
//! stock compiler's generated-template prefix span. Direct Debug rendering
//! reports a moved-value error without a native-handler argument wrapper.
//!
//! `println!` follows Rust stdout buffering without a cumulative output cap or
//! explicit flush (see [`execute`](crate::experimental_core::execute)).
//!
//! Internals are sealed:
//!
//! ```compile_fail
//! let _ = rune::experimental_core::value::Value::Unit;
//! ```
//!
//! ```compile_fail
//! let _ = rune::experimental_core::code::Builder::default();
//! ```
//!
//! ```compile_fail
//! let _ = rune::experimental_core::nominal::intern(0, "x");
//! ```
//!
//! Cancellation needs an exclusive borrow of the executor, which every invocation holds for
//! its whole duration, so no request can arrive while an invocation runs. The
//! `fresh-core-native` tier checks a pending request once, at region entry, and relies on
//! this. This tripwire stops compiling (and the test fails) if cancellation ever becomes
//! reachable through a shared borrow; the native tier's cancellation contract must then be
//! redesigned first.
//!
//! ```compile_fail,E0596
//! use rune::experimental_core::execute::Executor;
//! fn cancel_through_shared(e: &Executor) { e.request_cancel(); }
//! ```
//!
//! ```
//! use rune::experimental_core::execute::Executor;
//! fn cancel_through_exclusive(e: &mut Executor) { e.request_cancel(); }
//! ```

#![forbid(unsafe_code)]

pub(crate) mod arithmetic;
pub(crate) mod code;
mod detail;
pub mod error;
pub mod execute;
pub mod heap;
pub(crate) mod nominal;
mod receiver_domain;
pub mod value;

pub(crate) mod lower;

pub mod frontend;

pub(crate) mod constants;

#[cfg(feature = "fresh-core-native")]
pub(crate) mod region;

#[cfg(test)]
mod tests;

mod format_spec;
