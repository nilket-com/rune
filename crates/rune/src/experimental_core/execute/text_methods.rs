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
		if id >= 0x8000_0070 {
			return self.pipeline_call(store, f, base, out, id, start, len);
		}
		let m = crate::function::text::Method::ALL
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

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	fn pipeline_call(
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
		let end = start.checked_add(len).ok_or_else(invalid)?;
		let args = f
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		if args.len() > 2 || out >= f.slots {
			return Err(invalid());
		}
		let mut v = [Value::Unit; 2];
		for (dst, &slot) in v.iter_mut().zip(args) {
			if slot >= f.slots {
				return Err(invalid());
			}
			*dst = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let result = match id {
			0x8000_0070 => store.pipeline_chars(v[0])?,
			0x8000_0071 => store.pipeline_unwrap(v[0], v[1])?,
			0x8000_0072 | 0x8000_0073 => store.pipeline_adapter(v[0], v[1], id == 0x8000_0073)?,
			0x8000_0075 => {
				let next = store.iterator_next(v[0])?;
				let result = store.variant(super::super::value::Type::Option, next.is_some(), next);
				if let Some(n) = next {
					store.release(n)?;
				}
				result?
			}
			0x8000_0078 => store.pipeline_child(v[0])?,
			0x8000_0079 => store.pipeline_remaining(v[0], false)?,
			0x8000_007a => store.pipeline_remaining(v[0], true)?,
			0x8000_007b => store.pipeline_append(v[0], v[1])?,
			_ => return Err(invalid()),
		};
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}
