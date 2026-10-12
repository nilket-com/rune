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
//! # Declared differences from the stock VM
//!
//! - `&&` and `||` require `bool` operands; a non-`bool` operand is a
//!   [`error::ErrorKind::Type`](crate::experimental_core::error::ErrorKind::Type) error.
//! - Object debug text lists fields in insertion order.
//! - Run-time error classes ([`error::ErrorKind`](crate::experimental_core::error::ErrorKind)) are coarser than the stock error
//!   kinds.
//! - `println!` output is capped at 128 KiB per invocation, and each line is flushed as
//!   it is written (see [`execute`](crate::experimental_core::execute)).
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

#![forbid(unsafe_code)]

pub(crate) mod arithmetic;
pub(crate) mod code;
pub mod error;
pub mod execute;
pub mod heap;
pub(crate) mod nominal;
pub mod value;

pub(crate) mod lower;

pub mod frontend;

pub(crate) mod constants;

#[cfg(feature = "fresh-core-native")]
pub(crate) mod region;

#[cfg(test)]
mod tests;
