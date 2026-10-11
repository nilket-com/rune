//! Resolved-HIR to independent instructions. No legacy assembly or execution.
//! Construction subset: unsupported forms fail with their original source span.
//!
//! Slot discipline (A1): the frame is a stack. Every expression allocates its
//! result slot before its operands, so when it finishes the operands lie above
//! the result and are popped. A slot is "dirty" after an instruction that may
//! have stored a heap handle in it; popping emits one Release per dirty slot and
//! nothing for the others. Bare locals are read in place where the reference's
//! `writes_nothing` rule allows; nothing here moves a user local.

use super::arithmetic::{Binary, Unary};
use super::code::{Function, Op};
use super::error::{Error, ErrorKind, Location, Refusal, Result};
use super::value::{Type, Value};
use crate as rune;
use crate::alloc::{String, Vec};
use crate::ast::{BinOp, Span, UnOp};
use crate::{hir, Hash};

pub(crate) fn function(
	exprs: &hir::Exprs<'_>,
	item: &hir::ItemFn<'_>,
	source: u32,
	revision: u64,
	resolve: &mut dyn FnMut(Hash, Location) -> Result<u32>,
	constants: &crate::query::QueryInner<'_>,
	stdio_println: bool,
) -> Result<Function> {
	let mut cx = Lower {
		exprs,
		function: Function::new(1, 0)?,
		variables: Vec::new(),
		loops: Vec::new(),
		dirty: Vec::new(),
		next: 0,
		high: 0,
		depth: 0,
		source,
		revision,
		resolve,
		constants,
		stdio_println,
	};

	for arg in item.args {
		let hir::FnArg::Pat(pat) = arg;
		let slot = cx.slot()?;

		// Arguments arrive retained by the caller and may be anything.
		cx.raise(slot);
		cx.bind(&pat.pat, slot)?;
	}

	cx.function.parameters = cx.next;
	cx.block(&item.body, Mode::Return)?;
	cx.function.slots = cx.high.max(1);
	Ok(cx.function)
}

/// How a block's value is delivered.
#[derive(Clone, Copy)]
enum Mode {
	/// Into the given slot, allocated before the block's own slots.
	Into(u32),
	/// Not needed: the tail expression is evaluated and dropped.
	Discard,
	/// Returned from the function: a Return instruction, no cleanup code after it.
	Return,
}

struct Loop {
	label: Option<String>,
	start: u32,
	/// Slots at or above this mark belong to the body and are released on break/continue.
	cleanup: u32,
	/// Where a `break value` is delivered, when the loop's value is used.
	out: Option<u32>,
	breaks: Vec<u32>,
}

struct Lower<'a, 'hir> {
	exprs: &'a hir::Exprs<'hir>,
	function: Function,
	variables: Vec<(hir::Variable, u32)>,
	loops: Vec<Loop>,
	/// Per slot: may currently hold a heap handle that needs a Release.
	dirty: Vec<bool>,
	next: u32,
	high: u32,
	depth: usize,
	source: u32,
	revision: u64,
	resolve: &'a mut dyn FnMut(Hash, Location) -> Result<u32>,
	constants: &'a crate::query::QueryInner<'hir>,
	/// The Context's `println` is the certified stdio builtin; otherwise refuse.
	stdio_println: bool,
}

impl<'a, 'hir> Lower<'a, 'hir> {
	fn location(&self, span: Span) -> Location {
		Location {
			source: self.source,
			revision: self.revision,
			start: span.start.0,
			end: span.end.0,
		}
	}

	fn refused(&self, span: Span, refusal: Refusal) -> Error {
		Error::refused(refusal).at(self.location(span))
	}

	/// Allocate the next slot on the frame stack. A popped slot was released
	/// before it was popped, so a fresh allocation starts clean.
	fn slot(&mut self) -> Result<u32> {
		if self.next >= 65_536 {
			return Err(Error::new(ErrorKind::FrameCapacity, "lower slots"));
		}

		let slot = self.next;
		self.next += 1;

		if self.next > self.high {
			self.high = self.next;
		}

		while self.dirty.len() <= slot as usize {
			self.dirty.try_push(false)?;
		}

		self.dirty[slot as usize] = false;
		Ok(slot)
	}

	/// A slot's flag is a may-hold-heap fact over every path that reaches the
	/// current point, so a write only ever raises it; it falls only when a
	/// Release is emitted on the fall-through path (pop_to) or the slot is
	/// freshly allocated after such a pop. A scalar written on one path must
	/// not hide a heap value written on another (if/else, break values).
	fn raise(&mut self, slot: u32) {
		if let Some(flag) = self.dirty.get_mut(slot as usize) {
			*flag = true;
		}
	}

	/// Emit an instruction and record whether its destination may now hold a
	/// heap handle. This is the only place that classification lives.
	fn emit(&mut self, op: Op, span: Span) -> Result<u32> {
		match op {
			// Never a heap value: the flag is left as the other paths set it.
			Op::Literal { .. }
			| Op::Unary { .. }
			| Op::Cast { .. }
			| Op::Is { .. }
			| Op::Len { .. } => (),
			Op::Binary { out, op, .. } | Op::AssignBinary { out, op, .. } => {
				// Only string concatenation yields a heap value among the primitive rules.
				if op == Binary::Add {
					self.raise(out);
				}
			}

			Op::Copy { out, .. }
			| Op::String { out, .. }
			| Op::List { out, .. }
			| Op::Record { out, .. }
			| Op::Variant { out, .. }
			| Op::Template { out, .. }
			| Op::Range { out, .. }
			| Op::IterStart { out, .. }
			| Op::IterNext { out, .. }
			| Op::Clone { out, .. }
			| Op::Get { out, .. }
			| Op::Field { out, .. }
			| Op::TupleField { out, .. }
			| Op::Call { out, .. }
			| Op::CallValue { out, .. }
			| Op::Try { out, .. } => self.raise(out),
			// Release is only emitted by pop_to and release_from; pop_to clears
			// the flag itself, release_from deliberately does not.
			Op::Release { .. } => (),
			Op::Jump { .. }
			| Op::Branch { .. }
			| Op::Return { .. }
			| Op::Set { .. }
			| Op::SetField { .. }
			| Op::SetTupleField { .. }
			| Op::Push { .. }
			| Op::Drop { .. }
			| Op::Print { .. } => (),
		}

		self.function.push(op, self.location(span))
	}

	/// Release every dirty slot at or above `mark`, once, then pop to `mark`.
	fn pop_to(&mut self, mark: u32, span: Span) -> Result<()> {
		for value in (mark..self.next).rev() {
			if self.dirty[value as usize] {
				self.emit(Op::Release { value }, span)?;
				self.dirty[value as usize] = false;
			}
		}

		self.next = mark;
		Ok(())
	}

	/// Pop without emitting: only after a Return, where the frame is gone.
	fn forget_to(&mut self, mark: u32) {
		for value in mark..self.next {
			self.dirty[value as usize] = false;
		}

		self.next = mark;
	}

	/// Release dirty slots at or above `mark` on a path that leaves the loop,
	/// without forgetting them: the fall-through path still owns them.
	fn release_from(&mut self, mark: u32, span: Span) -> Result<()> {
		for value in (mark..self.next).rev() {
			if self.dirty[value as usize] {
				self.function
					.push(Op::Release { value }, self.location(span))?;
			}
		}

		Ok(())
	}

	fn pc(&self) -> u32 {
		self.function.len() as u32
	}

	fn variable(&self, variable: hir::Variable, span: Span) -> Result<u32> {
		self.variables
			.iter()
			.rev()
			.find(|(v, _)| *v == variable)
			.map(|(_, slot)| *slot)
			.ok_or_else(|| self.refused(span, Refusal::Variable))
	}

	fn bind(&mut self, pat: &hir::Pat<'_>, slot: u32) -> Result<()> {
		match pat.kind {
			hir::PatKind::Ignore => Ok(()),
			hir::PatKind::Path(hir::PatPathKind::Ident(variable)) => {
				self.variables.try_push((*variable, slot))?;
				Ok(())
			}

			hir::PatKind::Path(hir::PatPathKind::Kind(_))
			| hir::PatKind::Lit(_)
			| hir::PatKind::Sequence(_)
			| hir::PatKind::Object(_) => Err(self.refused(pat.span, Refusal::Pattern)),
		}
	}

	fn loop_index(&self, label: Option<&str>, span: Span) -> Result<usize> {
		self.loops
			.iter()
			.rposition(|context| label.is_none() || context.label.as_deref() == label)
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))
	}

	fn unit_into(&mut self, out: u32, span: Span) -> Result<()> {
		self.emit(
			Op::Literal {
				out,
				value: Value::Unit,
			},
			span,
		)?;
		Ok(())
	}

	fn block(&mut self, block: &hir::Block<'_>, mode: Mode) -> Result<()> {
		if block.label.is_some() {
			return Err(self.refused(block.span, Refusal::LabelledBlock));
		}

		let vars = self.variables.len();
		let mark = self.next;

		for statement in block.statements {
			match statement {
				hir::Stmt::Local(local) => {
					// The local owns its slot until the block ends; nothing can read
					// the slot before the initialiser has written it.
					let slot = self.slot()?;
					self.expr_into(local.expr, slot)?;
					self.bind(&local.pat.pat, slot)?;
				}

				hir::Stmt::Expr(_, expr) => self.statement(*expr)?,
			}
		}

		match (mode, block.value) {
			(Mode::Into(out), Some(value)) => {
				self.expr_into(value, out)?;
				self.pop_to(mark, block.span)?;
			}

			(Mode::Into(out), None) => {
				self.unit_into(out, block.span)?;
				self.pop_to(mark, block.span)?;
			}

			(Mode::Discard, Some(value)) => {
				self.statement(value)?;
				self.pop_to(mark, block.span)?;
			}
			(Mode::Discard, None) => self.pop_to(mark, block.span)?,
			(Mode::Return, value) => {
				let value = match value {
					Some(id) => self.operand(id)?,
					None => {
						let slot = self.slot()?;
						self.unit_into(slot, block.span)?;
						slot
					}
				};

				self.emit(Op::Return { value }, block.span)?;
				self.forget_to(mark);
			}
		}

		self.variables.truncate(vars);
		Ok(())
	}

	fn enter(&mut self, id: hir::ExprId) -> Result<hir::Expr<'hir>> {
		let expr = *self.exprs.get(id);

		// The frontend is recursive too. This is a named construction bound,
		// not an unbounded host-stack walk.
		if self.depth >= 128 {
			return Err(self.refused(expr.span, Refusal::DepthLimit));
		}

		self.depth += 1;
		Ok(expr)
	}

	/// Evaluate into a fresh slot, which is returned. Operand temporaries are
	/// released (if dirty) and popped; the result stays allocated for the caller.
	fn expr(&mut self, id: hir::ExprId) -> Result<u32> {
		let expr = self.enter(id)?;
		let out = self.slot()?;
		let result = self.inner(expr, out);
		self.depth -= 1;
		result?;
		self.pop_to(out + 1, expr.span)?;
		Ok(out)
	}

	/// Evaluate into `out`, a slot the caller allocated earlier (so no operand
	/// can alias it). Everything allocated here is released and popped.
	fn expr_into(&mut self, id: hir::ExprId, out: u32) -> Result<()> {
		let expr = self.enter(id)?;
		let mark = self.next;
		let result = self.inner(expr, out);
		self.depth -= 1;
		result?;
		self.pop_to(mark, expr.span)
	}

	/// Evaluate for effect only; no result slot survives.
	fn statement(&mut self, id: hir::ExprId) -> Result<()> {
		let expr = self.enter(id)?;
		let mark = self.next;
		let result = self.effect(expr);
		self.depth -= 1;
		result?;
		self.pop_to(mark, expr.span)
	}

	/// A read-only operand: a bare local is used in place, anything else is
	/// evaluated into a temporary. Valid until the consuming instruction runs;
	/// the caller must not let a later-evaluated expression replace the local.
	fn operand(&mut self, id: hir::ExprId) -> Result<u32> {
		match self.exprs.get(id).kind {
			hir::ExprKind::Variable(variable) => self.variable(variable, self.exprs.get(id).span),
			_ => self.expr(id),
		}
	}

	/// The reference assembler's conservative `writes_nothing` predicate:
	/// only a literal, a variable read or a type is known not to write.
	fn writes_nothing(&self, id: hir::ExprId) -> bool {
		matches!(
			self.exprs.get(id).kind,
			hir::ExprKind::Lit(..) | hir::ExprKind::Variable(..) | hir::ExprKind::Type(..)
		)
	}

	/// Operands evaluated left to right. A bare local is used in place only if
	/// every later operand is known not to write; otherwise it is snapshotted,
	/// because a later operand could rebind it before the instruction runs.
	fn operands(&mut self, ids: &[hir::ExprId]) -> Result<Vec<u32>> {
		let mut slots = Vec::try_with_capacity(ids.len())?;
		for (i, id) in ids.iter().enumerate() {
			let later_write = ids[i + 1..]
				.iter()
				.any(|later| !self.writes_nothing(*later));
			let slot = if later_write {
				self.expr(*id)?
			} else {
				self.operand(*id)?
			};

			slots.try_push(slot)?;
		}

		Ok(slots)
	}

	/// Assignment operands use deferred local addresses (observed reference
	/// behaviour): a bare local is read when the store executes, after the
	/// right side; a non-trivial expression materialises in source order.
	fn assignment_operand(&mut self, id: hir::ExprId, span: Span) -> Result<u32> {
		match self.exprs.get(id).kind {
			hir::ExprKind::Variable(variable) => self.variable(variable, span),
			_ => self.expr(id),
		}
	}

	/// `Ok(None)`: not a type this core knows. A fresh struct is interned
	/// process-wide; interner exhaustion is the `TypeTable` refusal.
	fn type_tag(&self, hash: Hash, span: Span) -> Result<Option<Type>> {
		if let Some(tag) = type_tag(hash) {
			return Ok(Some(tag));
		}

		if !self.constants.fresh_struct_type(hash) {
			return Ok(None);
		}

		self.struct_tag(hash, span).map(Some)
	}

	/// The identity is the type hash; the name recorded with it is the hash's
	/// text, formatted on the stack only when the hash is new (item paths are
	/// not plumbed into the lowerer).
	fn struct_tag(&self, hash: Hash, span: Span) -> Result<Type> {
		match super::nominal::intern_fmt(hash.into_inner(), format_args!("{hash}")) {
			Ok(nominal) => Ok(Type::Struct(nominal)),
			Err(e) => Err(e.at(self.location(span))),
		}
	}

	fn field_index(&mut self, index: usize, span: Span) -> Result<u32> {
		let index = i64::try_from(index).map_err(|_| self.refused(span, Refusal::FieldIndex))?;
		let out = self.slot()?;
		self.emit(
			Op::Literal {
				out,
				value: Value::I64(index),
			},
			span,
		)?;
		Ok(out)
	}

	fn read_field(
		&mut self,
		out: u32,
		value: u32,
		field: hir::ExprField<'_>,
		span: Span,
	) -> Result<()> {
		match field {
			hir::ExprField::Index(index) => {
				let index = self.field_index(index, span)?;
				self.emit(Op::TupleField { out, value, index }, span)?;
			}

			hir::ExprField::Ident(key) => {
				let key = self.function.string(key)?;
				self.emit(Op::Field { out, value, key }, span)?;
			}
			hir::ExprField::IdentGenerics(..) => return Err(self.refused(span, Refusal::Field)),
		}

		Ok(())
	}

	fn write_field(
		&mut self,
		value: u32,
		field: hir::ExprField<'_>,
		new: u32,
		span: Span,
	) -> Result<()> {
		match field {
			hir::ExprField::Index(index) => {
				let index = self.field_index(index, span)?;
				self.emit(Op::SetTupleField { value, index, new }, span)?;
			}

			hir::ExprField::Ident(key) => {
				let key = self.function.string(key)?;
				self.emit(Op::SetField { value, key, new }, span)?;
			}
			hir::ExprField::IdentGenerics(..) => return Err(self.refused(span, Refusal::Field)),
		}

		Ok(())
	}

	/// Lower an expression whose value is not needed. Forms with a natural
	/// effect-only shape avoid producing a unit; everything else goes through
	/// `inner` into a temporary that the caller pops.
	fn effect(&mut self, expr: hir::Expr<'hir>) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;

		match expr.kind {
			E::Assign(assign) => self.assign(assign, span),
			E::Binary(binary) if binary.op.is_assign() => self.compound_assign(binary, span),
			E::Block(block) => self.block(block, Mode::Discard),
			E::If(conditional) => self.conditional(conditional, None, span),
			E::Loop(loop_) => self.loop_(loop_, None, span),
			E::For(for_) => self.for_(for_, None, span),
			E::Call(call) => {
				if !self.effect_call(call, span)? {
					let out = self.slot()?;
					self.inner(expr, out)?;
				}

				Ok(())
			}

			E::Group(id) => self.statement(id),
			_ => {
				let out = self.slot()?;
				self.inner(expr, out)
			}
		}
	}

	/// Calls whose value is unit by construction: println, push, drop.
	/// Returns false (having emitted nothing) when the call is not one of them.
	fn effect_call(&mut self, call: &hir::ExprCall<'_>, span: Span) -> Result<bool> {
		match call.call {
			hir::Call::Associated { target, hash }
				if hash == Hash::ident("push") && call.args.len() == 1 =>
			{
				let slots = self.operands(&[target, call.args[0]])?;
				self.emit(
					Op::Push {
						value: slots[0],
						new: slots[1],
					},
					span,
				)?;
				Ok(true)
			}

			hir::Call::Meta { hash }
				if hash == Hash::type_hash(crate::item!(::std::io::println))
					&& call.args.len() == 1 =>
			{
				// The same host IO policy as the value position in `call`: a statement
				// `println!` must not bypass it.
				if !self.stdio_println {
					return Err(self.refused(span, Refusal::Native));
				}

				let value = self.operand(call.args[0])?;
				self.emit(Op::Print { value }, span)?;
				Ok(true)
			}

			hir::Call::Meta { hash }
				if hash == Hash::type_hash(crate::item!(::std::mem::drop))
					&& call.args.len() == 1 =>
			{
				let value = self.operand(call.args[0])?;
				self.emit(Op::Drop { value }, span)?;
				Ok(true)
			}
			_ => Ok(false),
		}
	}

	fn assign(&mut self, assign: &hir::ExprAssign, span: Span) -> Result<()> {
		use hir::ExprKind as E;
		match self.exprs.get(assign.lhs).kind {
			E::Variable(variable) => {
				// The right side is evaluated to a temporary first: it may read the
				// variable, so the variable's slot cannot be the destination.
				let out = self.variable(variable, span)?;
				let value = self.expr(assign.rhs)?;
				self.emit(Op::Copy { out, value }, span)?;
			}

			E::Index(index) => {
				// The reference defers a bare local address rather than copying
				// its value: RHS rebinding changes the receiver read at Set.
				// Nontrivial target expressions materialise before index/RHS.
				let target = self.assignment_operand(index.target, span)?;
				let key = self.assignment_operand(index.index, span)?;
				let value = self.operand(assign.rhs)?;
				self.emit(
					Op::Set {
						value: target,
						index: key,
						new: value,
					},
					span,
				)?;
			}

			E::FieldAccess(field) => {
				let target = self.assignment_operand(field.expr, span)?;
				let value = self.operand(assign.rhs)?;
				self.write_field(target, field.expr_field, value, span)?;
			}
			_ => return Err(self.refused(span, Refusal::Assignment)),
		}

		Ok(())
	}

	fn compound_assign(&mut self, binary: &hir::ExprBinary, span: Span) -> Result<()> {
		use hir::ExprKind as E;
		let op = binary_op(binary.op).ok_or_else(|| self.refused(span, Refusal::Operator))?;

		match self.exprs.get(binary.lhs).kind {
			E::Variable(variable) => {
				// The local is read by the instruction, after the right side ran.
				let target = self.variable(variable, span)?;
				let rhs = self.operand(binary.rhs)?;
				self.emit(
					Op::AssignBinary {
						out: target,
						op,
						lhs: target,
						rhs,
					},
					span,
				)?;
			}

			E::Index(index) => {
				let target = self.assignment_operand(index.target, span)?;
				let key = self.assignment_operand(index.index, span)?;
				let rhs = self.operand(binary.rhs)?;
				let old = self.slot()?;
				self.emit(
					Op::Get {
						out: old,
						value: target,
						index: key,
					},
					span,
				)?;
				self.emit(
					Op::AssignBinary {
						out: old,
						op,
						lhs: old,
						rhs,
					},
					span,
				)?;
				self.emit(
					Op::Set {
						value: target,
						index: key,
						new: old,
					},
					span,
				)?;
			}

			E::FieldAccess(field) => {
				let target = self.assignment_operand(field.expr, span)?;
				let rhs = self.operand(binary.rhs)?;
				let old = self.slot()?;
				self.read_field(old, target, field.expr_field, span)?;
				self.emit(
					Op::AssignBinary {
						out: old,
						op,
						lhs: old,
						rhs,
					},
					span,
				)?;
				self.write_field(target, field.expr_field, old, span)?;
			}

			_ => return Err(self.refused(span, Refusal::Assignment)),
		}

		Ok(())
	}

	fn conditional(
		&mut self,
		conditional: &hir::Conditional<'_>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let mut ends = Vec::new();

		// Condition temporaries stay allocated until the whole conditional ends,
		// where one pop releases them on every path (a release placed between
		// the Branch and its targets would be skipped by both).
		let mark = self.next;

		for branch in conditional.branches {
			let hir::Condition::Expr(_, id) = branch.condition else {
				return Err(self.refused(span, Refusal::Condition));
			};

			let condition = self.operand(*id)?;
			let test = self.emit(Op::Jump { target: 0 }, span)?;
			let yes = self.pc();

			match out {
				Some(out) => self.block(&branch.block, Mode::Into(out))?,
				None => self.block(&branch.block, Mode::Discard)?,
			}

			ends.try_push(self.emit(Op::Jump { target: 0 }, span)?)?;
			let no = self.pc();
			self.function
				.patch(test, Op::Branch { condition, yes, no })?;
		}

		match (conditional.fallback, out) {
			(Some(block), Some(out)) => self.block(block, Mode::Into(out))?,
			(Some(block), None) => self.block(block, Mode::Discard)?,
			(None, Some(out)) => self.unit_into(out, span)?,
			(None, None) => (),
		}

		let end = self.pc();
		for jump in ends {
			self.function.patch(jump, Op::Jump { target: end })?;
		}

		self.pop_to(mark, span)
	}

	fn loop_(&mut self, loop_: &hir::ExprLoop<'_>, out: Option<u32>, span: Span) -> Result<()> {
		if let Some(out) = out {
			// A loop that ends without `break value` is unit.
			self.unit_into(out, span)?;
		}

		// A condition temporary (if any) lives across the body and is released
		// once after the loop; each iteration overwrites it.
		let mark = self.next;
		let start = self.pc();
		let condition = if let Some(condition) = loop_.condition {
			let hir::Condition::Expr(_, id) = condition else {
				return Err(self.refused(span, Refusal::Condition));
			};

			let value = self.operand(*id)?;
			let pc = self.emit(Op::Jump { target: 0 }, span)?;
			Some((value, pc))
		} else {
			None
		};

		self.loops.try_push(Loop {
			label: loop_.label.map(String::try_from).transpose()?,
			start,
			cleanup: self.next,
			out,
			breaks: Vec::new(),
		})?;
		let body_start = self.pc();
		self.block(&loop_.body, Mode::Discard)?;
		self.emit(Op::Jump { target: start }, span)?;
		let end = self.pc();

		if let Some((condition, pc)) = condition {
			self.function.patch(
				pc,
				Op::Branch {
					condition,
					yes: body_start,
					no: end,
				},
			)?;
		}

		let context = self
			.loops
			.pop()
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}

		self.pop_to(mark, span)
	}

	fn for_(&mut self, for_: &hir::ExprFor<'_>, out: Option<u32>, span: Span) -> Result<()> {
		if let Some(out) = out {
			self.unit_into(out, span)?;
		}

		let vars = self.variables.len();
		let mark = self.next;
		let value = self.operand(for_.iter)?;
		let iterator = self.slot()?;
		self.emit(
			Op::IterStart {
				out: iterator,
				value,
			},
			span,
		)?;
		let binding = self.slot()?;
		self.bind(&for_.binding.pat, binding)?;
		let start = self.pc();
		let header = self.emit(
			Op::IterNext {
				out: binding,
				iterator,
				none: 0,
			},
			span,
		)?;
		self.loops.try_push(Loop {
			label: for_.label.map(String::try_from).transpose()?,
			start,
			cleanup: self.next,
			out,
			breaks: Vec::new(),
		})?;
		self.block(&for_.body, Mode::Discard)?;
		self.emit(Op::Jump { target: start }, span)?;
		let end = self.pc();
		self.function.patch(
			header,
			Op::IterNext {
				out: binding,
				iterator,
				none: end,
			},
		)?;
		let context = self
			.loops
			.pop()
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}

		// Ends the iterator's lease (and drops the iterable temporary, if any)
		// on every exit path, since breaks land here too.
		self.pop_to(mark, span)?;
		self.variables.truncate(vars);
		Ok(())
	}

	fn inner(&mut self, expr: hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;

		match expr.kind {
			E::Const(hash) => {
				let constant = self
					.constants
					.get_const_value(hash)
					.ok_or_else(|| self.refused(span, Refusal::UnresolvedConstant))?;
				match super::constants::read(constant)? {
					super::constants::Scalar::Primitive(value) => {
						self.emit(Op::Literal { out, value }, span)?;
					}

					super::constants::Scalar::Text(value) => {
						let text = self.function.string(&value)?;
						self.emit(Op::String { out, text }, span)?;
					}
				}
			}

			E::Type(tag) => {
				let value = Value::Type(
					self.type_tag(tag.into_hash(), span)?
						.ok_or_else(|| self.refused(span, Refusal::TypeExpr))?,
				);
				self.emit(Op::Literal { out, value }, span)?;
			}

			E::Fn(hash) => {
				let at = self.location(span);
				let id = (self.resolve)(hash, at)?;

				// Stamped with the program identity by `Builder::finish`.
				let value = Value::Function { code: 0, id };
				self.emit(Op::Literal { out, value }, span)?;
			}

			E::Group(id) => self.expr_into(id, out)?,
			E::Variable(variable) => {
				let value = self.variable(variable, span)?;
				self.emit(Op::Copy { out, value }, span)?;
			}

			E::Lit(lit) => {
				let value = match lit {
					hir::Lit::Bool(v) => Value::Bool(v),
					hir::Lit::Unsigned(v) => Value::U64(v),
					hir::Lit::Signed(v) => Value::I64(v),
					hir::Lit::Float(v) => Value::F64(v),
					hir::Lit::Char(v) => Value::Char(v),
					hir::Lit::Str(v) => {
						let text = self.function.string(v)?;
						self.emit(Op::String { out, text }, span)?;
						return Ok(());
					}
					hir::Lit::ByteStr(_) => return Err(self.refused(span, Refusal::ByteStr)),
				};

				self.emit(Op::Literal { out, value }, span)?;
			}

			E::Block(block) => self.block(block, Mode::Into(out))?,
			E::Range(range) => {
				let (start, end) = match range {
					hir::ExprRange::Range { start, end } => (start, end),
					hir::ExprRange::RangeFrom { .. }
					| hir::ExprRange::RangeFull
					| hir::ExprRange::RangeInclusive { .. }
					| hir::ExprRange::RangeToInclusive { .. }
					| hir::ExprRange::RangeTo { .. } => {
						return Err(self.refused(span, Refusal::Range));
					}
				};

				let slots = self.operands(&[*start, *end])?;
				self.emit(
					Op::Range {
						out,
						start: slots[0],
						end: slots[1],
					},
					span,
				)?;
			}

			E::For(for_) => self.for_(for_, Some(out), span)?,
			E::Loop(loop_) => self.loop_(loop_, Some(out), span)?,
			E::Break(break_) => {
				let index = self.loop_index(break_.label, span)?;
				match (self.loops[index].out, break_.expr) {
					(Some(target), Some(id)) => self.expr_into(id, target)?,
					(Some(target), None) => self.unit_into(target, span)?,
					(None, Some(id)) => self.statement(id)?,
					(None, None) => (),
				}

				let cleanup = self.loops[index].cleanup;
				self.release_from(cleanup, span)?;
				let pc = self.emit(Op::Jump { target: 0 }, span)?;
				self.loops[index].breaks.try_push(pc)?;

				// `out` is never written: no instruction after the jump runs.
			}

			E::Continue(continue_) => {
				let index = self.loop_index(continue_.label, span)?;
				let target = self.loops[index].start;
				let cleanup = self.loops[index].cleanup;
				self.release_from(cleanup, span)?;
				self.emit(Op::Jump { target }, span)?;
			}

			E::Unary(unary) => {
				let op = match unary.op {
					UnOp::Neg(..) => Unary::Neg,
					UnOp::Not(..) => Unary::Not,
					_ => return Err(self.refused(span, Refusal::Operator)),
				};

				let value = self.operand(unary.expr)?;
				self.emit(Op::Unary { out, op, value }, span)?;
			}

			E::Binary(binary) => self.binary(binary, out, span)?,
			E::Object(object) => {
				let tag = match object.kind {
					hir::ExprObjectKind::Anonymous => Type::Object,
					hir::ExprObjectKind::Struct { hash } => self.struct_tag(hash, span)?,
					hir::ExprObjectKind::ExternalType { .. } => {
						return Err(self.refused(span, Refusal::ObjectConstructor));
					}
				};

				let mut keys = Vec::try_with_capacity(object.assignments.len())?;
				let mut ids = Vec::try_with_capacity(object.assignments.len())?;

				for field in object.assignments {
					keys.try_push(self.function.string(field.key.1)?)?;
					ids.try_push(field.assign)?;
				}

				let values = self.operands(&ids)?;
				let mut fields = Vec::try_with_capacity(keys.len() * 2)?;

				for (key, value) in keys.iter().zip(values.iter()) {
					fields.try_push(*key)?;
					fields.try_push(*value)?;
				}

				let (args_start, args_len) = self.function.arguments(&fields)?;
				self.emit(
					Op::Record {
						out,
						tag,
						args_start,
						args_len,
					},
					span,
				)?;
			}

			E::FieldAccess(field) => {
				let value = self.operand(field.expr)?;
				self.read_field(out, value, field.expr_field, span)?;
			}

			E::Assign(assign) => {
				self.assign(assign, span)?;
				self.unit_into(out, span)?;
			}

			E::Index(index) => {
				let slots = self.operands(&[index.target, index.index])?;
				self.emit(
					Op::Get {
						out,
						value: slots[0],
						index: slots[1],
					},
					span,
				)?;
			}

			E::Template(template) => {
				if template.exprs.len() > 4096 {
					return Err(self.refused(span, Refusal::FormatParts));
				}

				let mut ids = Vec::try_with_capacity(template.exprs.len())?;
				let mut debugs = Vec::try_with_capacity(template.exprs.len())?;

				for id in template.exprs {
					let (id, debug) = match self.exprs.get(*id).kind {
						E::Format(format) => {
							let spec = format.spec;
							if spec.fill.is_some()
								|| spec.align.is_some() || spec.width.is_some()
								|| spec.precision.is_some()
								|| spec.flags.is_some_and(|f| !f.is_empty())
							{
								return Err(self.refused(span, Refusal::FormatSpec));
							}

							let debug = match spec.format_type.unwrap_or_default() {
								crate::runtime::format::Type::Display => false,
								crate::runtime::format::Type::Debug => true,
								_ => return Err(self.refused(span, Refusal::FormatSpec)),
							};

							(format.value, debug)
						}

						_ => (*id, false),
					};

					ids.try_push(id)?;
					debugs.try_push(u32::from(debug))?;
				}

				// Preserve argument effects: render only after all operands ran.
				let values = self.operands(&ids)?;
				let mut descriptors = Vec::try_with_capacity(ids.len() * 2)?;

				for (value, debug) in values.iter().zip(debugs.iter()) {
					descriptors.try_push(*value)?;
					descriptors.try_push(*debug)?;
				}

				let (args_start, args_len) = self.function.arguments(&descriptors)?;
				self.emit(
					Op::Template {
						out,
						args_start,
						args_len,
					},
					span,
				)?;
			}

			E::Tuple(sequence) | E::Vec(sequence) => {
				let tag = if matches!(expr.kind, E::Vec(..)) {
					Type::Vec
				} else {
					Type::Tuple
				};

				if tag == Type::Tuple && sequence.items.is_empty() {
					self.unit_into(out, span)?;
					return Ok(());
				}

				let values = self.operands(sequence.items)?;
				let (args_start, args_len) = self.function.arguments(&values)?;
				self.emit(
					Op::List {
						out,
						tag,
						args_start,
						args_len,
					},
					span,
				)?;
			}

			E::Try(value) => {
				let value = self.operand(value)?;
				self.emit(Op::Try { out, value }, span)?;
			}

			E::Return(value) => {
				let value = match value {
					Some(id) => self.operand(id)?,
					None => {
						let slot = self.slot()?;
						self.unit_into(slot, span)?;
						slot
					}
				};

				self.emit(Op::Return { value }, span)?;

				// `out` is never written: nothing after a Return runs.
			}

			E::Call(call) => self.call(call, out, span)?,
			E::If(conditional) => self.conditional(conditional, Some(out), span)?,
			E::Match(_) => return Err(self.refused(span, Refusal::Match)),
			E::AsyncBlock(_) => return Err(self.refused(span, Refusal::AsyncBlock)),
			E::Yield(_) => return Err(self.refused(span, Refusal::Yield)),
			E::Await(_) => return Err(self.refused(span, Refusal::Await)),
			E::Select(_) => return Err(self.refused(span, Refusal::Select)),
			E::CallClosure(_) => return Err(self.refused(span, Refusal::CallClosure)),
			E::Format(_) => return Err(self.refused(span, Refusal::Format)),
			E::Static(_) => return Err(self.refused(span, Refusal::Static)),
		}

		Ok(())
	}

	fn binary(&mut self, binary: &hir::ExprBinary, out: u32, span: Span) -> Result<()> {
		use hir::ExprKind as E;
		if matches!(binary.op, BinOp::As(..) | BinOp::Is(..) | BinOp::IsNot(..)) {
			let E::Type(tag) = self.exprs.get(binary.rhs).kind else {
				return Err(self.refused(span, Refusal::TypeExpr));
			};

			let tag = self
				.type_tag(tag.into_hash(), span)?
				.ok_or_else(|| self.refused(span, Refusal::TypeExpr))?;
			let value = self.operand(binary.lhs)?;

			if matches!(binary.op, BinOp::As(..)) {
				self.emit(Op::Cast { out, value, tag }, span)?;
			} else {
				self.emit(
					Op::Is {
						out,
						value,
						tag,
						negate: matches!(binary.op, BinOp::IsNot(..)),
					},
					span,
				)?;
			}

			return Ok(());
		}

		if matches!(binary.op, BinOp::And(..) | BinOp::Or(..)) {
			// `out` holds the left value while the right side is decided. It is a
			// fresh slot, so the right side cannot observe it through a local.
			self.expr_into(binary.lhs, out)?;
			let branch = self.emit(Op::Jump { target: 0 }, span)?;
			let rhs_pc = self.pc();
			let mark = self.next;
			let rhs = self.operand(binary.rhs)?;

			// A Branch validates bool on both the taken RHS and LHS.
			let check = self.emit(
				Op::Branch {
					condition: rhs,
					yes: 0,
					no: 0,
				},
				span,
			)?;
			let copy = self.pc();
			self.emit(Op::Copy { out, value: rhs }, span)?;
			self.pop_to(mark, span)?;
			let end = self.pc();
			self.function.patch(
				check,
				Op::Branch {
					condition: rhs,
					yes: copy,
					no: copy,
				},
			)?;
			let (yes, no) = if matches!(binary.op, BinOp::And(..)) {
				(rhs_pc, end)
			} else {
				(end, rhs_pc)
			};

			self.function.patch(
				branch,
				Op::Branch {
					condition: out,
					yes,
					no,
				},
			)?;
			return Ok(());
		}

		if binary.op.is_assign() {
			self.compound_assign(binary, span)?;
			self.unit_into(out, span)?;
			return Ok(());
		}

		let op = binary_op(binary.op).ok_or_else(|| self.refused(span, Refusal::Operator))?;
		let slots = self.operands(&[binary.lhs, binary.rhs])?;
		self.emit(
			Op::Binary {
				out,
				op,
				lhs: slots[0],
				rhs: slots[1],
			},
			span,
		)?;
		Ok(())
	}

	fn call(&mut self, call: &hir::ExprCall<'_>, out: u32, span: Span) -> Result<()> {
		if let hir::Call::Meta { hash } = call.call {
			let tag = if hash == Hash::type_hash(crate::item!(::std::option::Option::Some))
				|| hash == Hash::type_hash(crate::item!(::std::option::Option::None))
			{
				Some(Type::Option)
			} else if hash == Hash::type_hash(crate::item!(::std::result::Result::Ok))
				|| hash == Hash::type_hash(crate::item!(::std::result::Result::Err))
			{
				Some(Type::Result)
			} else {
				None
			};

			if let Some(tag) = tag {
				let none = hash == Hash::type_hash(crate::item!(::std::option::Option::None));
				if call.args.len() != usize::from(!none) {
					return Err(self.refused(span, Refusal::Arity));
				}

				let payload = if none {
					None
				} else {
					Some(self.operand(call.args[0])?)
				};

				let success = hash == Hash::type_hash(crate::item!(::std::option::Option::Some))
					|| hash == Hash::type_hash(crate::item!(::std::result::Result::Ok));
				self.emit(
					Op::Variant {
						out,
						tag,
						success,
						payload,
					},
					span,
				)?;
				return Ok(());
			}
		}

		if let hir::Call::Var { name } = call.call {
			// The callee token is read when the call executes, after the arguments.
			let value = self.variable(name, span)?;
			let args = self.operands(call.args)?;
			let (args_start, args_len) = self.function.arguments(&args)?;
			self.emit(
				Op::CallValue {
					out,
					value,
					args_start,
					args_len,
				},
				span,
			)?;
			return Ok(());
		}

		if let hir::Call::Associated { target, hash } = call.call {
			if hash == Hash::ident("len") && call.args.is_empty() {
				let value = self.operand(target)?;
				self.emit(Op::Len { out, value }, span)?;
			} else if hash == Hash::ident("clone") && call.args.is_empty() {
				let value = self.operand(target)?;
				self.emit(Op::Clone { out, value }, span)?;
			} else if hash == Hash::ident("push") && call.args.len() == 1 {
				self.effect_call(call, span)?;
				self.unit_into(out, span)?;
			} else {
				return Err(self.refused(span, Refusal::Method));
			}

			return Ok(());
		}

		let hash = match call.call {
			hir::Call::Meta { hash } => hash,
			hir::Call::Expr { .. } => return Err(self.refused(span, Refusal::Call)),
			hir::Call::ConstFn { .. } => return Err(self.refused(span, Refusal::ConstCall)),
			// These arms already returned above; retain an explicit disposition
			// so additions to Call cannot silently inherit a fallback.
			hir::Call::Var { .. } | hir::Call::Associated { .. } => {
				return Err(Error::new(ErrorKind::InvalidCode, "call disposition"));
			}
		};

		if hash == Hash::type_hash(crate::item!(::std::io::println)) && !self.stdio_println {
			// Host IO policy (capture, disable, custom or none): explicit compile-time
			// refusal before any fresh execution or output; the caller decides fallback.
			return Err(self.refused(span, Refusal::Native));
		}

		if hash == Hash::type_hash(crate::item!(::std::io::println))
			|| hash == Hash::type_hash(crate::item!(::std::mem::drop))
		{
			if call.args.len() != 1 {
				return Err(self.refused(span, Refusal::Arity));
			}

			self.effect_call(call, span)?;
			self.unit_into(out, span)?;
			return Ok(());
		}

		if hash == Hash::type_hash(crate::item!(::std::clone::clone)) {
			if call.args.len() != 1 {
				return Err(self.refused(span, Refusal::Arity));
			}

			let value = self.operand(call.args[0])?;
			self.emit(Op::Clone { out, value }, span)?;
			return Ok(());
		}

		let at = self.location(span);
		let function = (self.resolve)(hash, at).map_err(|e| e.at(self.location(span)))?;
		let args = self.operands(call.args)?;
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

pub(crate) fn binary_op(op: BinOp) -> Option<Binary> {
	Some(match op {
		BinOp::Add(..) | BinOp::AddAssign(..) => Binary::Add,
		BinOp::Sub(..) | BinOp::SubAssign(..) => Binary::Sub,
		BinOp::Mul(..) | BinOp::MulAssign(..) => Binary::Mul,
		BinOp::Div(..) | BinOp::DivAssign(..) => Binary::Div,
		BinOp::Rem(..) | BinOp::RemAssign(..) => Binary::Rem,
		BinOp::Eq(..) => Binary::Eq,
		BinOp::Neq(..) => Binary::Ne,
		BinOp::Lt(..) => Binary::Lt,
		BinOp::Lte(..) => Binary::Le,
		BinOp::Gt(..) => Binary::Gt,
		BinOp::Gte(..) => Binary::Ge,
		BinOp::BitAnd(..) | BinOp::BitAndAssign(..) => Binary::BitAnd,
		BinOp::BitOr(..) | BinOp::BitOrAssign(..) => Binary::BitOr,
		BinOp::BitXor(..) | BinOp::BitXorAssign(..) => Binary::BitXor,
		BinOp::Shl(..) | BinOp::ShlAssign(..) => Binary::Shl,
		BinOp::Shr(..) | BinOp::ShrAssign(..) => Binary::Shr,
		_ => return None,
	})
}

fn type_tag(hash: Hash) -> Option<Type> {
	use crate::TypeHash;
	Some(if hash == i64::HASH {
		Type::I64
	} else if hash == u64::HASH {
		Type::U64
	} else if hash == f64::HASH {
		Type::F64
	} else if hash == bool::HASH {
		Type::Bool
	} else if hash == char::HASH {
		Type::Char
	} else if hash == <()>::HASH {
		Type::Unit
	} else if hash == crate::alloc::String::HASH {
		Type::String
	} else if hash == crate::runtime::Vec::HASH {
		Type::Vec
	} else if hash == crate::runtime::OwnedTuple::HASH {
		Type::Tuple
	} else {
		return None;
	})
}
