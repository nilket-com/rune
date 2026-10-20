//! C17c numeric methods, `char::from_i64` and std numeric constants: one
//! certified cold call per site, or a literal.
use super::*;
use crate::function::fresh_number::{self as number, Method, Pair, Receiver};

/// Proof recursion depth, as for the other origin proofs.
const DEPTH: usize = 128;

impl Lower<'_, '_> {
	/// The std numeric constant `hash`, if the context still holds the stock value.
	pub(super) fn number_constant(&self, hash: Hash) -> Option<Value> {
		let expected = match number::constant(hash)? {
			number::Constant::I64(v) => Value::I64(v),
			number::Constant::U64(v) => Value::U64(v),
			number::Constant::F64(bits) => Value::F64(f64::from_bits(bits)),
		};
		let actual =
			match super::super::constants::read(self.context.get_const_value(hash)?).ok()? {
				super::super::constants::Scalar::Primitive(value) => value,
				super::super::constants::Scalar::Text(_) => return None,
			};
		let same = match (expected, actual) {
			(Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
			(a, b) => a == b,
		};
		same.then_some(expected)
	}

	/// The receiver type every evaluation of `id` has when it completes. Locals
	/// count when their initializer and every write have that type; a local
	/// being proven is assumed to have it (each write then preserves it).
	fn number_receiver(
		&self,
		id: hir::ExprId,
		depth: usize,
		assumed: &mut Vec<(hir::Variable, Receiver)>,
	) -> Option<Receiver> {
		if depth >= DEPTH {
			return None;
		}
		use hir::ExprKind as E;
		let scalar = |value| match value {
			Value::I64(_) => Some(Receiver::I64),
			Value::U64(_) => Some(Receiver::U64),
			Value::F64(_) => Some(Receiver::F64),
			Value::Char(_) => Some(Receiver::Char),
			_ => None,
		};
		match self.exprs.get(id).kind {
			E::Lit(hir::Lit::Signed(_)) => Some(Receiver::I64),
			E::Lit(hir::Lit::Unsigned(_)) => Some(Receiver::U64),
			E::Lit(hir::Lit::Float(_)) => Some(Receiver::F64),
			E::Lit(hir::Lit::Char(_)) => Some(Receiver::Char),
			E::Group(inner) => self.number_receiver(inner, depth + 1, assumed),
			// A labeled block can produce its value through `break`.
			E::Block(block) if block.label.is_none() => {
				self.number_receiver(block.value?, depth + 1, assumed)
			}
			E::Const(hash) => match self.constants.get_const_value(hash) {
				Some(constant) => match super::super::constants::read(constant).ok()? {
					super::super::constants::Scalar::Primitive(value) => scalar(value),
					super::super::constants::Scalar::Text(_) => None,
				},
				None => scalar(self.number_constant(hash)?),
			},
			E::Unary(unary) if matches!(unary.op, crate::ast::UnOp::Neg(_)) => {
				match self.number_receiver(unary.expr, depth + 1, assumed)? {
					r @ (Receiver::I64 | Receiver::F64) => Some(r),
					_ => None,
				}
			}
			E::Binary(binary) if arithmetic(binary.op) => {
				let lhs = self.number_receiver(binary.lhs, depth + 1, assumed)?;
				let rhs = self.number_receiver(binary.rhs, depth + 1, assumed)?;
				(lhs == rhs && lhs != Receiver::Char).then_some(lhs)
			}
			E::Call(call) => {
				let hir::Call::Associated {
					target,
					hash,
					name_hash,
				} = call.call
				else {
					return None;
				};
				if hash != name_hash {
					return None;
				}
				let receiver = self.number_receiver(target, depth + 1, assumed)?;
				if let Some(m) = Method::from_hash(hash) {
					Pair::new(m, receiver)?;
					return match m {
						Method::Sqrt
						| Method::Floor
						| Method::Ceil
						| Method::Exp
						| Method::Ln
						| Method::Powi => Some(Receiver::F64),
						Method::ToI64 => Some(Receiver::I64),
						Method::Min
						| Method::Max
						| Method::Pow
						| Method::WrappingPow
						| Method::SaturatingPow
						| Method::WrappingAdd
						| Method::WrappingSub
						| Method::WrappingMul
						| Method::WrappingDiv
						| Method::WrappingRem
						| Method::SaturatingAdd
						| Method::SaturatingSub
						| Method::SaturatingMul
						| Method::WrappingAbs
						| Method::SaturatingAbs
						| Method::Signum => Some(receiver),
						_ => None,
					};
				}
				use crate::function::fresh_numeric::Method as Old;
				match (Old::from_hash(hash)?, receiver) {
					(Old::Abs, Receiver::I64 | Receiver::F64) => Some(receiver),
					(Old::Round | Old::Powf, Receiver::F64) => Some(receiver),
					_ => None,
				}
			}
			E::Variable(v) => {
				if let Some((_, r)) = assumed.iter().find(|(x, _)| *x == v) {
					return Some(*r);
				}
				let r = self.number_receiver(self.text_init(v)?, depth + 1, assumed)?;
				assumed.try_push((v, r)).ok()?;
				let kept = self.exprs.fresh_expressions().all(|e| match e.kind {
					E::Assign(a) if matches!(self.exprs.get(a.lhs).kind, E::Variable(x) if x == v) => {
						self.number_receiver(a.rhs, depth + 1, assumed) == Some(r)
					}
					E::Binary(b)
						if b.op.is_assign()
							&& matches!(self.exprs.get(b.lhs).kind, E::Variable(x) if x == v) =>
					{
						compound_arithmetic(b.op)
							&& r != Receiver::Char
							&& self.number_receiver(b.rhs, depth + 1, assumed) == Some(r)
					}
					_ => true,
				});
				assumed.retain(|(x, _)| *x != v);
				kept.then_some(r)
			}
			_ => None,
		}
	}

	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn call_number_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		method: Method,
		generic: bool,
		out: u32,
		span: Span,
	) -> Result<()> {
		// A method turbofish names a parameterized stock hash nothing provides.
		if generic {
			return Err(self.refused(span, Refusal::Method));
		}
		if call.args.len() + 1 != method.arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
		let proven = self
			.number_receiver(target, 0, &mut Vec::new())
			.and_then(|r| Pair::new(method, r));
		let function = match proven {
			Some(pair) => {
				if !self.context.fresh_number_pair(pair) {
					return Err(self.refused(span, Refusal::Method));
				}
				pair.id()
			}
			None => {
				let id = method
					.dynamic_id()
					.ok_or_else(|| self.refused(span, Refusal::Method))?;
				self.transaction
					.number_method(self.context, self.constants, self.pool, method)
					.map_err(|e| e.at(self.location(span)))?;
				id
			}
		};
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		for id in call.args {
			ids.try_push(*id)?;
		}
		self.number_call(&ids, function, out, span)
	}

	/// A path call such as `f64::sqrt(x)` or `char::from_i64(i)`.
	#[cold]
	#[inline(never)]
	pub(super) fn call_number_path(
		&mut self,
		call: &hir::ExprCall<'_>,
		pair: Pair,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.fresh_number_pair(pair) {
			return Err(self.refused(span, Refusal::Native));
		}
		if call.args.len() != pair.method().arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
		self.number_call(call.args, pair.id(), out, span)
	}

	fn number_call(
		&mut self,
		ids: &[hir::ExprId],
		function: u32,
		out: u32,
		span: Span,
	) -> Result<()> {
		let args = self.operands(ids)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
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

fn arithmetic(op: crate::ast::BinOp) -> bool {
	matches!(
		op,
		crate::ast::BinOp::Add(_)
			| crate::ast::BinOp::Sub(_)
			| crate::ast::BinOp::Mul(_)
			| crate::ast::BinOp::Div(_)
			| crate::ast::BinOp::Rem(_)
	)
}

fn compound_arithmetic(op: crate::ast::BinOp) -> bool {
	matches!(
		op,
		crate::ast::BinOp::AddAssign(_)
			| crate::ast::BinOp::SubAssign(_)
			| crate::ast::BinOp::MulAssign(_)
			| crate::ast::BinOp::DivAssign(_)
			| crate::ast::BinOp::RemAssign(_)
	)
}
