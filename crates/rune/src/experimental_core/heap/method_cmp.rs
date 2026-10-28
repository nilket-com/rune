//! Certified associated cmp without VM or native invocation.
use super::*;
use crate::experimental_core::error::{
	Detail, HostCause, IntTarget, IntValue, MovedAccess, TypeName,
};

impl Store {
	fn cmp_argument_number(&self, value: Value) -> Result<Error> {
		Ok(
			err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
				cause: HostCause::ExpectedNumber,
				arg: 1,
				found: self
					.diagnostic_type(value)
					.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "Method"))?,
			}),
		)
	}
	fn cmp_argument_type(&self, expected: TypeName, value: Value) -> Result<Error> {
		let found = self
			.diagnostic_type(value)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "Method"))?;
		Ok(
			err(ErrorKind::Type).with_detail(Detail::NumericArgumentType {
				cause: HostCause::ExpectedType,
				arg: 1,
				expected,
				found,
			}),
		)
	}
	fn cmp_text(&self, value: Value, arg: u8) -> Result<&str> {
		self.text(value).map_err(|error| {
			if matches!(
				error.detail,
				Detail::Moved {
					access: MovedAccess::Read
				}
			) {
				error.with_detail(Detail::CmpArgumentAccess {
					arg,
					access: MovedAccess::Read,
				})
			} else {
				error
			}
		})
	}
	fn cmp_structural_argument(&self, value: Value, arg: u8) -> Result<()> {
		if let Value::Heap(handle) = value {
			self.payload(handle).map_err(|error| {
				if matches!(
					error.detail,
					Detail::Moved {
						access: MovedAccess::Read
					}
				) {
					error.with_detail(Detail::CmpArgumentAccess {
						arg,
						access: MovedAccess::Read,
					})
				} else {
					error
				}
			})?;
		}
		Ok(())
	}

	#[cold]
	#[inline(never)]
	pub(crate) fn method_cmp(&self, lhs: Value, rhs: Value) -> Result<Value> {
		let coerce = |value, to| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
				cause: HostCause::IntegerCoercion,
				arg: 1,
				value,
				to,
			})
		};
		let result = match lhs {
			Value::I64(a) => a.cmp(&match rhs {
				Value::I64(b) => b,
				Value::U64(b) => {
					i64::try_from(b).map_err(|_| coerce(IntValue::Unsigned(b), IntTarget::I64))?
				}
				_ => return Err(self.cmp_argument_number(rhs)?),
			}),
			Value::U64(a) => a.cmp(&match rhs {
				Value::U64(b) => b,
				Value::I64(b) => {
					u64::try_from(b).map_err(|_| coerce(IntValue::Signed(b), IntTarget::U64))?
				}
				_ => return Err(self.cmp_argument_number(rhs)?),
			}),
			Value::F64(a) => {
				let Value::F64(b) = rhs else {
					return Err(self.cmp_argument_type(TypeName::F64, rhs)?);
				};
				a.partial_cmp(&b).ok_or_else(|| {
					err(ErrorKind::IllegalFloatComparison).with_detail(Detail::FloatComparison {
						lhs_bits: a.to_bits(),
						rhs_bits: b.to_bits(),
					})
				})?
			}
			_ => {
				let target = self
					.diagnostic_type(lhs)
					.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "Method"))?;
				match target {
					TypeName::String => {
						// Borrow the receiver first; a moved receiver wins over a bad rhs.
						let a = self.cmp_text(lhs, 0)?;
						if self.diagnostic_type(rhs) != Some(TypeName::String) {
							return Err(self.cmp_argument_type(TypeName::String, rhs)?);
						}
						a.cmp(self.cmp_text(rhs, 1)?)
					}
					TypeName::Unit
					| TypeName::Tuple
					| TypeName::Vec
					| TypeName::Option
					| TypeName::Result
					| TypeName::Range
					| TypeName::RangeFrom
					| TypeName::RangeFull
					| TypeName::RangeInclusive
					| TypeName::RangeTo
					| TypeName::RangeToInclusive => {
						self.cmp_structural_argument(lhs, 0)?;
						self.cmp_structural_argument(rhs, 1)?;
						self.method_compare_order(lhs, rhs).map_err(|mut error| {
							error.operation = "Method";
							error
						})?
					}
					_ => {
						let instance = match target {
							TypeName::Struct(n) => crate::experimental_core::nominal::hash_copy(n)
								.map(crate::Hash::new),
							_ => target.builtin_hash(),
						}
						.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "Method"))?;
						return Err(err(ErrorKind::Type).with_detail(
							Detail::MissingInstanceFunction {
								target,
								instance,
								hash: crate::Hash::associated_function(
									instance,
									crate::Hash::ident("cmp"),
								),
							},
						));
					}
				}
			}
		};
		Ok(Value::Ordering(result))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::experimental_core::{arithmetic::Binary, value::Scalar};
	use core::cmp::Ordering::{Equal, Greater, Less};
	#[test]
	fn ordering_roots_are_immediate_owned_and_nonallocating() {
		let mut store = Store::new(Limits {
			entries: 1,
			bytes: 0,
		})
		.unwrap();
		let other = Store::new(Limits::default()).unwrap();
		for v in [Less, Equal, Greater] {
			let root = store.root_ordering(v);
			assert_eq!(
				other.root_scalar(&root).unwrap_err().kind,
				ErrorKind::InvalidHandle
			);
			assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::Ordering(v)));
			assert_eq!(store.root_tag(&root).unwrap(), Type::Ordering);
			let clone = store.clone_value(Value::Ordering(v)).unwrap();
			assert_eq!(clone, Value::Ordering(v));
			assert!(Store::is_slot_scalar(&clone));
			store.release_slot(clone).unwrap();
			let retained = store.root_retain(&root).unwrap();
			store.root_release(root).unwrap();
			store.root_release(retained).unwrap();
		}
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
	#[test]
	fn cmp_keeps_native_argument_coercion_nan_bits_and_missing_identity() {
		let store = Store::new(Limits::default()).unwrap();
		assert_eq!(
			store.method_cmp(Value::I64(-1), Value::U64(0)).unwrap(),
			Value::Ordering(Less)
		);
		assert_eq!(
			store.method_cmp(Value::U64(3), Value::I64(2)).unwrap(),
			Value::Ordering(Greater)
		);
		let err = store.method_cmp(Value::U64(3), Value::I64(-1)).unwrap_err();
		assert_eq!(
			err.detail.numeric_argument(),
			Some((HostCause::IntegerCoercion, 1))
		);
		let bits = 0xfff8_0000_0000_0042;
		let err = store
			.method_cmp(Value::F64(f64::from_bits(bits)), Value::F64(-0.0))
			.unwrap_err();
		assert_eq!(err.kind, ErrorKind::IllegalFloatComparison);
		assert_eq!(
			err.detail,
			Detail::FloatComparison {
				lhs_bits: bits,
				rhs_bits: (-0.0f64).to_bits()
			}
		);
		assert_eq!(
			store.method_cmp(Value::F64(-0.0), Value::F64(0.0)).unwrap(),
			Value::Ordering(Equal)
		);
		let err = store
			.method_cmp(Value::Ordering(Less), Value::Ordering(Equal))
			.unwrap_err();
		assert!(matches!(
			err.detail,
			Detail::MissingInstanceFunction {
				target: TypeName::Ordering,
				..
			}
		));
	}
	#[test]
	fn string_moves_keep_native_argument_index_and_refusal_does_not_mutate() {
		let mut store = Store::new(Limits::default()).unwrap();
		let a = store.string("a").unwrap();
		let b = store.string("b").unwrap();
		let before = store.state_for_test();
		assert_eq!(store.method_cmp(a, b).unwrap(), Value::Ordering(Less));
		assert_eq!(store.state_for_test(), before);
		store.invalidate(a).unwrap();
		for (lhs, rhs, arg) in [(a, b, 0), (b, a, 1)] {
			let err = store.method_cmp(lhs, rhs).unwrap_err();
			assert_eq!(
				err.detail.numeric_argument(),
				Some((HostCause::Access, arg))
			);
		}
		store.release(a).unwrap();
		store.release(b).unwrap();
		let vector = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let before = store.state_for_test();
		let err = store.method_cmp(vector, Value::Unit).unwrap_err();
		assert_eq!(
			(err.kind, err.operation, err.refusal),
			(ErrorKind::Type, "Method", None)
		);
		assert_eq!(store.state_for_test(), before);
		store.release(vector).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
	#[test]
	fn tuple_cmp_preserves_nan_bits_and_never_mutates_inputs() {
		let mut store = Store::new(Limits::default()).unwrap();
		let bits = 0x7ff8_0000_0000_1234;
		let left = store
			.list(Type::Tuple, &[Value::F64(f64::from_bits(bits))])
			.unwrap();
		let right = store.list(Type::Tuple, &[Value::F64(0.0)]).unwrap();
		let before = store.state_for_test();
		let error = store.method_cmp(left, right).unwrap_err();
		assert_eq!(
			(error.kind, error.operation),
			(ErrorKind::IllegalFloatComparison, "Method")
		);
		assert_eq!(
			error.detail,
			Detail::FloatComparison {
				lhs_bits: bits,
				rhs_bits: 0.0f64.to_bits()
			}
		);
		assert_eq!(store.state_for_test(), before);
		store.release(left).unwrap();
		store.release(right).unwrap();
	}

	#[test]
	fn structural_cmp_failure_preserves_store_and_bounds_the_walk() {
		let mut store = Store::new(Limits::default()).unwrap();
		let mut nested = Value::I64(1);
		for _ in 0..63 {
			let next = store.list(Type::Vec, &[nested]).unwrap();
			store.release(nested).unwrap();
			nested = next;
		}
		assert_eq!(
			store.method_cmp(nested, nested).unwrap(),
			Value::Ordering(Equal)
		);
		let deeper = store.list(Type::Vec, &[nested]).unwrap();
		assert_eq!(
			store.method_cmp(deeper, deeper).unwrap(),
			Value::Ordering(Equal)
		);
		let too_deep = store.list(Type::Vec, &[deeper]).unwrap();
		let before = store.state_for_test();
		assert_eq!(
			store.method_cmp(too_deep, too_deep).unwrap_err().kind,
			ErrorKind::ValueDepth
		);
		assert_eq!(before, store.state_for_test());
		assert_eq!(
			rune_alloc::limit::with(0, || store.method_cmp(nested, nested))
				.call()
				.unwrap_err()
				.kind,
			ErrorKind::Allocation
		);
		assert_eq!(before, store.state_for_test());
		let cycle = store.list(Type::Vec, &[]).unwrap();
		store.push(cycle, cycle).unwrap();
		let before = store.state_for_test();
		assert_eq!(
			store.method_cmp(cycle, cycle).unwrap_err().kind,
			ErrorKind::ValueDepth
		);
		assert_eq!(before, store.state_for_test());
		store.invalidate(nested).unwrap();
		for (lhs, rhs, arg) in [(nested, deeper, 0), (deeper, nested, 1)] {
			let before = store.state_for_test();
			let error = store.method_cmp(lhs, rhs).unwrap_err();
			assert_eq!(
				error.detail.numeric_argument(),
				Some((HostCause::Access, arg))
			);
			assert_eq!(before, store.state_for_test());
		}
	}

	#[test]
	fn ordering_nested_equality_debug_and_display_use_the_immediate_value() {
		let mut store = Store::new(Limits::default()).unwrap();
		let a = store
			.list(
				Type::Tuple,
				&[Value::Ordering(Less), Value::Ordering(Equal)],
			)
			.unwrap();
		let b = store.clone_value(a).unwrap();
		assert_eq!(store.compare(Binary::Eq, a, b).unwrap(), Value::Bool(true));
		assert_eq!(
			store.format_value(a, true).unwrap().as_str(),
			"(Less, Equal)"
		);
		assert_eq!(
			store
				.format_value(Value::Ordering(Greater), true)
				.unwrap()
				.as_str(),
			"Greater"
		);
		assert_eq!(
			store
				.format_value(Value::Ordering(Equal), false)
				.unwrap_err()
				.detail,
			Detail::NoDisplay {
				found: TypeName::Ordering
			}
		);
		store.release(a).unwrap();
		store.release(b).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}

#[cfg(test)]
mod diagnostic_coverage_tests {
	use super::*;
	#[test]
	fn function_and_type_receivers_and_arguments_have_real_type_metadata() {
		let store = Store::new(Limits::default()).unwrap();
		for (value, target) in [
			(Value::Function { code: 1, id: 0 }, TypeName::Function),
			(Value::Type(Type::I64), TypeName::Type),
		] {
			assert_eq!(store.diagnostic_type(value), Some(target));
			let receiver = store.method_cmp(value, Value::I64(1)).unwrap_err();
			assert_eq!(receiver.kind, ErrorKind::Type);
			assert!(
				matches!(receiver.detail,Detail::MissingInstanceFunction {target:t,..} if t==target)
			);
			let rhs = store.method_cmp(Value::I64(1), value).unwrap_err();
			assert!(matches!(rhs.detail,Detail::NumericArgumentNumber {found,..} if found==target));
			let rhs = store.method_cmp(Value::F64(1.0), value).unwrap_err();
			assert!(matches!(rhs.detail,Detail::NumericArgumentType {found,..} if found==target));
		}
	}
	#[test]
	fn missing_heap_metadata_is_an_internal_invariant() {
		let mut store = Store::new(Limits::default()).unwrap();
		let stale = store.string("stale").unwrap();
		store.release(stale).unwrap();
		assert_eq!(store.diagnostic_type(stale), None);
		for (lhs, rhs) in [
			(stale, Value::I64(1)),
			(Value::I64(1), stale),
			(Value::F64(1.0), stale),
		] {
			let e = store.method_cmp(lhs, rhs).unwrap_err();
			assert_eq!(
				(e.kind, e.operation, e.refusal),
				(ErrorKind::InvalidCode, "Method", None)
			);
		}
	}
}
