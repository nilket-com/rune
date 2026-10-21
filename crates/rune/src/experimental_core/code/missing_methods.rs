//! Cold absence proofs, owned by the compile and checked before execution.
use super::super::error::Refusal;
use super::*;
use crate::{Context, Hash};

impl Builder {
	/// Records `hash` as proven absent, once.
	fn record_absent(&mut self, hash: Hash) -> Result<()> {
		if self.absent.contains(&hash) {
			return Ok(());
		}
		self.push_absent(hash)
	}

	/// Appends a hash the caller knows is new. The absence list counts toward
	/// the code budget, so a full list is the [`Refusal::CodeLimit`] refusal.
	fn push_absent(&mut self, hash: Hash) -> Result<()> {
		if self.absent.len() >= MAX_CODE_BYTES / core::mem::size_of::<Hash>() {
			return Err(Error::refused(Refusal::CodeLimit));
		}
		self.absent.try_push(hash)?;
		Ok(())
	}
}

impl CompileTransaction {
	pub(crate) fn missing_nominal_method(
		&mut self,
		context: &Context,
		query: &crate::query::QueryInner<'_>,
		receiver: Hash,
		name: Hash,
		effective: Hash,
	) -> Result<()> {
		// This proof applies only to a statically closed source nominal receiver.
		// Script/trait installation remains conservatively closed by method name.
		if query.fresh_method_script.closed(name) {
			return Err(Error::refused(Refusal::Method));
		}
		let hash = Hash::associated_function(receiver, effective);
		if context.lookup_function(hash).is_some() {
			return Err(Error::refused(Refusal::Method));
		}
		self.builder.record_absent(hash)?;
		Ok(())
	}

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
			self.builder.push_absent(hash)?;
		}
		self.checked_absent.try_push(effective)?;
		Ok(())
	}
}

impl CompileTransaction {
	/// Proves that no native function takes the script instance function's
	/// place at `associated`: stock looks in the context first.
	pub(crate) fn static_method(&mut self, context: &Context, associated: Hash) -> Result<()> {
		if context.lookup_function(associated).is_some() {
			return Err(Error::refused(Refusal::Method));
		}
		self.builder.record_absent(associated)
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
			self.builder.record_absent(h)?;
		}
		self.checked_text |= m.bit();
		Ok(())
	}
}

impl CompileTransaction {
	pub(crate) fn pipeline_method(
		&mut self,
		c: &Context,
		q: &crate::query::QueryInner<'_>,
		p: &crate::compile::Pool,
		m: crate::function::fresh_pipeline::Method,
	) -> Result<()> {
		if self.checked_pipeline & (1 << m as u8) != 0 {
			return Ok(());
		}
		if !c.fresh_pipeline_method(m) || q.fresh_method_script.closed(m.hash()) {
			return Err(Error::refused(Refusal::Method));
		}
		// Non-origin registered iterator methods remain outside this new surface. For the
		// two plain native names, close over every representable receiver before effects.
		if matches!(
			m,
			crate::function::fresh_pipeline::Method::Chars
				| crate::function::fresh_pipeline::Method::Unwrap
		) {
			for receiver in crate::function::fresh_methods::RECEIVERS
				.iter()
				.copied()
				.chain(q.items.keys().map(|id| p.item_type_hash(*id)))
			{
				if m.provider(receiver).is_some() {
					continue;
				}
				let h = Hash::associated_function(receiver, m.hash());
				if c.lookup_function(h).is_some() {
					return Err(Error::refused(Refusal::Method));
				}
				self.builder.record_absent(h)?;
			}
		}
		self.checked_pipeline |= 1 << m as u8;
		Ok(())
	}
}

impl CompileTransaction {
	/// Proves that only the certified std Option and Result registrations provide
	/// `m` for every receiver a fresh value or a source item can have.
	pub(crate) fn variant_method(
		&mut self,
		c: &Context,
		q: &crate::query::QueryInner<'_>,
		p: &crate::compile::Pool,
		m: crate::function::fresh_variant::Method,
	) -> Result<()> {
		if self.checked_variant & m.bit() != 0 {
			return Ok(());
		}
		if !c.fresh_variant_method(m) || q.fresh_method_script.closed(m.hash()) {
			return Err(Error::refused(Refusal::Method));
		}
		for receiver in crate::function::fresh_methods::RECEIVERS
			.iter()
			.copied()
			.chain(q.items.keys().map(|id| p.item_type_hash(*id)))
		{
			if m.provider(receiver).is_some() {
				continue;
			}
			let h = Hash::associated_function(receiver, m.hash());
			if c.lookup_function(h).is_some() {
				return Err(Error::refused(Refusal::Method));
			}
			self.builder.record_absent(h)?;
		}
		self.checked_variant |= m.bit();
		Ok(())
	}
}

impl CompileTransaction {
	/// Certifies a C17b Object method site. A site whose receiver is statically
	/// proven Object needs only the certified std provider; any other site also
	/// needs proof that no other receiver a fresh value or a source item can
	/// have provides `m`.
	pub(crate) fn object_method(
		&mut self,
		c: &Context,
		q: &crate::query::QueryInner<'_>,
		p: &crate::compile::Pool,
		m: crate::function::fresh_object::Method,
		proven_object: bool,
	) -> Result<()> {
		if !c.fresh_object_method(m) || (!proven_object && m.shared_name()) {
			return Err(Error::refused(Refusal::Method));
		}
		if proven_object || self.checked_object & m.bit() != 0 {
			return Ok(());
		}
		if q.fresh_method_script.closed(m.hash()) {
			return Err(Error::refused(Refusal::Method));
		}
		for receiver in crate::function::fresh_methods::RECEIVERS
			.iter()
			.copied()
			.chain(q.items.keys().map(|id| p.item_type_hash(*id)))
		{
			if m.provider(receiver).is_some() {
				continue;
			}
			let h = Hash::associated_function(receiver, m.hash());
			if c.lookup_function(h).is_some() {
				return Err(Error::refused(Refusal::Method));
			}
			self.builder.record_absent(h)?;
		}
		self.checked_object |= m.bit();
		Ok(())
	}
}

impl CompileTransaction {
	/// Certifies a C17c numeric method site whose receiver type is not proven:
	/// every stock provider is its std registration, and no other receiver a
	/// fresh value or a source item can have provides `m`.
	pub(crate) fn number_method(
		&mut self,
		c: &Context,
		q: &crate::query::QueryInner<'_>,
		p: &crate::compile::Pool,
		m: crate::function::fresh_number::Method,
	) -> Result<()> {
		use crate::function::fresh_number::{Pair, Receiver};
		if m.shared_name() || !m.instance() || !c.fresh_number_method(m) {
			return Err(Error::refused(Refusal::Method));
		}
		let bit = 1u64 << m as u32;
		if self.checked_number & bit != 0 {
			return Ok(());
		}
		if q.fresh_method_script.closed(m.hash()) {
			return Err(Error::refused(Refusal::Method));
		}
		for receiver in crate::function::fresh_methods::RECEIVERS
			.iter()
			.copied()
			.chain(q.items.keys().map(|id| p.item_type_hash(*id)))
		{
			if Receiver::from_hash(receiver).is_some_and(|r| Pair::new(m, r).is_some()) {
				continue;
			}
			let h = Hash::associated_function(receiver, m.hash());
			if c.lookup_function(h).is_some() {
				return Err(Error::refused(Refusal::Method));
			}
			self.builder.record_absent(h)?;
		}
		self.checked_number |= bit;
		Ok(())
	}
}

impl CompileTransaction {
	/// Certifies a C17d terminal or adapter on its first use in this compile.
	pub(crate) fn iter_terminal(
		&mut self,
		c: &Context,
		t: crate::function::iter_terminal::Terminal,
	) -> Result<()> {
		if self.checked_terminal & t.bit() != 0 {
			return Ok(());
		}
		if !crate::function::iter_terminal::provided(c, t) {
			return Err(Error::refused(Refusal::Method));
		}
		self.checked_terminal |= t.bit();
		Ok(())
	}

	/// Certifies a C17e closure adapter or consumer on its first use in this
	/// compile. `rev` also proves the std iterators without one still lack it.
	pub(crate) fn iter_closure(
		&mut self,
		c: &Context,
		f: crate::function::iter_terminal::Closure,
	) -> Result<()> {
		use crate::function::iter_terminal::{closure_provided, Closure, IRREVERSIBLE};
		if self.checked_closure & f.bit() != 0 {
			return Ok(());
		}
		if !closure_provided(c, f) {
			return Err(Error::refused(Refusal::Method));
		}
		if f == Closure::Rev {
			for receiver in IRREVERSIBLE {
				let h = Hash::associated_function(receiver, f.hash());
				if c.lookup_function(h).is_some() {
					return Err(Error::refused(Refusal::Method));
				}
				self.builder.record_absent(h)?;
			}
		}
		self.checked_closure |= f.bit();
		Ok(())
	}

	/// Certifies a C17d terminal or adapter, or `next` (`None`), on the
	/// iterators only a C17e chain reaches.
	pub(crate) fn iter_extended(
		&mut self,
		c: &Context,
		t: Option<crate::function::iter_terminal::Terminal>,
	) -> Result<()> {
		let bit = t.map_or(1 << 15, |t| t.bit());
		if self.checked_extended & bit != 0 {
			return Ok(());
		}
		let provided = match t {
			Some(t) => crate::function::iter_terminal::extended_provided(c, t),
			None => crate::function::iter_terminal::extended_next_provided(c),
		};
		if !provided {
			return Err(Error::refused(Refusal::Method));
		}
		self.checked_extended |= bit;
		Ok(())
	}

	/// Certifies `::std::iter::range` on its first use in this compile.
	pub(crate) fn iter_range(&mut self, c: &Context) -> Result<()> {
		const RANGE: u16 = 1 << 13;
		if self.checked_terminal & RANGE != 0 {
			return Ok(());
		}
		if !crate::function::iter_terminal::range_provided(c) {
			return Err(Error::refused(Refusal::Native));
		}
		self.checked_terminal |= RANGE;
		Ok(())
	}
}
