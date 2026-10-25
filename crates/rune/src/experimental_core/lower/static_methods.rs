//! Script instance-method calls whose receiver type is statically certain.
use super::*;

/// A script instance function that a method call statically resolves to. Only
/// [`Lower::static_method`] builds one, from a receiver whose type is certain.
pub(super) struct StaticMethod {
	/// `Hash::associated_function(receiver type, name)`, where stock looks it up.
	pub(super) associated: Hash,
	/// The function's item hash.
	pub(super) item: Hash,
}

impl Lower<'_, '_> {
	/// The script method `target.name(..)` calls, when `target`'s type is
	/// certain and that type has a script instance function of the plain name.
	pub(super) fn static_method(
		&self,
		target: hir::ExprId,
		hash: Hash,
		name_hash: Hash,
	) -> Option<StaticMethod> {
		// Only a name some script impl defines can have a script target.
		if hash != name_hash || !self.constants.method_script.closed(hash) {
			return None;
		}
		let receiver = self.receiver_type(target, 0)?;
		let associated = Hash::associated_function(receiver, hash);
		let item = self.constants.instances.get(associated)?;
		Some(StaticMethod { associated, item })
	}

	/// The script type every value of `id` has: a constructor call, a struct or
	/// variant literal, a block, `if` or `match` whose every value arm has that
	/// type, or a local whose initializer and every assignment have it.
	fn receiver_type(&self, id: hir::ExprId, depth: usize) -> Option<Hash> {
		if depth >= 128 {
			return None;
		}
		match self.exprs.get(id).kind {
			hir::ExprKind::Group(id) => self.receiver_type(id, depth + 1),
			hir::ExprKind::Variable(v) => {
				let hash = self.receiver_type(self.text_init(v)?, depth + 1)?;
				self.text_writes(v, |rhs| self.receiver_type(rhs, depth + 1) == Some(hash))
					.then_some(hash)
			}
			hir::ExprKind::Call(call) => {
				let hir::Call::Meta { hash } = call.call else {
					return None;
				};
				self.nominal_type(hash)
			}
			hir::ExprKind::Object(object) => {
				let hir::ExprObjectKind::Struct { hash } = object.kind else {
					return None;
				};
				self.nominal_type(hash)
			}
			hir::ExprKind::Block(block) => self.block_type(block, depth),
			hir::ExprKind::If(conditional) => {
				let hash = self.block_type(conditional.fallback?, depth)?;
				conditional
					.branches
					.iter()
					.all(|branch| self.block_type(&branch.block, depth) == Some(hash))
					.then_some(hash)
			}
			hir::ExprKind::Match(m) => {
				let (first, rest) = m.branches.split_first()?;
				let hash = self.receiver_type(first.body, depth + 1)?;
				rest.iter()
					.all(|branch| self.receiver_type(branch.body, depth + 1) == Some(hash))
					.then_some(hash)
			}
			_ => None,
		}
	}

	/// A labeled block can also produce a value through `break 'label`.
	fn block_type(&self, block: &hir::Block<'_>, depth: usize) -> Option<Hash> {
		if block.label.is_some() {
			return None;
		}
		self.receiver_type(block.value?, depth + 1)
	}

	/// Lowers the call as a direct script call with the receiver first, after
	/// stock's evaluation order: the receiver, then the arguments left to right.
	#[cold]
	#[inline(never)]
	pub(super) fn call_static_method(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		method: StaticMethod,
		out: u32,
		span: Span,
	) -> Result<()> {
		let at = self.location(span);
		self.transaction
			.static_method(self.context, method.associated)
			.map_err(|e| e.at(at))?;
		let function = (self.resolve)(Resolve::Script(method.item), at).map_err(|e| e.at(at))?;
		let mut ids = Vec::try_with_capacity(call.args.len() + 1)?;
		ids.try_push(target)?;
		for id in call.args {
			ids.try_push(*id)?;
		}
		let args = self.operands(&ids)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		let call = self.transaction.script_call(function, args_len, at)?;
		self.emit(call.op(out, args_start), span)?;
		Ok(())
	}
}
