//! Resolved-HIR to independent instructions. No legacy VM assembly or execution.
//! Construction subset: unsupported forms fail with their original source span.
//!
//! Slot discipline (A1): the frame is a stack. Every expression allocates its
//! result slot before its operands, so when it finishes the operands lie above
//! the result and are popped. A slot is "dirty" after an instruction that may
//! have stored a heap handle in it; popping emits one Release per dirty slot and
//! nothing for the others. Bare locals are read in place where the reference's
//! `writes_nothing` rule allows; capturing move closures transfer payload ownership and track moved bindings.

mod dynamic_methods;
pub(super) use dynamic_methods::native_function as native_method_function;
mod iter_terminals;
mod missing_methods;
mod number_methods;
mod object_methods;
mod patterns;
mod self_receiver;
mod static_methods;
mod text_methods;
mod text_pipeline;
mod variant_methods;
mod vec_constructors;
mod vec_resize;
#[derive(Clone, Copy)]
pub(crate) enum Resolve {
	Script(Hash),
	Constructor {
		hash: Hash,
		tag: Type,
		arity: u32,
	},
	NativeMethod {
		hash: Hash,
		intrinsic: u32,
		arity: u32,
	},
	Sort,
	Any,
}
pub(super) mod small_forms;
pub(super) mod sorting;

/// Constructor values use ordinary functions, so CallValue retains its existing
/// arity, fuel, frame, ownership and error-site behavior.
pub(crate) fn constructor_function(tag: Type, arity: u32) -> Result<Function> {
	let slots = arity
		.checked_add(1)
		.ok_or_else(|| Error::refused(Refusal::Arity))?;
	let mut function = Function::new(slots, arity)?;
	let mut operands = Vec::try_with_capacity(arity as usize)?;
	for slot in 0..arity {
		operands.try_push(slot)?;
	}
	let (args_start, args_len) = function.arguments(&operands)?;
	let at = super::code::TRANSPARENT_SORT_LOCATION;
	function.push(
		Op::List {
			out: arity,
			tag,
			args_start,
			args_len,
		},
		at,
	)?;
	function.push(Op::Return { value: arity }, at)?;
	Ok(function)
}

use super::arithmetic::{Binary, Unary};
use super::code::{CloneMode, CompileTransaction, Function, Op, PRINT_RAW};
use super::error::{Error, ErrorKind, Location, Refusal, Result};
use super::value::{Type, Value};
use crate as rune;
use crate::alloc::{String, Vec};
use crate::ast::{BinOp, Span, UnOp};
use crate::{hir, Hash};

pub(crate) fn function(
	exprs: &hir::Exprs<'_>,
	item: &hir::ItemFn<'_>,
	captures: &[hir::Variable],
	source: u32,
	revision: u64,
	resolve: &mut dyn FnMut(Resolve, Location) -> Result<u32>,
	constants: &crate::query::QueryInner<'_>,
	pool: &crate::compile::Pool,
	context: &crate::Context,
	transaction: &mut CompileTransaction,
	moved_error: &core::cell::Cell<Option<(Span, Span)>>,
	self_receiver: Option<Hash>,
	self_proof: &core::cell::Cell<Option<Location>>,
	proofs: &mut super::frontend::receiver_proofs::Proofs,
	body_hash: Hash,
) -> Result<Function> {
	let mut cx = Lower {
		self_binding: self_receiver.and_then(|receiver| {
			self_receiver::binding(exprs, item).map(|binding| (binding, receiver))
		}),
		self_proof,
		proofs,
		body_hash,
		exprs,
		function: Function::new(1, 0)?,
		variables: Vec::new(),
		moved: Vec::new(),
		moved_error,
		text_bindings: Vec::new(),
		loops: Vec::new(),
		dirty: Vec::new(),
		format_specs: Vec::new(),
		next: 0,
		high: 0,
		depth: 0,
		source,
		revision,
		resolve,
		constants,
		context,
		pool,
		transaction,
	};
	for &variable in captures {
		let slot = cx.slot()?;
		cx.raise(slot);
		cx.variables.try_push((variable, slot))?;
	}

	for arg in item.args {
		let hir::FnArg::Pat(pat) = arg;
		let slot = cx.slot()?;

		// Arguments arrive retained by the caller and may be anything.
		cx.raise(slot);
		if matches!(
			pat.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		) {
			cx.bind(&pat.pat, slot)?;
		}
	}

	cx.function.parameters = cx.next;
	// Reserve every incoming argument before allocating extraction slots.
	// Destructuring consumes only its retained argument root.
	for (index, arg) in item.args.iter().enumerate() {
		let hir::FnArg::Pat(pat) = arg;
		if !matches!(
			pat.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		) {
			cx.pattern_local(pat, (captures.len() + index) as u32)?;
		}
	}
	cx.block(&item.body, Mode::Return)?;
	cx.function.slots = cx.high.max(1);
	for (descriptor, words) in core::mem::take(&mut cx.format_specs) {
		let (start, len) = cx.function.arguments(&words)?;
		if len != 6 {
			return Err(Error::new(ErrorKind::InvalidCode, "template fixup"));
		}
		*cx.function
			.args
			.get_mut(descriptor as usize)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "template fixup"))? = start
			.checked_add(2)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "template fixup"))?;
	}
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
	start: Option<u32>,
	/// Slots at or above this mark belong to the body and are released on break/continue.
	cleanup: u32,
	/// Where a `break value` is delivered, when the loop's value is used.
	out: Option<u32>,
	breaks: Vec<u32>,
}

struct Lower<'a, 'hir> {
	proofs: &'a mut super::frontend::receiver_proofs::Proofs,
	body_hash: Hash,
	self_binding: Option<(hir::Variable, Hash)>,
	self_proof: &'a core::cell::Cell<Option<Location>>,
	exprs: &'a hir::Exprs<'hir>,
	function: Function,
	variables: Vec<(hir::Variable, u32)>,
	moved: Vec<(hir::Variable, Span)>,
	moved_error: &'a core::cell::Cell<Option<(Span, Span)>>,
	text_bindings: Vec<(hir::Variable, hir::ExprId)>,
	loops: Vec<Loop>,
	/// Per slot: may currently hold a heap handle that needs a Release.
	dirty: Vec<bool>,
	/// Deferred descriptor fixups; all spec records are appended after executable arguments.
	format_specs: Vec<(u32, [u32; 6])>,
	next: u32,
	high: u32,
	depth: usize,
	source: u32,
	revision: u64,
	resolve: &'a mut dyn FnMut(Resolve, Location) -> Result<u32>,
	constants: &'a crate::query::QueryInner<'hir>,
	/// The Context's `println` is the certified stdio builtin; otherwise refuse.
	context: &'a crate::Context,
	pool: &'a crate::compile::Pool,
	transaction: &'a mut CompileTransaction,
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
	/// before it was popped, so a new allocation starts clean.
	fn slot(&mut self) -> Result<u32> {
		if self.next >= 65_536 {
			return Err(Error::refused(Refusal::CodeLimit));
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
			Op::Call { function, .. } if super::code::variant_test_type(function).is_some() => (),
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
			| Op::StringFrom { out, .. }
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
			| Op::FieldGetMut { out, .. }
			| Op::TupleField { out, .. }
			| Op::TupleFieldGetMut { out, .. }
			| Op::Call { out, .. }
			| Op::CallValue { out, .. }
			| Op::Try { out, .. } => self.raise(out),
			// pop_to clears its flag, release_from deliberately does not, and a
			// move closure clears the captured source after transferring ownership.
			Op::Release { .. } => (),
			Op::Jump { .. }
			| Op::Branch { .. }
			| Op::Return { .. }
			| Op::Set { .. }
			| Op::SetField { .. }
			| Op::FieldSetMut { .. }
			| Op::SetTupleField { .. }
			| Op::TupleFieldSetMut { .. }
			| Op::Push { .. }
			| Op::Drop { .. }
			| Op::Print { .. }
			| Op::Panic { .. } => (),
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
		if let Some((_, moved_at)) = self.moved.iter().find(|(v, _)| *v == variable) {
			self.moved_error.set(Some((span, *moved_at)));
			return Err(self.refused(span, Refusal::FrontendDiagnostic));
		}
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
			hir::PatKind::Path(hir::PatPathKind::Ident { variable, .. }) => {
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
			.rposition(|context| match label {
				Some(label) => context.label.as_deref() == Some(label),
				None => context.start.is_some(),
			})
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

	fn block(&mut self, block: &hir::Block<'hir>, mode: Mode) -> Result<()> {
		let Some(label) = block.label else {
			return self.block_body(block, mode);
		};

		let mark = self.next;
		let out = match mode {
			Mode::Into(out) => Some(out),
			Mode::Discard => None,
			Mode::Return => Some(self.slot()?),
		};
		self.loops.try_push(Loop {
			label: Some(String::try_from(label)?),
			start: None,
			cleanup: self.next,
			out,
			breaks: Vec::new(),
		})?;
		self.block_body(block, out.map_or(Mode::Discard, Mode::Into))?;
		let end = self.pc();
		let context = self
			.loops
			.pop()
			.ok_or_else(|| self.refused(block.span, Refusal::LoopControl))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		if let Mode::Return = mode {
			let value = out.ok_or_else(|| self.refused(block.span, Refusal::LoopControl))?;
			self.emit(Op::Return { value }, block.span)?;
			self.forget_to(mark);
		}
		Ok(())
	}

	fn block_body(&mut self, block: &hir::Block<'hir>, mode: Mode) -> Result<()> {
		let vars = self.variables.len();
		let mark = self.next;

		for statement in block.statements {
			match statement {
				hir::Stmt::Local(local) => {
					// The local owns its slot until the block ends; nothing can read
					// the slot before the initializer has written it.
					let slot = self.slot()?;
					self.expr_into(local.expr, slot)?;
					self.pattern_local(&local.pat, slot)?;
					if let hir::PatKind::Path(hir::PatPathKind::Ident { variable, .. }) =
						local.pat.pat.kind
					{
						self.text_bindings.try_push((*variable, local.expr))?;
					}
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

	/// Evaluate into a new slot, which is returned. Operand temporaries are
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
	/// behavior): a bare local is read when the store executes, after the
	/// right side; a non-trivial expression materializes in source order.
	fn assignment_operand(&mut self, id: hir::ExprId, _span: Span) -> Result<u32> {
		match self.exprs.get(id).kind {
			hir::ExprKind::Variable(variable) => self.variable(variable, self.exprs.get(id).span),
			_ => self.expr(id),
		}
	}

	/// `Ok(None)`: not a type this core knows. A script struct is staged locally
	/// until complete admission; table exhaustion is the `TypeTable` refusal.
	fn type_tag(&mut self, hash: Hash, span: Span) -> Result<Option<Type>> {
		if let Some(tag) = type_tag(hash) {
			return Ok(Some(tag));
		}

		if let Some(item) = self.constants.enum_item(hash) {
			if !self.context.permits_nominal_format(hash) {
				return Err(self.refused(span, Refusal::Native));
			}
			return self
				.transaction
				.stage(hash.into_inner(), format_args!("{}", self.pool.item(item)))
				.map(Type::Struct)
				.map(Some)
				.map_err(|e| e.at(self.location(span)));
		}

		if self.constants.struct_shape(hash).is_none() {
			return Ok(None);
		}

		self.struct_tag(hash, span).map(Some)
	}

	/// Identity is the type hash; presentation uses its real item path. The
	/// append-only table keeps the first published name for that identity.
	fn struct_tag(&mut self, hash: Hash, span: Span) -> Result<Type> {
		// Legacy Dynamic dispatch uses an enum's hash, never its variant hash.
		if self.constants.variant_shape(hash).is_some() {
			return self.variant_tag(hash, span);
		}
		if !self.context.permits_nominal_format(hash) {
			return Err(self.refused(span, Refusal::Native));
		}
		let shape = self
			.constants
			.struct_shape(hash)
			.ok_or_else(|| self.refused(span, Refusal::ObjectConstructor))?;
		let kind = match shape.fields {
			crate::compile::meta::Fields::Empty => super::nominal::Shape::Empty,
			crate::compile::meta::Fields::Unnamed(n) => super::nominal::Shape::Tuple(
				u32::try_from(*n).map_err(|_| self.refused(span, Refusal::Arity))?,
			),
			crate::compile::meta::Fields::Named(_) => super::nominal::Shape::Named,
		};
		self.transaction
			.stage_shape(
				hash.into_inner(),
				format_args!("{}", self.pool.item(shape.item)),
				kind,
			)
			.map(Type::Struct)
			.map_err(|e| e.at(self.location(span)))
	}

	fn variant_tag(&mut self, hash: Hash, span: Span) -> Result<Type> {
		let shape = self
			.constants
			.variant_shape(hash)
			.ok_or_else(|| self.refused(span, Refusal::Pattern))?;
		if !self.context.permits_nominal_format(shape.enum_hash) {
			return Err(self.refused(span, Refusal::Native));
		}
		let kind = match shape.fields {
			crate::compile::meta::Fields::Empty => super::nominal::Shape::Empty,
			crate::compile::meta::Fields::Unnamed(n) => super::nominal::Shape::Tuple(
				u32::try_from(*n).map_err(|_| self.refused(span, Refusal::Arity))?,
			),
			crate::compile::meta::Fields::Named(_) => super::nominal::Shape::Named,
		};
		let mut fields = Vec::new();
		if let crate::compile::meta::Fields::Named(named) = shape.fields {
			for f in named.fields.iter() {
				fields.try_push((f.position, f.name.as_ref()))?;
			}
			fields.sort_unstable_by_key(|f| f.0);
		}
		let mut names = Vec::new();
		for (_, name) in fields {
			names.try_push(name)?;
		}
		self.transaction
			.stage_variant(
				hash.into_inner(),
				format_args!("{}", self.pool.item(shape.item)),
				kind,
				shape.enum_hash.into_inner(),
				shape.ordinal,
				&names,
			)
			.map(Type::Struct)
			.map_err(|e| e.at(self.location(span)))
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
			E::Match(hir) => self.pattern_match(hir, None, span),
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
			hir::Call::Meta { hash } if hash == Hash::type_hash(crate::item!(::std::io::print)) => {
				let out = self.slot()?;
				self.print_call(call, out, span)?;
				Ok(true)
			}
			hir::Call::Meta { hash } if hash == Hash::type_hash(crate::item!(::std::panic)) => {
				if !self.context.panic() {
					return Err(self.refused(span, Refusal::Native));
				}
				if call.args.len() != 1 {
					return Err(self.refused(span, Refusal::Arity));
				}
				let value = self.operand(call.args[0])?;
				self.emit(Op::Panic { value }, span)?;
				Ok(true)
			}
			hir::Call::Associated { target, hash, .. }
				if hash == Hash::ident("push")
					&& call.args.len() == 1
					&& !self.constants.method_script.closed(hash) =>
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
				if !self.context.stdio_println() || !self.context.formatting() {
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

	fn print_call(&mut self, call: &hir::ExprCall<'_>, out: u32, span: Span) -> Result<()> {
		if !self.context.stdio_print() || !self.context.formatting() {
			return Err(self.refused(span, Refusal::Native));
		}
		if call.args.len() != 1 {
			return Err(self.refused(span, Refusal::Arity));
		}
		let value = self.operand(call.args[0])?;
		let (args_start, args_len) = self.function.arguments(&[value])?;
		self.emit(
			Op::Call {
				out,
				function: PRINT_RAW,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
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
				// Nontrivial target expressions materialize before index/RHS.
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
				match field.expr_field {
					hir::ExprField::Index(index) => {
						let index = self.field_index(index, span)?;
						self.emit(
							Op::TupleFieldGetMut {
								out: old,
								value: target,
								index,
							},
							span,
						)?;
					}
					hir::ExprField::Ident(key) => {
						let key = self.function.string(key)?;
						self.emit(
							Op::FieldGetMut {
								out: old,
								value: target,
								key,
							},
							span,
						)?;
					}
					hir::ExprField::IdentGenerics(..) => {
						return Err(self.refused(span, Refusal::Field));
					}
				}
				self.emit(
					Op::AssignBinary {
						out: old,
						op,
						lhs: old,
						rhs,
					},
					span,
				)?;
				match field.expr_field {
					hir::ExprField::Index(index) => {
						let index = self.field_index(index, span)?;
						self.emit(
							Op::TupleFieldSetMut {
								value: target,
								index,
								new: old,
							},
							span,
						)?;
					}
					hir::ExprField::Ident(key) => {
						let key = self.function.string(key)?;
						self.emit(
							Op::FieldSetMut {
								value: target,
								key,
								new: old,
							},
							span,
						)?;
					}
					hir::ExprField::IdentGenerics(..) => {
						return Err(self.refused(span, Refusal::Field));
					}
				}
			}

			_ => return Err(self.refused(span, Refusal::Assignment)),
		}

		Ok(())
	}

	fn conditional(
		&mut self,
		conditional: &hir::Conditional<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		if conditional
			.branches
			.iter()
			.any(|branch| matches!(branch.condition, hir::Condition::ExprLet(_)))
		{
			return self.pattern_conditional(conditional, out, span);
		}
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
			let test = self.emit(Op::Jump { target: 0 }, self.exprs.get(*id).span)?;
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

	fn loop_(&mut self, loop_: &hir::ExprLoop<'hir>, out: Option<u32>, span: Span) -> Result<()> {
		if matches!(loop_.condition, Some(hir::Condition::ExprLet(_))) {
			return self.pattern_loop(loop_, out, span);
		}
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
			let pc = self.emit(Op::Jump { target: 0 }, self.exprs.get(*id).span)?;
			Some((value, pc))
		} else {
			None
		};

		self.loops.try_push(Loop {
			label: loop_.label.map(String::try_from).transpose()?,
			start: Some(start),
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

	fn for_(&mut self, for_: &hir::ExprFor<'hir>, out: Option<u32>, span: Span) -> Result<()> {
		if self.pipeline_is_origin(for_.iter) {
			return self.pipeline_for(for_, out, span);
		}
		if self.adapted(for_.iter) {
			return self.terminal_for(for_, out, span);
		}
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
			self.exprs.get(for_.iter).span,
		)?;
		let binding = self.slot()?;
		let destructure = !matches!(
			for_.binding.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		);
		if !destructure {
			self.bind(&for_.binding.pat, binding)?;
		}
		let start = self.pc();
		let header = self.emit(
			Op::IterNext {
				out: binding,
				iterator,
				none: 0,
			},
			span,
		)?;
		// Extraction runs only after a successful next. The root is released by
		// pattern_local; extracted bindings live only for this iteration.
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
		}
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
			E::Const(..) => self.inner_const(&expr, out),

			E::Type(..) => self.inner_type(&expr, out),

			E::Fn(..) => self.inner_fn(&expr, out),

			E::Group(id) => self.expr_into(id, out),
			E::Variable(..) => self.inner_variable(&expr, out),

			E::Lit(..) => self.inner_lit(&expr, out),

			E::Block(block) => self.block(block, Mode::Into(out)),
			E::Range(..) => self.inner_range(&expr, out),

			E::For(for_) => self.for_(for_, Some(out), span),
			E::Loop(loop_) => self.loop_(loop_, Some(out), span),
			E::Break(..) => self.inner_break(&expr, out),

			E::Continue(..) => self.inner_continue(&expr, out),

			E::Unary(..) => self.inner_unary(&expr, out),

			E::Binary(binary) => self.binary(binary, out, span),
			E::Object(..) => self.inner_object(&expr, out),

			E::FieldAccess(..) => self.inner_field_access(&expr, out),

			E::Assign(..) => self.inner_assign(&expr, out),

			E::Index(..) => self.inner_index(&expr, out),

			E::Template(..) => self.inner_template(&expr, out),

			E::Tuple(..) | E::Vec(..) => self.inner_sequence(&expr, out),

			E::Try(..) => self.inner_try(&expr, out),

			E::Return(..) => self.inner_return(&expr, out),

			E::Call(call) => self.call(call, out, span),
			E::If(conditional) => self.conditional(conditional, Some(out), span),
			E::Match(hir) => self.pattern_match(hir, Some(out), span),
			E::AsyncBlock(_) => Err(self.refused(span, Refusal::AsyncBlock)),
			E::Yield(_) => Err(self.refused(span, Refusal::Yield)),
			E::Await(_) => Err(self.refused(span, Refusal::Await)),
			E::Select(_) => Err(self.refused(span, Refusal::Select)),
			E::CallClosure(_) => self.inner_call_closure(&expr, out),

			E::Format(_) => Err(self.refused(span, Refusal::Format)),
			E::Static(_) => Err(self.refused(span, Refusal::Static)),
		}
	}

	#[cold]
	#[inline(never)]
	fn inner_call_closure(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::CallClosure(closure) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let at = self.location(span);
		let id = (self.resolve)(Resolve::Script(closure.hash), at)?;
		if !closure.captures.is_empty() {
			let mut captured = Vec::try_with_capacity(closure.captures.len())?;
			for &variable in closure.captures {
				let source = self.variable(variable, span)?;
				let value = if closure.do_move {
					let out = self.slot()?;
					let (args_start, args_len) = self.function.arguments(&[source])?;
					self.emit(
						Op::Call {
							out,
							function: super::code::CAPTURE_TAKE,
							args_start,
							args_len,
						},
						span,
					)?;
					self.moved.try_push((variable, span))?;
					out
				} else {
					source
				};
				captured.try_push(value)?;
			}
			let (capture_start, capture_len) = self.function.arguments(&captured)?;
			let captures = self.slot()?;
			self.emit(
				Op::List {
					out: captures,
					tag: Type::Tuple,
					args_start: capture_start,
					args_len: capture_len,
				},
				span,
			)?;
			let function = self.slot()?;
			self.emit(
				Op::Literal {
					out: function,
					value: Value::U64(u64::from(id)),
				},
				span,
			)?;
			let args = [function, captures];
			let (args_start, args_len) = self.function.arguments(&args)?;
			self.emit(
				Op::Call {
					out,
					function: super::code::CLOSURE_CREATE,
					args_start,
					args_len,
				},
				span,
			)?;
			return Ok(());
		}
		self.emit(
			Op::Literal {
				out,
				value: Value::Function { code: 0, id },
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn inner_const(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Const(hash) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let Some(constant) = self.constants.get_const_value(hash) else {
			let value = self
				.number_constant(hash)
				.ok_or_else(|| self.refused(span, Refusal::UnresolvedConstant))?;
			self.emit(Op::Literal { out, value }, span)?;
			return Ok(());
		};
		match super::constants::read(constant)? {
			super::constants::Scalar::Primitive(value) => {
				self.emit(Op::Literal { out, value }, span)?;
			}

			super::constants::Scalar::Text(value) => {
				let text = self.function.string(&value)?;
				self.emit(Op::String { out, text }, span)?;
			}
		}

		Ok(())
	}

	#[inline(never)]
	fn inner_type(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Type(tag) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let hash = tag.into_hash();
		let tag = match self.type_tag(hash, span)? {
			Some(tag) => tag,
			None => self
				.type_value_tag(hash, span)?
				.ok_or_else(|| self.refused(span, Refusal::TypeExpr))?,
		};
		self.emit(
			Op::Literal {
				out,
				value: Value::Type(tag),
			},
			span,
		)?;

		Ok(())
	}

	/// Type paths that are values but not `is`/`as` targets: a named variant,
	/// `std::any::Type` and the `std::ops` range types.
	///
	/// Stock keys a named-variant path such as `E::D` by the variant hash,
	/// which no value reports: a variant value's type is its enum. It
	/// therefore gets its own plain identity, never the variant's value
	/// identity (whose hash is the enum hash).
	#[cold]
	#[inline(never)]
	fn type_value_tag(&mut self, hash: Hash, span: Span) -> Result<Option<Type>> {
		use crate::runtime as rt;
		use crate::TypeHash;
		if hash == rt::Type::HASH {
			return self
				.metatype(span)
				.map(|nominal| Some(Type::Struct(nominal)));
		}
		for (range, tag) in [
			(rt::Range::HASH, Type::Range),
			(rt::RangeFrom::HASH, Type::RangeFrom),
			(rt::RangeFull::HASH, Type::RangeFull),
			(rt::RangeInclusive::HASH, Type::RangeInclusive),
			(rt::RangeTo::HASH, Type::RangeTo),
			(rt::RangeToInclusive::HASH, Type::RangeToInclusive),
		] {
			if hash == range {
				return Ok(Some(tag));
			}
		}
		let Some(shape) = self.constants.variant_shape(hash) else {
			return Ok(None);
		};
		self.transaction
			.stage(
				hash.into_inner(),
				format_args!("{}", self.pool.item(shape.item)),
			)
			.map(|nominal| Some(Type::Struct(nominal)))
			.map_err(|e| e.at(self.location(span)))
	}

	/// `std::any::Type`, the type of every type value. It is a nominal
	/// identity under the stock hash, so no hot `Type` variant exists for it.
	fn metatype(&mut self, span: Span) -> Result<super::value::Nominal> {
		use crate::TypeHash;
		self.transaction
			.stage(
				crate::runtime::Type::HASH.into_inner(),
				format_args!("::std::any::Type"),
			)
			.map_err(|e| e.at(self.location(span)))
	}

	/// `x is std::any::Type`. The hot `Is` arm answers false for every type
	/// value, so this lowers to `Type::of_val(x) == std::any::Type`.
	#[cold]
	#[inline(never)]
	fn is_metatype(&mut self, binary: &hir::ExprBinary, out: u32, span: Span) -> Result<()> {
		let value = self.operand(binary.lhs)?;
		let meta = self.slot()?;
		let nominal = self.metatype(span)?;
		self.emit(
			Op::Literal {
				out: meta,
				value: Value::Type(Type::Struct(nominal)),
			},
			span,
		)?;
		let of = self.slot()?;
		let (args_start, args_len) = self.function.arguments(&[value, meta])?;
		self.emit(
			Op::Call {
				out: of,
				function: super::code::ANY_TYPE_OF,
				args_start,
				args_len,
			},
			span,
		)?;
		let op = if matches!(binary.op, BinOp::IsNot(..)) {
			Binary::Ne
		} else {
			Binary::Eq
		};
		self.emit(
			Op::Binary {
				out,
				op,
				lhs: of,
				rhs: meta,
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn inner_fn(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Fn(hash) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let at = self.location(span);
		let resolve = match self.constants.struct_shape(hash) {
			Some(shape) => match shape.fields {
				crate::compile::meta::Fields::Unnamed(arity) if *arity > 0 => {
					let arity =
						u32::try_from(*arity).map_err(|_| self.refused(span, Refusal::Arity))?;
					Resolve::Constructor {
						hash,
						tag: self.struct_tag(hash, span)?,
						arity,
					}
				}
				_ => Resolve::Script(hash),
			},
			None => match self.constants.variant_shape(hash) {
				Some(shape) => match shape.fields {
					crate::compile::meta::Fields::Unnamed(arity) if *arity > 0 => {
						let arity = u32::try_from(*arity)
							.map_err(|_| self.refused(span, Refusal::Arity))?;
						Resolve::Constructor {
							hash,
							tag: self.variant_tag(hash, span)?,
							arity,
						}
					}
					_ => Resolve::Script(hash),
				},
				None => Resolve::Script(hash),
			},
		};
		let id = (self.resolve)(resolve, at)?;

		self.emit(
			Op::Literal {
				out,
				value: Value::Function { code: 0, id },
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn inner_variable(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Variable(variable) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let value = self.variable(variable, span)?;
		self.emit(Op::Copy { out, value }, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_lit(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Lit(lit) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
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

		Ok(())
	}

	#[inline(never)]
	fn inner_range(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Range(range) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let (start, end) = match range {
			hir::ExprRange::Range { start, end } => (start, end),
			other => return self.other_range(other, out, span),
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

		Ok(())
	}

	#[inline(never)]
	fn inner_break(&mut self, expr: &hir::Expr<'hir>, _out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Break(break_) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
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

		Ok(())
	}

	#[inline(never)]
	fn inner_continue(&mut self, expr: &hir::Expr<'hir>, _out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Continue(continue_) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let index = self.loop_index(continue_.label, span)?;
		let target = self.loops[index]
			.start
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))?;
		let cleanup = self.loops[index].cleanup;
		self.release_from(cleanup, span)?;
		self.emit(Op::Jump { target }, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_unary(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Unary(unary) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let op = match unary.op {
			UnOp::Neg(..) => Unary::Neg,
			UnOp::Not(..) => Unary::Not,
			_ => return Err(self.refused(span, Refusal::Operator)),
		};

		let value = self.operand(unary.expr)?;
		self.emit(Op::Unary { out, op, value }, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_object(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Object(object) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		// Check the complete declared shape before staging any identity.
		let order = if let hir::ExprObjectKind::Struct { hash } = object.kind {
			let shape = self
				.constants
				.value_shape(hash)
				.ok_or_else(|| self.refused(span, Refusal::ObjectConstructor))?;
			let crate::compile::meta::Fields::Named(fields) = shape.fields else {
				return Err(self.refused(span, Refusal::ObjectConstructor));
			};
			Some(
				declaration_order(&fields.fields, object.assignments.iter().map(|f| f.key.1))
					.map_err(|e| e.at(self.location(span)))?,
			)
		} else {
			None
		};

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

		// operands() snapshots in SOURCE order. Only the completed slot
		// pairs are permuted to declaration order for the record layout.
		for position in 0..keys.len() {
			let index = order.as_ref().map_or(position, |order| order[position]);
			fields.try_push(keys[index])?;
			fields.try_push(values[index])?;
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

		Ok(())
	}

	#[inline(never)]
	fn inner_field_access(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::FieldAccess(field) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let value = self.operand(field.expr)?;
		self.read_field(out, value, field.expr_field, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_assign(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Assign(assign) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		self.assign(assign, span)?;
		self.unit_into(out, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_index(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Index(index) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		// Stock defers a bare-local receiver until the index has run, as Set
		// and compound assignment already do. Computed targets run first.
		let target = self.assignment_operand(index.target, span)?;
		let key = self.operand(index.index)?;
		self.emit(
			Op::Get {
				out,
				value: target,
				index: key,
			},
			span,
		)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_template(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		if !self.context.formatting() {
			return Err(self.refused(expr.span, Refusal::Native));
		}
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Template(template) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		if template.exprs.len() > 4096 {
			return Err(self.refused(span, Refusal::FormatParts));
		}

		let mut ids = Vec::try_with_capacity(template.exprs.len())?;
		let mut descriptors_spec = Vec::try_with_capacity(template.exprs.len())?;
		let mut expansion_span = None;
		for id in template.exprs {
			let expr = self.exprs.get(*id);
			let (id, spec, span) = match expr.kind {
				hir::ExprKind::Format(format) => {
					let spec = template_spec(format.spec)
						.map_err(|_| self.refused(expr.span, Refusal::FormatSpec))?;
					(format.value, Some(spec), format.span)
				}
				hir::ExprKind::Lit(hir::Lit::Str(_)) => (*id, None, expr.span),
				_ => (*id, None, expr.span),
			};
			let display = match expr.kind {
				hir::ExprKind::Format(format) => {
					format.spec.format_type.is_none()
						|| format.spec.format_type == Some(crate::runtime::format::Type::Debug)
				}
				hir::ExprKind::Lit(hir::Lit::Str(_)) => false,
				_ => true,
			};
			if display {
				expansion_span = Some(expansion_span.map_or(span, |prev: Span| prev.join(span)));
			}
			ids.try_push(id)?;
			descriptors_spec.try_push(spec)?;
		}

		// Preserve argument effects: render only after all operands ran.
		let values = self.operands(&ids)?;
		let mut descriptors = Vec::try_with_capacity(ids.len() * 2)?;

		for (value, spec) in values.iter().zip(descriptors_spec.iter()) {
			descriptors.try_push(*value)?;
			descriptors.try_push(spec.and_then(|s| s.legacy()).unwrap_or(if spec.is_some() {
				2
			} else {
				0
			}))?;
		}
		let (args_start, args_len) = self.function.arguments(&descriptors)?;
		for (i, spec) in descriptors_spec.into_iter().enumerate() {
			if let Some(spec) = spec.filter(|s| s.legacy().is_none()) {
				let offset = u32::try_from(
					i.checked_mul(2)
						.and_then(|n| n.checked_add(1))
						.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "template fixup"))?,
				)
				.map_err(|_| Error::new(ErrorKind::InvalidCode, "template fixup"))?;
				self.format_specs.try_push((
					args_start
						.checked_add(offset)
						.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "template fixup"))?,
					spec.0,
				))?;
			}
		}

		self.emit(
			Op::Template {
				out,
				args_start,
				args_len,
			},
			expansion_span.unwrap_or(span),
		)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_sequence(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let (E::Tuple(sequence) | E::Vec(sequence)) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
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

		Ok(())
	}

	#[inline(never)]
	fn inner_try(&mut self, expr: &hir::Expr<'hir>, out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Try(value) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
		let value = self.operand(value)?;
		self.emit(Op::Try { out, value }, span)?;

		Ok(())
	}

	#[inline(never)]
	fn inner_return(&mut self, expr: &hir::Expr<'hir>, _out: u32) -> Result<()> {
		use hir::ExprKind as E;
		let span = expr.span;
		let E::Return(value) = expr.kind else {
			return Err(Error::new(ErrorKind::InvalidCode, "lowering arm"));
		};
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

		Ok(())
	}

	fn binary(&mut self, binary: &hir::ExprBinary, out: u32, span: Span) -> Result<()> {
		use hir::ExprKind as E;
		if matches!(binary.op, BinOp::As(..) | BinOp::Is(..) | BinOp::IsNot(..)) {
			let E::Type(tag) = self.exprs.get(binary.rhs).kind else {
				return Err(self.refused(span, Refusal::TypeExpr));
			};

			let hash = tag.into_hash();
			let Some(tag) = self.type_tag(hash, span)? else {
				if !matches!(binary.op, BinOp::As(..))
					&& hash == crate::hash_in!(crate, ::std::any::Type)
				{
					return self.is_metatype(binary, out, span);
				}
				return Err(self.refused(span, Refusal::TypeExpr));
			};
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
			// new slot, so the right side cannot observe it through a local.
			self.expr_into(binary.lhs, out)?;
			let branch = self.emit(Op::Jump { target: 0 }, self.exprs.get(binary.lhs).span)?;
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
				self.exprs.get(binary.rhs).span,
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

	#[inline(never)]
	fn call(&mut self, call: &hir::ExprCall<'_>, out: u32, span: Span) -> Result<()> {
		if let hir::Call::Meta { hash } = call.call {
			if ordering_constructor(hash).is_some() {
				return self.call_ordering_constructor(hash, call, out, span);
			}
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

			if self.tuple_struct_site(hash) {
				return self.call_tuple_struct(hash, call, out, span);
			}
			if self.constants.variant_shape(hash).is_some() {
				return self.call_tuple_variant(hash, call, out, span);
			}
		}

		if let Some(site) = self.chain_site(call) {
			return self.call_chain_site(call, site, out, span);
		}
		self.call_non_variant(call, out, span)
	}

	#[cold]
	#[inline(never)]
	fn call_ordering_constructor(
		&mut self,
		hash: Hash,
		call: &hir::ExprCall<'_>,
		out: u32,
		span: Span,
	) -> Result<()> {
		let value =
			ordering_constructor(hash).ok_or_else(|| self.refused(span, Refusal::Pattern))?;
		if !call.args.is_empty() {
			return Err(self.refused(span, Refusal::Arity));
		}
		self.emit(
			Op::Literal {
				out,
				value: Value::Ordering(value),
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn call_numeric_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		method: crate::function::numeric::Method,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.numeric_method(method) || self.constants.numeric_script & method.bit() != 0
		{
			return Err(self.refused(span, Refusal::Method));
		}
		if call.args.len() + 1 != method.arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
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
				function: crate::function::numeric::Call::method(method).id(),
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn call_numeric_direct(
		&mut self,
		call: &hir::ExprCall<'_>,
		pair: crate::function::numeric::Pair,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.numeric_direct(pair) {
			return Err(self.refused(span, Refusal::Native));
		}
		if call.args.len() != pair.method().arity() as usize {
			return Err(self.refused(span, Refusal::Arity));
		}
		let args = self.operands(call.args)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		self.emit(
			Op::Call {
				out,
				function: crate::function::numeric::Call::direct(pair).id(),
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}

	#[inline(never)]
	fn call_expression(
		&mut self,
		call: &hir::ExprCall<'_>,
		mut callee: hir::ExprId,
		out: u32,
		span: Span,
	) -> Result<()> {
		// Parentheses preserve stock's late read of a plain variable. Other
		// expression callees own a snapshot before argument evaluation.
		while let hir::ExprKind::Group(inner) = self.exprs.get(callee).kind {
			callee = inner;
		}
		let value = match self.exprs.get(callee).kind {
			hir::ExprKind::Variable(name) => self.variable(name, span)?,
			_ => self.expr(callee)?,
		};
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
		Ok(())
	}

	#[inline(never)]
	fn call_non_variant(&mut self, call: &hir::ExprCall<'_>, out: u32, span: Span) -> Result<()> {
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

		if let hir::Call::Expr { expr } = call.call {
			return self.call_expression(call, expr, out, span);
		}

		if let hir::Call::Associated {
			target,
			hash,
			name_hash,
		} = call.call
		{
			return self.call_associated(call, target, hash, name_hash, out, span);
		}

		let hash = match call.call {
			hir::Call::Meta { hash } => hash,
			hir::Call::ConstFn { .. } => return Err(self.refused(span, Refusal::ConstCall)),
			// These arms already returned above; retain an explicit disposition
			// so additions to Call cannot silently inherit a fallback.
			hir::Call::Var { .. } | hir::Call::Expr { .. } | hir::Call::Associated { .. } => {
				return Err(Error::new(ErrorKind::InvalidCode, "call disposition"));
			}
		};

		if let Some(with_capacity) = vec_constructors::constructor(hash) {
			return self.call_vec_constructor(call, with_capacity, out, span);
		}
		if crate::function::pipeline::any_kind(hash).is_some() {
			return self.call_any_value(hash, call, out, span);
		}
		if let Some(pair) = crate::function::numeric::Pair::from_item(hash) {
			return self.call_numeric_direct(call, pair, out, span);
		}
		if hash == Hash::type_hash(crate::item!(::std::string::String::from))
			|| hash
				== Hash::associated_function(
					Hash::type_hash(crate::item!(::std::string::String)),
					Hash::ident("from"),
				) {
			if !self.context.string_from() {
				return Err(self.refused(span, Refusal::Native));
			}
			if call.args.len() != 1 {
				return Err(self.refused(span, Refusal::Arity));
			}
			let value = self.operand(call.args[0])?;
			self.emit(Op::StringFrom { out, value }, span)?;
			return Ok(());
		}
		if hash == Hash::type_hash(crate::item!(::std::io::print)) {
			return self.print_call(call, out, span);
		}
		if hash == Hash::type_hash(crate::item!(::std::panic)) {
			self.effect_call(call, span)?;
			self.unit_into(out, span)?;
			return Ok(());
		}

		if hash == Hash::type_hash(crate::item!(::std::io::println))
			&& !self.context.stdio_println()
		{
			// Host IO policy (capture, disable, custom or none): explicit compile-time
			// refusal before any execution core execution or output; the caller decides fallback.
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
			self.emit(
				Op::Clone {
					out,
					value,
					mode: CloneMode::Global,
				},
				span,
			)?;
			return Ok(());
		}

		if let Some(binding) = self.context.host(hash) {
			if call.args.len() != usize::from(binding.signature.arity) {
				return Err(self.refused(span, Refusal::Arity));
			}
			let id = self
				.transaction
				.host(binding)
				.map_err(|e| e.at(self.location(span)))?;
			let args = self.operands(call.args)?;
			let (args_start, args_len) = self.function.arguments(&args)?;
			self.emit(
				Op::Call {
					out,
					function: id,
					args_start,
					args_len,
				},
				span,
			)?;
			return Ok(());
		}

		if let Some(pair) = crate::function::number::Pair::from_item(hash) {
			return self.call_number_path(call, pair, out, span);
		}

		let at = self.location(span);
		let function = (self.resolve)(Resolve::Script(hash), at).map_err(|e| e.at(at))?;
		let args = self.operands(call.args)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		let call = self.transaction.script_call(function, args_len, at)?;
		self.emit(call.op(out, args_start), span)?;
		Ok(())
	}

	/// Lowers a method call. Out of line so its arms stay out of the `call_non_variant`
	/// frame, which every nested plain call holds while its arguments lower.
	#[inline(never)]
	fn call_associated(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		hash: Hash,
		name_hash: Hash,
		out: u32,
		span: Span,
	) -> Result<()> {
		let before = (
			self.function.code.len(),
			self.function.args.len(),
			self.next,
			self.dirty.len(),
		);
		let original = self.call_associated_original(call, target, hash, name_hash, out, span);
		let Err(error) = original else {
			return original;
		};
		let at = self.location(span);
		// Only the original site's own Method refusal may use a new proof. An
		// operand's error, an admitted native path, and generic calls stay intact.
		if error.refusal != Some(Refusal::Method)
			|| error.location != Some(at)
			|| hash != name_hash
			|| self.proofs.disabled
			|| before
				!= (
					self.function.code.len(),
					self.function.args.len(),
					self.next,
					self.dirty.len(),
				) {
			return Err(error);
		}
		let Some(receiver) = self.proofs.expression(self.body_hash, target) else {
			return Err(error);
		};
		let methods = match self.proofs.methods(self.constants, self.pool, hash) {
			Err(failure) if failure.refusal == Some(Refusal::CodeLimit) => return Err(error),
			result => result?,
		};
		if methods.is_empty() {
			return Err(error);
		}
		let site = self.proofs.sites.len();
		self.proofs
			.sites
			.try_push(super::frontend::receiver_proofs::Site {
				diagnostic_checkpoint: self.proofs.diagnostic_checkpoint,
				diagnostic_owner: self.proofs.diagnostic_owner,
				diagnostic_span: self.proofs.diagnostic_span,
				body: self.body_hash,
				receiver,
				methods,
				at,
				instruction: None,
			})?;
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		ids.try_extend_from_slice(call.args)?;
		let values = self.operands(&ids)?;
		let (args_start, args_len) = self.function.arguments(&values)?;
		let pc = self.function.code.len() as u32;
		self.emit(
			Op::Call {
				out,
				function: 0,
				args_start,
				args_len,
			},
			span,
		)?;
		self.proofs.sites[site].instruction = Some((pc, out, args_start, args_len));
		Ok(())
	}

	fn call_associated_original(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		hash: Hash,
		name_hash: Hash,
		out: u32,
		span: Span,
	) -> Result<()> {
		if let Some(method) = self.static_method(target, hash, name_hash) {
			return self.call_static_method(call, target, method, out, span);
		}
		if self.nominal_missing_site(call) {
			return self.call_missing_method(call, target, name_hash, hash, out, span);
		}
		if hash == name_hash
			&& self.constants.method_script.closed(hash)
			&& !self.proven_number_method(target, hash, call.args.len())
			&& self.transaction.supports_dynamic_method(
				self.context,
				self.constants,
				self.pool,
				hash,
			) {
			return self.call_dynamic_method(call, target, hash, out, span);
		}
		if let Some(method) = self.self_method(target, hash, name_hash) {
			self.self_proof.set(Some(self.location(span)));
			return self.call_static_method(call, target, method, out, span);
		}
		if hash == name_hash && hash == crate::function::clone::RESIZE_NAME {
			return self.call_vec_resize(call, target, out, span);
		}
		if let Some(method) = crate::function::text::Method::from_hash(hash) {
			return self.call_text_method(call, target, method, out, span);
		}
		if let Some(method) = crate::function::variant::Method::from_hash(hash) {
			return self.call_variant_method(call, target, method, out, span);
		}
		if let Some(method) = crate::function::object::Method::from_hash(hash) {
			return self.call_object_method(call, target, method, out, span);
		}
		if let Some(method) = crate::function::number::Method::from_hash(name_hash) {
			return self.call_number_method(call, target, method, hash != name_hash, out, span);
		}
		if name_hash == Hash::ident("collect") {
			return self.call_text_collect(call, target, hash, out, span);
		}
		if let Some(name) = crate::function::iter::Name::ALL
			.into_iter()
			.find(|name| hash == name.hash())
		{
			return self.call_iterator_method(call, target, name, out, span);
		}
		if hash == Hash::ident("sort_by") {
			return self.call_sort_by(call, target, out, span);
		}
		if let Some(method) = crate::function::numeric::Method::from_hash(hash) {
			return self.call_numeric_method(call, target, method, out, span);
		}
		// A script method of the same name may be the target, so the native
		// forms below apply only to names no script impl defines.
		if self.constants.method_script.closed(hash) {
			if hash == name_hash {
				return self.call_dynamic_method(call, target, hash, out, span);
			}
			return self.call_missing_method(call, target, name_hash, hash, out, span);
		}
		if hash == Hash::ident("len") && call.args.is_empty() {
			let value = self.operand(target)?;
			self.emit(Op::Len { out, value }, span)?;
		} else if hash == Hash::ident("clone") && call.args.is_empty() {
			let value = self.operand(target)?;
			self.emit(
				Op::Clone {
					out,
					value,
					mode: CloneMode::Associated,
				},
				span,
			)?;
		} else if hash == Hash::ident("push") && call.args.len() == 1 {
			self.effect_call(call, span)?;
			self.unit_into(out, span)?;
		} else {
			return self.call_missing_method(call, target, name_hash, hash, out, span);
		}

		Ok(())
	}

	#[cold]
	#[inline(never)]
	fn tuple_struct_site(&self, hash: Hash) -> bool {
		self.constants.struct_shape(hash).is_some_and(|shape| {
			matches!(
				shape.fields,
				crate::compile::meta::Fields::Unnamed(_) | crate::compile::meta::Fields::Empty
			)
		})
	}

	#[cold]
	#[inline(never)]
	fn call_tuple_variant(
		&mut self,
		hash: Hash,
		call: &hir::ExprCall<'_>,
		out: u32,
		span: Span,
	) -> Result<()> {
		let shape = self
			.constants
			.variant_shape(hash)
			.ok_or_else(|| self.refused(span, Refusal::ObjectConstructor))?;
		let arity = match shape.fields {
			crate::compile::meta::Fields::Empty => 0,
			crate::compile::meta::Fields::Unnamed(n) => *n,
			_ => return Err(self.refused(span, Refusal::ObjectConstructor)),
		};
		if call.args.len() != arity {
			return Err(self.refused(span, Refusal::Arity));
		}
		let tag = self.variant_tag(hash, span)?;
		let values = self.operands(call.args)?;
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
		Ok(())
	}

	// Script unit and tuple constructors share existing owned List storage.
	#[cold]
	#[inline(never)]
	fn call_tuple_struct(
		&mut self,
		hash: Hash,
		call: &hir::ExprCall<'_>,
		out: u32,
		span: Span,
	) -> Result<()> {
		let shape = self
			.constants
			.struct_shape(hash)
			.ok_or_else(|| self.refused(span, Refusal::ObjectConstructor))?;
		let arity = match shape.fields {
			crate::compile::meta::Fields::Empty => 0,
			crate::compile::meta::Fields::Unnamed(n) => *n,
			_ => return Err(self.refused(span, Refusal::ObjectConstructor)),
		};
		if call.args.len() != arity {
			return Err(self.refused(span, Refusal::Arity));
		}
		let tag = self.struct_tag(hash, span)?;
		let values = self.operands(call.args)?;
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
		Ok(())
	}
	#[cold]
	#[inline(never)]
	fn call_any_value(
		&mut self,
		hash: Hash,
		call: &hir::ExprCall<'_>,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.any_function(hash) {
			return Err(self.refused(span, Refusal::Native));
		}
		if hash == crate::function::pipeline::TYPE_NAME {
			self.check_type_names(span)?;
		}
		if call.args.len() != 1 {
			return Err(self.refused(span, Refusal::Arity));
		}
		let mut values = self.operands(call.args)?;
		let function = if crate::function::pipeline::any_kind(hash) == Some(true) {
			let nominal = self.metatype(span)?;
			let meta = self.slot()?;
			self.emit(
				Op::Literal {
					out: meta,
					value: Value::Type(Type::Struct(nominal)),
				},
				span,
			)?;
			values.try_push(meta)?;
			super::code::ANY_TYPE_OF
		} else {
			super::code::ANY_TYPE_NAME
		};
		let (args_start, args_len) = self.function.arguments(&values)?;
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
	#[cold]
	#[inline(never)]
	fn check_type_names(&self, span: Span) -> Result<()> {
		use super::error::TypeName as N;
		for name in [
			N::Unit,
			N::Bool,
			N::I64,
			N::U64,
			N::F64,
			N::Char,
			N::Ordering,
			N::String,
			N::Vec,
			N::Tuple,
			N::Object,
			N::Option,
			N::Result,
			N::Range,
			N::RangeFrom,
			N::RangeFull,
			N::RangeInclusive,
			N::RangeToInclusive,
			N::RangeTo,
			N::Function,
			N::Type,
			N::VecIter,
			N::ObjectIter,
			N::OptionIter,
			N::StringSplit,
			N::CharSplit,
			N::Chars,
			N::Skip,
			N::Take,
			N::RangeIter,
			N::UnsignedRangeIter,
			N::CharRangeIter,
			N::RangeInclusiveIter,
			N::UnsignedRangeInclusiveIter,
			N::CharRangeInclusiveIter,
			N::RangeFromIter,
			N::UnsignedRangeFromIter,
			N::CharRangeFromIter,
		] {
			let hash = name
				.builtin_hash()
				.ok_or_else(|| self.refused(span, Refusal::Native))?;
			let key = Hash::associated_function(hash, &crate::runtime::Protocol::INTO_TYPE_NAME);
			let text = format!("::{name}");
			let expected = text
				.split('<')
				.next()
				.ok_or_else(|| self.refused(span, Refusal::Native))?;
			if self
				.context
				.get_const_value(key)
				.and_then(|v| v.as_string().ok())
				!= Some(expected)
			{
				return Err(self.refused(span, Refusal::Native));
			}
		}
		for item in self.constants.items.keys() {
			let hash = self.pool.item_type_hash(*item);
			if let Some(shape) = self.constants.struct_shape(hash) {
				let key =
					Hash::associated_function(hash, &crate::runtime::Protocol::INTO_TYPE_NAME);
				if let Some(v) = self.context.get_const_value(key) {
					let expected = format!("{}", self.pool.item(shape.item));
					if v.as_string().ok() != Some(expected.as_str()) {
						return Err(self.refused(span, Refusal::Native));
					}
				}
			}
		}
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
	} else if hash == core::cmp::Ordering::HASH {
		Type::Ordering
	} else if hash == char::HASH {
		Type::Char
	} else if hash == <()>::HASH {
		Type::Unit
	} else if hash == crate::alloc::String::HASH {
		Type::String
	} else if hash == <crate::runtime::Vec as crate::TypeHash>::HASH {
		Type::Vec
	} else if hash == crate::runtime::OwnedTuple::HASH {
		Type::Tuple
	} else if hash == <Option<crate::runtime::Value> as crate::TypeHash>::HASH { Type::Option
	} else if hash == <core::result::Result<crate::runtime::Value, crate::runtime::Value> as crate::TypeHash>::HASH { Type::Result
	} else if hash == crate::runtime::Object::HASH { Type::Object
	} else if hash == crate::runtime::Function::HASH {
		Type::Function
	} else {
		return None;
	})
}

/// Validate positions and the literal's exact key set independently of HIR.
fn declaration_order<'a>(
	fields: &[crate::compile::meta::FieldMeta],
	keys: impl Iterator<Item = &'a str> + Clone,
) -> Result<Vec<usize>> {
	let refused = || Error::refused(Refusal::ObjectConstructor);
	if keys.clone().count() != fields.len() {
		return Err(refused());
	}
	let mut order = Vec::try_with_capacity(fields.len())?;
	order.try_resize(fields.len(), usize::MAX)?;
	for field in fields {
		if field.position >= order.len() || order[field.position] != usize::MAX {
			return Err(refused());
		}
		let index = keys
			.clone()
			.position(|key| key == field.name.as_ref())
			.ok_or_else(refused)?;
		if keys
			.clone()
			.filter(|key| *key == field.name.as_ref())
			.count() != 1
			|| order.contains(&index)
		{
			return Err(refused());
		}
		order[field.position] = index;
	}
	Ok(order)
}

#[cfg(test)]
mod shape_tests {
	use super::*;
	use crate::compile::meta::FieldMeta;
	fn field(name: &str, position: usize) -> FieldMeta {
		FieldMeta {
			name: crate::alloc::Box::<str>::try_from(name).unwrap(),
			position,
		}
	}
	#[test]
	fn declaration_positions_and_exact_key_sets_are_validated() {
		let fields = [field("x", 0), field("y", 1)];
		assert_eq!(
			declaration_order(&fields, ["y", "x"].into_iter()).unwrap(),
			[1, 0]
		);
		for keys in [["x", "x"], ["x", "z"]] {
			assert_eq!(
				declaration_order(&fields, keys.into_iter())
					.unwrap_err()
					.refusal,
				Some(Refusal::ObjectConstructor)
			);
		}
		assert!(declaration_order(&fields, ["x"].into_iter()).is_err());
		for fields in [
			[field("x", 0), field("y", 0)],
			[field("x", 0), field("y", 2)],
			[field("x", 0), field("x", 1)],
		] {
			assert!(declaration_order(&fields, ["x", "y"].into_iter()).is_err());
		}
	}
}

fn template_spec(h: hir::BuiltInFormatSpec) -> Result<super::format_spec::Spec> {
	let runtime = crate::runtime::format::FormatSpec::new(
		h.flags.unwrap_or_default(),
		h.fill.unwrap_or(' '),
		h.align.unwrap_or_default(),
		h.width,
		h.precision,
		h.format_type.unwrap_or_default(),
	);
	super::format_spec::Spec::from_spec(runtime)
}

#[cfg(test)]
mod spec_tests {
	use super::*;

	#[test]
	fn hir_format_bounds_and_unknown_flags_are_rejected() {
		let good = hir::BuiltInFormatSpec {
			fill: None,
			align: None,
			width: None,
			precision: None,
			flags: None,
			format_type: None,
		};
		assert!(template_spec(good).is_ok());
		for spec in [
			hir::BuiltInFormatSpec {
				width: core::num::NonZeroUsize::new(65536),
				..good
			},
			hir::BuiltInFormatSpec {
				precision: Some(65536),
				..good
			},
			hir::BuiltInFormatSpec {
				flags: Some(crate::runtime::format::Flags::from(16)),
				..good
			},
		] {
			assert!(template_spec(spec).is_err());
		}
	}
}

// Fixed builtin identity, shared by constructors and empty-variant patterns.
fn ordering_constructor(hash: Hash) -> Option<core::cmp::Ordering> {
	Some(
		if hash == crate::hash_in!(crate, ::std::cmp::Ordering::Less) {
			core::cmp::Ordering::Less
		} else if hash == crate::hash_in!(crate, ::std::cmp::Ordering::Equal) {
			core::cmp::Ordering::Equal
		} else if hash == crate::hash_in!(crate, ::std::cmp::Ordering::Greater) {
			core::cmp::Ordering::Greater
		} else {
			return None;
		},
	)
}
