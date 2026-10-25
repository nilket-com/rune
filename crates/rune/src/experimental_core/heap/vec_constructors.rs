//! Vec constructor allocation is checked before any store publication.
use super::super::error::{Detail, HostCause, IntTarget, IntValue};
use super::*;

impl Store {
	pub(crate) fn vec_capacity_argument(&self, value: Value) -> Result<usize> {
		let int = IntValue::of(value).ok_or_else(|| {
			let Some(found) = self.diagnostic_type(value) else {
				return err(ErrorKind::InvalidCode);
			};
			err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
				cause: HostCause::ExpectedNumber,
				arg: 0,
				found,
			})
		})?;
		usize::try_from(int.wide()).map_err(|_| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
				cause: HostCause::IntegerCoercion,
				arg: 0,
				value: int,
				to: IntTarget::Usize,
			})
		})
	}

	pub(crate) fn vec_with_capacity(&mut self, capacity: usize) -> Result<Value> {
		// Stock checks the layout of its own Value, independently of store budgets.
		if capacity > isize::MAX as usize / size_of::<crate::runtime::Value>() {
			return Err(err(ErrorKind::Allocation).with_detail(Detail::CapacityOverflow));
		}
		// Capacity is not observable in the new subset. Treat the reservation as
		// a bounded hint, so unused space cannot exhaust the payload budget.
		// The usual closed-store and entry-slot limits still apply to the vector.
		self.admission_room(0)?;
		let room = self.limits.bytes.saturating_sub(self.bytes) / size_of::<Value>();
		let values = Vec::try_with_capacity(capacity.min(room).min(8))?;
		self.admit(Payload::List {
			tag: Type::Vec,
			values,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn vec_reservations_are_charged_and_transactional() {
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 1024,
		})
		.unwrap();
		let value = store.vec_with_capacity(5).unwrap();
		assert_eq!(store.len(value).unwrap(), 0);
		let Value::Heap(handle) = value else { panic!() };
		let Payload::List {
			tag: Type::Vec,
			values,
		} = store.payload(handle).unwrap()
		else {
			panic!()
		};
		assert_eq!(values.capacity(), 5);
		assert_eq!(store.payload_bytes(), 5 * size_of::<Value>());
		let large = store.vec_with_capacity(2_000_000).unwrap();
		let Value::Heap(large_handle) = large else {
			panic!()
		};
		let Payload::List { values, .. } = store.payload(large_handle).unwrap() else {
			panic!()
		};
		assert_eq!(values.capacity(), 8);
		assert_eq!(store.payload_bytes(), 13 * size_of::<Value>());
		let before = (store.live_entries(), store.payload_bytes());
		assert_eq!(
			store.vec_with_capacity(usize::MAX).unwrap_err().detail,
			Detail::CapacityOverflow
		);
		assert_eq!((store.live_entries(), store.payload_bytes()), before);
		store.release(large).unwrap();
		store.release(value).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));

		// A full payload budget reduces the hint to zero without rejecting an
		// empty vector; existing contents remain charged until release.
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 5 * size_of::<Value>(),
		})
		.unwrap();
		let full = store.vec_with_capacity(5).unwrap();
		let empty = store.vec_with_capacity(2_000_000).unwrap();
		let Value::Heap(handle) = empty else { panic!() };
		let Payload::List { values, .. } = store.payload(handle).unwrap() else {
			panic!()
		};
		assert_eq!(values.capacity(), 0);
		assert_eq!(store.payload_bytes(), 5 * size_of::<Value>());
		store.release(empty).unwrap();
		store.release(full).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}

	#[test]
	fn vec_argument_conversion_preserves_native_index() {
		let store = Store::new(Limits::default()).unwrap();
		for n in [0, 1, 5000] {
			assert_eq!(
				store.vec_capacity_argument(Value::I64(n)).unwrap(),
				n as usize
			);
			assert_eq!(
				store.vec_capacity_argument(Value::U64(n as u64)).unwrap(),
				n as usize
			);
		}
		for value in [Value::I64(-1), Value::Bool(true), Value::F64(1.0)] {
			let error = store.vec_capacity_argument(value).unwrap_err();
			assert_eq!(error.detail.numeric_argument().unwrap().1, 0);
			assert!(error.refusal.is_none());
		}
	}
}
