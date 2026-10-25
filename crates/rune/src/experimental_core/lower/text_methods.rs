//! Conservative binding-identity proof and cold text-method lowering.
use super::*;
use crate::function::text::{self, Method};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
	String,
	Char,
	Never,
}
impl Lower<'_, '_> {
	pub(super) fn text_init(&self, v: hir::Variable) -> Option<hir::ExprId> {
		self.text_bindings
			.iter()
			.rev()
			.find(|(x, _)| *x == v)
			.map(|(_, id)| *id)
	}
	fn literal_kind(&self, id: hir::ExprId) -> Option<bool> {
		match self.exprs.get(id).kind {
			hir::ExprKind::Lit(hir::Lit::Str(_)) => Some(true),
			hir::ExprKind::Lit(hir::Lit::Char(_)) => Some(false),
			_ => None,
		}
	}
	pub(super) fn text_writes(
		&self,
		v: hir::Variable,
		mut accept: impl FnMut(hir::ExprId) -> bool,
	) -> bool {
		use hir::ExprKind as E;
		self.exprs.expressions().all(|e| match e.kind {
			E::Assign(a) if matches!(self.exprs.get(a.lhs).kind,E::Variable(x) if x==v) => {
				accept(a.rhs)
			}
			E::Binary(b)
				if b.op.is_assign()
					&& matches!(self.exprs.get(b.lhs).kind,E::Variable(x) if x==v) =>
			{
				false
			}
			_ => true,
		})
	}
	pub(super) fn delimiter(&self, id: hir::ExprId) -> Option<bool> {
		if let Some(k) = self.literal_kind(id) {
			return Some(k);
		}
		let hir::ExprKind::Variable(v) = self.exprs.get(id).kind else {
			return None;
		};
		let kind = self.literal_kind(self.text_init(v)?)?;
		self.text_writes(v, |rhs| self.literal_kind(rhs) == Some(kind))
			.then_some(kind)
	}
	fn nonrange(&self, id: hir::ExprId, depth: usize) -> bool {
		if depth >= 128 {
			return false;
		}
		use hir::ExprKind as E;
		match self.exprs.get(id).kind {
			E::Lit(_) | E::Vec(_) | E::Tuple(_) | E::Object(_) => true,
			E::Group(i) => self.nonrange(i, depth + 1),
			E::Variable(v) => {
				self.text_init(v)
					.is_some_and(|i| self.nonrange(i, depth + 1))
					&& self.text_writes(v, |i| self.nonrange(i, depth + 1))
			}
			E::Call(c) => {
				// A script method of the name may return anything.
				matches!(c.call,hir::Call::Associated{hash,..} if matches!(Method::from_hash(hash),Some(Method::Trim|Method::Replace)) && !self.constants.method_script.closed(hash))
			}
			_ => false,
		}
	}
	fn nonstring(&self, id: hir::ExprId, depth: usize) -> bool {
		if depth >= 128 {
			return false;
		}
		use hir::ExprKind as E;
		match self.exprs.get(id).kind {
			E::Lit(hir::Lit::Str(_)) => false,
			E::Lit(_) | E::Vec(_) | E::Tuple(_) | E::Object(_) | E::Range(_) => true,
			E::Call(c) if matches!(c.call,hir::Call::Meta{hash} if hash==crate::hash_in!(crate,::std::option::Option::None) || hash==crate::hash_in!(crate,::std::option::Option::Some) || hash==crate::hash_in!(crate,::std::result::Result::Ok) || hash==crate::hash_in!(crate,::std::result::Result::Err)) => {
				true
			}
			E::Group(i) => self.nonstring(i, depth + 1),
			E::Variable(v) => {
				self.text_init(v)
					.is_some_and(|i| self.nonstring(i, depth + 1))
					&& self.text_writes(v, |i| self.nonstring(i, depth + 1))
			}
			_ => false,
		}
	}
	fn split_origin(&self, id: hir::ExprId, depth: usize) -> Option<Origin> {
		if depth >= 128 {
			return None;
		}
		match self.exprs.get(id).kind {
			hir::ExprKind::Call(c) => match c.call {
				hir::Call::Associated { hash, target, .. }
					if hash == Method::Split.hash()
						&& c.args.len() == 1
						&& !self.constants.method_script.closed(hash) =>
				{
					if self.nonstring(target, 0) {
						Some(Origin::Never)
					} else {
						self.delimiter(c.args[0])
							.map(|s| if s { Origin::String } else { Origin::Char })
					}
				}
				// Split has no iter() method: a successfully compiled call cannot return.
				hir::Call::Associated { hash, target, .. }
					if hash == crate::function::iter::Name::Iter.hash()
						&& c.args.is_empty()
						&& !self.constants.method_script.closed(hash) =>
				{
					self.split_origin(target, depth + 1).map(|_| Origin::Never)
				}
				_ => None,
			},
			hir::ExprKind::Variable(v) => {
				let k = self.split_origin(self.text_init(v)?, depth + 1)?;
				self.text_writes(v, |i| self.split_origin(i, depth + 1) == Some(k))
					.then_some(k)
			}
			_ => None,
		}
	}

	#[cold]
	#[inline(never)]
	pub(super) fn call_text_method(
		&mut self,
		c: &hir::ExprCall<'_>,
		target: hir::ExprId,
		m: Method,
		out: u32,
		span: Span,
	) -> Result<()> {
		if c.args.len() + 1 != m.arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
		if (m == Method::Split && self.delimiter(c.args[0]).is_none() && !self.nonstring(target, 0))
			|| (m == Method::Contains && !self.nonrange(target, 0))
		{
			return Err(self.refused(span, Refusal::Method));
		}
		self.transaction
			.text_method(self.context, self.constants, self.pool, m)
			.map_err(|e| e.at(self.location(span)))?;
		let mut ids = Vec::try_with_capacity(c.args.len() + 1)?;
		ids.try_push(target)?;
		for i in c.args {
			ids.try_push(*i)?;
		}
		let operands = self.operands(&ids)?;
		let (args_start, args_len) = self.function.arguments(&operands)?;
		self.emit(
			Op::Call {
				out,
				function: m.id(),
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
	#[cold]
	#[inline(never)]
	pub(super) fn call_text_collect(
		&mut self,
		c: &hir::ExprCall<'_>,
		target: hir::ExprId,
		hash: Hash,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !c.args.is_empty() {
			return Err(self.refused(span, Refusal::Arity));
		}
		if hash != text::collect_hash()
			|| self.split_origin(target, 0).is_none()
			|| !self.context.text_collect()
			|| self.constants.method_script.closed(Hash::ident("collect"))
		{
			return Err(self.refused(span, Refusal::Method));
		}
		let iterator = self.operand(target)?;
		let element = self.slot()?;
		self.raise(element);
		let (args_start, args_len) = self.function.arguments(&[])?;
		self.emit(
			Op::List {
				out,
				tag: Type::Vec,
				args_start,
				args_len,
			},
			span,
		)?;
		let head = self.function.len() as u32;
		self.emit(
			Op::IterNext {
				out: element,
				iterator,
				none: 0,
			},
			span,
		)?;
		self.emit(
			Op::Push {
				value: out,
				new: element,
			},
			span,
		)?;
		self.emit(Op::Release { value: element }, span)?;
		self.emit(Op::Jump { target: head }, span)?;
		let end = self.function.len() as u32;
		self.function.patch(
			head,
			Op::IterNext {
				out: element,
				iterator,
				none: end,
			},
		)?;
		Ok(())
	}
}
