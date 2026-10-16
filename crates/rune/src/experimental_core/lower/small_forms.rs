//! Cold C12 lowering, using the existing certified Call instruction.
use super::super::code;
use super::*;
impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn other_range(
		&mut self,
		range: &hir::ExprRange,
		out: u32,
		span: Span,
	) -> Result<()> {
		let (id, operands) = match range {
			hir::ExprRange::RangeFrom { start } => (code::RANGE_FROM, self.operands(&[*start])?),
			hir::ExprRange::RangeFull => (code::RANGE_FULL, Vec::new()),
			hir::ExprRange::RangeInclusive { start, end } => {
				(code::RANGE_INCLUSIVE, self.operands(&[*start, *end])?)
			}
			hir::ExprRange::RangeToInclusive { end } => {
				(code::RANGE_TO_INCLUSIVE, self.operands(&[*end])?)
			}
			hir::ExprRange::RangeTo { end } => (code::RANGE_TO, self.operands(&[*end])?),
			_ => return Err(Error::new(ErrorKind::InvalidCode, "cold range lowering")),
		};
		let (args_start, args_len) = self.function.arguments(&operands)?;
		self.emit(
			Op::Call {
				out,
				function: id,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}

/// A normal charged loop: each predicate uses CallValue, with the existing
/// callback frame/depth/fuel cleanup. The receiver is the iterator itself,
/// so a later next()/any() continues at the same cursor.
pub(in crate::experimental_core) fn any_function() -> Result<Function> {
	let mut f = Function::new(5, 2)?;
	let at = code::TRANSPARENT_SORT_LOCATION;
	let (args_start, args_len) = f.arguments(&[2])?;
	let (ready_start, ready_len) = f.arguments(&[0])?;
	f.push(
		Op::Call {
			out: 0,
			function: code::ANY_READY,
			args_start: ready_start,
			args_len: ready_len,
		},
		at,
	)?;
	f.push(
		Op::IterNext {
			out: 2,
			iterator: 0,
			none: 8,
		},
		at,
	)?;
	f.push(
		Op::CallValue {
			out: 3,
			value: 1,
			args_start,
			args_len,
		},
		at,
	)?;
	f.push(
		Op::Branch {
			condition: 3,
			yes: 6,
			no: 4,
		},
		at,
	)?;
	f.push(Op::Release { value: 2 }, at)?;
	f.push(Op::Jump { target: 1 }, at)?;
	f.push(
		Op::Literal {
			out: 4,
			value: Value::Bool(true),
		},
		at,
	)?;
	f.push(Op::Return { value: 4 }, at)?;
	f.push(
		Op::Literal {
			out: 4,
			value: Value::Bool(false),
		},
		at,
	)?;
	f.push(Op::Return { value: 4 }, at)?;
	Ok(f)
}

impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn call_iterator_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		name: crate::function::fresh_iter::Name,
		out: u32,
		span: Span,
	) -> Result<()> {
		use crate::function::fresh_iter::Name;
		if !self.context.fresh_iter_method(name)
			|| self.constants.fresh_iter_script & name.bit() != 0
		{
			return Err(self.refused(span, Refusal::Method));
		}
		let arity = usize::from(name == Name::Any);
		if call.args.len() != arity {
			return Err(self.refused(span, Refusal::Arity));
		}
		let operands = if name == Name::Any {
			self.operands(&[target, call.args[0]])?
		} else {
			self.operands(&[target])?
		};
		let (args_start, args_len) = self.function.arguments(&operands)?;
		let function = match name {
			Name::Iter => code::ITER_METHOD,
			Name::Next => code::NEXT_METHOD,
			Name::Any => {
				let at = self.location(span);
				(self.resolve)(Resolve::Any, at)?
			}
		};
		self.emit(
			Op::Call {
				out,
				function,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
