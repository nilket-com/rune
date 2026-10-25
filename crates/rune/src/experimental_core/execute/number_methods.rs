//! C17c numeric calls. Operands were evaluated in source order.
use super::*;
use crate::function::number::{Method, Pair, Receiver};

impl Executor {
	/// A provider id calls its stock provider; a method id first selects the
	/// provider by the receiver's run-time type.
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn number_call(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let (pair, method) = match Pair::from_id(id) {
			Some(pair) => (Some(pair), pair.method()),
			None => (None, Method::from_dynamic_id(id).ok_or_else(invalid)?),
		};
		if count != method.arity() || out >= function.slots {
			return Err(invalid());
		}
		let end = start.checked_add(count).ok_or_else(invalid)?;
		let args = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let mut values = [Value::Unit; 2];
		for (value, &slot) in values.iter_mut().zip(args) {
			if slot >= function.slots {
				return Err(invalid());
			}
			*value = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let values = &values[..count as usize];
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		let pair = match pair {
			Some(pair) => pair,
			None => {
				let receiver = match values[0] {
					Value::I64(_) => Some(Receiver::I64),
					Value::U64(_) => Some(Receiver::U64),
					Value::F64(_) => Some(Receiver::F64),
					Value::Char(_) => Some(Receiver::Char),
					_ => None,
				};
				match receiver.and_then(|r| Pair::new(method, r)) {
					Some(pair) => pair,
					None => return Err(missing_method(code, store, values[0], method)),
				}
			}
		};
		let result = store.number(pair, values)?;
		self.put(store, destination, result)
	}
}

/// The stock missing-method error; the compile proved no receiver of this
/// type provides the name.
fn missing_method(code: &Code, store: &Store, receiver: Value, method: Method) -> Error {
	let invalid = || error(ErrorKind::InvalidCode);
	let Some(target) = store.diagnostic_type(receiver) else {
		return invalid();
	};
	let instance = match target {
		TypeName::Struct(n) => super::super::nominal::hash_copy(n).map(crate::Hash::new),
		_ => target.builtin_hash(),
	};
	let Some(instance) = instance else {
		return invalid();
	};
	let hash = crate::Hash::associated_function(instance, method.hash());
	if !code.absent_method(hash) {
		return invalid();
	}
	error(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
		target,
		instance,
		hash,
	})
}
