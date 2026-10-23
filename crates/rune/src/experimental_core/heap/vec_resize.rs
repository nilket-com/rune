//! Certified resize keeps stock's borrow window and committed clone prefix.
use super::super::error::{Detail, HostCause, MovedAccess, TypeName};
use super::*;

impl Store {
	#[cold]
	#[inline(never)]
	pub(crate) fn vec_resize(&mut self, receiver: Value, length: Value, fill: Value) -> Result<()> {
		if self.tag(receiver)? != Type::Vec {
			return Err(
				err(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
					cause: HostCause::ExpectedType,
					arg: 0,
					expected: TypeName::Vec,
					found: self
						.diagnostic_type(receiver)
						.ok_or_else(|| err(ErrorKind::InvalidCode))?,
				}),
			);
		}
		self.argument(receiver, 0, MovedAccess::Write)?;
		let Value::Heap(parent) = receiver else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let new_len = self.vec_capacity_argument(length).map_err(|mut error| {
			match &mut error.detail {
				Detail::NumericArgumentNumber { arg, .. }
				| Detail::NumericArgumentCoerce { arg, .. } => *arg = 1,
				_ => {}
			}
			error
		})?;
		let len = self.len(receiver)?;
		if new_len < len {
			let mut values = self.resize_hold(parent);
			let mut failure = None;
			// Detach first so even a malformed child cannot restore released slots.
			for old in values.drain(new_len..) {
				self.edges -= Self::heap_edge(old);
				if let Err(error) = self.release(old) {
					if failure.is_none() {
						failure = Some(error);
					}
				}
			}
			self.resize_restore(parent, values);
			return failure.map_or(Ok(()), Err);
		}
		if new_len == len {
			return Ok(());
		}
		if !matches!(fill, Value::Heap(_) | Value::Function { .. }) {
			return self.resize_inline(parent, new_len, fill);
		}
		for _ in len..new_len {
			let values = self.resize_hold(parent);
			let cloned = self.clone_value(fill);
			self.resize_restore(parent, values);
			let cloned = cloned?;
			let pushed = self.push(receiver, cloned);
			let released = self.release(cloned);
			pushed?;
			released?;
		}
		Ok(())
	}

	fn resize_inline(&mut self, parent: Handle, new_len: usize, fill: Value) -> Result<()> {
		if new_len > isize::MAX as usize / size_of::<crate::runtime::Value>() {
			return Err(err(ErrorKind::Allocation).with_detail(Detail::CapacityOverflow));
		}
		let Payload::List { values, .. } = self.mutable(parent)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		if values.capacity() >= new_len {
			while values.len() < new_len {
				values
					.push_within_capacity(fill)
					.map_err(|_| err(ErrorKind::InvalidCode))?;
			}
			return Ok(());
		}
		let planned = new_len
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		self.replacement_bytes(parent, planned)?;
		let mut replacement = Vec::try_with_capacity(new_len)?;
		let charge = replacement
			.capacity()
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		let bytes = self.replacement_bytes(parent, charge)?;
		let Payload::List { values, .. } = self.payload(parent)? else {
			return Err(err(ErrorKind::InvalidCode));
		};
		for &value in values {
			replacement.try_push(value)?;
		}
		while replacement.len() < new_len {
			replacement.try_push(fill)?;
		}
		// Existing child edges transfer only after the complete reservation succeeds.
		let entry = self.entry_mut(parent)?;
		entry.payload = Some(Payload::List {
			tag: Type::Vec,
			values: replacement,
		});
		entry.set_charged_bytes(charge);
		self.bytes = bytes;
		self.assert_work();
		Ok(())
	}

	fn resize_hold(&mut self, parent: Handle) -> Vec<Value> {
		let entry = self.entry_mut(parent).expect("live resize receiver");
		let Some(Payload::List {
			tag: Type::Vec,
			values,
		}) = entry.payload.replace(Payload::Moved(Type::Vec))
		else {
			unreachable!()
		};
		entry.readers = sorting::EXCLUSIVE;
		values
	}

	fn resize_restore(&mut self, parent: Handle, values: Vec<Value>) {
		// Certified clones cannot call user code or consume the receiver's root.
		let entry = self.entry_mut(parent).expect("live resize receiver");
		entry.payload = Some(Payload::List {
			tag: Type::Vec,
			values,
		});
		entry.readers = 0;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resize_shrink_drains_the_tail_after_an_invalid_child() {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.list(Type::Vec, &[]).unwrap();
		let v = s
			.list(Type::Vec, &[Value::I64(7), Value::Unit, child])
			.unwrap();
		s.release(child).unwrap();
		let stale = s.list(Type::Vec, &[]).unwrap();
		s.release(stale).unwrap();
		s.reserve_work(1, 0).unwrap();
		let Value::Heap(parent) = v else {
			unreachable!()
		};
		let Payload::List { values, .. } = s.mutable(parent).unwrap() else {
			unreachable!()
		};
		values[1] = stale;
		s.edges += 1;
		assert_eq!(
			s.vec_resize(v, Value::I64(1), Value::Unit)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidHandle
		);
		assert_eq!(s.len(v).unwrap(), 1);
		assert_eq!(s.get(v, 0).unwrap(), Value::I64(7));
		assert_eq!(s.live_entries(), 1);
		s.release(v).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}

	#[test]
	fn resize_self_fill_restores_the_exclusive_receiver() {
		let mut s = Store::new(Limits::default()).unwrap();
		let v = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		for wrapped in [false, true] {
			let fill = if wrapped {
				s.variant(Type::Option, true, Some(v)).unwrap()
			} else {
				s.retain(v).unwrap();
				v
			};
			let before = (s.live_entries(), s.payload_bytes());
			assert_eq!(
				s.vec_resize(v, Value::I64(3), fill).unwrap_err().detail,
				Detail::ReadExclusive
			);
			assert_eq!(s.len(v).unwrap(), 1);
			assert_eq!((s.live_entries(), s.payload_bytes()), before);
			s.release(fill).unwrap();
		}
		s.vec_resize(v, Value::I64(1), v).unwrap();
		s.vec_resize(v, Value::I64(0), v).unwrap();
		s.release(v).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}

	#[test]
	fn resize_clones_outer_vectors_and_releases_shrunk_slots() {
		let mut s = Store::new(Limits::default()).unwrap();
		let fill = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let v = s.list(Type::Vec, &[]).unwrap();
		s.vec_resize(v, Value::I64(2), fill).unwrap();
		let a = s.get(v, 0).unwrap();
		let b = s.get(v, 1).unwrap();
		assert_ne!(a, b);
		assert_ne!(a, fill);
		s.push(a, Value::I64(9)).unwrap();
		assert_eq!(
			(s.len(fill).unwrap(), s.len(a).unwrap(), s.len(b).unwrap()),
			(1, 2, 1)
		);
		s.release(a).unwrap();
		s.release(b).unwrap();
		s.vec_resize(v, Value::I64(0), Value::Unit).unwrap();
		assert_eq!(s.live_entries(), 2);
		s.release(v).unwrap();
		s.release(fill).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}

	#[test]
	fn resize_inline_reservation_failure_keeps_original_contents() {
		let mut s = Store::new(Limits {
			entries: 8,
			bytes: 4 * size_of::<Value>(),
		})
		.unwrap();
		let v = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let bytes = s.payload_bytes();
		assert_eq!(
			s.vec_resize(v, Value::I64(8), Value::Unit)
				.unwrap_err()
				.kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!((s.len(v).unwrap(), s.payload_bytes()), (1, bytes));
		s.release(v).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}

	#[test]
	fn resize_entry_limit_preserves_the_successful_clone_prefix() {
		let mut s = Store::new(Limits {
			entries: 5,
			bytes: 4096,
		})
		.unwrap();
		let fill = s.variant(Type::Option, false, None).unwrap();
		let v = s.list(Type::Vec, &[]).unwrap();
		assert_eq!(
			s.vec_resize(v, Value::I64(8), fill).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!(s.len(v).unwrap(), 3);
		assert_eq!(s.live_entries(), 5);
		s.vec_resize(v, Value::I64(0), Value::Unit).unwrap();
		s.release(v).unwrap();
		s.release(fill).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
}

#[cfg(test)]
mod default_budget_tests {
	use super::*;
	#[test]
	fn resize_default_entry_boundary_has_an_exact_charged_peak() {
		let mut store = Store::new(Limits::default()).unwrap();
		let value = store.list(Type::Vec, &[]).unwrap();
		let fill = store.variant(Type::Option, false, None).unwrap();
		let error = store
			.vec_resize(value, Value::I64(65537), fill)
			.unwrap_err();
		assert_eq!(error.kind, ErrorKind::StoreCapacity);
		assert_eq!(store.live_entries(), 65536);
		assert_eq!(store.len(value).unwrap(), 65534);
		assert!(store.payload_bytes() < 16 << 20);
		std::println!("C17F_BUDGET entries={} bytes={} completed_slots={} limit_entries=65536 limit_bytes=16777216",store.live_entries(),store.payload_bytes(),store.len(value).unwrap());
		store.release(value).unwrap();
		store.release(fill).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
