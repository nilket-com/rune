//! Transfer heap payload ownership without cloning children; inline values are Copy.
use super::*;
use crate::experimental_core::error::{Detail, MovedAccess};

impl Store {
	#[cold]
	#[inline(never)]
	pub(crate) fn take_capture(&mut self, value: Value) -> Result<Value> {
		if self.closed {
			return Err(err(ErrorKind::ClosedStore));
		}
		let Value::Heap(old) = value else {
			return Ok(value);
		};
		let entry = self.entry(old)?;
		if entry.readers != 0 {
			return Err(sorting::conflict(sorting::Access::Take, entry));
		}
		let payload = entry
			.payload
			.as_ref()
			.ok_or_else(|| err(ErrorKind::InvalidCode))?;
		if matches!(payload, Payload::Moved(_)) {
			return Err(err(ErrorKind::AccessConflict).with_detail(Detail::Moved {
				access: MovedAccess::Take,
			}));
		}
		if matches!(payload, Payload::SortGuard(_)) {
			return Err(err(ErrorKind::InvalidCode));
		}
		let tag = payload.tag();
		let charged = entry.charged_bytes();
		// Reserve metadata before mutation. This empty payload has no child edges,
		// byte charge or lease, so admission cannot alter the old payload's owners.
		let destination = self.admit(Payload::List {
			tag: Type::Tuple,
			values: Vec::new(),
		})?;
		let Value::Heap(new) = destination else {
			unreachable!()
		};
		// Admission ran no external code; both entries remain live. Transfer the
		// existing child edges and any iterator lease without retain/release.
		let entry = self.slots[old.slot as usize]
			.entry
			.as_mut()
			.expect("validated capture source");
		let payload = entry
			.payload
			.replace(Payload::Moved(tag))
			.expect("validated capture payload");
		entry.set_charged_bytes(0);
		let entry = self.slots[new.slot as usize]
			.entry
			.as_mut()
			.expect("reserved capture destination");
		entry.payload = Some(payload);
		entry.set_charged_bytes(charged);
		self.assert_work();
		Ok(destination)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn take_invalidates_aliases_and_transfers_children_without_retain() {
		let mut s = Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap();
		let child = s.string("held").unwrap();
		let old = s.list(Type::Vec, &[child]).unwrap();
		s.retain(old).unwrap();
		let bytes = s.payload_bytes();
		let new = s.take_capture(old).unwrap();
		assert_ne!(new, old);
		assert_eq!(s.payload_bytes(), bytes);
		assert_eq!(s.len(new).unwrap(), 1);
		assert_eq!(
			s.len(old).unwrap_err().detail,
			Detail::Moved {
				access: MovedAccess::Read
			}
		);
		assert_eq!(
			s.take_capture(old).unwrap_err().detail,
			Detail::Moved {
				access: MovedAccess::Take
			}
		);
		if let Value::Heap(h) = child {
			assert_eq!(s.entry(h).unwrap().strong, 2);
		} else {
			unreachable!()
		}
		s.release(old).unwrap();
		s.release(old).unwrap();
		s.release(new).unwrap();
		s.release(child).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	#[test]
	fn failed_take_lease_or_capacity_preserves_original() {
		let mut s = Store::new(Limits {
			entries: 4,
			bytes: 4096,
		})
		.unwrap();
		let old = s.list(Type::Vec, &[Value::I64(7)]).unwrap();
		let iter = s.iterator(old).unwrap();
		let state = (s.live_entries(), s.payload_bytes());
		assert_eq!(
			s.take_capture(old).unwrap_err().detail,
			Detail::TakeBorrowed
		);
		assert_eq!((s.live_entries(), s.payload_bytes()), state);
		assert_eq!(s.len(old).unwrap(), 1);
		s.release(iter).unwrap();
		let a = s.string("a").unwrap();
		let b = s.string("b").unwrap();
		let c = s.string("c").unwrap();
		let state = (s.live_entries(), s.payload_bytes());
		assert_eq!(
			s.take_capture(old).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!((s.live_entries(), s.payload_bytes()), state);
		assert_eq!(s.len(old).unwrap(), 1);
		for v in [a, b, c, old] {
			s.release(v).unwrap();
		}
		assert_eq!(s.live_entries(), 0);
	}
	#[test]
	fn iterator_transfer_preserves_one_lease_and_cursor() {
		let mut s = Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap();
		let v = s.list(Type::Vec, &[Value::I64(7), Value::I64(8)]).unwrap();
		let old = s.iterator(v).unwrap();
		assert_eq!(s.iterator_next(old).unwrap(), Some(Value::I64(7)));
		let new = s.take_capture(old).unwrap();
		assert_eq!(s.iterator_next(new).unwrap(), Some(Value::I64(8)));
		assert!(s.iterator_next(old).is_err());
		if let Value::Heap(h) = v {
			assert_eq!(s.entry(h).unwrap().readers, 1);
		} else {
			unreachable!()
		}
		s.release(old).unwrap();
		s.release(new).unwrap();
		if let Value::Heap(h) = v {
			assert_eq!(s.entry(h).unwrap().readers, 0);
		} else {
			unreachable!()
		}
		s.release(v).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
	#[test]
	fn self_edge_stays_an_alias_of_the_moved_original() {
		let mut s = Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap();
		let old = s.list(Type::Vec, &[]).unwrap();
		s.push(old, old).unwrap();
		let new = s.take_capture(old).unwrap();
		assert_eq!(s.len(new).unwrap(), 1);
		let Value::Heap(h) = new else { unreachable!() };
		let Payload::List { values, .. } = s.payload(h).unwrap() else {
			unreachable!()
		};
		assert_eq!(values.as_slice(), &[old]);
		s.release(new).unwrap();
		s.release(old).unwrap();
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
}
