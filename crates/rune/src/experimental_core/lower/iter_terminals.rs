//! Direct iterator chains: an `iter()`, `std::iter::range` or text source,
//! adapters (C17d `skip`/`take`; C17e `map`, `filter`, `filter_map`,
//! `flat_map`, `enumerate`, `chain` and `rev`) and one consumer, expanded into
//! ordinary charged instructions. Adapters become state in slots, so no
//! iterator value other than the source and the iterators a `chain` or
//! `flat_map` converts exists. A chain that is stored, passed or returned is
//! not direct and keeps its earlier refusal.
use super::*;
use crate::function::iter::Name;
use crate::function::iter_terminal::{Closure, Collection, Intrinsic, Number, Terminal, RANGE};

/// Chain recursion bound, as for the other origin proofs.
const DEPTH: usize = 128;

/// The innermost expression of a chain.
#[derive(Clone, Copy)]
enum Source {
	/// `receiver.iter()`, lowered as the certified `iter` call.
	Iter(hir::ExprId),
	/// A `std::iter::range(start, end)` call.
	Range(hir::ExprId),
	/// A C14b text iterator call, `chars()` or `split(literal)`.
	Text(hir::ExprId),
}

/// One adapter call around the chain so far, with its operands.
#[derive(Clone, Copy)]
enum Step {
	Skip(hir::ExprId),
	Take(hir::ExprId),
	Map(hir::ExprId),
	Filter(hir::ExprId),
	FilterMap(hir::ExprId),
	FlatMap(hir::ExprId),
	Enumerate,
	Chain(hir::ExprId),
	Rev,
}

impl Step {
	/// Whether this is a C17e adapter rather than a C17d `skip`/`take`.
	fn closure(self) -> bool {
		!matches!(self, Self::Skip(_) | Self::Take(_))
	}
}

#[derive(Clone, Copy)]
struct Adapter {
	step: Step,
	span: Span,
}

/// A source and its adapters, innermost first.
struct Chain {
	source: Source,
	adapters: Vec<Adapter>,
}

impl Chain {
	/// Whether the chain has a C17e adapter.
	fn closures(&self) -> bool {
		self.adapters.iter().any(|a| a.step.closure())
	}

	/// Whether the chain reaches a C17e iterator type.
	fn extended(&self) -> bool {
		matches!(self.source, Source::Text(_)) || self.closures()
	}
}

/// A call site with a cold chain lowering.
#[derive(Clone, Copy)]
pub(super) enum Site {
	/// A `std::iter::range(start, end)` value.
	Range,
	/// The consumer of a direct chain.
	Terminal(End, hir::ExprId),
	/// A C14b text pipeline method.
	Text(hir::ExprId, crate::function::pipeline::Method),
}

/// What consumes a chain at a call site.
#[derive(Clone, Copy)]
pub(super) enum End {
	Terminal(Terminal),
	/// `next()` after at least one adapter.
	Next,
	/// A C17e consumer that takes a function value.
	Closure(Closure),
}

/// An adapter's state after setup, in slots.
#[derive(Clone, Copy)]
enum Layer {
	Skip {
		count: u32,
	},
	Take {
		count: u32,
	},
	Map {
		function: u32,
		input: u32,
	},
	Filter {
		function: u32,
		test: u32,
	},
	FilterMap {
		function: u32,
		input: u32,
		result: u32,
	},
	/// `inner` holds the current result's iterator while `active` is true.
	FlatMap {
		function: u32,
		input: u32,
		result: u32,
		inner: u32,
		active: u32,
	},
	Enumerate {
		count: u32,
		input: u32,
	},
	/// `first` is true until the chain so far is exhausted, then `second`
	/// is drained.
	Chain {
		second: u32,
		first: u32,
	},
}

/// A `rev` window over the source: the offsets `lo..=last` from its front,
/// read back to front, until `empty`.
#[derive(Clone, Copy)]
struct Window {
	empty: u32,
	lo: u32,
	last: u32,
}

/// The evaluated chain: the source iterator, the adapter state and the
/// slots the expansion shares.
struct Setup {
	node: u32,
	layers: Vec<Layer>,
	window: Option<Window>,
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
				hir::Call::Associated {
					target,
					hash,
					name_hash,
				} => {
					// A script method's result is not a known iterator source.
					if self.static_method(target, hash, name_hash).is_some() {
						return None;
					}
					let step = match (Terminal::from_hash(hash), Closure::from_hash(hash), c.args) {
						(Some(Terminal::Skip), _, &[n]) => Step::Skip(n),
						(Some(Terminal::Take), _, &[n]) => Step::Take(n),
						(_, Some(Closure::Map), &[f]) => Step::Map(f),
						(_, Some(Closure::Filter), &[f]) => Step::Filter(f),
						(_, Some(Closure::FilterMap), &[f]) => Step::FilterMap(f),
						(_, Some(Closure::FlatMap), &[f]) => Step::FlatMap(f),
						(_, Some(Closure::Enumerate), &[]) => Step::Enumerate,
						(_, Some(Closure::Chain), &[b]) => Step::Chain(b),
						(_, Some(Closure::Rev), &[]) => Step::Rev,
						_ if hash == Name::Iter.hash() => {
							return c.args.is_empty().then_some(Source::Iter(id));
						}
						_ => {
							return self
								.pipeline_shape(id, 0)
								.is_some_and(|s| s.len() == 1)
								.then_some(Source::Text(id));
						}
					};
					let source = self.chain_into(target, depth + 1, adapters)?;
					adapters
						.try_push(Adapter {
							step,
							span: expr.span,
						})
						.ok()?;
					Some(source)
				}
				hir::Call::Meta { hash } if hash == RANGE => Some(Source::Range(id)),
				_ => None,
			},
			_ => None,
		}
	}

	/// The chain at `id` if it is direct, has at least one adapter, and, over
	/// a text source, at least one C17e adapter.
	fn adapted_chain(&self, id: hir::ExprId) -> Option<Chain> {
		use hir::ExprKind as E;
		let mut at = id;
		for _ in 0..DEPTH {
			match self.exprs.get(at).kind {
				E::Group(inner) => at = inner,
				E::Call(_) => {
					let chain = self.chain(id)?;
					let text = matches!(chain.source, Source::Text(_));
					return (!chain.adapters.is_empty() && (!text || chain.closures()))
						.then_some(chain);
				}
				_ => return None,
			}
		}
		None
	}

	/// Whether `id` is a direct chain with at least one adapter.
	pub(super) fn adapted(&self, id: hir::ExprId) -> bool {
		self.adapted_chain(id).is_some()
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

	/// The consumer and the chain at a method call site, if it ends a direct
	/// chain. A text source needs a C17e adapter or consumer, so the C14b
	/// pipeline keeps its own chains. `any` on a bare source stays the C12
	/// small form.
	fn terminal_site(&self, call: &hir::ExprCall<'_>) -> Option<(End, hir::ExprId)> {
		let hir::Call::Associated { target, hash, .. } = call.call else {
			return None;
		};
		let end = match (Terminal::from_hash(hash), Closure::from_hash(hash)) {
			(Some(Terminal::Skip | Terminal::Take), _) => return None,
			(Some(terminal), _) => End::Terminal(terminal),
			(None, Some(closure)) if !closure.adapter() => End::Closure(closure),
			_ if hash == Name::Next.hash() => End::Next,
			_ => return None,
		};
		let direct = match end {
			// The C12 small form keeps `any` on a bare source, except where it
			// refuses a text source (`chars()`, or a `split` with adapters) and
			// where the argument is never a function.
			End::Closure(Closure::Any) => self.chain(target).is_some_and(|c| match c.source {
				Source::Text(id) => {
					!c.adapters.is_empty() || self.pipeline_shape(id, 0).is_some_and(|s| s[0] == 2)
				}
				_ => {
					!c.adapters.is_empty()
						|| call.args.first().is_some_and(|&f| self.never_function(f))
				}
			}),
			End::Next => self.adapted_chain(target).is_some(),
			End::Terminal(_) => self
				.chain(target)
				.is_some_and(|c| !matches!(c.source, Source::Text(_)) || c.closures()),
			End::Closure(_) => self.chain(target).is_some(),
		};
		direct.then_some((end, target))
	}

	/// Whether `id` is a value that can never be a function, so `any` takes
	/// the C17e lowering and its stock argument conversion error.
	fn never_function(&self, id: hir::ExprId) -> bool {
		use hir::ExprKind as E;
		matches!(
			self.exprs.get(id).kind,
			E::Lit(_) | E::Object(_) | E::Vec(_) | E::Tuple(_) | E::Range(_) | E::Template(_)
		)
	}

	/// Certifies every iterator function the chain calls.
	fn chain_proof(&mut self, chain: &Chain, end: End, span: Span) -> Result<()> {
		let extended = chain.extended() || matches!(end, End::Closure(_));
		if let Source::Range(_) = chain.source {
			self.transaction
				.iter_range(self.context)
				.map_err(|e| e.at(self.location(span)))?;
		}
		for adapter in &chain.adapters {
			let at = self.location(adapter.span);
			match adapter.step {
				Step::Skip(_) | Step::Take(_) => {
					let terminal = if let Step::Take(_) = adapter.step {
						Terminal::Take
					} else {
						Terminal::Skip
					};
					self.transaction
						.iter_terminal(self.context, terminal)
						.map_err(|e| e.at(at))?;
					if extended {
						self.transaction
							.iter_extended(self.context, Some(terminal))
							.map_err(|e| e.at(at))?;
					}
				}
				step => {
					let closure = match step {
						Step::Map(_) => Closure::Map,
						Step::Filter(_) => Closure::Filter,
						Step::FilterMap(_) => Closure::FilterMap,
						Step::FlatMap(_) => Closure::FlatMap,
						Step::Enumerate => Closure::Enumerate,
						Step::Chain(_) => Closure::Chain,
						_ => Closure::Rev,
					};
					self.transaction
						.iter_closure(self.context, closure)
						.map_err(|e| e.at(at))?;
				}
			}
		}
		let at = self.location(span);
		match end {
			End::Terminal(terminal) => {
				self.transaction
					.iter_terminal(self.context, terminal)
					.map_err(|e| e.at(at))?;
				if extended {
					self.transaction
						.iter_extended(self.context, Some(terminal))
						.map_err(|e| e.at(at))?;
				}
			}
			// `next` and the for-loop NEXT protocol on Skip and Take, and on the
			// C17e adapters.
			End::Next => {
				self.transaction
					.pipeline_method(
						self.context,
						self.constants,
						self.pool,
						crate::function::pipeline::Method::Next,
					)
					.map_err(|e| e.at(at))?;
				if extended {
					self.transaction
						.iter_extended(self.context, None)
						.map_err(|e| e.at(at))?;
				}
			}
			End::Closure(closure) => self
				.transaction
				.iter_closure(self.context, closure)
				.map_err(|e| e.at(at))?,
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

	/// A function value operand whose parameter count is known before it
	/// runs: a literal closure, a fn item, or a local bound once to one.
	/// Each must take `passes` arguments, or the program is refused and stock
	/// keeps the call's run-time behavior.
	fn function_operand(&mut self, id: hir::ExprId, passes: u32, span: Span) -> Result<u32> {
		use hir::ExprKind as E;
		let mut init = id;
		if let E::Variable(v) = self.exprs.get(id).kind {
			init = self
				.text_init(v)
				.filter(|_| self.text_writes(v, |_| false))
				.ok_or_else(|| self.refused(span, Refusal::Method))?;
		}
		let at = self.location(span);
		let (hash, captures) = match self.exprs.get(init).kind {
			E::Fn(hash) => (hash, 0),
			E::CallClosure(closure) => (closure.hash, closure.captures.len()),
			_ => return Err(self.refused(span, Refusal::Method)),
		};
		let constructor = self
			.constants
			.value_shape(hash)
			.map(|shape| match shape.fields {
				crate::compile::meta::Fields::Unnamed(n) => *n,
				_ => 0,
			});
		match constructor {
			Some(0) => return Err(self.refused(span, Refusal::Method)),
			Some(n) if n != passes as usize => return Err(self.refused(span, Refusal::Arity)),
			Some(_) => (),
			None => {
				let id = (self.resolve)(Resolve::Script(hash), at)?;
				let parameters = u32::try_from(captures)
					.ok()
					.and_then(|c| c.checked_add(passes))
					.ok_or_else(|| self.refused(span, Refusal::Arity))?;
				self.transaction.require_parameters(id, parameters, at)?;
			}
		}
		self.operand(id)
	}

	/// Whether the receiver of a `rev` over adapters is statically a source
	/// stock iterates from the back: a Vec, Tuple, `Some` or bounded range
	/// literal, or `std::iter::range`. With a `skip` or `take` below the `rev`
	/// stock also takes its length, so only a Vec or Tuple literal qualifies.
	fn reversible_source(&self, source: Source, counted: bool) -> bool {
		use hir::ExprKind as E;
		let id = match source {
			Source::Range(_) => return !counted,
			Source::Text(_) => return false,
			Source::Iter(id) => id,
		};
		let E::Call(c) = self.exprs.get(id).kind else {
			return false;
		};
		let hir::Call::Associated { target, .. } = c.call else {
			return false;
		};
		let mut at = target;
		for _ in 0..DEPTH {
			match self.exprs.get(at).kind {
				E::Group(inner) => at = inner,
				E::Vec(_) | E::Tuple(_) => return true,
				E::Range(hir::ExprRange::Range { .. } | hir::ExprRange::RangeInclusive { .. }) => {
					return !counted
				}
				E::Call(c) => {
					return !counted
						&& matches!(c.call, hir::Call::Meta { hash }
						if hash == Hash::type_hash(crate::item!(::std::option::Option::Some)))
				}
				_ => return false,
			}
		}
		false
	}

	/// Evaluates the source, then each adapter's operands in source order.
	fn chain_setup(&mut self, chain: &Chain, span: Span) -> Result<Setup> {
		let node = match chain.source {
			Source::Iter(id) | Source::Text(id) => self.expr(id)?,
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
		let mut setup = Setup {
			node,
			layers: Vec::try_with_capacity(chain.adapters.len())?,
			window: None,
			zero: 0,
			one: 0,
			condition: 0,
		};
		// A C17d chain keeps its slot order: these follow the counts.
		let closures = chain.closures();
		if closures {
			self.shared_slots(&mut setup, span)?;
		}
		for adapter in &chain.adapters {
			let at = adapter.span;
			let layer = match adapter.step {
				Step::Skip(n) | Step::Take(n) => {
					let value = self.operand(n)?;
					let count = self.slot()?;
					self.intrinsic(Intrinsic::AdapterCount, &[value], count, at)?;
					if let Step::Take(_) = adapter.step {
						Layer::Take { count }
					} else {
						Layer::Skip { count }
					}
				}
				Step::Map(f) => Layer::Map {
					function: self.function_operand(f, 1, at)?,
					input: self.slot()?,
				},
				Step::Filter(f) => Layer::Filter {
					function: self.function_operand(f, 1, at)?,
					test: self.slot()?,
				},
				Step::FilterMap(f) => Layer::FilterMap {
					function: self.function_operand(f, 1, at)?,
					input: self.slot()?,
					result: self.slot()?,
				},
				Step::FlatMap(f) => {
					let function = self.function_operand(f, 1, at)?;
					let (input, result, inner) = (self.slot()?, self.slot()?, self.slot()?);
					let active = self.literal(Value::Bool(false), at)?;
					Layer::FlatMap {
						function,
						input,
						result,
						inner,
						active,
					}
				}
				Step::Enumerate => Layer::Enumerate {
					count: self.literal(Value::U64(0), at)?,
					input: self.slot()?,
				},
				Step::Chain(b) => {
					// An iterator argument is a passed iterator; stock converts any
					// other value when `chain` is called.
					if self.chain(b).is_some() || self.pipeline_shape(b, 0).is_some() {
						return Err(self.refused(at, Refusal::Method));
					}
					let value = self.operand(b)?;
					let second = self.slot()?;
					self.intrinsic(Intrinsic::IntoIter, &[value], second, at)?;
					let first = self.literal(Value::Bool(true), at)?;
					Layer::Chain { second, first }
				}
				Step::Rev => {
					self.reverse(chain, &mut setup, at)?;
					continue;
				}
			};
			setup.layers.try_push(layer)?;
		}
		if !closures {
			self.shared_slots(&mut setup, span)?;
		}
		Ok(setup)
	}

	fn shared_slots(&mut self, setup: &mut Setup, span: Span) -> Result<()> {
		setup.zero = self.literal(Value::U64(0), span)?;
		setup.one = self.literal(Value::U64(1), span)?;
		setup.condition = self.slot()?;
		Ok(())
	}

	/// `rev`: the source is read back to front. A `skip`/`take` below it
	/// narrows the window from the front, as stock's length-based
	/// `next_back` does; `map`/`filter`/`filter_map` below it apply per
	/// element as their `next_back` does. Anything else stays refused.
	fn reverse(&mut self, chain: &Chain, setup: &mut Setup, span: Span) -> Result<()> {
		let below = setup.layers.len();
		let counted = setup
			.layers
			.iter()
			.take_while(|l| matches!(l, Layer::Skip { .. } | Layer::Take { .. }))
			.count();
		let closures_only = setup.layers[counted..].iter().all(|l| {
			matches!(
				l,
				Layer::Map { .. } | Layer::Filter { .. } | Layer::FilterMap { .. }
			)
		});
		if setup.window.is_some()
			|| !closures_only
			|| matches!(chain.source, Source::Text(_))
			|| (below > 0 && !self.reversible_source(chain.source, counted > 0))
		{
			return Err(self.refused(span, Refusal::Method));
		}
		let empty = self.slot()?;
		self.intrinsic(Intrinsic::RevEmpty, &[setup.node], empty, span)?;
		let last = self.slot()?;
		self.intrinsic(Intrinsic::RevLast, &[setup.node], last, span)?;
		let lo = self.literal(Value::U64(0), span)?;
		let (zero, one, condition) = (setup.zero, setup.one, setup.condition);
		let width = self.slot()?;
		for index in 0..counted {
			// `width` is `last - lo`: the window holds `width + 1` offsets.
			self.emit(
				Op::Binary {
					out: width,
					op: Binary::Sub,
					lhs: last,
					rhs: lo,
				},
				span,
			)?;
			match setup.layers[index] {
				Layer::Skip { count } => {
					self.emit(
						Op::Binary {
							out: condition,
							op: Binary::Gt,
							lhs: count,
							rhs: width,
						},
						span,
					)?;
					let past = self.branch(condition, span)?;
					let keep = self.pc();
					self.emit(
						Op::Binary {
							out: lo,
							op: Binary::Add,
							lhs: lo,
							rhs: count,
						},
						span,
					)?;
					let jump = self.emit(Op::Jump { target: 0 }, span)?;
					let none = self.pc();
					self.emit(
						Op::Literal {
							out: empty,
							value: Value::Bool(true),
						},
						span,
					)?;
					let end = self.pc();
					self.function.patch(
						past,
						Op::Branch {
							condition,
							yes: none,
							no: keep,
						},
					)?;
					self.function.patch(jump, Op::Jump { target: end })?;
				}
				Layer::Take { count } => {
					self.emit(
						Op::Binary {
							out: condition,
							op: Binary::Eq,
							lhs: count,
							rhs: zero,
						},
						span,
					)?;
					let nothing = self.branch(condition, span)?;
					let compare = self.pc();
					// `count - 1 < width`: the window shrinks to `count` offsets.
					self.emit(
						Op::Binary {
							out: count,
							op: Binary::Sub,
							lhs: count,
							rhs: one,
						},
						span,
					)?;
					self.emit(
						Op::Binary {
							out: condition,
							op: Binary::Lt,
							lhs: count,
							rhs: width,
						},
						span,
					)?;
					let shrink = self.branch(condition, span)?;
					let set = self.pc();
					self.emit(
						Op::Binary {
							out: last,
							op: Binary::Add,
							lhs: lo,
							rhs: count,
						},
						span,
					)?;
					let jump = self.emit(Op::Jump { target: 0 }, span)?;
					let none = self.pc();
					self.emit(
						Op::Literal {
							out: empty,
							value: Value::Bool(true),
						},
						span,
					)?;
					let end = self.pc();
					self.function.patch(
						nothing,
						Op::Branch {
							condition,
							yes: none,
							no: compare,
						},
					)?;
					self.function.patch(
						shrink,
						Op::Branch {
							condition,
							yes: set,
							no: end,
						},
					)?;
					self.function.patch(jump, Op::Jump { target: end })?;
				}
				_ => return Err(Error::new(ErrorKind::InvalidCode, "rev window")),
			}
		}
		let mut rest = Vec::try_with_capacity(below - counted)?;
		for layer in &setup.layers[counted..] {
			rest.try_push(*layer)?;
		}
		setup.layers = rest;
		setup.window = Some(Window { empty, lo, last });
		Ok(())
	}

	fn literal(&mut self, value: Value, span: Span) -> Result<u32> {
		let out = self.slot()?;
		self.emit(Op::Literal { out, value }, span)?;
		Ok(out)
	}

	fn call_value(&mut self, out: u32, value: u32, args: &[u32], span: Span) -> Result<()> {
		let (args_start, args_len) = self.function.arguments(args)?;
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

	/// The next source element into `out`, or a jump recorded in `exhausted`.
	fn source_next(
		&mut self,
		setup: &Setup,
		out: u32,
		exhausted: &mut Vec<Exhausted>,
		span: Span,
	) -> Result<()> {
		let Some(window) = setup.window else {
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
		let pc = self.branch(window.empty, span)?;
		exhausted.try_push(Exhausted::Branch {
			pc,
			condition: window.empty,
			no: pc + 1,
		})?;
		self.intrinsic(Intrinsic::RevAt, &[setup.node, window.last], out, span)?;
		self.emit(
			Op::Binary {
				out: setup.condition,
				op: Binary::Eq,
				lhs: window.last,
				rhs: window.lo,
			},
			span,
		)?;
		let at_front = self.branch(setup.condition, span)?;
		let step = self.pc();
		self.emit(
			Op::Binary {
				out: window.last,
				op: Binary::Sub,
				lhs: window.last,
				rhs: setup.one,
			},
			span,
		)?;
		let jump = self.emit(Op::Jump { target: 0 }, span)?;
		let done = self.pc();
		self.emit(
			Op::Literal {
				out: window.empty,
				value: Value::Bool(true),
			},
			span,
		)?;
		let end = self.pc();
		self.function.patch(
			at_front,
			Op::Branch {
				condition: setup.condition,
				yes: done,
				no: step,
			},
		)?;
		self.function.patch(jump, Op::Jump { target: end })
	}

	/// The next element of the chain's first `layers` adapters into `out`, or
	/// a jump recorded in `exhausted`. Each adapter behaves as its stock
	/// `next`: `take` stops without pulling once its count is spent; `skip`
	/// pulls and discards until its count is spent; the function value of a
	/// closure adapter runs once per element it pulls.
	fn chain_next(
		&mut self,
		setup: &Setup,
		layers: usize,
		out: u32,
		exhausted: &mut Vec<Exhausted>,
		span: Span,
	) -> Result<()> {
		let Some(index) = layers.checked_sub(1) else {
			return self.source_next(setup, out, exhausted, span);
		};
		let condition = setup.condition;
		match setup.layers[index] {
			Layer::Take { count } => {
				self.emit(
					Op::Binary {
						out: condition,
						op: Binary::Eq,
						lhs: count,
						rhs: setup.zero,
					},
					span,
				)?;
				let pc = self.branch(condition, span)?;
				let no = self.pc();
				exhausted.try_push(Exhausted::Branch { pc, condition, no })?;
				self.emit(
					Op::Binary {
						out: count,
						op: Binary::Sub,
						lhs: count,
						rhs: setup.one,
					},
					span,
				)?;
				self.chain_next(setup, index, out, exhausted, span)
			}
			Layer::Skip { count } => {
				let top = self.pc();
				self.chain_next(setup, index, out, exhausted, span)?;
				self.emit(
					Op::Binary {
						out: condition,
						op: Binary::Eq,
						lhs: count,
						rhs: setup.zero,
					},
					span,
				)?;
				let pc = self.branch(condition, span)?;
				let discard = self.pc();
				self.emit(Op::Release { value: out }, span)?;
				self.emit(
					Op::Binary {
						out: count,
						op: Binary::Sub,
						lhs: count,
						rhs: setup.one,
					},
					span,
				)?;
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
			Layer::Map { function, input } => {
				self.chain_next(setup, index, input, exhausted, span)?;
				self.call_value(out, function, &[input], span)?;
				self.emit(Op::Release { value: input }, span)?;
				Ok(())
			}
			Layer::Filter { function, test } => {
				let top = self.pc();
				self.chain_next(setup, index, out, exhausted, span)?;
				self.call_value(test, function, &[out], span)?;
				let pc = self.branch(test, span)?;
				let discard = self.pc();
				self.emit(Op::Release { value: out }, span)?;
				self.emit(Op::Jump { target: top }, span)?;
				let done = self.pc();
				self.function.patch(
					pc,
					Op::Branch {
						condition: test,
						yes: done,
						no: discard,
					},
				)
			}
			Layer::FilterMap {
				function,
				input,
				result,
			} => {
				let top = self.pc();
				self.chain_next(setup, index, input, exhausted, span)?;
				self.call_value(result, function, &[input], span)?;
				self.emit(Op::Release { value: input }, span)?;
				self.intrinsic(Intrinsic::OptionSome, &[result], condition, span)?;
				let pc = self.branch(condition, span)?;
				let none = self.pc();
				self.emit(Op::Release { value: result }, span)?;
				self.emit(Op::Jump { target: top }, span)?;
				let some = self.pc();
				self.emit(
					Op::TupleField {
						out,
						value: result,
						index: setup.zero,
					},
					span,
				)?;
				self.emit(Op::Release { value: result }, span)?;
				self.function.patch(
					pc,
					Op::Branch {
						condition,
						yes: some,
						no: none,
					},
				)
			}
			Layer::FlatMap {
				function,
				input,
				result,
				inner,
				active,
			} => {
				let top = self.pc();
				let pc = self.branch(active, span)?;
				let pull = self.pc();
				let next = self.emit(
					Op::IterNext {
						out,
						iterator: inner,
						none: 0,
					},
					span,
				)?;
				let found = self.emit(Op::Jump { target: 0 }, span)?;
				let drained = self.pc();
				self.emit(Op::Release { value: inner }, span)?;
				self.emit(
					Op::Literal {
						out: active,
						value: Value::Bool(false),
					},
					span,
				)?;
				let fetch = self.pc();
				self.chain_next(setup, index, input, exhausted, span)?;
				self.call_value(result, function, &[input], span)?;
				self.emit(Op::Release { value: input }, span)?;
				self.intrinsic(Intrinsic::IntoIter, &[result], inner, span)?;
				self.emit(Op::Release { value: result }, span)?;
				self.emit(
					Op::Literal {
						out: active,
						value: Value::Bool(true),
					},
					span,
				)?;
				self.emit(Op::Jump { target: top }, span)?;
				let done = self.pc();
				self.function.patch(
					pc,
					Op::Branch {
						condition: active,
						yes: pull,
						no: fetch,
					},
				)?;
				self.function.patch(
					next,
					Op::IterNext {
						out,
						iterator: inner,
						none: drained,
					},
				)?;
				self.function.patch(found, Op::Jump { target: done })
			}
			Layer::Enumerate { count, input } => {
				self.chain_next(setup, index, input, exhausted, span)?;
				let (args_start, args_len) = self.function.arguments(&[count, input])?;
				self.emit(
					Op::List {
						out,
						tag: Type::Tuple,
						args_start,
						args_len,
					},
					span,
				)?;
				self.emit(Op::Release { value: input }, span)?;
				self.emit(
					Op::Binary {
						out: count,
						op: Binary::Add,
						lhs: count,
						rhs: setup.one,
					},
					span,
				)?;
				Ok(())
			}
			Layer::Chain { second, first } => {
				let pc = self.branch(first, span)?;
				let head = self.pc();
				let mut drained = Vec::new();
				self.chain_next(setup, index, out, &mut drained, span)?;
				let found = self.emit(Op::Jump { target: 0 }, span)?;
				// Stock drops the exhausted first part: its iterators, and the
				// leases they hold, end here.
				let fuse = self.pc();
				self.patch_exhausted(&drained, fuse)?;
				self.emit(
					Op::Literal {
						out: first,
						value: Value::Bool(false),
					},
					span,
				)?;
				self.emit(Op::Release { value: setup.node }, span)?;
				for layer in &setup.layers[..index] {
					let held = match *layer {
						Layer::FlatMap { inner, .. } => inner,
						Layer::Chain { second, .. } => second,
						_ => continue,
					};
					self.emit(Op::Release { value: held }, span)?;
				}
				let tail = self.pc();
				let next = self.emit(
					Op::IterNext {
						out,
						iterator: second,
						none: 0,
					},
					span,
				)?;
				exhausted.try_push(Exhausted::Next {
					pc: next,
					out,
					iterator: second,
				})?;
				let done = self.pc();
				self.function.patch(
					pc,
					Op::Branch {
						condition: first,
						yes: head,
						no: tail,
					},
				)?;
				self.function.patch(found, Op::Jump { target: done })
			}
		}
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
			End::Closure(closure) => closure.arity(),
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
			End::Closure(closure) => self.consume(call, &setup, closure, element, out, span)?,
		}
		self.pop_to(mark, span)
	}

	/// `fold`, `reduce`, `find`, `any` and `all`: their arguments evaluate
	/// after the chain, as stock's call operands do.
	fn consume(
		&mut self,
		call: &hir::ExprCall<'_>,
		setup: &Setup,
		closure: Closure,
		element: u32,
		out: u32,
		span: Span,
	) -> Result<()> {
		let passes = closure
			.passes()
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "consumer"))?;
		let mut args = call.args.iter().copied();
		let mut next_arg = || {
			args.next()
				.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "consumer"))
		};
		match closure {
			Closure::Fold => {
				let init = next_arg()?;
				let f = next_arg()?;
				self.expr_into(init, out)?;
				let function = self.function_operand(f, passes, span)?;
				self.chain_loop(setup, element, span, &mut |cx| {
					cx.call_value(out, function, &[out, element], span)?;
					cx.emit(Op::Release { value: element }, span)?;
					Ok(())
				})
			}
			Closure::Reduce => {
				let function = self.function_operand(next_arg()?, passes, span)?;
				let acc = self.slot()?;
				let mut empty = Vec::new();
				self.chain_next(setup, setup.layers.len(), acc, &mut empty, span)?;
				self.chain_loop(setup, element, span, &mut |cx| {
					cx.call_value(acc, function, &[acc, element], span)?;
					cx.emit(Op::Release { value: element }, span)?;
					Ok(())
				})?;
				self.emit(
					Op::Variant {
						out,
						tag: Type::Option,
						success: true,
						payload: Some(acc),
					},
					span,
				)?;
				let jump = self.emit(Op::Jump { target: 0 }, span)?;
				let none = self.pc();
				self.patch_exhausted(&empty, none)?;
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
				self.function.patch(jump, Op::Jump { target: after })
			}
			Closure::Find | Closure::Any | Closure::All => {
				let function = self.function_operand(next_arg()?, passes, span)?;
				let test = self.slot()?;
				let mut exhausted = Vec::new();
				let start = self.pc();
				self.chain_next(setup, setup.layers.len(), element, &mut exhausted, span)?;
				self.call_value(test, function, &[element], span)?;
				let pc = self.branch(test, span)?;
				let again = self.pc();
				self.emit(Op::Release { value: element }, span)?;
				self.emit(Op::Jump { target: start }, span)?;
				// `find`/`any` stop at the first true; `all` at the first false.
				let stop = self.pc();
				match closure {
					Closure::Find => self.emit(
						Op::Variant {
							out,
							tag: Type::Option,
							success: true,
							payload: Some(element),
						},
						span,
					)?,
					_ => self.emit(
						Op::Literal {
							out,
							value: Value::Bool(closure == Closure::Any),
						},
						span,
					)?,
				};
				let jump = self.emit(Op::Jump { target: 0 }, span)?;
				let none = self.pc();
				self.patch_exhausted(&exhausted, none)?;
				match closure {
					Closure::Find => self.emit(
						Op::Variant {
							out,
							tag: Type::Option,
							success: false,
							payload: None,
						},
						span,
					)?,
					_ => self.emit(
						Op::Literal {
							out,
							value: Value::Bool(closure == Closure::All),
						},
						span,
					)?,
				};
				let after = self.pc();
				self.function.patch(jump, Op::Jump { target: after })?;
				let (yes, no) = if closure == Closure::All {
					(again, stop)
				} else {
					(stop, again)
				};
				self.function.patch(
					pc,
					Op::Branch {
						condition: test,
						yes,
						no,
					},
				)
			}
			_ => Err(Error::new(ErrorKind::InvalidCode, "consumer")),
		}
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
			.adapted_chain(for_.iter)
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
