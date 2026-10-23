//! Script instance-method calls whose receiver type is not certain, dispatched
//! at run time in stock's order: a certified native function at the associated
//! hash, the script function, then the missing method. Uncertified providers
//! refuse the site at compile time.
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
		// Stock also accepts callable split patterns. The core implements only
		// string and char delimiters, so keep the existing operand proof even
		// when the receiver is dispatched at run time.
		if name == crate::function::fresh_text::Method::Split.hash()
			&& call.args.len() == 1
			&& self.delimiter(call.args[0]).is_none()
		{
			return Err(self.refused(span, Refusal::Method));
		}
		let at = self.location(span);
		let resolve = &mut *self.resolve;
		self.transaction
			.dynamic_method(
				self.context,
				self.constants,
				self.pool,
				name,
				&mut |item, native| {
					let target = match native {
						Some((intrinsic, arity)) => Resolve::NativeMethod {
							hash: item,
							intrinsic,
							arity,
						},
						None => Resolve::Script(item),
					};
					resolve(target, at)
				},
			)
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
		// The lookup token is private scratch, never an operand of the script
		// target or a host callback. Only the adjacent CallValue consumes it.
		debug_assert_ne!(out, callee);
		debug_assert!(!values.contains(&callee));
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

/// A native adapter is an ordinary, transparent frame. It reuses the existing
/// certified intrinsic and function-value arity checking without a driver arm.
pub(crate) fn native_function(intrinsic: u32, arity: u32) -> Result<Function> {
	let mut function = Function::new(arity + 1, arity)?;
	let mut args = Vec::try_with_capacity(arity as usize)?;
	for slot in 0..arity {
		args.try_push(slot)?;
	}
	let (args_start, args_len) = function.arguments(&args)?;
	let at = super::super::code::TRANSPARENT_SORT_LOCATION;
	function.push(
		Op::Call {
			out: arity,
			function: intrinsic,
			args_start,
			args_len,
		},
		at,
	)?;
	function.push(Op::Return { value: arity }, at)?;
	Ok(function)
}
