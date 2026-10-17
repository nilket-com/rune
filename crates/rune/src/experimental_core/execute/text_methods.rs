//! Cold, fixed-arity string intrinsics. No runtime routing into legacy handlers.
use super::*;
impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn text_call(
		&mut self,
		store: &mut Store,
		f: &super::super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let m = crate::function::fresh_text::Method::ALL
			.into_iter()
			.find(|m| m.id() == id)
			.ok_or_else(invalid)?;
		if len != m.arity() || out >= f.slots {
			return Err(invalid());
		}
		let end = start.checked_add(len).ok_or_else(invalid)?;
		let args = f
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let mut values = [Value::Unit; 3];
		for (v, &slot) in values.iter_mut().zip(args) {
			if slot >= f.slots {
				return Err(invalid());
			}
			*v = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let result = store.text_method(m, &values[..len as usize])?;
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}
