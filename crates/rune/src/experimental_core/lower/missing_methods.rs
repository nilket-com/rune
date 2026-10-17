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
		self.transaction
			.missing_method(self.context, self.constants, self.pool, name, hash)
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
				function: super::super::code::MISSING_METHOD,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
