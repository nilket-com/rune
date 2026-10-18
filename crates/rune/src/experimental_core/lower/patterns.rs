//! C3: decision trees using existing instructions only.
use super::*;
use crate::TypeHash;

// A miss edge keeps its condition and orientation until the cleanup block exists.
type Miss = (u32, u32, bool);

enum Walk<'p, 'hir> {
	Node(&'p hir::Pat<'hir>, u32, usize),
	Index(&'p hir::Pat<'hir>, u32, usize, usize),
	Field(&'p hir::Binding<'hir>, u32, usize, bool),
}

impl<'a, 'hir> Lower<'a, 'hir> {
	#[cold]
	#[inline(never)]
	fn pattern_ordering_constructor(
		&mut self,
		value: u32,
		variant_hash: Hash,
		misses: &mut Vec<Miss>,
		span: Span,
	) -> Result<()> {
		let ordering = ordering_constructor(variant_hash)
			.ok_or_else(|| self.refused(span, Refusal::Pattern))?;
		self.pattern_is(value, Type::Ordering, misses, span)?;
		let rhs = self.slot()?;
		self.emit(
			Op::Literal {
				out: rhs,
				value: Value::Ordering(ordering),
			},
			span,
		)?;
		let condition = self.slot()?;
		self.emit(
			Op::Binary {
				out: condition,
				op: Binary::Eq,
				lhs: value,
				rhs,
			},
			span,
		)?;
		self.pattern_require(condition, true, misses, span)?;
		Ok(())
	}

	fn pattern_root(&mut self, id: hir::ExprId, refusal: Refusal) -> Result<u32> {
		use hir::ExprKind as E;
		let mut normalized = id;
		let mut groups = 0;
		while let E::Group(inner) = self.exprs.get(normalized).kind {
			groups += 1;
			if self.depth + groups >= 128 {
				return Err(self.refused(self.exprs.get(id).span, Refusal::DepthLimit));
			}
			normalized = inner;
		}
		match self.exprs.get(normalized).kind {
			E::Variable(variable) => self.variable(variable, self.exprs.get(id).span),
			E::Block(_)
			| E::Lit(_)
			| E::Call(_)
			| E::Object(_)
			| E::Tuple(_)
			| E::Vec(_)
			| E::Binary(_)
			| E::Unary(_)
			| E::FieldAccess(_)
			| E::Index(_) => self.expr(id),
			_ => Err(self.refused(self.exprs.get(id).span, refusal)),
		}
	}

	fn pattern_slots(&mut self, count: usize) -> Result<Vec<u32>> {
		let mut slots = Vec::try_with_capacity(count)?;
		for _ in 0..count {
			slots.try_push(self.slot()?)?;
		}
		Ok(slots)
	}

	fn pattern_dirty(&self, mark: u32) -> Result<Vec<u32>> {
		let mut slots = Vec::new();
		for slot in (mark..self.next).rev() {
			if self.dirty[slot as usize] {
				slots.try_push(slot)?;
			}
		}
		Ok(slots)
	}

	// Alternate-path cleanup must not mutate the fallthrough's compiler facts.
	fn pattern_release(&mut self, slots: &[u32], span: Span) -> Result<()> {
		for &value in slots {
			self.function
				.push(Op::Release { value }, self.location(span))?;
		}
		Ok(())
	}

	fn pattern_require(
		&mut self,
		condition: u32,
		pass: bool,
		misses: &mut Vec<Miss>,
		span: Span,
	) -> Result<()> {
		let next = self.pc() + 1;
		let pc = self.emit(
			Op::Branch {
				condition,
				yes: if pass { next } else { 0 },
				no: if pass { 0 } else { next },
			},
			span,
		)?;
		misses.try_push((pc, condition, pass))?;
		Ok(())
	}

	fn pattern_patch(&mut self, misses: &[Miss], target: u32) -> Result<()> {
		for &(pc, condition, pass) in misses {
			self.function.patch(
				pc,
				Op::Branch {
					condition,
					yes: if pass { pc + 1 } else { target },
					no: if pass { target } else { pc + 1 },
				},
			)?;
		}
		Ok(())
	}

	fn pattern_is(
		&mut self,
		value: u32,
		tag: Type,
		misses: &mut Vec<Miss>,
		span: Span,
	) -> Result<()> {
		let out = self.slot()?;
		self.emit(
			Op::Is {
				out,
				value,
				tag,
				negate: false,
			},
			span,
		)?;
		self.pattern_require(out, true, misses, span)
	}

	fn pattern_name(
		&mut self,
		variable: hir::Variable,
		value: u32,
		names: &[hir::Variable],
		slots: &[u32],
		span: Span,
	) -> Result<()> {
		let index = names
			.iter()
			.position(|name| *name == variable)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "pattern binding"))?;
		let out = *slots
			.get(index)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "pattern binding slot"))?;
		self.emit(Op::Copy { out, value }, span)?;
		Ok(())
	}

	fn pattern_kind(
		&mut self,
		kind: hir::PatSequenceKind,
		value: u32,
		misses: &mut Vec<Miss>,
		span: Span,
	) -> Result<()> {
		match kind {
			hir::PatSequenceKind::Sequence {
				hash,
				count,
				is_open,
			} => {
				let tuple = hash == crate::runtime::OwnedTuple::HASH;
				if !tuple
					&& hash != crate::runtime::Vec::HASH
					&& hash != crate::runtime::Object::HASH
				{
					return Err(self.refused(span, Refusal::Pattern));
				}
				let unit = if tuple && count == 0 {
					let condition = self.slot()?;
					self.emit(
						Op::Is {
							out: condition,
							value,
							tag: Type::Unit,
							negate: false,
						},
						span,
					)?;
					let pc = self.emit(
						Op::Branch {
							condition,
							yes: 0,
							no: self.pc() + 1,
						},
						span,
					)?;
					Some((pc, condition))
				} else {
					None
				};
				self.pattern_is(
					value,
					if tuple {
						Type::Tuple
					} else if hash == crate::runtime::Object::HASH {
						Type::Object
					} else {
						Type::Vec
					},
					misses,
					span,
				)?;
				let length = self.slot()?;
				self.emit(Op::Len { out: length, value }, span)?;
				// Even an open zero-prefix pattern must perform the Len access.
				if count != 0 || !is_open {
					let bound = self.slot()?;
					let count = u64::try_from(count)
						.map_err(|_| Error::new(ErrorKind::FrameCapacity, "pattern length"))?;
					self.emit(
						Op::Literal {
							out: bound,
							value: Value::U64(count),
						},
						span,
					)?;
					let condition = self.slot()?;
					self.emit(
						Op::Binary {
							out: condition,
							op: if is_open { Binary::Ge } else { Binary::Eq },
							lhs: length,
							rhs: bound,
						},
						span,
					)?;
					self.pattern_require(condition, true, misses, span)?;
				}
				if let Some((pc, condition)) = unit {
					self.function.patch(
						pc,
						Op::Branch {
							condition,
							yes: self.pc(),
							no: pc + 1,
						},
					)?;
				}
			}
			hir::PatSequenceKind::Type { hash, variant_hash } => {
				if hash == crate::hash_in!(crate, ::std::cmp::Ordering) {
					return self.pattern_ordering_constructor(value, variant_hash, misses, span);
				}
				if hash == Hash::type_hash(crate::item!(::std::option::Option)) {
					let some =
						variant_hash == Hash::type_hash(crate::item!(::std::option::Option::Some));
					if !some
						&& variant_hash
							!= Hash::type_hash(crate::item!(::std::option::Option::None))
					{
						return Err(self.refused(span, Refusal::Pattern));
					}
					self.pattern_is(value, Type::Option, misses, span)?;
					let none = self.slot()?;
					self.emit(
						Op::Variant {
							out: none,
							tag: Type::Option,
							success: false,
							payload: None,
						},
						span,
					)?;
					let condition = self.slot()?;
					self.emit(
						Op::Binary {
							out: condition,
							op: Binary::Eq,
							lhs: value,
							rhs: none,
						},
						span,
					)?;
					self.pattern_require(condition, !some, misses, span)?;
				} else if hash == Hash::type_hash(crate::item!(::std::result::Result)) {
					let ok =
						variant_hash == Hash::type_hash(crate::item!(::std::result::Result::Ok));
					if !ok
						&& variant_hash != Hash::type_hash(crate::item!(::std::result::Result::Err))
					{
						return Err(self.refused(span, Refusal::Pattern));
					}
					self.pattern_is(value, Type::Result, misses, span)?;
					let condition = self.slot()?;
					let (args_start, args_len) = self.function.arguments(&[value])?;
					self.emit(
						Op::Call {
							out: condition,
							function: super::super::code::VARIANT_SUCCESS,
							args_start,
							args_len,
						},
						span,
					)?;
					self.pattern_require(condition, ok, misses, span)?;
				} else {
					if variant_hash != Hash::EMPTY {
						let shape = self
							.constants
							.fresh_variant_shape(variant_hash)
							.ok_or_else(|| self.refused(span, Refusal::Pattern))?;
						if shape.enum_hash != hash {
							return Err(self.refused(span, Refusal::Pattern));
						}
						let tag = self.variant_tag(variant_hash, span)?;
						let condition = self.slot()?;
						let function = self.function.variant_test(tag)?;
						let (args_start, args_len) = self.function.arguments(&[value])?;
						self.emit(
							Op::Call {
								out: condition,
								function,
								args_start,
								args_len,
							},
							span,
						)?;
						self.pattern_require(condition, true, misses, span)?;
						return Ok(());
					}
					let _shape = self
						.constants
						.fresh_struct_shape(hash)
						.ok_or_else(|| self.refused(span, Refusal::Pattern))?;
					let tag = self.struct_tag(hash, span)?;
					self.pattern_is(value, tag, misses, span)?;
				}
			}
		}
		Ok(())
	}

	fn pattern_tree(
		&mut self,
		binding: &hir::PatBinding<'hir>,
		value: u32,
		slots: &[u32],
	) -> Result<Vec<Miss>> {
		let mut work = Vec::new();
		work.try_push(Walk::Node(&binding.pat, value, 0))?;
		let mut misses = Vec::new();
		while let Some(task) = work.pop() {
			match task {
				Walk::Index(pat, parent, index, depth) => {
					let index = self.field_index(index, pat.span)?;
					let value = self.slot()?;
					self.emit(
						Op::TupleField {
							out: value,
							value: parent,
							index,
						},
						pat.span,
					)?;
					work.try_push(Walk::Node(pat, value, depth))?;
				}
				Walk::Field(field, parent, depth, anonymous) => {
					let (span, key) = match field {
						hir::Binding::Binding(_, key, pat) => (pat.span, *key),
						hir::Binding::Ident(span, key, _) => (*span, *key),
					};
					if self.depth + depth >= 128 {
						return Err(self.refused(span, Refusal::DepthLimit));
					}
					let key = self.function.string(key)?;
					if anonymous {
						let text = self.slot()?;
						self.emit(
							Op::String {
								out: text,
								text: key,
							},
							span,
						)?;
						let condition = self.slot()?;
						let (args_start, args_len) = self.function.arguments(&[parent, text])?;
						self.emit(
							Op::Call {
								out: condition,
								function: super::super::code::OBJECT_HAS_FIELD,
								args_start,
								args_len,
							},
							span,
						)?;
						self.pattern_require(condition, true, &mut misses, span)?;
					}
					let value = self.slot()?;
					self.emit(
						Op::Field {
							out: value,
							value: parent,
							key,
						},
						span,
					)?;
					match field {
						hir::Binding::Binding(_, _, pat) => {
							work.try_push(Walk::Node(pat, value, depth))?
						}
						hir::Binding::Ident(_, _, variable) => {
							self.pattern_name(*variable, value, binding.names, slots, span)?
						}
					}
				}
				Walk::Node(pat, value, depth) => {
					if self.depth + depth >= 128 {
						return Err(self.refused(pat.span, Refusal::DepthLimit));
					}
					match pat.kind {
						hir::PatKind::Ignore => (),
						hir::PatKind::Path(hir::PatPathKind::Ident { variable, .. }) => {
							self.pattern_name(*variable, value, binding.names, slots, pat.span)?
						}
						hir::PatKind::Path(hir::PatPathKind::Kind(kind)) => {
							self.pattern_kind(**kind, value, &mut misses, pat.span)?
						}
						hir::PatKind::Lit(id) => {
							let expr = *self.exprs.get(id);
							let hir::ExprKind::Lit(lit) = expr.kind else {
								return Err(self.refused(pat.span, Refusal::Pattern));
							};
							let tag = match lit {
								hir::Lit::Bool(_) => Type::Bool,
								hir::Lit::Unsigned(_) => Type::U64,
								hir::Lit::Signed(_) => Type::I64,
								hir::Lit::Char(_) => Type::Char,
								hir::Lit::Str(_) => Type::String,
								_ => return Err(self.refused(pat.span, Refusal::Pattern)),
							};
							self.pattern_is(value, tag, &mut misses, pat.span)?;
							if let hir::Lit::Bool(pass) = lit {
								self.pattern_require(value, pass, &mut misses, pat.span)?;
							} else {
								let constant = self.slot()?;
								self.inner_lit(&expr, constant)?;
								let condition = self.slot()?;
								self.emit(
									Op::Binary {
										out: condition,
										op: Binary::Eq,
										lhs: value,
										rhs: constant,
									},
									pat.span,
								)?;
								self.pattern_require(condition, true, &mut misses, pat.span)?;
							}
						}
						hir::PatKind::Sequence(sequence) => {
							if matches!(sequence.kind, hir::PatSequenceKind::Type { hash, .. } if hash == Hash::type_hash(crate::item!(::std::result::Result)))
								&& sequence.items.len() > 1
							{
								return Err(self.refused(pat.span, Refusal::Pattern));
							}
							self.pattern_kind(sequence.kind, value, &mut misses, pat.span)?;
							for (index, child) in sequence.items.iter().enumerate().rev() {
								work.try_push(Walk::Index(child, value, index, depth + 1))?;
							}
						}
						hir::PatKind::Object(object) => {
							let anonymous = matches!(object.kind, hir::PatSequenceKind::Sequence { hash, .. } if hash == crate::runtime::Object::HASH);

							self.pattern_kind(object.kind, value, &mut misses, pat.span)?;
							for field in object.bindings.iter().rev() {
								work.try_push(Walk::Field(field, value, depth + 1, anonymous))?;
							}
						}
					}
				}
			}
		}
		Ok(misses)
	}

	fn pattern_bindings(&mut self, binding: &hir::PatBinding<'hir>, slots: &[u32]) -> Result<()> {
		for (name, slot) in binding.names.iter().zip(slots) {
			self.variables.try_push((*name, *slot))?;
		}
		Ok(())
	}

	/// Local tuple and Vec bindings use the general pattern walker. The
	/// ordinary bind entry remains unchanged for parameters and for-loop bindings.
	#[inline(never)]
	pub(super) fn pattern_local(
		&mut self,
		binding: &hir::PatBinding<'hir>,
		root: u32,
	) -> Result<()> {
		if matches!(
			binding.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		) {
			return self.bind(&binding.pat, root);
		}
		self.local_tuple_shape(&binding.pat)?;
		let span = binding.pat.span;
		let linear = self.pattern_slots(binding.names.len())?;
		let temps = self.next;
		let misses = self.pattern_tree(binding, root, &linear)?;
		// Misses skip any later child extraction. Releasing a still-Unit slot
		// is safe, so preserve the complete potential ownership set now.
		let failure_cleanup = self.pattern_dirty(0)?;
		self.pop_to(temps, span)?;
		// The RHS slot is a temporary, even when initialized by a bare local.
		// Its children have their own retained binding slots on success.
		self.emit(Op::Release { value: root }, span)?;
		self.dirty[root as usize] = false;
		if !misses.is_empty() {
			let done = self.emit(Op::Jump { target: 0 }, span)?;
			self.pattern_patch(&misses, self.pc())?;
			// The miss terminates this frame. Free owned handles before the
			// existing String admission for the existing typed Panic operation.
			// Do not alter the successful path's compiler ownership facts.
			self.pattern_release(&failure_cleanup, span)?;
			let message = self.slot()?;
			let text = self.function.string("pattern did not match")?;
			self.emit(Op::String { out: message, text }, span)?;
			self.emit(Op::Panic { value: message }, span)?;
			self.forget_to(temps);
			self.function.patch(done, Op::Jump { target: self.pc() })?;
		}
		self.pattern_bindings(binding, &linear)
	}

	/// Validate local tuple and Vec shapes before the general match walker.
	/// Only Vec patterns can have a trailing nonbinding rest pattern here.
	#[inline(never)]
	fn local_tuple_shape(&self, root: &hir::Pat<'hir>) -> Result<()> {
		let mut work = Vec::new();
		let mut names = Vec::new();
		work.try_push((root, 0usize))?;
		while let Some((pat, depth)) = work.pop() {
			if self.depth + depth >= 128 {
				return Err(self.refused(pat.span, Refusal::DepthLimit));
			}
			match pat.kind {
				hir::PatKind::Ignore => (),
				hir::PatKind::Path(hir::PatPathKind::Ident {
					name: hir::Name::Str(name),
					..
				}) => {
					if names.contains(name) {
						return Err(self.refused(pat.span, Refusal::Pattern));
					}
					names.try_push(*name)?;
				}
				hir::PatKind::Sequence(sequence) => {
					let supported = match sequence.kind {
						hir::PatSequenceKind::Sequence { hash, is_open, .. } => {
							hash == crate::runtime::Vec::HASH
								|| (!is_open && hash == crate::runtime::OwnedTuple::HASH)
						}
						hir::PatSequenceKind::Type { hash, variant_hash }
							if variant_hash == Hash::EMPTY =>
						{
							self.constants
								.fresh_struct_shape(hash)
								.is_some_and(|shape| {
									matches!(
										shape.fields,
										crate::compile::meta::Fields::Empty
											| crate::compile::meta::Fields::Unnamed(_)
									)
								})
						}
						_ => false,
					};
					if !supported {
						return Err(self.refused(pat.span, Refusal::Pattern));
					}
					for child in sequence.items.iter().rev() {
						work.try_push((child, depth + 1))?;
					}
				}
				_ => return Err(self.refused(pat.span, Refusal::Pattern)),
			}
		}
		Ok(())
	}

	pub(super) fn pattern_match(
		&mut self,
		hir: &hir::ExprMatch<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let vars = self.variables.len();
		let mark = self.next;
		let root = self.pattern_root(hir.expr, Refusal::Match)?;
		let count = hir
			.branches
			.iter()
			.map(|arm| arm.pat.names.len())
			.max()
			.unwrap_or(0);
		let linear = self.pattern_slots(count)?;
		let mut ends = Vec::new();
		for arm in hir.branches {
			let arm_mark = self.next;
			let misses = self
				.pattern_tree(&arm.pat, root, &linear)
				.map_err(|e| self.pattern_refusal(e, Refusal::Match, span))?;
			let test_cleanup = self.pattern_dirty(arm_mark)?;
			self.pop_to(arm_mark, arm.pat.pat.span)?;
			self.pattern_bindings(&arm.pat, &linear)?;
			let guard = if let Some(id) = arm.condition {
				let guard_mark = self.next;
				let condition = self.operand(id)?;
				let pc = self.emit(Op::Jump { target: 0 }, self.exprs.get(id).span)?;
				let cleanup = self.pattern_dirty(guard_mark)?;
				let yes = self.pc();
				self.pop_to(guard_mark, self.exprs.get(id).span)?;
				Some((pc, condition, yes, cleanup, self.exprs.get(id).span))
			} else {
				None
			};
			if let Some(out) = out {
				self.expr_into(arm.body, out)?;
			} else {
				self.statement(arm.body)?;
			}
			ends.try_push(self.emit(Op::Jump { target: 0 }, span)?)?;
			self.variables.truncate(vars);
			if let Some((pc, condition, yes, cleanup, guard_span)) = guard {
				let no = self.pc();
				self.pattern_release(&cleanup, guard_span)?;
				let next = self.emit(Op::Jump { target: 0 }, guard_span)?;
				self.function.patch(pc, Op::Branch { condition, yes, no })?;
				let miss = self.pc();
				self.pattern_patch(&misses, miss)?;
				self.pattern_release(&test_cleanup, arm.pat.pat.span)?;
				self.function.patch(next, Op::Jump { target: self.pc() })?;
			} else {
				let miss = self.pc();
				self.pattern_patch(&misses, miss)?;
				self.pattern_release(&test_cleanup, arm.pat.pat.span)?;
			}
		}
		if let Some(out) = out {
			self.unit_into(out, span)?;
		}
		let end = self.pc();
		for pc in ends {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		self.pop_to(mark, span)?;
		self.variables.truncate(vars);
		Ok(())
	}

	fn pattern_refusal(&self, e: Error, refusal: Refusal, span: Span) -> Error {
		if e.refusal == Some(Refusal::Pattern) {
			self.refused(span, refusal)
		} else {
			e
		}
	}

	pub(super) fn pattern_conditional(
		&mut self,
		hir: &hir::Conditional<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let vars = self.variables.len();
		let mark = self.next;
		let count = hir
			.branches
			.iter()
			.filter_map(|branch| branch.condition.count())
			.max()
			.unwrap_or(0);
		let linear = self.pattern_slots(count)?;
		let mut ends = Vec::new();
		for branch in hir.branches {
			let temps = self.next;
			let (misses, cleanup) = match *branch.condition {
				hir::Condition::Expr(_, id) => {
					let condition = self.operand(id)?;
					let mut misses = Vec::new();
					self.pattern_require(condition, true, &mut misses, self.exprs.get(id).span)?;
					let cleanup = self.pattern_dirty(temps)?;
					self.pop_to(temps, span)?;
					(misses, cleanup)
				}
				hir::Condition::ExprLet(let_) => {
					let root = self.pattern_root(let_.expr, Refusal::Condition)?;
					let misses = self
						.pattern_tree(&let_.pat, root, &linear)
						.map_err(|e| self.pattern_refusal(e, Refusal::Condition, span))?;
					let cleanup = self.pattern_dirty(temps)?;
					self.pop_to(temps, let_.pat.pat.span)?;
					self.pattern_bindings(&let_.pat, &linear)?;
					(misses, cleanup)
				}
			};
			if let Some(out) = out {
				self.block(&branch.block, Mode::Into(out))?;
			} else {
				self.block(&branch.block, Mode::Discard)?;
			}
			ends.try_push(self.emit(Op::Jump { target: 0 }, span)?)?;
			self.variables.truncate(vars);
			let miss = self.pc();
			self.pattern_patch(&misses, miss)?;
			self.pattern_release(&cleanup, span)?;
		}
		match (hir.fallback, out) {
			(Some(block), Some(out)) => self.block(block, Mode::Into(out))?,
			(Some(block), None) => self.block(block, Mode::Discard)?,
			(None, Some(out)) => self.unit_into(out, span)?,
			(None, None) => (),
		}
		let end = self.pc();
		for pc in ends {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		self.pop_to(mark, span)?;
		self.variables.truncate(vars);
		Ok(())
	}

	pub(super) fn pattern_loop(
		&mut self,
		hir: &hir::ExprLoop<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let Some(hir::Condition::ExprLet(let_)) = hir.condition else {
			return Err(self.refused(span, Refusal::Condition));
		};
		if let Some(out) = out {
			self.unit_into(out, span)?;
		}
		let vars = self.variables.len();
		let mark = self.next;
		// Linear slots precede condition temporaries so each iteration can reuse them.
		let linear = self.pattern_slots(let_.pat.names.len())?;
		let start = self.pc();
		let root = self.pattern_root(let_.expr, Refusal::Condition)?;
		let temps = self.next;
		let misses = self
			.pattern_tree(&let_.pat, root, &linear)
			.map_err(|e| self.pattern_refusal(e, Refusal::Condition, span))?;
		let cleanup = self.pattern_dirty(temps)?;
		self.pop_to(temps, let_.pat.pat.span)?;
		self.pattern_bindings(&let_.pat, &linear)?;
		self.loops.try_push(Loop {
			label: hir.label.map(String::try_from).transpose()?,
			start,
			cleanup: self.next,
			out,
			breaks: Vec::new(),
		})?;
		self.block(&hir.body, Mode::Discard)?;
		self.emit(Op::Jump { target: start }, span)?;
		let miss = self.pc();
		self.pattern_patch(&misses, miss)?;
		self.pattern_release(&cleanup, let_.pat.pat.span)?;
		let end = self.pc();
		let context = self
			.loops
			.pop()
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "pattern loop"))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		self.variables.truncate(vars);
		self.pop_to(mark, span)?;
		Ok(())
	}
}
