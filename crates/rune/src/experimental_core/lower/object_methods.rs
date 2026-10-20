//! C17b Object methods and `is_empty`: one certified cold call per site.
use super::*;
use crate::function::fresh_object::Method;

impl Lower<'_, '_> {
	/// Whether `id` always evaluates to an anonymous Object: an object literal,
	/// or a local whose initializer and every assignment are Object-origin.
	/// Parameters, fields, elements and call results are never proven.
	fn object_origin(&self, id: hir::ExprId, depth: usize) -> bool {
		if depth >= 128 {
			return false;
		}
		use hir::ExprKind as E;
		match self.exprs.get(id).kind {
			E::Object(object) => matches!(object.kind, hir::ExprObjectKind::Anonymous),
			E::Group(inner) => self.object_origin(inner, depth + 1),
			// A labeled block can produce its value through `break`.
			E::Block(block) if block.label.is_none() => block
				.value
				.is_some_and(|value| self.object_origin(value, depth + 1)),
			E::Variable(v) => {
				self.text_init(v)
					.is_some_and(|init| self.object_origin(init, depth + 1))
					&& self.text_writes(v, |rhs| self.object_origin(rhs, depth + 1))
			}
			_ => false,
		}
	}

	#[cold]
	#[inline(never)]
	pub(super) fn call_object_method(
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
		let proven = self.object_origin(target, 0);
		self.transaction
			.object_method(self.context, self.constants, self.pool, method, proven)
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
