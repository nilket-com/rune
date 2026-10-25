//! Integers.

use core::cmp::Ordering;
use core::num::ParseIntError;

use crate as rune;
use crate::alloc::string::TryToString;
use crate::{ContextError, Module};

/// Unsigned integers.
///
/// This provides methods for computing over and parsing 64-bit unsigned integers.
#[rune::module(::std::u64)]
pub fn module() -> Result<Module, ContextError> {
	let mut m = Module::from_meta(self::module__meta)?;
	unsigned!(m, u64);
	#[cfg(feature = "execution")]
	{
		m.numeric = 0b0010_0000;
		m.number = crate::function::number::marker::U64;
	}
	Ok(m)
}

unsigned_fns!(u64);
