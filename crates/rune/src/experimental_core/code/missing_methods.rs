//! Cold absence proofs, owned by the compile and checked before execution.
use super::super::error::Refusal;
use super::*;
use crate::{Context, Hash};

impl CompileTransaction {
	pub(crate) fn missing_method(
		&mut self,
		context: &Context,
		query: &crate::query::QueryInner<'_>,
		pool: &crate::compile::Pool,
		name: Hash,
		effective: Hash,
	) -> Result<()> {
		if context.fresh_method_closed(name) || query.fresh_method_script.closed(name) {
			return Err(Error::refused(Refusal::Method));
		}
		if self.checked_absent.contains(&effective) {
			return Ok(());
		}
		// Source item hashes are a conservative superset of source nominal hashes.
		// The complete index is available before fresh lowering begins.
		for receiver in crate::function::fresh_methods::RECEIVERS
			.iter()
			.copied()
			.chain(query.items.keys().map(|id| pool.item_type_hash(*id)))
		{
			let hash = Hash::associated_function(receiver, effective);
			if context.lookup_function(hash).is_some() {
				return Err(Error::refused(Refusal::Method));
			}
			if self.builder.absent.len() >= MAX_CODE_BYTES / core::mem::size_of::<Hash>() {
				return Err(invalid());
			}
			self.builder.absent.try_push(hash)?;
		}
		self.checked_absent.try_push(effective)?;
		Ok(())
	}
}

impl Code {
	#[cold]
	#[inline(never)]
	pub(super) fn validate_absence(
		&self,
		runtime: Option<&crate::runtime::RuntimeContext>,
	) -> Result<()> {
		if let Some(runtime) = runtime {
			for hash in &self.absent {
				if runtime.function(hash).is_some() {
					return Err(invalid());
				}
			}
		}
		Ok(())
	}

	pub(crate) fn absent_method(&self, hash: Hash) -> bool {
		self.absent.contains(&hash)
	}
}

impl CompileTransaction {
	pub(crate) fn text_method(
		&mut self,
		c: &Context,
		q: &crate::query::QueryInner<'_>,
		p: &crate::compile::Pool,
		m: crate::function::fresh_text::Method,
	) -> Result<()> {
		use crate::TypeHash;
		if self.checked_text & m.bit() != 0 {
			return Ok(());
		}
		if !c.fresh_text_method(m) || q.fresh_method_script.closed(m.hash()) {
			return Err(Error::refused(Refusal::Method));
		}
		for receiver in crate::function::fresh_methods::RECEIVERS
			.iter()
			.copied()
			.chain(q.items.keys().map(|id| p.item_type_hash(*id)))
		{
			if receiver == crate::alloc::String::HASH
				|| (m == crate::function::fresh_text::Method::Contains
					&& crate::function::fresh_text::range(receiver))
			{
				continue;
			}
			let h = Hash::associated_function(receiver, m.hash());
			if c.lookup_function(h).is_some() {
				return Err(Error::refused(Refusal::Method));
			}
			if !self.builder.absent.contains(&h) {
				if self.builder.absent.len() >= MAX_CODE_BYTES / core::mem::size_of::<Hash>() {
					return Err(invalid());
				}
				self.builder.absent.try_push(h)?;
			}
		}
		self.checked_text |= m.bit();
		Ok(())
	}
}
