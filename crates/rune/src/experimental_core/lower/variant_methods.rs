//! C17a Option and Result methods: one certified cold call per site.
use super::*;
use crate::function::fresh_variant::Method;

impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn call_variant_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		method: Method,
		out: u32,
		span: Span,
	) -> Result<()> {
		if call.args.len() + 1 != method.arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
		self.transaction
			.variant_method(self.context, self.constants, self.pool, method)
			.map_err(|e| e.at(self.location(span)))?;
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		for id in call.args {
			ids.try_push(*id)?;
		}
		let args = self.operands(&ids)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		self.emit(
			Op::Call {
				out,
				function: method.id(),
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
