//! Exact stock Vec constructor identities on the cold Call path.
use super::super::code;
use super::*;

pub(super) fn constructor(hash: Hash) -> Option<bool> {
	for (name, item, capacity) in [
		("new", crate::hash_in!(crate, ::std::vec::Vec::new), false),
		(
			"with_capacity",
			crate::hash_in!(crate, ::std::vec::Vec::with_capacity),
			true,
		),
	] {
		if hash == item
			|| hash
				== Hash::associated_function(
					crate::hash_in!(crate, ::std::vec::Vec),
					Hash::ident(name),
				) {
			return Some(capacity);
		}
	}
	None
}

impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn call_vec_constructor(
		&mut self,
		call: &hir::ExprCall<'_>,
		capacity: bool,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.fresh_vec_constructor(capacity) {
			return Err(self.refused(span, Refusal::Native));
		}
		if call.args.len() != usize::from(capacity) {
			return Err(self.refused(span, Refusal::Arity));
		}
		let args = self.operands(call.args)?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		self.emit(
			Op::Call {
				out,
				function: if capacity {
					code::VEC_WITH_CAPACITY
				} else {
					code::VEC_NEW
				},
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
