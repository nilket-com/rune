//! Named-field facts are an optional second phase over an independent graph.
//! A dispatch seed proves a tag, never the contents of an associated receiver.
use super::*;
use crate::runtime::TypeHash;

const NAME_BYTES: usize = 4 << 20;

struct Field {
	nominal: Hash,
	name: String,
}

struct Write {
	nominal: Option<Hash>,
	receiver: Option<Id>,
	key: Option<String>,
	value: Option<Id>,
}

struct Read {
	node: Id,
	receiver: Id,
	key: String,
}

#[derive(Default)]
pub(super) struct Fields {
	pub(super) disabled: bool,
	count: usize,
	bytes: usize,
	work: usize,
	fields: Vec<Field>,
	stores: Vec<Write>,
	reads: Vec<Read>,
	objects: Vec<Id>,
	associated: Vec<(Id, Id, Hash)>,
}

impl Fields {
	fn scan(&mut self, work: usize) -> Result<()> {
		if work > LIMIT.saturating_sub(self.work) {
			return Err(limit());
		}
		self.work += work;
		Ok(())
	}

	fn charge(&mut self, bytes: usize) -> Result<()> {
		if self.count >= LIMIT || bytes > NAME_BYTES.saturating_sub(self.bytes) {
			return Err(limit());
		}
		self.count += 1;
		self.bytes += bytes;
		Ok(())
	}

	pub(super) fn associated(&mut self, seed: Id, actual: Id, nominal: Hash) -> Result<()> {
		if !self.disabled {
			self.charge(0)?;
			self.associated.try_push((seed, actual, nominal))?;
		}
		Ok(())
	}
}

impl Builder<'_, '_, '_> {
	// All body terms already exist. No field summary adds nodes to phase one.
	fn field_term(&self, id: hir::ExprId) -> Option<Id> {
		self.body.expressions.get(id.index()).copied().flatten()
	}

	pub(super) fn fields(&mut self) -> Result<()> {
		for i in 0..self.pending.len() {
			let id = self.pending[i];
			let expr = self.exprs.get(id);
			match expr.kind {
				hir::ExprKind::Object(object) => match object.kind {
					hir::ExprObjectKind::Anonymous => {
						if let Some(node) = self.field_term(id) {
							self.proofs.fields.charge(0)?;
							self.proofs.fields.objects.try_push(node)?;
						}
					}
					hir::ExprObjectKind::Struct { hash } => {
						// Enum variants never share their parent enum's field domain.
						let Some(shape) = self.query.struct_shape(hash) else {
							if self.query.variant_shape(hash).is_some() {
								continue;
							}
							// A missing early declaration must not make a conflicting
							// construction disappear from a later body's field join.
							return Err(limit());
						};
						let crate::compile::meta::Fields::Named(named) = shape.fields else {
							continue;
						};
						if named.fields.len() != object.assignments.len() {
							return Err(limit());
						}
						for field in named.fields.iter() {
							self.proofs.fields.scan(self.proofs.fields.fields.len())?;
							if !self
								.proofs
								.fields
								.fields
								.iter()
								.any(|f| f.nominal == hash && f.name == field.name.as_ref())
							{
								self.proofs.fields.charge(field.name.len())?;
								self.proofs.fields.fields.try_push(Field {
									nominal: hash,
									name: String::try_from(field.name.as_ref())?,
								})?;
							}
						}
						for assignment in object.assignments {
							let value = self.field_term(assignment.assign);
							self.proofs.fields.charge(assignment.key.1.len())?;
							self.proofs.fields.stores.try_push(Write {
								nominal: Some(hash),
								receiver: None,
								key: Some(String::try_from(assignment.key.1)?),
								value,
							})?;
						}
					}
					hir::ExprObjectKind::ExternalType { .. } => (),
				},
				hir::ExprKind::FieldAccess(field) => {
					if let (hir::ExprField::Ident(key), Some(node), Some(receiver)) = (
						field.expr_field,
						self.field_term(id),
						self.field_term(field.expr),
					) {
						self.proofs.fields.charge(key.len())?;
						self.proofs.fields.reads.try_push(Read {
							node,
							receiver,
							key: String::try_from(key)?,
						})?;
					}
				}
				hir::ExprKind::Assign(assign) => self.field_store(assign.lhs, Some(assign.rhs))?,
				hir::ExprKind::Binary(binary) if binary.op.is_assign() => {
					self.field_store(binary.lhs, None)?
				}
				_ => (),
			}
		}
		Ok(())
	}

	fn field_store(&mut self, lhs: hir::ExprId, rhs: Option<hir::ExprId>) -> Result<()> {
		let Some((receiver, key)) = field_target(self.exprs, lhs) else {
			return Ok(());
		};
		let receiver = receiver.and_then(|id| self.field_term(id));
		let value = rhs.and_then(|id| self.field_term(id));
		self.proofs.fields.charge(key.map_or(0, str::len))?;
		self.proofs.fields.stores.try_push(Write {
			nominal: None,
			receiver,
			key: key.map(String::try_from).transpose()?,
			value,
		})?;
		Ok(())
	}
}

// None is reserved for known local or tuple storage. Any unmodeled target,
// including one beyond the grouping bound, poisons every named-field domain.
fn field_target<'hir>(
	exprs: &hir::Exprs<'hir>,
	mut lhs: hir::ExprId,
) -> Option<(Option<hir::ExprId>, Option<&'hir str>)> {
	for _ in 0..128 {
		let hir::ExprKind::Group(inner) = exprs.get(lhs).kind else {
			break;
		};
		lhs = inner;
	}
	match exprs.get(lhs).kind {
		hir::ExprKind::Variable(_) => None,
		hir::ExprKind::FieldAccess(field) => match field.expr_field {
			hir::ExprField::Ident(key) => Some((Some(field.expr), Some(key))),
			hir::ExprField::Index(_) => None,
			_ => Some((Some(field.expr), None)),
		},
		hir::ExprKind::Index(index) => {
			let mut key = index.index;
			for _ in 0..128 {
				let hir::ExprKind::Group(inner) = exprs.get(key).kind else {
					break;
				};
				key = inner;
			}
			let key = match exprs.get(key).kind {
				hir::ExprKind::Lit(hir::Lit::Str(key)) => Some(key),
				_ => None,
			};
			Some((Some(index.target), key))
		}
		_ => Some((None, None)),
	}
}

impl Proofs {
	fn copy_graph(&self) -> Result<Self> {
		let mut copy = Self::default();
		for node in &self.nodes {
			let id = copy.node(node.kind)?;
			for edge in &node.edges {
				copy.edge(id, edge.node, edge.forward)?;
			}
		}
		Ok(copy)
	}

	fn projection(&mut self, receiver: Id, value: Id) -> Result<Id> {
		let node = self.node(Kind::Projection)?;
		self.edge(node, receiver, false)?;
		self.edge(node, value, false)?;
		Ok(node)
	}

	pub(super) fn field_graph(&self, old: &[Fact]) -> Result<Option<Self>> {
		if self.fields.disabled || self.fields.reads.is_empty() {
			return Ok(None);
		}
		// Bound even scans that find no matching field; node/edge budgets alone
		// would not bound a quadratic series of failed name comparisons.
		if self
			.fields
			.fields
			.len()
			.checked_mul(self.fields.stores.len() + self.fields.reads.len())
			.is_none_or(|work| work > LIMIT)
		{
			return Err(limit());
		}
		let mut graph = self.copy_graph()?;
		// Object-disjointness is independently proven. It does not change phase
		// one's nominal-only facts or any old site's proof obligations.
		for &node in &self.fields.objects {
			graph.nodes[node].kind = Kind::Exact(crate::runtime::Object::HASH);
		}
		let objects = graph.solve_graph()?;
		let mut poisoned = Vec::new();
		for &(seed, actual, nominal) in &self.fields.associated {
			// Even an impossible type edge carries an entry obligation: a host
			// must not enter its caller with a different nominal parameter.
			graph.edge(seed, actual, false)?;
			match old[actual] {
				Fact::Exact(hash) if hash == nominal => (),
				Fact::Exact(_) => graph.nodes[seed].kind = Kind::Never,
				_ if objects[actual] == Fact::Exact(crate::runtime::Object::HASH) => {
					graph.nodes[seed].kind = Kind::Never
				}
				_ => {
					graph.nodes[seed].kind = Kind::Unknown;
					if !poisoned.contains(&nominal) {
						poisoned.try_push(nominal)?;
					}
				}
			}
		}
		let mut fields = Vec::new();
		for field in &self.fields.fields {
			let joined = graph.node(if poisoned.contains(&field.nominal) {
				Kind::Unknown
			} else {
				Kind::Join
			})?;
			let obligations = graph.node(Kind::Unknown)?;
			fields.try_push(graph.projection(obligations, joined)?)?;
			for store in &self.fields.stores {
				if store.key.as_ref().is_some_and(|key| key != &field.name) {
					continue;
				}
				// Excluding an Object or another nominal is itself a proof.
				// Preserve its receiver dependencies even though its value does
				// not participate in this field's type join.
				if let Some(receiver) = store.receiver {
					graph.edge(obligations, receiver, false)?;
				}
				let target = store
					.nominal
					.map(Fact::Exact)
					.or_else(|| store.receiver.map(|id| old[id]))
					.unwrap_or(Fact::Unknown);
				match target {
					Fact::Exact(hash) if hash != field.nominal => continue,
					_ if store.receiver.is_some_and(|id| {
						objects[id] == Fact::Exact(crate::runtime::Object::HASH)
					}) =>
					{
						continue
					}
					_ => (),
				}
				let value = if target == Fact::Exact(field.nominal) && store.key.is_some() {
					match store.value {
						Some(id) => id,
						None => graph.node(Kind::Unknown)?,
					}
				} else {
					graph.node(Kind::Unknown)?
				};
				let source = match store.receiver {
					Some(receiver) => graph.projection(receiver, value)?,
					None => value,
				};
				graph.edge(joined, source, false)?;
			}
		}
		for read in &self.fields.reads {
			let Fact::Exact(nominal) = old[read.receiver] else {
				continue;
			};
			let Some(index) = self
				.fields
				.fields
				.iter()
				.position(|f| f.nominal == nominal && f.name == read.key)
			else {
				continue;
			};
			graph.nodes[read.node].kind = Kind::Projection;
			graph.edge(read.node, read.receiver, false)?;
			graph.edge(read.node, fields[index], false)?;
		}
		// Every capture, including one hidden in an actual associated receiver,
		// poisons a dependent field read. A returned closure can be called through
		// a relay after its captured record was mutated by the host.
		let mut reverse = Vec::new();
		reverse.try_resize_with(graph.nodes.len(), Vec::<Id>::new)?;
		for (id, node) in graph.nodes.iter().enumerate() {
			for edge in &node.edges {
				reverse[edge.node].try_push(id)?;
			}
		}
		let mut captured = Vec::new();
		captured.try_resize(graph.nodes.len(), false)?;
		let mut work = Vec::new();
		for body in &self.bodies {
			for &id in &body.parameters[..body.captures] {
				if !captured[id] {
					captured[id] = true;
					work.try_push(id)?;
				}
			}
		}
		while let Some(id) = work.pop() {
			for &parent in &reverse[id] {
				if !captured[parent] {
					captured[parent] = true;
					work.try_push(parent)?;
				}
			}
		}
		for read in &self.fields.reads {
			if captured[read.node] {
				graph.nodes[read.node].kind = Kind::Unknown;
			}
		}

		Ok(Some(graph))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn grouped_and_unmodeled_write_targets_cannot_disappear() {
		let (dot, bracket, tuple);
		let mut exprs = hir::Exprs::new();
		let span = crate::ast::Span::empty();
		let variable = hir::Variable(core::num::NonZeroU32::new(1).unwrap().into());
		let receiver = exprs
			.insert(hir::Expr {
				span,
				kind: hir::ExprKind::Variable(variable),
			})
			.unwrap();
		let key = exprs
			.insert(hir::Expr {
				span,
				kind: hir::ExprKind::Lit(hir::Lit::Str("f")),
			})
			.unwrap();
		dot = hir::ExprFieldAccess {
			expr: receiver,
			expr_field: hir::ExprField::Ident("f"),
		};
		bracket = hir::ExprIndex {
			target: receiver,
			index: key,
		};
		let dot_id = exprs
			.insert(hir::Expr {
				span,
				kind: hir::ExprKind::FieldAccess(&dot),
			})
			.unwrap();
		let bracket_id = exprs
			.insert(hir::Expr {
				span,
				kind: hir::ExprKind::Index(&bracket),
			})
			.unwrap();
		// Both plain and compound stores use this classification. The stock
		// assembler currently rejects a grouped lvalue; exercise the HIR layer
		// directly so that frontend rejection cannot hide a missing summary.
		for mut target in [dot_id, bracket_id] {
			for _ in 0..128 {
				target = exprs
					.insert(hir::Expr {
						span,
						kind: hir::ExprKind::Group(target),
					})
					.unwrap();
				assert_eq!(
					field_target(&exprs, target),
					Some((Some(receiver), Some("f")))
				);
			}
			target = exprs
				.insert(hir::Expr {
					span,
					kind: hir::ExprKind::Group(target),
				})
				.unwrap();
			assert_eq!(field_target(&exprs, target), Some((None, None)));
		}
		tuple = hir::ExprFieldAccess {
			expr: receiver,
			expr_field: hir::ExprField::Index(0),
		};
		let tuple_id = exprs
			.insert(hir::Expr {
				span,
				kind: hir::ExprKind::FieldAccess(&tuple),
			})
			.unwrap();
		assert_eq!(field_target(&exprs, receiver), None);
		assert_eq!(field_target(&exprs, tuple_id), None);
		assert_eq!(field_target(&exprs, key), Some((None, None)));
	}
}
