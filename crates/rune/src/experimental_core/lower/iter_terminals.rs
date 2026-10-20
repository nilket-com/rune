//! C17d direct iterator chains: an `iter()` or `std::iter::range` source,
//! `skip`/`take` adapters and one consumer, expanded into ordinary charged
//! instructions. Adapters become counters in slots, so no iterator value
//! other than the source exists. A chain that is stored, passed or returned
//! is not direct and keeps its earlier refusal.
use super::*;
use crate::function::fresh_iter::Name;
use crate::function::iter_terminal::{Collection, Intrinsic, Number, Terminal, RANGE};

/// Chain recursion bound, as for the other origin proofs.
const DEPTH: usize = 128;

/// The innermost expression of a chain.
#[derive(Clone, Copy)]
enum Source {
	/// `receiver.iter()`, lowered as the certified `iter` call.
	Iter(hir::ExprId),
	/// A `std::iter::range(start, end)` call.
	Range(hir::ExprId),
}

/// A `skip(n)` or `take(n)` call around the chain so far.
#[derive(Clone, Copy)]
struct Adapter {
	take: bool,
	count: hir::ExprId,
	span: Span,
}

/// A source and its adapters, innermost first.
struct Chain {
	source: Source,
	adapters: Vec<Adapter>,
}

/// A call site with a cold chain lowering.
#[derive(Clone, Copy)]
pub(super) enum Site {
	/// A `std::iter::range(start, end)` value.
	Range,
	/// The consumer of a direct chain.
	Terminal(End, hir::ExprId),
	/// A C14b text pipeline method.
	Text(hir::ExprId, crate::function::fresh_pipeline::Method),
}

/// What consumes a chain at a call site.
#[derive(Clone, Copy)]
pub(super) enum End {
	Terminal(Terminal),
	/// `next()` after at least one adapter.
	Next,
}

/// An adapter's counter after setup.
#[derive(Clone, Copy)]
struct Layer {
	take: bool,
	count: u32,
}

/// The evaluated chain: the source iterator, the counters and the slots the
/// expansion shares.
struct Setup {
	node: u32,
	layers: Vec<Layer>,
	zero: u32,
	one: u32,
	condition: u32,
}

/// A jump to the chain's exhaustion target, patched once it is known.
#[derive(Clone, Copy)]
enum Exhausted {
	Next { pc: u32, out: u32, iterator: u32 },
	Branch { pc: u32, condition: u32, no: u32 },
}

impl<'hir> Lower<'_, 'hir> {
	/// The chain whose outermost call is `id`, if `id` is a direct chain.
	fn chain(&self, id: hir::ExprId) -> Option<Chain> {
		let mut adapters = Vec::new();
		let source = self.chain_into(id, 0, &mut adapters)?;
		Some(Chain { source, adapters })
	}

	fn chain_into(
		&self,
		id: hir::ExprId,
		depth: usize,
		adapters: &mut Vec<Adapter>,
	) -> Option<Source> {
		if depth >= DEPTH {
			return None;
		}
		use hir::ExprKind as E;
		let expr = self.exprs.get(id);
		match expr.kind {
			E::Group(inner) => self.chain_into(inner, depth + 1, adapters),
			E::Call(c) => match c.call {
				hir::Call::Associated { target, hash, .. }
					if hash == Terminal::Skip.hash() || hash == Terminal::Take.hash() =>
				{
					let &[count] = c.args else {
						return None;
					};
					let source = self.chain_into(target, depth + 1, adapters)?;
					adapters
						.try_push(Adapter {
							take: hash == Terminal::Take.hash(),
							count,
							span: expr.span,
						})
						.ok()?;
					Some(source)
				}
				hir::Call::Associated { hash, .. } if hash == Name::Iter.hash() => {
					c.args.is_empty().then_some(Source::Iter(id))
				}
				hir::Call::Meta { hash } if hash == RANGE => Some(Source::Range(id)),
				_ => None,
			},
			_ => None,
		}
	}

	/// Whether `id` is a direct chain with at least one adapter.
	pub(super) fn adapted(&self, id: hir::ExprId) -> bool {
		use hir::ExprKind as E;
		let mut id = id;
		for _ in 0..DEPTH {
			match self.exprs.get(id).kind {
				E::Group(inner) => id = inner,
				E::Call(c) => {
					return matches!(c.call, hir::Call::Associated { hash, .. }
						if hash == Terminal::Skip.hash() || hash == Terminal::Take.hash())
						&& self.chain(id).is_some();
				}
				_ => return false,
			}
		}
		false
	}

	/// The cold lowering a call site needs, if any. One query keeps the
	/// recursive `call` frame small.
	#[cold]
	#[inline(never)]
	pub(super) fn chain_site(&self, call: &hir::ExprCall<'_>) -> Option<Site> {
		if matches!(call.call, hir::Call::Meta { hash } if hash == RANGE) {
			return Some(Site::Range);
		}
		if let Some((end, target)) = self.terminal_site(call) {
			return Some(Site::Terminal(end, target));
		}
		self.pipeline_site(call)
			.map(|(target, method)| Site::Text(target, method))
	}

	#[cold]
	#[inline(never)]
	pub(super) fn call_chain_site(
		&mut self,
		call: &hir::ExprCall<'_>,
		site: Site,
		out: u32,
		span: Span,
	) -> Result<()> {
		match site {
			Site::Range => self.call_range(call, out, span),
			Site::Terminal(end, target) => self.call_terminal(call, end, target, out, span),
			Site::Text(target, method) => self.call_pipeline(call, target, method, out, span),
		}
	}

	/// The consumer and the chain at a method call site, if it ends a direct chain.
	fn terminal_site(&self, call: &hir::ExprCall<'_>) -> Option<(End, hir::ExprId)> {
		let hir::Call::Associated { target, hash, .. } = call.call else {
			return None;
		};
		let end = match Terminal::from_hash(hash) {
			Some(Terminal::Skip | Terminal::Take) => return None,
			Some(terminal) => End::Terminal(terminal),
			None if hash == Name::Next.hash() => End::Next,
			None => return None,
		};
		let direct = match end {
			End::Next => self.adapted(target),
			End::Terminal(_) => self.chain(target).is_some(),
		};
		direct.then_some((end, target))
	}

	/// Certifies every iterator function the chain calls.
	fn chain_proof(&mut self, chain: &Chain, end: End, span: Span) -> Result<()> {
		if let Source::Range(_) = chain.source {
			self.transaction
				.iter_range(self.context)
				.map_err(|e| e.at(self.location(span)))?;
		}
		for adapter in &chain.adapters {
			let terminal = if adapter.take {
				Terminal::Take
			} else {
				Terminal::Skip
			};
			self.transaction
				.iter_terminal(self.context, terminal)
				.map_err(|e| e.at(self.location(adapter.span)))?;
		}
		match end {
			End::Terminal(terminal) => {
				self.transaction
					.iter_terminal(self.context, terminal)
					.map_err(|e| e.at(self.location(span)))?;
			}
			// `next` and the for-loop NEXT protocol on Skip and Take.
			End::Next => self
				.transaction
				.pipeline_method(
					self.context,
					self.constants,
					self.pool,
					crate::function::fresh_pipeline::Method::Next,
				)
				.map_err(|e| e.at(self.location(span)))?,
		}
		Ok(())
	}

	fn intrinsic(
		&mut self,
		intrinsic: Intrinsic,
		args: &[u32],
		out: u32,
		span: Span,
	) -> Result<()> {
		let (args_start, args_len) = self.function.arguments(args)?;
		self.emit(
			Op::Call {
				out,
				function: intrinsic.id(),
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}

	/// `std::iter::range(start, end)` as a value: the same iterator as
	/// `(start..end).iter()`, after the stock `i64` argument conversions.
	#[cold]
	#[inline(never)]
	fn call_range(&mut self, call: &hir::ExprCall<'_>, out: u32, span: Span) -> Result<()> {
		self.transaction
			.iter_range(self.context)
			.map_err(|e| e.at(self.location(span)))?;
		if call.args.len() != 2 {
			return Err(self.refused(span, Refusal::Arity));
		}
		let args = self.operands(call.args)?;
		self.intrinsic(Intrinsic::Range, &args, out, span)
	}

	/// Evaluates the source, then each adapter's count in source order.
	fn chain_setup(&mut self, chain: &Chain, span: Span) -> Result<Setup> {
		let node = match chain.source {
			Source::Iter(id) => self.expr(id)?,
			Source::Range(id) => {
				let expr = *self.exprs.get(id);
				let hir::ExprKind::Call(call) = expr.kind else {
					return Err(Error::new(ErrorKind::InvalidCode, "range source"));
				};
				let node = self.slot()?;
				let mark = self.next;
				self.call_range(call, node, expr.span)?;
				self.pop_to(mark, expr.span)?;
				node
			}
		};
		let mut layers = Vec::try_with_capacity(chain.adapters.len())?;
		for adapter in &chain.adapters {
			let value = self.operand(adapter.count)?;
			let count = self.slot()?;
			self.intrinsic(Intrinsic::AdapterCount, &[value], count, adapter.span)?;
			layers.try_push(Layer {
				take: adapter.take,
				count,
			})?;
		}
		let zero = self.literal(Value::U64(0), span)?;
		let one = self.literal(Value::U64(1), span)?;
		let condition = self.slot()?;
		Ok(Setup {
			node,
			layers,
			zero,
			one,
			condition,
		})
	}

	fn literal(&mut self, value: Value, span: Span) -> Result<u32> {
		let out = self.slot()?;
		self.emit(Op::Literal { out, value }, span)?;
		Ok(out)
	}

	/// The next element of the chain's first `layers` adapters into `out`, or
	/// a jump recorded in `exhausted`. Each adapter behaves as its stock `next`:
	/// `take` stops without pulling once its count is spent; `skip` pulls and
	/// discards until its count is spent.
	fn chain_next(
		&mut self,
		setup: &Setup,
		layers: usize,
		out: u32,
		exhausted: &mut Vec<Exhausted>,
		span: Span,
	) -> Result<()> {
		let Some(index) = layers.checked_sub(1) else {
			let pc = self.emit(
				Op::IterNext {
					out,
					iterator: setup.node,
					none: 0,
				},
				span,
			)?;
			exhausted.try_push(Exhausted::Next {
				pc,
				out,
				iterator: setup.node,
			})?;
			return Ok(());
		};
		let layer = setup.layers[index];
		let condition = setup.condition;
		let spent = Op::Binary {
			out: condition,
			op: Binary::Eq,
			lhs: layer.count,
			rhs: setup.zero,
		};
		let decrement = Op::Binary {
			out: layer.count,
			op: Binary::Sub,
			lhs: layer.count,
			rhs: setup.one,
		};
		if layer.take {
			self.emit(spent, span)?;
			let pc = self.branch(condition, span)?;
			let no = self.pc();
			exhausted.try_push(Exhausted::Branch { pc, condition, no })?;
			self.emit(decrement, span)?;
			return self.chain_next(setup, index, out, exhausted, span);
		}
		let top = self.pc();
		self.chain_next(setup, index, out, exhausted, span)?;
		self.emit(spent, span)?;
		let pc = self.branch(condition, span)?;
		let discard = self.pc();
		self.emit(Op::Release { value: out }, span)?;
		self.emit(decrement, span)?;
		self.emit(Op::Jump { target: top }, span)?;
		let done = self.pc();
		self.function.patch(
			pc,
			Op::Branch {
				condition,
				yes: done,
				no: discard,
			},
		)
	}

	fn branch(&mut self, condition: u32, span: Span) -> Result<u32> {
		self.emit(
			Op::Branch {
				condition,
				yes: 0,
				no: 0,
			},
			span,
		)
	}

	fn patch_exhausted(&mut self, exhausted: &[Exhausted], target: u32) -> Result<()> {
		for exit in exhausted {
			match *exit {
				Exhausted::Next { pc, out, iterator } => self.function.patch(
					pc,
					Op::IterNext {
						out,
						iterator,
						none: target,
					},
				)?,
				Exhausted::Branch { pc, condition, no } => self.function.patch(
					pc,
					Op::Branch {
						condition,
						yes: target,
						no,
					},
				)?,
			}
		}
		Ok(())
	}

	/// A loop over every element: `body` runs with the element in `element`.
	fn chain_loop(
		&mut self,
		setup: &Setup,
		element: u32,
		span: Span,
		body: &mut dyn FnMut(&mut Self) -> Result<()>,
	) -> Result<()> {
		let mut exhausted = Vec::new();
		let start = self.pc();
		self.chain_next(setup, setup.layers.len(), element, &mut exhausted, span)?;
		body(self)?;
		self.emit(Op::Jump { target: start }, span)?;
		let end = self.pc();
		self.patch_exhausted(&exhausted, end)
	}

	#[cold]
	#[inline(never)]
	fn call_terminal(
		&mut self,
		call: &hir::ExprCall<'_>,
		end: End,
		target: hir::ExprId,
		out: u32,
		span: Span,
	) -> Result<()> {
		let arity = match end {
			End::Terminal(terminal) => terminal.arity(),
			End::Next => 1,
		};
		if call.args.len() + 1 != arity {
			return Err(self.refused(span, Refusal::Arity));
		}
		let chain = self
			.chain(target)
			.ok_or_else(|| self.refused(span, Refusal::Method))?;
		self.chain_proof(&chain, end, span)?;
		let mark = self.next;
		let setup = self.chain_setup(&chain, span)?;
		let element = self.slot()?;
		match end {
			End::Next => {
				let mut exhausted = Vec::new();
				self.chain_next(&setup, setup.layers.len(), element, &mut exhausted, span)?;
				self.emit(
					Op::Variant {
						out,
						tag: Type::Option,
						success: true,
						payload: Some(element),
					},
					span,
				)?;
				let jump = self.emit(Op::Jump { target: 0 }, span)?;
				let none = self.pc();
				self.patch_exhausted(&exhausted, none)?;
				self.emit(
					Op::Variant {
						out,
						tag: Type::Option,
						success: false,
						payload: None,
					},
					span,
				)?;
				let after = self.pc();
				self.function.patch(jump, Op::Jump { target: after })?;
			}
			End::Terminal(Terminal::Count) => {
				self.emit(
					Op::Literal {
						out,
						value: Value::U64(0),
					},
					span,
				)?;
				let one = setup.one;
				self.chain_loop(&setup, element, span, &mut |cx| {
					cx.emit(
						Op::Binary {
							out,
							op: Binary::Add,
							lhs: out,
							rhs: one,
						},
						span,
					)?;
					Ok(())
				})?;
			}
			End::Terminal(Terminal::Collect(collection)) => {
				self.collect(&setup, collection, element, out, span)?
			}
			End::Terminal(Terminal::Sum(number)) => {
				self.fold(&setup, number, Intrinsic::Sum, element, out, span)?
			}
			End::Terminal(Terminal::Product(number)) => {
				self.fold(&setup, number, Intrinsic::Product, element, out, span)?
			}
			End::Terminal(Terminal::Skip | Terminal::Take) => {
				return Err(Error::new(ErrorKind::InvalidCode, "adapter terminal"));
			}
		}
		self.pop_to(mark, span)
	}

	fn collect(
		&mut self,
		setup: &Setup,
		collection: Collection,
		element: u32,
		out: u32,
		span: Span,
	) -> Result<()> {
		let (args_start, args_len) = self.function.arguments(&[])?;
		match collection {
			Collection::Vec | Collection::Tuple => {
				let vec = if collection == Collection::Vec {
					out
				} else {
					self.slot()?
				};
				self.emit(
					Op::List {
						out: vec,
						tag: Type::Vec,
						args_start,
						args_len,
					},
					span,
				)?;
				self.chain_loop(setup, element, span, &mut |cx| {
					cx.emit(
						Op::Push {
							value: vec,
							new: element,
						},
						span,
					)?;
					Ok(())
				})?;
				if collection == Collection::Tuple {
					self.intrinsic(Intrinsic::IntoTuple, &[vec], out, span)?;
				}
			}
			Collection::Object | Collection::String => {
				if collection == Collection::Object {
					self.emit(
						Op::Record {
							out,
							tag: Type::Object,
							args_start,
							args_len,
						},
						span,
					)?;
				} else {
					let text = self.function.string("")?;
					self.emit(Op::String { out, text }, span)?;
				}
				let step = if collection == Collection::Object {
					Intrinsic::Insert
				} else {
					Intrinsic::Append
				};
				let unit = self.slot()?;
				self.chain_loop(setup, element, span, &mut |cx| {
					cx.intrinsic(step, &[out, element], unit, span)
				})?;
			}
		}
		Ok(())
	}

	/// `sum`/`product`: the first element seeds the accumulator in the
	/// turbofish type; an empty chain gives that type's zero or one.
	fn fold(
		&mut self,
		setup: &Setup,
		number: Number,
		step: Intrinsic,
		element: u32,
		out: u32,
		span: Span,
	) -> Result<()> {
		let mut empty = Vec::new();
		self.chain_next(setup, setup.layers.len(), element, &mut empty, span)?;
		self.intrinsic(Intrinsic::Seed(number), &[element], out, span)?;
		self.chain_loop(setup, element, span, &mut |cx| {
			cx.intrinsic(step, &[out, element], out, span)
		})?;
		let jump = self.emit(Op::Jump { target: 0 }, span)?;
		let none = self.pc();
		self.patch_exhausted(&empty, none)?;
		let product = step == Intrinsic::Product;
		let value = match number {
			Number::I64 => Value::I64(i64::from(product)),
			Number::U64 => Value::U64(u64::from(product)),
			Number::F64 => Value::F64(if product { 1.0 } else { 0.0 }),
		};
		self.emit(Op::Literal { out, value }, span)?;
		let after = self.pc();
		self.function.patch(jump, Op::Jump { target: after })
	}

	/// `for` over a direct chain with at least one adapter.
	#[cold]
	#[inline(never)]
	pub(super) fn terminal_for(
		&mut self,
		for_: &hir::ExprFor<'hir>,
		out: Option<u32>,
		span: Span,
	) -> Result<()> {
		let chain = self
			.chain(for_.iter)
			.ok_or_else(|| self.refused(span, Refusal::Method))?;
		self.chain_proof(&chain, End::Next, span)?;
		if let Some(out) = out {
			self.unit_into(out, span)?;
		}
		let vars = self.variables.len();
		let mark = self.next;
		let setup = self.chain_setup(&chain, span)?;
		let binding = self.slot()?;
		let destructure = !matches!(
			for_.binding.pat.kind,
			hir::PatKind::Ignore | hir::PatKind::Path(hir::PatPathKind::Ident { .. })
		);
		if !destructure {
			self.bind(&for_.binding.pat, binding)?;
		}
		let mut exhausted = Vec::new();
		let start = self.pc();
		self.chain_next(&setup, setup.layers.len(), binding, &mut exhausted, span)?;
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
		self.patch_exhausted(&exhausted, end)?;
		let context = self
			.loops
			.pop()
			.ok_or_else(|| self.refused(span, Refusal::LoopControl))?;
		for pc in context.breaks {
			self.function.patch(pc, Op::Jump { target: end })?;
		}
		self.pop_to(mark, span)?;
		self.variables.truncate(vars);
		Ok(())
	}
}
