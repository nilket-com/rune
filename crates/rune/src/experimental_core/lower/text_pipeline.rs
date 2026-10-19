//! Statically proven text iterator shapes, expanded into ordinary charged code.
use super::super::code;
use super::*;
use crate::function::fresh_pipeline::Method;
// Bottom-up: StringSplit=0, CharSplit=1, Chars=2, Skip=3, Take=4.
impl<'hir> Lower<'_, 'hir> {
	#[cold]
	#[inline(never)]
	pub(super) fn pipeline_site(&self, c: &hir::ExprCall<'_>) -> Option<(hir::ExprId, Method)> {
		let hir::Call::Associated { target, hash, .. } = c.call else {
			return None;
		};
		if self.nominal_missing_site(c) {
			return None;
		}
		self.pipeline_method(target, hash).map(|m| (target, m))
	}

	#[cold]
	#[inline(never)]
	pub(super) fn pipeline_is_origin(&self, target: hir::ExprId) -> bool {
		self.pipeline_shape(target, 0).is_some()
	}
	#[cold]
	#[inline(never)]
	pub(super) fn pipeline_method(&self, target: hir::ExprId, hash: Hash) -> Option<Method> {
		if let Some(m) = [
			Method::Chars,
			Method::Unwrap,
			Method::Skip,
			Method::Take,
			Method::Count,
		]
		.into_iter()
		.find(|m| m.hash() == hash)
		{
			return Some(m);
		}
		if !self.pipeline_is_origin(target) {
			return None;
		}
		use crate::TypeHash;
		if hash == Method::Next.hash() {
			return Some(Method::Next);
		}
		if hash
			== Hash::ident("collect")
				.with_function_parameters(Hash::parameters([crate::runtime::Vec::HASH]))
		{
			return Some(Method::CollectVec);
		}
		if hash
			== Hash::ident("collect")
				.with_function_parameters(Hash::parameters([crate::alloc::String::HASH]))
		{
			return Some(Method::CollectString);
		}
		None
	}

	pub(super) fn pipeline_shape(&self, id: hir::ExprId, depth: usize) -> Option<Vec<u8>> {
		if depth >= 128 {
			return None;
		}
		use hir::ExprKind as E;
		match self.exprs.get(id).kind {
			E::Group(i) => self.pipeline_shape(i, depth + 1),
			E::Variable(v) => {
				let shape = self.pipeline_shape(self.text_init(v)?, depth + 1)?;
				self.text_writes(v, |rhs| {
					self.pipeline_shape(rhs, depth + 1).as_ref() == Some(&shape)
				})
				.then_some(shape)
			}
			E::Call(c) => {
				let hir::Call::Associated { target, hash, .. } = c.call else {
					return None;
				};
				if hash == crate::function::fresh_text::Method::Split.hash() && c.args.len() == 1 {
					let mut v = Vec::new();
					v.try_push(if self.delimiter(c.args[0])? { 0 } else { 1 })
						.ok()?;
					Some(v)
				} else if hash == Method::Chars.hash() && c.args.is_empty() {
					let mut v = Vec::new();
					v.try_push(2).ok()?;
					Some(v)
				} else if (hash == Method::Skip.hash() || hash == Method::Take.hash())
					&& c.args.len() == 1
				{
					let mut v = self.pipeline_shape(target, depth + 1)?;
					v.try_push(if hash == Method::Skip.hash() { 3 } else { 4 })
						.ok()?;
					Some(v)
				} else {
					None
				}
			}
			_ => None,
		}
	}
	fn pipeline_proof(&mut self, m: Method, span: Span) -> Result<()> {
		self.transaction
			.pipeline_method(self.context, self.constants, self.pool, m)
			.map_err(|e| e.at(self.location(span)))
	}
	fn pipeline_call_slots(&mut self, id: u32, args: &[u32], out: u32, span: Span) -> Result<()> {
		let (args_start, args_len) = self.function.arguments(args)?;
		self.emit(
			Op::Call {
				out,
				function: id,
				args_start,
				args_len,
			},
			span,
		)?;
		self.raise(out);
		Ok(())
	}
	fn pipeline_flag(&mut self, value: u32, span: Span) -> Result<u32> {
		let out = self.slot()?;
		self.pipeline_call_slots(code::VARIANT_SUCCESS, &[value], out, span)?;
		Ok(out)
	}
	fn pipeline_branch(&mut self, condition: u32, span: Span) -> Result<u32> {
		self.emit(
			Op::Branch {
				condition,
				yes: 0,
				no: 0,
			},
			span,
		)
	}
	fn pipeline_patch(&mut self, pc: u32, condition: u32, yes: u32, no: u32) -> Result<()> {
		self.function.patch(pc, Op::Branch { condition, yes, no })
	}
	// Compile-time continuations only. Each runtime layer has one emitted child block.
	fn pipeline_next(&mut self, shape: &[u8], mut node: u32, out: u32, span: Span) -> Result<()> {
		struct Layer {
			kind: u8,
			child: u32,
			state: u32,
			branch: u32,
			start: u32,
			zero: u32,
			one: u32,
			condition: u32,
		}
		let mut layers = Vec::try_with_capacity(shape.len())?;
		for &kind in shape.iter().rev().take(shape.len().saturating_sub(1)) {
			let child = self.slot()?;
			self.pipeline_call_slots(0x8000_0078, &[node], child, span)?;
			let state = self.slot()?;
			if kind == 4 {
				self.pipeline_call_slots(0x8000_007a, &[node], state, span)?;
				let branch = self.pipeline_branch(state, span)?;
				layers.try_push(Layer {
					kind,
					child,
					state,
					branch,
					start: self.pc(),
					zero: 0,
					one: 0,
					condition: 0,
				})?;
			} else if kind == 3 {
				self.pipeline_call_slots(0x8000_0079, &[node], state, span)?;
				let zero = self.slot()?;
				self.emit(
					Op::Literal {
						out: zero,
						value: Value::U64(0),
					},
					span,
				)?;
				let one = self.slot()?;
				self.emit(
					Op::Literal {
						out: one,
						value: Value::U64(1),
					},
					span,
				)?;
				let condition = self.slot()?;
				layers.try_push(Layer {
					kind,
					child,
					state,
					branch: 0,
					start: self.pc(),
					zero,
					one,
					condition,
				})?;
			} else {
				return Err(self.refused(span, Refusal::Method));
			}
			node = child;
		}
		if !shape.first().is_some_and(|leaf| *leaf < 3) {
			return Err(self.refused(span, Refusal::Method));
		}
		self.pipeline_call_slots(Method::Next.id(), &[node], out, span)?;
		for layer in layers.into_iter().rev() {
			if layer.kind == 4 {
				let jump = self.emit(Op::Jump { target: 0 }, span)?;
				let no = self.pc();
				self.emit(
					Op::Variant {
						out,
						tag: Type::Option,
						success: false,
						payload: None,
					},
					span,
				)?;
				let end = self.pc();
				self.function.patch(jump, Op::Jump { target: end })?;
				self.pipeline_patch(layer.branch, layer.state, layer.start, no)?;
			} else {
				self.emit(
					Op::Binary {
						out: layer.condition,
						op: Binary::Eq,
						lhs: layer.state,
						rhs: layer.zero,
					},
					span,
				)?;
				let done = self.pipeline_branch(layer.condition, span)?;
				let flag = self.pipeline_flag(out, span)?;
				let valid = self.pipeline_branch(flag, span)?;
				let discard = self.pc();
				self.emit(Op::Release { value: out }, span)?;
				self.emit(
					Op::Binary {
						out: layer.state,
						op: Binary::Sub,
						lhs: layer.state,
						rhs: layer.one,
					},
					span,
				)?;
				self.emit(
					Op::Jump {
						target: layer.start,
					},
					span,
				)?;
				let end = self.pc();
				self.pipeline_patch(done, layer.condition, end, done + 1)?;
				self.pipeline_patch(valid, flag, discard, end)?;
			}
			self.emit(Op::Release { value: layer.child }, span)?;
		}
		Ok(())
	}
	#[cold]
	#[inline(never)]
	pub(super) fn call_pipeline(
		&mut self,
		c: &hir::ExprCall<'_>,
		target: hir::ExprId,
		m: Method,
		out: u32,
		span: Span,
	) -> Result<()> {
		if c.args.len() + 1 != m.arity() {
			return Err(self.refused(span, Refusal::Arity));
		}
		let shape = if matches!(m, Method::Chars | Method::Unwrap) {
			None
		} else {
			Some(
				self.pipeline_shape(target, 0)
					.ok_or_else(|| self.refused(span, Refusal::Method))?,
			)
		};
		self.pipeline_proof(m, span)?;
		if matches!(
			m,
			Method::Chars | Method::Unwrap | Method::Skip | Method::Take
		) {
			let mut ids = Vec::new();
			ids.try_push(target)?;
			for i in c.args {
				ids.try_push(*i)?;
			}
			let values = self.operands(&ids)?;
			return self.pipeline_call_slots(m.id(), &values, out, span);
		}
		self.pipeline_proof(Method::Next, span)?;
		let node = self.operand(target)?;
		let shape = shape.ok_or_else(|| self.refused(span, Refusal::Method))?;
		if m == Method::Next {
			return self.pipeline_next(&shape, node, out, span);
		}
		self.pipeline_consume(&shape, node, m, out, span)
	}
	fn pipeline_consume(
		&mut self,
		shape: &[u8],
		node: u32,
		m: Method,
		out: u32,
		span: Span,
	) -> Result<()> {
		match m {
			Method::Count => {
				self.emit(
					Op::Literal {
						out,
						value: Value::U64(0),
					},
					span,
				)?;
			}
			Method::CollectString => {
				let text = self.function.string("")?;
				self.emit(Op::String { out, text }, span)?;
			}
			Method::CollectVec => {
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
			}
			_ => return Err(self.refused(span, Refusal::Method)),
		}
		let zero = self.slot()?;
		self.emit(
			Op::Literal {
				out: zero,
				value: Value::U64(0),
			},
			span,
		)?;
		let option = self.slot()?;
		let value = self.slot()?;
		let scratch = self.slot()?;
		let one = self.slot()?;
		self.emit(
			Op::Literal {
				out: one,
				value: Value::U64(1),
			},
			span,
		)?;
		let start = self.pc();
		self.pipeline_next(shape, node, option, span)?;
		let flag = self.pipeline_flag(option, span)?;
		let branch = self.pipeline_branch(flag, span)?;
		let yes = self.pc();
		match m {
			Method::Count => {
				self.emit(
					Op::Binary {
						out,
						op: Binary::Add,
						lhs: out,
						rhs: one,
					},
					span,
				)?;
			}
			_ => {
				self.emit(
					Op::TupleField {
						out: value,
						value: option,
						index: zero,
					},
					span,
				)?;
				self.raise(value);
				if m == Method::CollectVec {
					self.emit(
						Op::Push {
							value: out,
							new: value,
						},
						span,
					)?;
				} else {
					self.pipeline_call_slots(0x8000_007b, &[out, value], scratch, span)?;
				}
				self.emit(Op::Release { value }, span)?;
			}
		}
		self.emit(Op::Release { value: option }, span)?;
		self.emit(Op::Jump { target: start }, span)?;
		let end = self.pc();
		self.pipeline_patch(branch, flag, yes, end)?;
		self.emit(Op::Release { value: option }, span)?;
		self.raise(out);
		Ok(())
	}
	#[cold]
	#[inline(never)]
	pub(super) fn pipeline_for(
		&mut self,
		for_: &hir::ExprFor<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let shape = self
			.pipeline_shape(for_.iter, 0)
			.ok_or_else(|| self.refused(span, Refusal::Method))?;
		self.pipeline_proof(Method::Next, span)?;
		if let Some(out) = out {
			self.unit_into(out, span)?;
		}
		let vars = self.variables.len();
		let mark = self.next;
		let node = self.operand(for_.iter)?;
		let zero = self.slot()?;
		self.emit(
			Op::Literal {
				out: zero,
				value: Value::U64(0),
			},
			span,
		)?;
		let option = self.slot()?;
		let binding = self.slot()?;
		let destructure = !matches!(
			for_.binding.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		);
		if !destructure {
			self.bind(&for_.binding.pat, binding)?;
		}
		let start = self.pc();
		self.pipeline_next(&shape, node, option, span)?;
		let flag = self.pipeline_flag(option, span)?;
		let branch = self.pipeline_branch(flag, span)?;
		let yes = self.pc();
		self.emit(
			Op::TupleField {
				out: binding,
				value: option,
				index: zero,
			},
			span,
		)?;
		self.raise(binding);
		self.emit(Op::Release { value: option }, span)?;
		if destructure {
			self.pattern_local(&for_.binding, binding)?;
		}
		self.loops.try_push(Loop {
			label: for_.label.map(String::try_from).transpose()?,
			start: Some(start),
			cleanup: if destructure { binding + 1 } else { self.next },
			out,
			breaks: Vec::new(),
		})?;
		self.block(&for_.body, Mode::Discard)?;
		if destructure {
			self.pop_to(binding + 1, span)?;
		} else {
			self.emit(Op::Release { value: binding }, span)?;
		}
		self.emit(Op::Jump { target: start }, span)?;
		let end = self.pc();
		self.pipeline_patch(branch, flag, yes, end)?;
		let context = self
			.loops
			.pop()
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		self.variables.truncate(vars);
		self.pop_to(mark, span)
	}
}
