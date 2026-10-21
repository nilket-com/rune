//! Script instance-method calls whose receiver type is not certain, dispatched
//! at run time in stock's order: a native function at the associated hash
//! (refused at compile time), the script function, then the missing method.
use super::*;

impl Lower<'_, '_> {
	/// Lowers `target.name(args)` as an instance lookup on the saved receiver
	/// followed by a call of the function value it returns, so the parameter
	/// count is checked when the call runs, like stock.
	#[cold]
	#[inline(never)]
	pub(super) fn call_dynamic_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		name: Hash,
		out: u32,
		span: Span,
	) -> Result<()> {
		let at = self.location(span);
		let resolve = &mut *self.resolve;
		self.transaction
			.dynamic_method(self.context, self.constants, self.pool, name, &mut |item| {
				resolve(Resolve::Script(item), at)
			})
			.map_err(|e| e.at(at))?;
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		for id in call.args {
			ids.try_push(*id)?;
		}
		// The arguments may write the receiver's binding, so `operands` copies
		// it first; the lookup and the call both read that saved value.
		let values = self.operands(&ids)?;
		let method = self.slot()?;
		self.emit(
			Op::Literal {
				out: method,
				value: Value::U64(name.into_inner()),
			},
			span,
		)?;
		let callee = self.slot()?;
		let (lookup_start, lookup_len) = self.function.arguments(&[method, values[0]])?;
		self.emit(
			Op::Call {
				out: callee,
				function: super::super::code::INSTANCE_METHOD,
				args_start: lookup_start,
				args_len: lookup_len,
			},
			span,
		)?;
		let (args_start, args_len) = self.function.arguments(&values)?;
		self.emit(
			Op::CallValue {
				out,
				value: callee,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
