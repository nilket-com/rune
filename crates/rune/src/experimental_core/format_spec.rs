//! Validated template metadata in a function-local argument-buffer suffix.
use crate::experimental_core::error::{Error, ErrorKind, Result};
use crate::runtime::format::{Alignment, Flags, FormatSpec, Type};
use core::num::NonZeroUsize;

pub(crate) const WORDS: usize = 6;
pub(crate) const MAX_ARGUMENT: usize = u16::MAX as usize;
fn invalid() -> Error {
	Error::new(ErrorKind::InvalidCode, "template spec")
}

#[derive(Clone, Copy)]
pub(crate) struct Spec(pub(crate) [u32; WORDS]);
impl Spec {
	pub(crate) fn from_words(words: &[u32]) -> Result<Self> {
		let words: [u32; WORDS] = words.try_into().map_err(|_| invalid())?;
		if words[0] & !15 != 0
			|| char::from_u32(words[1]).is_none()
			|| words[2] > 2
			|| words[3] > MAX_ARGUMENT as u32
			|| words[4] > (MAX_ARGUMENT + 1) as u32
			|| words[5] > 1
		{
			return Err(invalid());
		}
		Ok(Self(words))
	}
	pub(crate) fn from_spec(spec: FormatSpec) -> Result<Self> {
		let align = match spec.align {
			Alignment::Left => 0,
			Alignment::Center => 1,
			Alignment::Right => 2,
		};
		let kind = match spec.format_type {
			Type::Display => 0,
			Type::Debug => 1,
			_ => return Err(invalid()),
		};
		let width = spec.width.map_or(0, NonZeroUsize::get);
		let precision = spec
			.precision
			.map(|n| n.checked_add(1).ok_or_else(invalid))
			.transpose()?
			.unwrap_or(0);
		if width > MAX_ARGUMENT || precision > MAX_ARGUMENT + 1 {
			return Err(invalid());
		}
		Self::from_words(&[
			spec.flags.into_u32(),
			spec.fill as u32,
			align,
			width as u32,
			precision as u32,
			kind,
		])
	}
	pub(crate) fn legacy(self) -> Option<u32> {
		(self.0[..5] == [0, ' ' as u32, 0, 0, 0]).then_some(self.0[5])
	}
	pub(crate) fn runtime(self) -> Result<FormatSpec> {
		let w = self.0;
		let align = match w[2] {
			0 => Alignment::Left,
			1 => Alignment::Center,
			2 => Alignment::Right,
			_ => return Err(invalid()),
		};
		Ok(FormatSpec::new(
			Flags::from(w[0]),
			char::from_u32(w[1]).ok_or_else(invalid)?,
			align,
			NonZeroUsize::new(w[3] as usize),
			w[4].checked_sub(1).map(|p| p as usize),
			if w[5] == 0 {
				Type::Display
			} else {
				Type::Debug
			},
		))
	}
}
pub(crate) fn load(args: &[u32], descriptor: u32) -> Result<(bool, Option<FormatSpec>)> {
	if descriptor < 2 {
		return Ok((descriptor != 0, None));
	}
	let start = descriptor.checked_sub(2).ok_or_else(invalid)? as usize;
	let end = start.checked_add(WORDS).ok_or_else(invalid)?;
	let spec = Spec::from_words(args.get(start..end).ok_or_else(invalid)?)?;
	Ok((spec.0[5] != 0, Some(spec.runtime()?)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_invalid_record_words_and_descriptor_bounds() {
		let good = [0, ' ' as u32, 0, 12, 3, 0];
		assert!(Spec::from_words(&good).is_ok());
		for (index, value) in [
			(0, 16),
			(1, 0xd800),
			(1, 0x110000),
			(2, 3),
			(3, 65536),
			(4, 65537),
			(5, 2),
		] {
			let mut bad = good;
			bad[index] = value;
			assert_eq!(
				Spec::from_words(&bad).err().unwrap().kind,
				ErrorKind::InvalidCode
			);
		}
		for len in 0..WORDS {
			assert_eq!(
				Spec::from_words(&good[..len]).err().unwrap().kind,
				ErrorKind::InvalidCode
			);
		}
		for descriptor in [3, u32::MAX] {
			assert_eq!(
				load(&good, descriptor).err().unwrap().kind,
				ErrorKind::InvalidCode
			);
		}
	}

	#[test]
	fn legacy_defaults_and_precision_zero_round_trip() {
		for kind in 0..2 {
			let default = Spec::from_words(&[0, ' ' as u32, 0, 0, 0, kind]).unwrap();
			assert_eq!(default.legacy(), Some(kind));
			let (debug, spec) = load(&[], kind).unwrap();
			assert_eq!(debug, kind == 1);
			assert!(spec.is_none());
		}
		for precision in [0, 1, 65535] {
			let spec = Spec::from_words(&[0, '😀' as u32, 2, 65535, precision + 1, 1]).unwrap();
			let runtime = spec.runtime().unwrap();
			assert_eq!(runtime.precision, Some(precision as usize));
			assert_eq!(Spec::from_spec(runtime).unwrap().0, spec.0);
		}
	}
}
