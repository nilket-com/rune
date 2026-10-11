//! Restricted scalar constant evaluation, independent of the legacy evaluator.
use super::{
	arithmetic,
	error::{Error, ErrorKind, Result},
	lower::binary_op,
	value::Value,
};
use crate::runtime::{ConstNodeKind, ConstValue, ConstValueBuf, Inline};
use crate::{
	alloc::String,
	ast::{BinOp, UnOp},
	hir,
	query::QueryInner,
};
pub(crate) enum Scalar {
	Primitive(Value),
	Text(String),
}
pub(crate) fn read(value: &ConstValue) -> Result<Scalar> {
	Ok(match value.kind() {
		ConstNodeKind::Inline(value) => Scalar::Primitive(match *value {
			Inline::Unit => Value::Unit,
			Inline::Bool(v) => Value::Bool(v),
			Inline::Char(v) => Value::Char(v),
			Inline::Signed(v) => Value::I64(v),
			Inline::Unsigned(v) => Value::U64(v),
			Inline::Float(v) => Value::F64(v),
			_ => return Err(refused()),
		}),
		ConstNodeKind::String(v) => Scalar::Text(String::try_from(v.as_ref())?),
		_ => return Err(refused()),
	})
}
fn refused() -> Error {
	Error::new(ErrorKind::Unsupported, "restricted scalar constant")
}
pub(crate) fn eval(
	inner: &QueryInner<'_>,
	exprs: &hir::Exprs<'_>,
	expr: &hir::Expr<'_>,
) -> Result<ConstValueBuf> {
	let value = evaluate(inner, exprs, expr, 0)?;
	Ok(match value {
		Scalar::Text(v) => ConstValueBuf::try_from(v)?,
		Scalar::Primitive(v) => ConstValueBuf::from(match v {
			Value::Unit => Inline::Unit,
			Value::Bool(v) => Inline::Bool(v),
			Value::I64(v) => Inline::Signed(v),
			Value::U64(v) => Inline::Unsigned(v),
			Value::F64(v) => Inline::Float(v),
			Value::Char(v) => Inline::Char(v),
			_ => return Err(refused()),
		}),
	})
}
fn evaluate(
	inner: &QueryInner<'_>,
	exprs: &hir::Exprs<'_>,
	expr: &hir::Expr<'_>,
	depth: usize,
) -> Result<Scalar> {
	if depth >= 128 {
		return Err(refused());
	}
	let next = |id| evaluate(inner, exprs, exprs.get(id), depth + 1);
	Ok(match expr.kind {
		hir::ExprKind::Group(id) => return next(id),
		hir::ExprKind::Const(hash) => {
			return read(inner.get_const_value(hash).ok_or_else(refused)?)
		}
		hir::ExprKind::Lit(lit) => match lit {
			hir::Lit::Str(s) if s.len() <= 16 << 20 => Scalar::Text(String::try_from(s)?),
			hir::Lit::Bool(v) => Scalar::Primitive(Value::Bool(v)),
			hir::Lit::Signed(v) => Scalar::Primitive(Value::I64(v)),
			hir::Lit::Unsigned(v) => Scalar::Primitive(Value::U64(v)),
			hir::Lit::Float(v) => Scalar::Primitive(Value::F64(v)),
			hir::Lit::Char(v) => Scalar::Primitive(Value::Char(v)),
			_ => return Err(refused()),
		},
		hir::ExprKind::Tuple(s) if s.items.is_empty() => Scalar::Primitive(Value::Unit),
		hir::ExprKind::Unary(unary) => {
			let Scalar::Primitive(value) = next(unary.expr)? else {
				return Err(refused());
			};
			let op = match unary.op {
				UnOp::Neg(..) => arithmetic::Unary::Neg,
				UnOp::Not(..) => arithmetic::Unary::Not,
				_ => return Err(refused()),
			};
			Scalar::Primitive(arithmetic::unary(op, value)?)
		}
		hir::ExprKind::Binary(binary) => {
			if binary.op.is_assign() {
				return Err(refused());
			}
			let lhs = next(binary.lhs)?;
			if let Scalar::Primitive(Value::Bool(value)) = lhs {
				if matches!(binary.op, BinOp::And(..)) && !value
					|| matches!(binary.op, BinOp::Or(..)) && value
				{
					return Ok(Scalar::Primitive(Value::Bool(value)));
				}
			}
			let rhs = next(binary.rhs)?;
			match (lhs, rhs) {
				(Scalar::Primitive(Value::Bool(a)), Scalar::Primitive(Value::Bool(b)))
					if matches!(binary.op, BinOp::And(..) | BinOp::Or(..)) =>
				{
					Scalar::Primitive(Value::Bool(if matches!(binary.op, BinOp::And(..)) {
						a && b
					} else {
						a || b
					}))
				}
				(Scalar::Primitive(a), Scalar::Primitive(b)) => Scalar::Primitive(
					arithmetic::binary(binary_op(binary.op).ok_or_else(refused)?, a, b)?,
				),
				(Scalar::Text(mut a), Scalar::Text(b)) if matches!(binary.op, BinOp::Add(..)) => {
					if a.len()
						.checked_add(b.len())
						.filter(|n| *n <= 16 << 20)
						.is_none()
					{
						return Err(refused());
					}
					a.try_push_str(&b)?;
					Scalar::Text(a)
				}
				_ => return Err(refused()),
			}
		}
		_ => return Err(refused()),
	})
}
