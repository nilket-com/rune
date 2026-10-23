//! Owned compile-time provenance. No HIR borrow survives the handoff.
//!
//! The HIR handoff copies each body's expression dependencies and call edges.
//! A body owns its return expressions; a shared closure arena supplies writes
//! and function-value uses, never another body's returns. Fields stay Unknown.
//!
//! Binding facts join every initializer and assignment by variable identity.
//! Parameter facts join all script incoming edges. Genuine opaque closures
//! supply their captured binding facts; unknown function-value ingress poisons
//! parameter facts. Consuming a parameter or capture fact denies direct host
//! entry to exactly the bodies whose arguments the proof assumes away.
//!
//! Finish solves dependency components on heap work stacks. A recursive component
//! can use external nominal seeds only when every member is an unmodified
//! forwarding parameter. Return cycles and other dependency cycles stay Unknown.
//! Deferred calls publish only after target, Context absence and arity checks.
//!
//! Collection has fixed node, edge and expression-slot budgets. Exceeding one
//! disables the new proofs and retains the original located Method refusal;
//! programs that use only existing lowering keep their admission and code.
use super::*;
use crate::hir;

const LIMIT: usize = 262_144;
type Id = usize;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fact {
	Unknown,
	Never,
	Exact(Hash),
}

#[derive(Clone, Copy)]
enum Kind {
	Unknown,
	Never,
	Exact(Hash),
	Join,
	Parameter(Hash),
	Return,
	End,
	Call(Hash),
}

struct Edge {
	node: Id,
	forward: bool,
}

struct Node {
	kind: Kind,
	edges: Vec<Edge>,
}

struct Body {
	hash: Hash,
	associated: Option<Hash>,
	parameters: Vec<Id>,
	captures: usize,
	returns: Id,
	expressions: Vec<Option<Id>>,
}

struct Call {
	target: Hash,
	arguments: Vec<Id>,
	captures_only: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Method {
	pub(crate) receiver: Hash,
	pub(crate) associated: Hash,
	pub(crate) item: Hash,
}

pub(crate) struct Site {
	pub(crate) diagnostic_owner: Hash,
	pub(crate) diagnostic_span: Option<crate::ast::Span>,
	pub(crate) diagnostic_checkpoint: usize,
	pub(crate) body: Hash,
	pub(crate) receiver: Id,
	pub(crate) methods: Vec<Method>,
	pub(crate) at: Location,
	pub(crate) instruction: Option<(u32, u32, u32, u32)>,
}

pub(crate) struct Failure {
	pub(crate) error: Error,
	pub(crate) checkpoint: usize,
	pub(crate) span: crate::ast::Span,
}

#[derive(Default)]
pub(crate) struct Proofs {
	pub(crate) failures: Vec<Failure>,
	resolution_error: Option<Error>,
	pub(crate) diagnostic_owner: Hash,
	pub(crate) diagnostic_span: Option<crate::ast::Span>,
	pub(crate) diagnostic_checkpoint: usize,
	nodes: Vec<Node>,
	edges: usize,
	expression_slots: usize,
	pub(crate) disabled: bool,
	checked: bool,
	bodies: Vec<Body>,
	calls: Vec<Call>,
	source_uses: Vec<Hash>,
	method_cache: crate::hash::Map<Vec<Method>>,
	method_count: usize,
	arenas: Vec<Hash>,
	pub(crate) sites: Vec<Site>,
	pub(crate) failure_after: Option<usize>,
}

fn limit() -> Error {
	Error::refused(Refusal::CodeLimit)
}

fn join(a: Fact, b: Fact) -> Fact {
	match (a, b) {
		(Fact::Never, b) => b,
		(a, Fact::Never) => a,
		(Fact::Exact(a), Fact::Exact(b)) if a == b => Fact::Exact(a),
		_ => Fact::Unknown,
	}
}

fn nominal(query: &crate::query::QueryInner<'_>, hash: Hash) -> Option<Hash> {
	query
		.fresh_variant_shape(hash)
		.map(|shape| shape.enum_hash)
		.or_else(|| query.fresh_struct_shape(hash).map(|_| hash))
}

pub(crate) fn methods(
	query: &crate::query::QueryInner<'_>,
	pool: &Pool,
	name: Hash,
) -> Result<Vec<Method>> {
	let mut out = Vec::new();
	if !query.fresh_instances.complete() {
		return Ok(out);
	}
	for receiver in query.items.keys().map(|id| pool.item_type_hash(*id)) {
		let associated = Hash::associated_function(receiver, name);
		let Some(item) = query.fresh_instances.get(associated) else {
			continue;
		};
		if query.fresh_self_method(item, receiver) != Some(associated)
			|| out.iter().any(|m: &Method| m.associated == associated)
		{
			continue;
		}
		if out.len() >= 65_536 {
			return Err(limit());
		}
		out.try_push(Method {
			receiver,
			associated,
			item,
		})?;
	}
	Ok(out)
}

impl Proofs {
	pub(crate) fn methods(
		&mut self,
		query: &crate::query::QueryInner<'_>,
		pool: &Pool,
		name: Hash,
	) -> Result<Vec<Method>> {
		if !self.method_cache.contains_key(&name) {
			let found = methods(query, pool, name)?;
			if found.len() > LIMIT.saturating_sub(self.method_count)
				|| self.method_cache.len() >= LIMIT
			{
				return Err(limit());
			}
			self.method_count += found.len();
			self.method_cache.try_insert(name, found)?;
		}
		let cached = self.method_cache.get(&name).unwrap();
		let mut copy = Vec::try_with_capacity(cached.len())?;
		for &method in cached {
			copy.try_push(method)?;
		}
		Ok(copy)
	}

	fn node(&mut self, kind: Kind) -> Result<Id> {
		if self.nodes.len() >= LIMIT {
			return Err(limit());
		}
		let id = self.nodes.len();
		self.nodes.try_push(Node {
			kind,
			edges: Vec::new(),
		})?;
		Ok(id)
	}

	fn edge(&mut self, target: Id, source: Id, forward: bool) -> Result<()> {
		if self.edges >= LIMIT {
			return Err(limit());
		}
		self.edges += 1;
		self.nodes[target].edges.try_push(Edge {
			node: source,
			forward,
		})?;
		Ok(())
	}

	pub(crate) fn expression(&self, body: Hash, expr: hir::ExprId) -> Option<Id> {
		self.bodies
			.iter()
			.find(|b| b.hash == body)?
			.expressions
			.get(expr.fresh_index())
			.copied()
			.flatten()
	}

	pub(crate) fn observe(
		&mut self,
		arena: Hash,
		hash: Hash,
		receiver: Option<Hash>,
		immediate: bool,
		exprs: &hir::Exprs<'_>,
		item: &hir::ItemFn<'_>,
		captures: &[hir::Variable],
		query: &crate::query::QueryInner<'_>,
		pool: &Pool,
		context: &Context,
	) -> Result<()> {
		if self.bodies.iter().any(|body| body.hash == hash) {
			return Ok(());
		}
		if !self.arenas.contains(&arena) {
			if self.arenas.len() >= 65_536 {
				return Err(limit());
			}
			self.arenas.try_push(arena)?;
			for expr in exprs.fresh_expressions() {
				if let hir::ExprKind::Fn(hash) = expr.kind {
					if self.source_uses.len() >= LIMIT {
						return Err(limit());
					}
					self.source_uses.try_push(hash)?;
				}
			}
		}
		let count = exprs.fresh_expressions().count();
		if count > LIMIT.saturating_sub(self.expression_slots) {
			return Err(limit());
		}
		self.expression_slots += count;
		let mut builder = Builder {
			proofs: self,
			exprs,
			query,
			pool,
			context,
			body: Body {
				hash,
				associated: receiver.and_then(|receiver| query.fresh_self_method(hash, receiver)),
				parameters: Vec::new(),
				captures: captures.len(),
				returns: 0,
				expressions: Vec::new(),
			},
			owned: Vec::new(),
			bindings: Vec::new(),
			binding_indices: crate::alloc::HashMap::default(),
			pending: Vec::new(),
			returns: Vec::new(),
			return_unknown: !immediate,
		};
		builder.body.expressions.try_resize(count, None)?;
		builder.owned.try_resize(count, false)?;
		for &variable in captures {
			builder.parameter(variable)?;
		}
		for arg in item.args {
			let hir::FnArg::Pat(pat) = arg;
			let id = builder.proofs.node(Kind::Parameter(hash))?;
			builder.body.parameters.try_push(id)?;
			for &variable in pat.names {
				let input = if simple(&pat.pat) == Some(variable) {
					id
				} else {
					builder.proofs.node(Kind::Unknown)?
				};
				builder.bind(variable, input, None)?;
			}
		}
		builder.block(&item.body)?;
		builder.walk()?;
		// Allocate every binding's final node before resolving any initializer.
		// This makes aliases independent of the order in which the tree was walked.
		let mut assignments: crate::alloc::HashMap<hir::Variable, (Vec<hir::ExprId>, bool)> =
			crate::alloc::HashMap::default();
		for expr in exprs.fresh_expressions() {
			let (variable, rhs) = match expr.kind {
				hir::ExprKind::Assign(a) => (variable_at(exprs, a.lhs), Some(a.rhs)),
				hir::ExprKind::Binary(b) if b.op.is_assign() => (variable_at(exprs, b.lhs), None),
				_ => continue,
			};
			let Some(variable) = variable else { continue };
			if !assignments.contains_key(&variable) {
				assignments.try_insert(variable, (Vec::new(), false))?;
			}
			let (writes, poisoned) = assignments.get_mut(&variable).unwrap();
			if let Some(rhs) = rhs {
				writes.try_push(rhs)?;
			} else {
				*poisoned = true;
			}
		}
		let mut inputs = Vec::new();
		for i in 0..builder.bindings.len() {
			let (variable, input, init) = builder.bindings[i];
			let (writes, poisoned) = assignments.remove(&variable).unwrap_or_default();
			let joined = init.is_some() || !writes.is_empty() || poisoned;
			if joined {
				builder.bindings[i].1 = builder.proofs.node(Kind::Join)?;
			}
			inputs.try_push((input, init, writes, poisoned, joined))?;
		}
		for (i, (input, init, writes, poisoned, joined)) in inputs.into_iter().enumerate() {
			if !joined {
				continue;
			}
			let target = builder.bindings[i].1;
			let initial = match init {
				Some(init) => builder.term(init, 0)?,
				None => input,
			};
			builder.proofs.edge(target, initial, false)?;
			for rhs in writes {
				let source = builder.term(rhs, 0)?;
				builder.proofs.edge(target, source, false)?;
			}
			if poisoned {
				let unknown = builder.proofs.node(Kind::Unknown)?;
				builder.proofs.edge(target, unknown, false)?;
			}
		}
		let returns = builder.proofs.node(if builder.return_unknown {
			Kind::Unknown
		} else {
			Kind::Return
		})?;
		builder.body.returns = returns;
		let tail = builder.block_term(&item.body, 0)?;
		builder.proofs.edge(returns, tail, false)?;
		for i in 0..builder.returns.len() {
			let id = builder.returns[i];
			let source = match id {
				Some(id) => builder.term(id, 0)?,
				None => builder.proofs.node(Kind::Unknown)?,
			};
			builder.proofs.edge(returns, source, false)?;
		}
		// Walking each body's tree rather than its shared arena keeps a closure's
		// calls and returns out of its enclosing function's summary.
		builder.calls()?;
		builder.proofs.bodies.try_push(builder.body)?;
		Ok(())
	}

	fn connect(&mut self) -> Result<()> {
		for i in 0..self.nodes.len() {
			if let Kind::Call(hash) = self.nodes[i].kind {
				if let Some(body) = self.bodies.iter().find(|body| body.hash == hash) {
					let target = body.returns;
					self.nodes[i].kind = Kind::Join;
					self.edge(i, target, false)?;
				} else {
					self.nodes[i].kind = Kind::Unknown;
				}
			}
		}
		let calls = core::mem::take(&mut self.calls);
		for call in calls {
			let Some(body) = self.bodies.iter().find(|body| body.hash == call.target) else {
				continue;
			};
			let expected = if call.captures_only {
				body.captures
			} else {
				body.parameters.len()
			};
			if call.arguments.len() != expected {
				continue;
			}
			let mut parameters = Vec::new();
			parameters.try_extend_from_slice(&body.parameters)?;
			for (parameter, argument) in parameters.into_iter().zip(call.arguments) {
				let forward = matches!(self.nodes[argument].kind, Kind::Parameter(_));
				self.edge(parameter, argument, forward)?;
			}
		}
		for body in &self.bodies {
			if self.source_uses.contains(&body.hash)
				|| body
					.associated
					.is_some_and(|hash| self.source_uses.contains(&hash))
			{
				for &parameter in &body.parameters {
					self.nodes[parameter].kind = Kind::Unknown;
				}
			}
		}
		Ok(())
	}
}

fn simple(pat: &hir::Pat<'_>) -> Option<hir::Variable> {
	match pat.kind {
		hir::PatKind::Path(hir::PatPathKind::Ident { variable, .. }) => Some(*variable),
		_ => None,
	}
}

fn variable_at(exprs: &hir::Exprs<'_>, mut id: hir::ExprId) -> Option<hir::Variable> {
	for _ in 0..128 {
		match exprs.get(id).kind {
			hir::ExprKind::Group(inner) => id = inner,
			hir::ExprKind::Variable(v) => return Some(v),
			_ => return None,
		}
	}
	None
}

struct Builder<'a, 'hir, 'query> {
	proofs: &'a mut Proofs,
	exprs: &'a hir::Exprs<'hir>,
	query: &'a crate::query::QueryInner<'query>,
	pool: &'a Pool,
	context: &'a Context,
	body: Body,
	owned: Vec<bool>,
	bindings: Vec<(hir::Variable, Id, Option<hir::ExprId>)>,
	binding_indices: crate::alloc::HashMap<hir::Variable, usize>,
	pending: Vec<hir::ExprId>,
	returns: Vec<Option<hir::ExprId>>,
	return_unknown: bool,
}

impl Builder<'_, '_, '_> {
	fn bind(&mut self, variable: hir::Variable, node: Id, init: Option<hir::ExprId>) -> Result<()> {
		let index = self.bindings.len();
		self.bindings.try_push((variable, node, init))?;
		self.binding_indices.try_insert(variable, index)?;
		Ok(())
	}

	fn binding(&self, variable: hir::Variable) -> Option<Id> {
		self.binding_indices
			.get(&variable)
			.map(|&index| self.bindings[index].1)
	}

	fn parameter(&mut self, variable: hir::Variable) -> Result<()> {
		let id = self.proofs.node(Kind::Parameter(self.body.hash))?;
		self.body.parameters.try_push(id)?;
		self.bind(variable, id, None)?;
		Ok(())
	}

	fn local(&mut self, pat: &hir::PatBinding<'_>, init: Option<hir::ExprId>) -> Result<()> {
		for &variable in pat.names {
			let id = self.proofs.node(Kind::Unknown)?;
			self.bind(
				variable,
				id,
				if simple(&pat.pat) == Some(variable) {
					init
				} else {
					None
				},
			)?;
		}
		Ok(())
	}

	fn block(&mut self, block: &hir::Block<'_>) -> Result<()> {
		if block.label.is_some() {
			self.return_unknown = true;
		}
		for statement in block.statements {
			match statement {
				hir::Stmt::Local(local) => {
					self.local(&local.pat, Some(local.expr))?;
					self.pending.try_push(local.expr)?;
				}
				hir::Stmt::Expr(_, id) => self.pending.try_push(*id)?,
			}
		}
		if let Some(id) = block.value {
			self.pending.try_push(id)?;
		}
		Ok(())
	}

	fn condition(&mut self, condition: &hir::Condition<'_>) -> Result<()> {
		match condition {
			hir::Condition::Expr(_, id) => self.pending.try_push(*id)?,
			hir::Condition::ExprLet(local) => {
				self.local(&local.pat, None)?;
				self.pending.try_push(local.expr)?;
			}
		}
		Ok(())
	}

	fn walk(&mut self) -> Result<()> {
		// Retain the visited IDs as a separate list after the heap work stack drains.
		let mut visited = Vec::new();
		while let Some(id) = self.pending.pop() {
			if self.owned[id.fresh_index()] {
				continue;
			}
			self.owned[id.fresh_index()] = true;
			visited.try_push(id)?;
			use hir::ExprKind as E;
			match self.exprs.get(id).kind {
				E::Assign(a) => {
					self.pending.try_push(a.lhs)?;
					self.pending.try_push(a.rhs)?;
				}
				E::Binary(b) => {
					self.pending.try_push(b.lhs)?;
					self.pending.try_push(b.rhs)?;
				}
				E::Unary(u) => self.pending.try_push(u.expr)?,
				E::Group(id) | E::Await(id) => self.pending.try_push(id)?,
				E::Try(id) => {
					self.return_unknown = true;
					self.pending.try_push(id)?;
				}
				E::Return(value) => {
					self.returns.try_push(value)?;
					if let Some(id) = value {
						self.pending.try_push(id)?;
					}
				}
				E::Yield(value) => {
					self.return_unknown = true;
					if let Some(id) = value {
						self.pending.try_push(id)?;
					}
				}
				E::Block(block) => self.block(block)?,
				E::Loop(loop_) => {
					if loop_.label.is_some() {
						self.return_unknown = true;
					}
					if let Some(c) = loop_.condition {
						self.condition(c)?;
					}
					self.block(&loop_.body)?;
				}
				E::For(loop_) => {
					self.local(&loop_.binding, None)?;
					self.pending.try_push(loop_.iter)?;
					self.block(&loop_.body)?;
				}
				E::If(c) => {
					for b in c.branches {
						self.condition(b.condition)?;
						self.block(&b.block)?;
					}
					if let Some(b) = c.fallback {
						self.block(b)?;
					}
				}
				E::Match(m) => {
					self.pending.try_push(m.expr)?;
					for b in m.branches {
						self.local(&b.pat, None)?;
						if let Some(c) = b.condition {
							self.pending.try_push(c)?;
						}
						self.pending.try_push(b.body)?;
					}
				}
				E::Call(call) => {
					match call.call {
						hir::Call::Associated { target, .. } => self.pending.try_push(target)?,
						hir::Call::Expr { expr } => self.pending.try_push(expr)?,
						_ => (),
					}
					self.pending.try_extend_from_slice(call.args)?;
				}
				E::FieldAccess(field) => self.pending.try_push(field.expr)?,
				E::Index(index) => {
					self.pending.try_push(index.target)?;
					self.pending.try_push(index.index)?;
				}
				E::Object(object) => {
					for field in object.assignments {
						self.pending.try_push(field.assign)?;
					}
				}
				E::Tuple(seq) | E::Vec(seq) => self.pending.try_extend_from_slice(seq.items)?,
				E::Break(b) => {
					if b.label.is_some() || b.expr.is_some() {
						self.return_unknown = true;
					}
					if let Some(id) = b.expr {
						self.pending.try_push(id)?;
					}
				}
				E::Select(s) => {
					self.return_unknown = true;
					self.pending.try_extend_from_slice(s.exprs)?;
					for b in s.branches {
						self.local(&b.pat, None)?;
						self.pending.try_push(b.body)?;
					}
					if let Some(id) = s.default {
						self.pending.try_push(id)?;
					}
				}
				E::Template(t) => self.pending.try_extend_from_slice(t.exprs)?,
				E::Format(f) => self.pending.try_push(f.value)?,
				E::Range(range) => {
					use hir::ExprRange as R;
					match *range {
						R::RangeFrom { start } => self.pending.try_push(start)?,
						R::RangeTo { end } | R::RangeToInclusive { end } => {
							self.pending.try_push(end)?
						}
						R::Range { start, end } | R::RangeInclusive { start, end } => {
							self.pending.try_push(start)?;
							self.pending.try_push(end)?;
						}
						R::RangeFull => (),
					}
				}
				E::Variable(_)
				| E::Type(_)
				| E::Fn(_)
				| E::AsyncBlock(_)
				| E::CallClosure(_)
				| E::Lit(_)
				| E::Continue(_)
				| E::Const(_)
				| E::Static(_) => (),
			}
		}
		self.pending = visited;
		Ok(())
	}

	fn block_term(&mut self, block: &hir::Block<'_>, depth: usize) -> Result<Id> {
		if block.label.is_some() {
			return self.proofs.node(Kind::Unknown);
		}
		if let Some(id) = block.value {
			return self.term(id, depth + 1);
		}
		if let Some(hir::Stmt::Expr(_, id)) = block.statements.last() {
			let value = self.term(*id, depth + 1)?;
			let end = self.proofs.node(Kind::End)?;
			self.proofs.edge(end, value, false)?;
			return Ok(end);
		}
		self.proofs.node(Kind::Unknown)
	}

	fn term(&mut self, id: hir::ExprId, depth: usize) -> Result<Id> {
		if depth >= 128 || !self.owned[id.fresh_index()] {
			return self.proofs.node(Kind::Unknown);
		}
		if let Some(node) = self.body.expressions[id.fresh_index()] {
			return Ok(node);
		}
		let placeholder = self.proofs.node(Kind::Unknown)?;
		self.body.expressions[id.fresh_index()] = Some(placeholder);
		use hir::ExprKind as E;
		let node = match self.exprs.get(id).kind {
			E::Variable(v) => self.binding(v).unwrap_or(placeholder),
			E::Group(inner) => self.term(inner, depth + 1)?,
			E::Call(call) => match call.call {
				hir::Call::Meta { hash } if self.context.lookup_function(hash).is_none() => {
					self.proofs.node(
						nominal(self.query, hash)
							.map(Kind::Exact)
							.unwrap_or(Kind::Call(hash)),
					)?
				}
				_ => placeholder,
			},
			E::Object(object) => match object.kind {
				hir::ExprObjectKind::Struct { hash } => self.proofs.node(
					nominal(self.query, hash)
						.map(Kind::Exact)
						.unwrap_or(Kind::Unknown),
				)?,
				_ => placeholder,
			},
			E::Block(block) => self.block_term(block, depth + 1)?,
			E::If(c) => {
				let joined = self.proofs.node(Kind::Join)?;
				for branch in c.branches {
					let value = self.block_term(&branch.block, depth + 1)?;
					self.proofs.edge(joined, value, false)?;
				}
				let tail = match c.fallback {
					Some(block) => self.block_term(block, depth + 1)?,
					None => self.proofs.node(Kind::Unknown)?,
				};
				self.proofs.edge(joined, tail, false)?;
				joined
			}
			E::Match(m) => {
				let joined = self.proofs.node(Kind::Join)?;
				for branch in m.branches {
					let value = self.term(branch.body, depth + 1)?;
					self.proofs.edge(joined, value, false)?;
				}
				joined
			}
			E::Return(_) => self.proofs.node(Kind::Never)?,
			_ => placeholder,
		};
		self.body.expressions[id.fresh_index()] = Some(node);
		Ok(node)
	}

	fn calls(&mut self) -> Result<()> {
		for i in 0..self.pending.len() {
			let id = self.pending[i];
			self.term(id, 0)?;
			match self.exprs.get(id).kind {
				hir::ExprKind::Call(call) => {
					let mut arguments = Vec::new();
					for &argument in call.args {
						arguments.try_push(self.term(argument, 0)?)?;
					}
					match call.call {
						hir::Call::Meta { hash }
							if self.context.lookup_function(hash).is_none() =>
						{
							self.proofs.calls.try_push(Call {
								target: hash,
								arguments,
								captures_only: false,
							})?
						}
						hir::Call::Associated {
							hash, name_hash, ..
						} if hash == name_hash && self.query.fresh_method_script.closed(hash) => {
							for method in self.proofs.methods(self.query, self.pool, hash)? {
								if self.context.lookup_function(method.associated).is_some()
									|| self.context.lookup_function(method.item).is_some()
								{
									continue;
								}
								let mut inputs = Vec::new();
								inputs.try_push(self.proofs.node(Kind::Exact(method.receiver))?)?;
								inputs.try_extend_from_slice(&arguments)?;
								self.proofs.calls.try_push(Call {
									target: method.item,
									arguments: inputs,
									captures_only: false,
								})?;
							}
						}
						_ => (),
					}
				}
				hir::ExprKind::CallClosure(closure) => {
					let mut arguments = Vec::new();
					for &variable in closure.captures {
						let node = match self.binding(variable) {
							Some(node) => node,
							None => self.proofs.node(Kind::Unknown)?,
						};
						arguments.try_push(node)?;
					}
					self.proofs.calls.try_push(Call {
						target: closure.hash,
						arguments,
						captures_only: true,
					})?;
				}
				_ => (),
			}
		}
		Ok(())
	}
}

impl Proofs {
	pub(crate) fn ready_to_seal(&self) -> bool {
		self.sites.is_empty()
			|| (self.checked && self.failure_after.is_none() && self.resolution_error.is_none())
	}

	/// Solve dependency components on heap work stacks. Only an SCC made solely
	/// of unmodified parameter-forwarding edges can use external nominal seeds.
	fn solve(&mut self) -> Result<Vec<Fact>> {
		self.connect()?;
		let count = self.nodes.len();
		let mut reverse = Vec::new();
		reverse.try_resize_with(count, Vec::<Id>::new)?;
		for (i, node) in self.nodes.iter().enumerate() {
			for edge in &node.edges {
				reverse[edge.node].try_push(i)?;
			}
		}
		let mut seen = Vec::new();
		seen.try_resize(count, false)?;
		let mut order = Vec::new();
		let mut stack = Vec::new();
		for start in 0..count {
			if seen[start] {
				continue;
			}
			seen[start] = true;
			stack.try_push((start, 0usize))?;
			while let Some((node, cursor)) = stack.last_mut() {
				if let Some(edge) = self.nodes[*node].edges.get(*cursor) {
					*cursor += 1;
					if !seen[edge.node] {
						seen[edge.node] = true;
						stack.try_push((edge.node, 0))?;
					}
				} else {
					let (node, _) = stack.pop().unwrap();
					order.try_push(node)?;
				}
			}
		}
		let mut component = Vec::new();
		component.try_resize(count, usize::MAX)?;
		let mut groups = Vec::new();
		let mut work = Vec::new();
		for &start in order.iter().rev() {
			if component[start] != usize::MAX {
				continue;
			}
			let group = groups.len();
			let mut members = Vec::new();
			component[start] = group;
			work.try_push(start)?;
			while let Some(node) = work.pop() {
				members.try_push(node)?;
				for &parent in &reverse[node] {
					if component[parent] == usize::MAX {
						component[parent] = group;
						work.try_push(parent)?;
					}
				}
			}
			groups.try_push(members)?;
		}
		let mut facts = Vec::new();
		facts.try_resize_with(count, || Fact::Unknown)?;
		for (group, members) in groups.iter().enumerate().rev() {
			let cyclic = members.len() > 1
				|| self.nodes[members[0]]
					.edges
					.iter()
					.any(|e| e.node == members[0]);
			if cyclic {
				let mut value = Fact::Never;
				let mut seeded = false;
				let mut allowed = true;
				for &id in members {
					allowed &= matches!(self.nodes[id].kind, Kind::Parameter(_));
					for edge in &self.nodes[id].edges {
						if component[edge.node] == group {
							allowed &= edge.forward;
						} else {
							value = join(value, facts[edge.node]);
							seeded = true;
						}
					}
				}
				if !allowed || !seeded {
					value = Fact::Unknown;
				}
				for &id in members {
					facts[id] = value;
				}
			} else {
				let id = members[0];
				let node = &self.nodes[id];
				facts[id] = match node.kind {
					Kind::Unknown | Kind::Call(_) => Fact::Unknown,
					Kind::Never => Fact::Never,
					Kind::Exact(hash) => Fact::Exact(hash),
					Kind::End => {
						if node
							.edges
							.first()
							.is_some_and(|e| facts[e.node] == Fact::Never)
						{
							Fact::Never
						} else {
							Fact::Unknown
						}
					}
					Kind::Join | Kind::Parameter(_) | Kind::Return => {
						if node.edges.is_empty() {
							Fact::Unknown
						} else {
							node.edges
								.iter()
								.fold(Fact::Never, |v, e| join(v, facts[e.node]))
						}
					}
				};
			}
		}
		Ok(facts)
	}

	pub(crate) fn resolve(
		&mut self,
		entries: &mut [(Hash, Option<Function>, Option<Location>)],
		transaction: &mut CompileTransaction,
		context: &Context,
		ineligible: &mut Vec<u32>,
	) -> Result<()> {
		if self.checked {
			return self.resolution_error.map_or(Ok(()), Err);
		}
		if self.sites.is_empty() {
			return Ok(());
		}
		let facts = if self.disabled {
			None
		} else {
			match self.solve() {
				Err(error) if error.refusal == Some(Refusal::CodeLimit) => None,
				result => Some(result?),
			}
		};
		let mut used = Vec::new();
		used.try_resize(self.nodes.len(), false)?;
		let mut work = Vec::new();
		let mut failed_owners = crate::hash::Map::<()>::default();
		let mut first = None;
		for (index, site) in self.sites.iter().enumerate() {
			if failed_owners.contains_key(&site.diagnostic_owner) {
				continue;
			}
			let result = (|| {
				let Some(facts) = &facts else {
					return Err(Error::refused(Refusal::Method).at(site.at));
				};
				let Fact::Exact(receiver) = facts[site.receiver] else {
					return Err(Error::refused(Refusal::Method).at(site.at));
				};
				let Some(method) = site
					.methods
					.iter()
					.find(|method| method.receiver == receiver)
				else {
					return Err(Error::refused(Refusal::Method).at(site.at));
				};
				transaction
					.static_method(context, method.associated)
					.map_err(|e| e.at(site.at))?;
				if context.lookup_function(method.item).is_some() {
					return Err(Error::refused(Refusal::Native).at(site.at));
				}
				// A direct item call also carries its Context absence obligation.
				transaction
					.static_method(context, method.item)
					.map_err(|e| e.at(site.at))?;

				let callee = entries
					.iter()
					.position(|(hash, _, _)| *hash == method.item)
					.ok_or_else(|| Error::refused(Refusal::Method).at(site.at))?
					as u32;
				if let Some((pc, out, args_start, args_len)) = site.instruction {
					let target = self
						.bodies
						.iter()
						.find(|b| b.hash == method.item)
						.ok_or_else(|| Error::refused(Refusal::Method).at(site.at))?;
					if target.parameters.len() != args_len as usize {
						return Err(Error::refused(Refusal::Arity).at(site.at));
					}
					let caller = entries
						.iter_mut()
						.find(|(hash, _, _)| *hash == site.body)
						.and_then(|(_, function, _)| function.as_mut());
					if let Some(caller) = caller {
						let certified = transaction.script_call(callee, args_len, site.at)?;
						caller.patch(pc, certified.op(out, args_start))?;
					}
				}
				work.try_push(site.receiver)?;
				while let Some(node) = work.pop() {
					if used[node] {
						continue;
					}
					used[node] = true;
					if let Kind::Parameter(body) = self.nodes[node].kind {
						let id = entries
							.iter()
							.position(|(hash, _, _)| *hash == body)
							.ok_or_else(|| {
								Error::new(ErrorKind::InvalidCode, "receiver proof entry")
							})? as u32;
						if !ineligible.contains(&id) {
							ineligible.try_push(id)?;
						}
					}
					for edge in &self.nodes[node].edges {
						work.try_push(edge.node)?;
					}
				}
				Ok(())
			})();
			if let Err(error) = result {
				let error = error.at(site.at);
				failed_owners.try_insert(site.diagnostic_owner, ())?;
				self.failures.try_push(Failure {
					error,
					checkpoint: site.diagnostic_checkpoint,
					span: site
						.diagnostic_span
						.unwrap_or_else(|| crate::ast::Span::new(site.at.start, site.at.end)),
				})?;
				if first.is_none() && self.failure_after.is_none_or(|end| index < end) {
					first = Some(error);
				}
			}
		}

		self.resolution_error = first;
		self.checked = true;
		first.map_or(Ok(()), Err)
	}
}
