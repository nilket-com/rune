//! Cold iterator kinds share the pre-existing boxed payload discriminant.
use super::*;
use crate::experimental_core::error::{Error, TypeName};
#[derive(Debug)]
pub(super) struct State {
	pub(super) source: Option<Handle>,
	pub(super) kind: Kind,
}
#[derive(Debug)]
pub(super) enum Kind {
	Range(ranges::RangeCursor),
	Split(Box<text_methods::Split>),
	Chars(Box<text_pipeline::Chars>),
	Skip(Box<text_pipeline::Adapter>),
	Take(Box<text_pipeline::Adapter>),
}
impl State {
	#[cold]
	pub(super) fn diagnostic_type(&self) -> TypeName {
		match &self.kind {
			Kind::Range(state) => state.diagnostic_type(),
			Kind::Split(state) => state.kind(),
			Kind::Chars(_) => TypeName::Chars,
			Kind::Skip(_) => TypeName::Skip,
			Kind::Take(_) => TypeName::Take,
		}
	}
	pub(super) fn is_split(&self) -> bool {
		!matches!(self.kind, Kind::Range(_))
	}
}
impl Store {
	#[cold]
	#[inline(never)]
	pub(super) fn prepare_boxed_detach(&mut self, payload: &mut Payload) -> Result<Option<Error>> {
		match payload {
			Payload::SortGuard(state) if state.active => {
				self.sort_restore(state)?;
				Ok(None)
			}
			Payload::OtherRangeIterator(state) => Ok(self.detach_boxed_lease(state).err()),
			_ => Ok(None),
		}
	}
	#[cold]
	#[inline(never)]
	pub(super) fn detach_boxed_lease(&mut self, state: &State) -> Result<()> {
		if matches!(state.kind, Kind::Chars(_) | Kind::Split(_)) {
			if let Some(h) = state.source {
				let entry = self.entry_mut(h)?;
				entry.readers = entry
					.readers
					.checked_sub(1)
					.ok_or_else(|| err(ErrorKind::InvalidHandle))?;
			}
		}
		Ok(())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn boxed_layout_charges_the_common_source_prefix() {
		assert_eq!(size_of::<State>(), 56);
		assert_eq!(size_of::<ranges::RangeCursor>(), 40);
		assert_eq!(size_of::<text_methods::Split>(), 40);
	}
}
