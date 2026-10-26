//! Private, effective-key formatting authority. Never inferred from public metadata.
use crate::alloc::prelude::*;
use crate::alloc::{self, HashSet};
use crate::sync::Arc;
use crate::{runtime::Protocol, Hash};

#[derive(Default)]
pub(crate) struct State {
	pub(crate) trusted_install: bool,
	pub(crate) modified: bool,
	certificate: Option<Arc<Certificate>>,
}
impl State {
	pub(crate) fn begin(&mut self, key: Hash, marked: bool) -> alloc::Result<()> {
		if self.trusted_install || (marked && self.certificate.is_none()) {
			return Ok(());
		}
		// A published certificate is immutable. Copy before the attempted insertion,
		// including conflicts; a failed attempt never restores an earlier proof.
		if let Some(current) = self.certificate.as_mut() {
			if Arc::get_mut(current).is_none() {
				let next = Arc::try_new(Certificate {
					keys: current.keys.try_clone()?,
				})?;
				*current = next;
			}
		} else {
			self.certificate = Some(Arc::try_new(Certificate {
				keys: HashSet::new(),
			})?);
		}
		if !marked {
			Arc::get_mut(self.certificate.as_mut().unwrap())
				.unwrap()
				.keys
				.try_insert(key)?;
			self.modified |= crate::compile::context::formats_key(key);
		}
		Ok(())
	}
	pub(crate) fn permits(&self, receiver: Hash) -> bool {
		self.certificate
			.as_ref()
			.is_none_or(|c| c.permits(receiver))
	}
	pub(crate) fn snapshot(&self) -> Option<Arc<Certificate>> {
		self.certificate.clone()
	}
}

pub(crate) struct Certificate {
	keys: HashSet<Hash>,
}
impl Certificate {
	/// Equivalent independently built Contexts share the same authority.
	pub(crate) fn equivalent(&self, other: &Self) -> bool {
		self.keys == other.keys
	}

	/// Mirror runtime/value.rs dispatch: Dynamic script nominal debug is structural;
	/// only display_fmt_with consults its native formatter. Children are checked
	/// separately by the entry walk and builtin effective-key predicate.
	pub(crate) fn permits(&self, receiver: Hash) -> bool {
		!self
			.keys
			.contains(&Hash::associated_function(receiver, &Protocol::DISPLAY_FMT))
	}
}
