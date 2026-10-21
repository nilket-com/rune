use super::*;
impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn call_missing_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		name: Hash,
		hash: Hash,
		out: u32,
		span: Span,
	) -> Result<()> {
		let location = self.location(span);
		if let Some(receiver) = self.nominal_origin(target, 0) {
			self.transaction.missing_nominal_method(
				self.context,
				self.constants,
				receiver,
				name,
				hash,
			)
		} else {
			self.transaction
				.missing_method(self.context, self.constants, self.pool, name, hash)
		}
		.map_err(|error| error.at(location))?;
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		for id in call.args {
			ids.try_push(*id)?;
		}
		// Every explicit operand stays alive in its frame slot until failure. The
		// failing intrinsic only needs the receiver, so its certificate stays arity 2.
		let values = self.operands(&ids)?;
		let method = self.slot()?;
		self.emit(
			Op::Literal {
				out: method,
				value: Value::U64(hash.into_inner()),
			},
			span,
		)?;
		let (args_start, args_len) = self.function.arguments(&[method, values[0]])?;
		self.emit(
			Op::Call {
				out,
				function: super::super::code::INSTANCE_METHOD,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}

impl Lower<'_, '_> {
	pub(super) fn nominal_origin(&self, id: hir::ExprId, depth: usize) -> Option<Hash> {
		if depth >= 128 {
			return None;
		}
		match self.exprs.get(id).kind {
			hir::ExprKind::Group(id) => self.nominal_origin(id, depth + 1),
			hir::ExprKind::Variable(v) => {
				let hash = self.nominal_origin(self.text_init(v)?, depth + 1)?;
				self.text_writes(v, |rhs| self.nominal_origin(rhs, depth + 1) == Some(hash))
					.then_some(hash)
			}
			hir::ExprKind::Call(call) => {
				let hir::Call::Meta { hash } = call.call else {
					return None;
				};
				self.nominal_type(hash)
			}
			_ => None,
		}
	}

	/// The runtime type of a value built by the script struct or variant `hash`.
	pub(super) fn nominal_type(&self, hash: Hash) -> Option<Hash> {
		self.constants
			.fresh_variant_shape(hash)
			.map(|shape| shape.enum_hash)
			.or_else(|| self.constants.fresh_struct_shape(hash).map(|_| hash))
	}
}

impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn nominal_missing_site(&self, call: &hir::ExprCall<'_>) -> bool {
		let hir::Call::Associated {
			target,
			hash,
			name_hash,
		} = call.call
		else {
			return false;
		};
		use crate::TypeHash;
		let plain = hash == name_hash;
		let collect = name_hash == Hash::ident("collect")
			&& [crate::runtime::Vec::HASH, crate::alloc::String::HASH]
				.into_iter()
				.any(|ty| hash == name_hash.with_function_parameters(Hash::parameters([ty])));
		(plain || collect) && self.nominal_origin(target, 0).is_some()
	}
}
