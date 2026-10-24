//! Cold diagnostic identity for opaque objects; release/access paths never consult it.
use super::*;
use crate::experimental_core::error::TypeName;

impl Store {
	#[cold]
	pub(super) fn prepare_opaque_drop(&mut self, h: Handle) -> Result<Option<TypeName>> {
		if self.entry(h)?.tag()? != Type::Iterator {
			return Ok(None);
		}
		let name = self
			.diagnostic_type(Value::Heap(h))
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		if self.opaque_names.len() >= self.opaque_sweep_at {
			let slots = &self.slots;
			self.opaque_names.retain(|(key, _)| {
				slots.get(key.slot as usize).is_some_and(|s| {
					s.generation == key.generation
						&& matches!(
							s.entry.as_ref().and_then(|e| e.payload.as_ref()),
							Some(Payload::Moved(Type::Iterator))
						)
				})
			});
			// Bound growth between sweeps against the last sweep's survivors, not the
			// instantaneous live count: frees deliberately do not touch this metadata.
			self.opaque_sweep_at = self.opaque_names.len().saturating_mul(2).max(8);
		}
		self.opaque_names.try_reserve(1)?;
		Ok(Some(name))
	}

	#[cold]
	pub(super) fn moved_iterator_name(&self, h: Handle) -> Option<TypeName> {
		if !matches!(
			self.entry(h).ok()?.payload.as_ref(),
			Some(Payload::Moved(Type::Iterator))
		) {
			return None;
		}
		self.opaque_names
			.iter()
			.rev()
			.find_map(|(key, name)| (*key == h).then_some(*name))
	}
}

// The opaque fallback uses full item spelling, including :: inside generic
// arguments. Diagnostic TypeName Display intentionally omits those prefixes.
pub(super) fn full_name(name: TypeName) -> Option<&'static str> {
	Some(match name {
		TypeName::Range => "::std::ops::Range",
		TypeName::RangeInclusive => "::std::ops::RangeInclusive",
		TypeName::RangeFrom => "::std::ops::RangeFrom",
		TypeName::RangeTo => "::std::ops::RangeTo",
		TypeName::RangeToInclusive => "::std::ops::RangeToInclusive",
		TypeName::RangeFull => "::std::ops::RangeFull",
		TypeName::VecIter => "::std::slice::Iter",
		TypeName::ObjectIter => "::std::object::Iter",
		TypeName::OptionIter => "::std::option::Iter",
		TypeName::RangeIter => "::std::ops::RangeIter<::std::i64>",
		TypeName::UnsignedRangeIter => "::std::ops::RangeIter<::std::u64>",
		TypeName::CharRangeIter => "::std::ops::RangeIter<::std::char>",
		TypeName::RangeFromIter => "::std::ops::RangeFromIter<::std::i64>",
		TypeName::UnsignedRangeFromIter => "::std::ops::RangeFromIter<::std::u64>",
		TypeName::CharRangeFromIter => "::std::ops::RangeFromIter<::std::char>",
		TypeName::RangeInclusiveIter => "::std::ops::RangeInclusiveIter<::std::i64>",
		TypeName::UnsignedRangeInclusiveIter => "::std::ops::RangeInclusiveIter<::std::u64>",
		TypeName::CharRangeInclusiveIter => "::std::ops::RangeInclusiveIter<::std::char>",
		TypeName::StringSplit => "::std::string::Split<::std::string::String>",
		TypeName::CharSplit => "::std::string::Split<::std::char>",
		TypeName::Chars => "::std::string::Chars",
		TypeName::Skip => "::std::iter::Skip",
		TypeName::Take => "::std::iter::Take",
		_ => return None,
	})
}
