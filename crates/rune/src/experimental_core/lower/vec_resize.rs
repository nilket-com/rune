//! Cold fill proofs never speculate about untyped parameters or mutable wrappers.
use super::*;
use crate::function::clone::{self, Provider};

#[derive(Clone, Copy)]
struct Fill {
	providers: u8,
	aliased: bool,
	vec_only: bool,
	scalar_only: bool,
}
impl Fill {
	fn scalar() -> Self {
		Self {
			providers: 0,
			aliased: false,
			vec_only: false,
			scalar_only: true,
		}
	}
	fn leaf(provider: Option<Provider>) -> Self {
		Self {
			providers: provider.map_or(0, Provider::bit),
			vec_only: provider == Some(Provider::Vec),
			scalar_only: false,
			..Self::scalar()
		}
	}
	fn merge(mut self, other: Self) -> Self {
		self.providers |= other.providers;
		self.aliased |= other.aliased;
		self.vec_only &= other.vec_only;
		self.scalar_only &= other.scalar_only;
		self
	}
}

fn ungroup(exprs: &hir::Exprs<'_>, mut id: hir::ExprId) -> hir::ExprId {
	while let hir::ExprKind::Group(inner) = exprs.get(id).kind {
		id = inner;
	}
	id
}

impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	fn resize_fill(&self, id: hir::ExprId, depth: usize) -> Option<Fill> {
		if depth >= 128 {
			return None;
		}
		use hir::ExprKind as E;
		match self.exprs.get(id).kind {
			E::Group(id) => self.resize_fill(id, depth + 1),
			E::Lit(hir::Lit::Str(_)) => Some(Fill::leaf(Some(Provider::String))),
			E::Lit(hir::Lit::ByteStr(_)) => None,
			E::Lit(_) | E::Type(_) => Some(Fill::scalar()),
			E::Vec(_) => Some(Fill::leaf(Some(Provider::Vec))),
			E::Object(object) => match object.kind {
				hir::ExprObjectKind::Anonymous => Some(Fill::leaf(Some(Provider::Object))),
				hir::ExprObjectKind::Struct { hash }
					if self.constants.value_shape(hash).is_some() =>
				{
					Some(Fill::leaf(None))
				}
				_ => None,
			},
			E::Tuple(seq) => {
				if seq.items.is_empty() {
					return Some(Fill::scalar());
				}
				let mut proof = Fill::leaf(Some(Provider::Tuple));
				for &item in seq.items {
					proof = proof.merge(self.resize_fill(item, depth + 1)?);
				}
				Some(proof)
			}
			E::Fn(_) | E::CallClosure(_) => Some(Fill::leaf(Some(Provider::Function))),
			E::Variable(v) => {
				let mut proof = self.resize_fill(self.text_init(v)?, depth + 1)?;
				for expr in self.exprs.expressions() {
					match expr.kind {
						E::Assign(a) if matches!(self.exprs.get(ungroup(self.exprs,a.lhs)).kind,E::Variable(x)if x==v) => {
							proof = proof.merge(self.resize_fill(a.rhs, depth + 1)?)
						}
						E::Binary(b)
							if b.op.is_assign()
								&& matches!(self.exprs.get(ungroup(self.exprs,b.lhs)).kind,E::Variable(x)if x==v) =>
						{
							return None
						}
						_ => {}
					}
				}
				proof.aliased |= !proof.scalar_only;
				Some(proof)
			}
			E::Call(call) => {
				let hir::Call::Meta { hash } = call.call else {
					return None;
				};
				if super::vec_constructors::constructor(hash).is_some() {
					return Some(Fill::leaf(Some(Provider::Vec)));
				}
				if self.constants.value_shape(hash).is_some() {
					return Some(Fill::leaf(None));
				}
				let (provider, arity) =
					if hash == crate::hash_in!(crate, ::std::option::Option::None) {
						(Provider::Option, 0)
					} else if hash == crate::hash_in!(crate, ::std::option::Option::Some) {
						(Provider::Option, 1)
					} else if hash == crate::hash_in!(crate, ::std::result::Result::Ok)
						|| hash == crate::hash_in!(crate, ::std::result::Result::Err)
					{
						(Provider::Result, 1)
					} else {
						return None;
					};
				if call.args.len() != arity {
					return None;
				}
				let mut proof = Fill::leaf(Some(provider));
				for &arg in call.args {
					proof = proof.merge(self.resize_fill(arg, depth + 1)?);
				}
				Some(proof)
			}
			E::Block(block) if block.label.is_none() => self.resize_fill(block.value?, depth + 1),
			E::If(conditional) => {
				let fallback = conditional.fallback?;
				if fallback.label.is_some() {
					return None;
				}
				let mut proof = self.resize_fill(fallback.value?, depth + 1)?;
				for branch in conditional.branches {
					if branch.block.label.is_some() {
						return None;
					}
					proof = proof.merge(self.resize_fill(branch.block.value?, depth + 1)?);
				}
				Some(proof)
			}
			E::Unary(unary) => {
				let proof = self.resize_fill(unary.expr, depth + 1)?;
				proof.scalar_only.then_some(proof)
			}
			E::Binary(binary) if !binary.op.is_assign() => {
				let proof = self
					.resize_fill(binary.lhs, depth + 1)?
					.merge(self.resize_fill(binary.rhs, depth + 1)?);
				proof.scalar_only.then_some(proof)
			}
			_ => None,
		}
	}

	fn resize_prefix_unchanged(&self, current: &hir::ExprCall<'_>, span: Span) -> bool {
		use hir::ExprKind as E;
		let site = span.range();
		self.exprs.expressions().all(|expr| {
			let range = expr.span.range();
			// Constructing a closure does not execute its body. A preceding call
			// still reaches the unknown-call check outside this dormant range.
			if self.exprs.expressions().any(|definition| {
				let E::CallClosure(_) = definition.kind else {
					return false;
				};
				let body = definition.span.range();
				!core::ptr::eq(expr, definition)
					&& !(body.start <= site.start && body.end >= site.end)
					&& body.start <= range.start
					&& body.end >= range.end
			}) {
				return true;
			}

			if matches!(expr.kind, E::Loop(_) | E::For(_))
				&& range.start <= site.start
				&& range.end >= site.end
			{
				return false;
			}
			if range.start >= site.end {
				return true;
			}
			match expr.kind {
				// Earlier aliases cannot mutate or move a proven heap value through these paths.
				E::Assign(_) => false,
				E::Binary(binary) => {
					!binary.op.is_assign()
						&& self
							.resize_fill(binary.lhs, 0)
							.is_some_and(|p| p.scalar_only)
						&& self
							.resize_fill(binary.rhs, 0)
							.is_some_and(|p| p.scalar_only)
				}
				E::Unary(unary) => self
					.resize_fill(unary.expr, 0)
					.is_some_and(|p| p.scalar_only),
				E::CallClosure(closure) => !closure.do_move,
				E::Call(call) if core::ptr::eq(call, current) => true,
				E::Call(call) => match call.call {
					hir::Call::Meta { hash } => {
						super::vec_constructors::constructor(hash).is_some()
							|| self.constants.value_shape(hash).is_some()
							|| [
								crate::hash_in!(crate, ::std::option::Option::None),
								crate::hash_in!(crate, ::std::option::Option::Some),
								crate::hash_in!(crate, ::std::result::Result::Ok),
								crate::hash_in!(crate, ::std::result::Result::Err),
							]
							.contains(&hash)
					}
					_ => false,
				},
				_ => true,
			}
		})
	}

	#[cold]
	#[inline(never)]
	pub(super) fn call_vec_resize(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		out: u32,
		span: Span,
	) -> Result<()> {
		if call.args.len() != 2 {
			return Err(self.refused(span, Refusal::Arity));
		}
		if !self.context.vec_resize() {
			return Err(self.refused(span, Refusal::Native));
		}
		let fill = self
			.resize_fill(call.args[1], 0)
			.ok_or_else(|| self.refused(span, Refusal::Method))?;
		if fill.aliased && !self.resize_prefix_unchanged(call, span) {
			return Err(self.refused(span, Refusal::Method));
		}
		if !self.constants.instances.complete() {
			return Err(self.refused(span, Refusal::Method));
		}
		for provider in Provider::ALL {
			if fill.providers & provider.bit() != 0
				&& (!self.context.clone_provider(provider)
					|| !provider.unit_absent(&self.constants.instances))
			{
				return Err(self.refused(span, Refusal::Method));
			}
		}

		if !self
			.resize_fill(target, 0)
			.is_some_and(|proof| proof.vec_only)
		{
			return self.call_dynamic_method(call, target, clone::RESIZE_NAME, out, span);
		}
		let values = self.operands(&[target, call.args[0], call.args[1]])?;
		let (args_start, args_len) = self.function.arguments(&values)?;
		self.emit(
			Op::Call {
				out,
				function: super::super::code::VEC_RESIZE,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
