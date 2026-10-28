//! Compile warnings for the execution frontend, over its already lowered HIR.
//!
//! This walk models the diagnostic decisions in v2::assemble: ignored needs,
//! convergence, pattern refutability, and parent context. It neither assembles
//! instructions nor invokes the legacy compiler. An explicit work stack keeps
//! nested input off the Rust stack.

use crate::alloc::{self, Vec};
use crate::ast::{Span, Spanned};
use crate::{hir, Diagnostics, SourceId};

#[derive(Clone, Copy)]
enum Children<'hir> {
	Slice(&'hir [hir::ExprId]),
	Small([Option<hir::ExprId>; 3]),
	Object(&'hir [hir::FieldAssign<'hir>]),
	Call(&'hir hir::ExprCall<'hir>),
}

impl Children<'_> {
	fn get(self, at: usize) -> Option<hir::ExprId> {
		match self {
			Self::Slice(ids) => ids.get(at).copied(),
			Self::Small(ids) => ids.into_iter().flatten().nth(at),
			Self::Object(fields) => fields.get(at).map(|f| f.assign),
			Self::Call(call) => {
				let first = match call.call {
					hir::Call::Associated { target, .. } => Some(target),
					hir::Call::Expr { expr } => Some(expr),
					_ => None,
				};
				first.into_iter().chain(call.args.iter().copied()).nth(at)
			}
		}
	}
}

struct Branches<'hir> {
	hir: &'hir hir::Conditional<'hir>,
	at: usize,
	bodies: Vec<&'hir hir::ConditionalBranch<'hir>>,
	needed: bool,
	context: Span,
	all_diverge: bool,
}

struct Match<'hir> {
	hir: &'hir hir::ExprMatch<'hir>,
	at: usize,
	bodies: usize,
	irrefutable: bool,
	needed: bool,
	context: Span,
	all_diverge: bool,
}

struct Break<'hir> {
	label: Option<&'hir str>,
	needed: bool,
	is_loop: bool,
}

enum Work<'hir> {
	Expr(&'hir hir::Expr<'hir>, bool, Span),
	Block(&'hir hir::Block<'hir>, bool, bool),
	BlockNext(&'hir hir::Block<'hir>, bool, bool, usize, Option<Span>),
	BlockAfter(&'hir hir::Block<'hir>, bool, bool, usize),
	BlockEnd(bool),
	Sequence(Children<'hir>, usize, Span),
	SequenceAfter(Children<'hir>, usize, Span),
	Diverge,
	Converge,
	PopBreak,
	For(&'hir hir::ExprFor<'hir>, Span),
	IfCondition(Branches<'hir>),
	IfConditionAfter(Branches<'hir>),
	IfBodies(Branches<'hir>),
	IfBodyAfter(Branches<'hir>),
	MatchValue(Match<'hir>),
	MatchPattern(Match<'hir>),
	MatchGuard(Match<'hir>),
	MatchBodies(Match<'hir>),
	MatchBodyAfter(Match<'hir>),
	Template(&'hir hir::BuiltInTemplate<'hir>, usize, Span),
	TemplateAfter(&'hir hir::BuiltInTemplate<'hir>, usize, Span),
	Conditional(hir::ExprId, Span),
}

/// The legacy pattern walk classifies the root pattern, not its bindings.
fn refutable(pat: &hir::Pat<'_>) -> bool {
	!matches!(
		pat.kind,
		hir::PatKind::Ignore | hir::PatKind::Path(&hir::PatPathKind::Ident { .. })
	)
}

fn condition(condition: &hir::Condition<'_>) -> hir::ExprId {
	match condition {
		hir::Condition::Expr(_, expr) => *expr,
		hir::Condition::ExprLet(expr) => expr.expr,
	}
}

pub(crate) fn function(
	exprs: &hir::Exprs<'_>,
	hir: &hir::ItemFn<'_>,
	context: Span,
	source: SourceId,
	diagnostics: &mut Diagnostics,
) -> alloc::Result<()> {
	for hir::FnArg::Pat(pat) in hir.args {
		if refutable(&pat.pat) {
			diagnostics.let_pattern_might_panic(source, pat, Some(&context))?;
		}
	}
	walk(
		exprs,
		Work::Block(&hir.body, hir.body.value.is_some(), true),
		source,
		diagnostics,
	)
}

pub(crate) fn closure(
	exprs: &hir::Exprs<'_>,
	hir: &hir::ExprClosure<'_>,
	source: SourceId,
	diagnostics: &mut Diagnostics,
) -> alloc::Result<()> {
	for hir::FnArg::Pat(pat) in hir.args {
		if refutable(&pat.pat) {
			diagnostics.let_pattern_might_panic(source, pat, Some(&hir.span))?;
		}
	}
	walk(
		exprs,
		Work::Expr(exprs.get(hir.body), true, hir.span),
		source,
		diagnostics,
	)
}

fn walk<'hir>(
	exprs: &'hir hir::Exprs<'hir>,
	first: Work<'hir>,
	source: SourceId,
	diagnostics: &mut Diagnostics,
) -> alloc::Result<()> {
	let mut work = Vec::new();
	let mut breaks = Vec::<Break<'hir>>::new();
	let mut diverging = false;
	work.try_push(first)?;
	while let Some(next) = work.pop() {
		match next {
			Work::Expr(expr, needed, context) => {
				diverging = false;
				let small = |a, b, c| Work::Sequence(Children::Small([a, b, c]), 0, context);
				match expr.kind {
					hir::ExprKind::Lit(..)
					| hir::ExprKind::Const(..)
					| hir::ExprKind::CallClosure(..) => {
						if !needed {
							diagnostics.not_used(source, expr, Some(&context))?;
						}
					}
					hir::ExprKind::Variable(..)
					| hir::ExprKind::Type(..)
					| hir::ExprKind::Fn(..)
					| hir::ExprKind::Static(..)
					| hir::ExprKind::AsyncBlock(..) => {}
					hir::ExprKind::Group(id) => {
						work.try_push(Work::Expr(exprs.get(id), needed, context))?
					}
					hir::ExprKind::Block(block) => {
						work.try_push(Work::Block(block, needed, false))?
					}
					hir::ExprKind::Return(value) => {
						work.try_push(Work::Diverge)?;
						if let Some(id) = value {
							work.try_push(Work::Expr(exprs.get(id), true, context))?;
						}
					}
					// Legacy expr_continue emits a jump but reports convergence.
					hir::ExprKind::Continue(..) => {}
					hir::ExprKind::Break(value) => {
						let needed = breaks
							.iter()
							.rev()
							.find(|b| match value.label {
								Some(label) => b.label == Some(label),
								None => b.is_loop,
							})
							.is_some_and(|b| b.needed);
						work.try_push(Work::Diverge)?;
						if let Some(id) = value.expr {
							work.try_push(Work::Expr(exprs.get(id), needed, context))?;
						}
					}
					hir::ExprKind::Yield(value) => {
						if let Some(id) = value {
							work.try_push(Work::Expr(exprs.get(id), true, context))?;
						}
					}
					hir::ExprKind::Await(id) | hir::ExprKind::Try(id) => {
						work.try_push(Work::Expr(exprs.get(id), true, context))?
					}
					hir::ExprKind::Unary(unary) => {
						work.try_push(Work::Expr(exprs.get(unary.expr), true, context))?
					}
					hir::ExprKind::Format(format) => {
						work.try_push(Work::Expr(exprs.get(format.value), needed, context))?
					}
					hir::ExprKind::FieldAccess(field) => {
						work.try_push(Work::Expr(exprs.get(field.expr), true, context))?
					}
					hir::ExprKind::Index(index) => {
						work.try_push(small(Some(index.target), Some(index.index), None))?
					}
					hir::ExprKind::Binary(binary) if binary.op.is_conditional() => {
						work.try_push(Work::Conditional(binary.rhs, context))?;
						work.try_push(Work::Expr(exprs.get(binary.lhs), true, context))?;
					}
					hir::ExprKind::Binary(binary) if binary.op.is_assign() => {
						work.try_push(assignment(exprs, binary.lhs, binary.rhs, context))?;
					}
					hir::ExprKind::Binary(binary) => {
						work.try_push(small(Some(binary.lhs), Some(binary.rhs), None))?
					}
					hir::ExprKind::Assign(assign) => {
						work.try_push(assignment(exprs, assign.lhs, assign.rhs, context))?
					}
					hir::ExprKind::Tuple(seq) | hir::ExprKind::Vec(seq) => {
						work.try_push(Work::Sequence(Children::Slice(seq.items), 0, context))?
					}
					hir::ExprKind::Object(object) => work.try_push(Work::Sequence(
						Children::Object(object.assignments),
						0,
						context,
					))?,
					hir::ExprKind::Call(call) => {
						work.try_push(Work::Sequence(Children::Call(call), 0, context))?
					}
					hir::ExprKind::Range(range) => {
						let (a, b) = match *range {
							hir::ExprRange::RangeFrom { start } => (Some(start), None),
							hir::ExprRange::RangeFull => (None, None),
							hir::ExprRange::Range { start, end }
							| hir::ExprRange::RangeInclusive { start, end } => (Some(start), Some(end)),
							hir::ExprRange::RangeTo { end }
							| hir::ExprRange::RangeToInclusive { end } => (Some(end), None),
						};
						work.try_push(small(a, b, None))?;
					}
					hir::ExprKind::Template(template) => {
						work.try_push(Work::Template(template, 0, context))?
					}
					hir::ExprKind::Loop(loop_) => {
						breaks.try_push(Break {
							label: loop_.label,
							needed,
							is_loop: true,
						})?;
						work.try_push(Work::PopBreak)?;
						work.try_push(Work::Converge)?;
						work.try_push(Work::Block(&loop_.body, false, false))?;
						if let Some(c) = loop_.condition {
							work.try_push(Work::Expr(exprs.get(condition(c)), true, context))?;
						}
					}
					hir::ExprKind::For(for_) => {
						work.try_push(Work::For(for_, context))?;
						work.try_push(Work::Expr(exprs.get(for_.iter), true, context))?;
					}
					hir::ExprKind::If(if_) => work.try_push(Work::IfCondition(Branches {
						hir: if_,
						at: 0,
						bodies: Vec::new(),
						needed,
						context,
						all_diverge: false,
					}))?,
					hir::ExprKind::Match(match_) => {
						work.try_push(Work::MatchValue(Match {
							hir: match_,
							at: 0,
							bodies: 0,
							irrefutable: false,
							needed,
							context,
							all_diverge: false,
						}))?;
						work.try_push(Work::Expr(exprs.get(match_.expr), true, context))?;
					}
					// The collector refuses select before this admitted-body walk.
					hir::ExprKind::Select(..) => {}
				}
			}
			Work::Diverge => diverging = true,
			Work::Converge => diverging = false,
			Work::PopBreak => {
				breaks.pop();
			}
			Work::Conditional(rhs, context) => {
				if !diverging {
					work.try_push(Work::Converge)?;
					work.try_push(Work::Expr(exprs.get(rhs), true, context))?;
				}
			}
			Work::Sequence(ids, at, context) => {
				diverging = false;
				if let Some(id) = ids.get(at) {
					work.try_push(Work::SequenceAfter(ids, at + 1, context))?;
					work.try_push(Work::Expr(exprs.get(id), true, context))?;
				}
			}
			Work::SequenceAfter(ids, at, context) => {
				if !diverging {
					work.try_push(Work::Sequence(ids, at, context))?;
				}
			}
			Work::Block(block, needed, root) => {
				if block.label.is_some() {
					breaks.try_push(Break {
						label: block.label,
						needed,
						is_loop: false,
					})?;
					work.try_push(Work::PopBreak)?;
				}
				work.try_push(Work::BlockEnd(block.label.is_some()))?;
				work.try_push(Work::BlockNext(block, needed, root, 0, None))?;
			}
			Work::BlockEnd(labeled) => {
				if labeled {
					diverging = false;
				}
			}
			Work::BlockNext(block, needed, root, at, cause) => {
				if let Some(stmt) = block.statements.get(at) {
					if let Some(cause) = cause {
						diagnostics.unreachable(source, stmt, &cause)?;
						work.try_push(Work::BlockNext(block, needed, root, at + 1, Some(cause)))?;
					} else {
						work.try_push(Work::BlockAfter(block, needed, root, at))?;
						let (id, needed) = match stmt {
							hir::Stmt::Local(local) => (
								local.expr,
								!root || !matches!(local.pat.pat.kind, hir::PatKind::Ignore),
							),
							hir::Stmt::Expr(_, id) => (*id, false),
						};
						work.try_push(Work::Expr(exprs.get(id), needed, block.span))?;
					}
				} else if let Some(id) = block.value {
					if let Some(cause) = cause {
						diagnostics.unreachable(source, exprs.get(id), &cause)?;
						diverging = true;
					} else {
						work.try_push(Work::Expr(exprs.get(id), needed, block.span))?;
					}
				} else {
					diverging = cause.is_some();
				}
			}
			Work::BlockAfter(block, needed, root, at) => {
				let stmt = &block.statements[at];
				if let hir::Stmt::Local(local) = stmt {
					// The legacy root-pattern path stops at its unbound-name error
					// after a diverging container load. Preserve only its warnings
					// before that point; the execution frontend still admits it.
					if root && diverging && refutable(&local.pat.pat) && !local.pat.names.is_empty()
					{
						break;
					}
				}
				if !diverging {
					if let hir::Stmt::Local(local) = stmt {
						if refutable(&local.pat.pat) {
							diagnostics.let_pattern_might_panic(
								source,
								&local.pat,
								Some(&block.span),
							)?;
						}
					}
				}
				work.try_push(Work::BlockNext(
					block,
					needed,
					root,
					at + 1,
					diverging.then(|| stmt.span()),
				))?;
			}
			Work::For(for_, context) => {
				if diverging {
					diagnostics.unreachable(source, &for_.body, exprs.get(for_.iter))?;
				} else {
					if refutable(&for_.binding.pat) {
						diagnostics.let_pattern_might_panic(
							source,
							&for_.binding,
							Some(&context),
						)?;
					}
					breaks.try_push(Break {
						label: for_.label,
						needed: false,
						is_loop: true,
					})?;
					work.try_push(Work::PopBreak)?;
					work.try_push(Work::Converge)?;
					work.try_push(Work::Block(&for_.body, false, false))?;
				}
			}
			Work::Template(template, mut at, context) => {
				while template.exprs.get(at).is_some_and(|id| {
					matches!(exprs.get(*id).kind, hir::ExprKind::Lit(hir::Lit::Str(_)))
				}) {
					at += 1;
				}
				if let Some(id) = template.exprs.get(at) {
					work.try_push(Work::TemplateAfter(template, at + 1, context))?;
					work.try_push(Work::Expr(exprs.get(*id), true, context))?;
				} else {
					if template.from_literal
						&& template.exprs.iter().all(|id| {
							matches!(exprs.get(*id).kind, hir::ExprKind::Lit(hir::Lit::Str(_)))
						}) {
						diagnostics.template_without_expansions(
							source,
							template,
							Some(&context),
						)?;
					}
					diverging = false;
				}
			}
			Work::TemplateAfter(template, at, context) => {
				if !diverging {
					work.try_push(Work::Template(template, at, context))?;
				}
			}
			Work::IfCondition(mut state) => {
				if let Some(branch) = state.hir.branches.get(state.at) {
					let id = condition(branch.condition);
					let context = state.context;
					state.at += 1;
					work.try_push(Work::IfConditionAfter(state))?;
					work.try_push(Work::Expr(exprs.get(id), true, context))?;
				} else {
					state.at = 0;
					state.all_diverge = state.bodies.is_empty();
					if let Some(fallback) = state.hir.fallback {
						let needed = state.needed;
						work.try_push(Work::IfBodyAfter(state))?;
						work.try_push(Work::Block(fallback, needed, false))?;
					} else {
						work.try_push(Work::IfBodies(state))?;
					}
				}
			}
			Work::IfConditionAfter(mut state) => {
				if !diverging {
					state.bodies.try_push(&state.hir.branches[state.at - 1])?;
				}
				work.try_push(Work::IfCondition(state))?;
			}
			Work::IfBodies(mut state) => {
				if let Some(branch) = state.bodies.get(state.at).copied() {
					state.at += 1;
					let needed = state.needed && state.hir.fallback.is_some();
					work.try_push(Work::IfBodyAfter(state))?;
					work.try_push(Work::Block(&branch.block, needed, false))?;
				} else {
					diverging = state.all_diverge;
				}
			}
			Work::IfBodyAfter(mut state) => {
				if state.at == 0 {
					state.all_diverge = diverging;
				} else {
					state.all_diverge &= diverging;
				}
				work.try_push(Work::IfBodies(state))?;
			}
			Work::MatchValue(state) => {
				if !diverging {
					work.try_push(Work::MatchPattern(state))?;
				}
			}
			Work::MatchPattern(mut state) => {
				if !state.irrefutable {
					if let Some(branch) = state.hir.branches.get(state.at) {
						state.at += 1;
						if let Some(guard) = branch.condition {
							let context = state.context;
							work.try_push(Work::MatchGuard(state))?;
							work.try_push(Work::Expr(exprs.get(guard), true, context))?;
							continue;
						}
						state.bodies += 1;
						state.irrefutable = !refutable(&branch.pat.pat);
						work.try_push(Work::MatchPattern(state))?;
						continue;
					}
				}
				state.at = 0;
				state.all_diverge = state.irrefutable;
				work.try_push(Work::MatchBodies(state))?;
			}
			Work::MatchGuard(mut state) => {
				if diverging {
					state.irrefutable = !refutable(&state.hir.branches[state.at - 1].pat.pat);
				} else {
					state.bodies += 1;
				}
				work.try_push(Work::MatchPattern(state))?;
			}
			Work::MatchBodies(mut state) => {
				if state.at < state.bodies {
					let branch = &state.hir.branches[state.at];
					state.at += 1;
					let (needed, context) = (state.needed, state.context);
					work.try_push(Work::MatchBodyAfter(state))?;
					work.try_push(Work::Expr(exprs.get(branch.body), needed, context))?;
				} else {
					diverging = state.all_diverge;
				}
			}
			Work::MatchBodyAfter(mut state) => {
				state.all_diverge &= diverging;
				work.try_push(Work::MatchBodies(state))?;
			}
		}
	}
	Ok(())
}

fn assignment<'hir>(
	exprs: &hir::Exprs<'hir>,
	lhs: hir::ExprId,
	rhs: hir::ExprId,
	context: Span,
) -> Work<'hir> {
	let ids = match exprs.get(lhs).kind {
		hir::ExprKind::FieldAccess(field) => [Some(field.expr), Some(rhs), None],
		hir::ExprKind::Index(index) => [Some(index.target), Some(index.index), Some(rhs)],
		_ => [Some(rhs), None, None],
	};
	Work::Sequence(Children::Small(ids), 0, context)
}

#[cfg(test)]
mod tests;
