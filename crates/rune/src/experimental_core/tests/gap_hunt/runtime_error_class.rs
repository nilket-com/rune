fn runtime_error_class(k: &crate::runtime::VmErrorKind) -> String {
	use crate::runtime::VmErrorKind as K;
	let class = match k {
		K::Overflow | K::Underflow => "Overflow",
		K::ValueToIntegerCoercionError { .. } => "IntegerCoercion",
		K::DivideByZero => "DivisionByZero",
		K::BadArgumentCount { .. } => "ArgumentCount",
		K::AccessError { .. } => "AccessConflict",
		K::Halted {
			halt: crate::runtime::VmHaltInfo::Limited,
		} => "FuelExhausted",
		K::MissingField { .. }
		| K::MissingIndexInteger { .. }
		| K::MissingIndex { .. }
		| K::OutOfRange { .. } => "IndexOutOfBounds",
		K::UnsupportedBinaryOperation { .. }
		| K::UnsupportedUnaryOperation { .. }
		| K::UnsupportedIndexGet { .. }
		| K::UnsupportedIndexSet { .. }
		| K::UnsupportedObjectSlotIndexGet { .. }
		| K::UnsupportedObjectSlotIndexSet { .. }
		| K::UnsupportedTupleIndexGet { .. }
		| K::UnsupportedTupleIndexSet { .. }
		| K::ExpectedNumber { .. }
		| K::ExpectedType { .. }
		| K::MissingInstanceFunction { .. }
		| K::MissingProtocolFunction { .. }
		| K::UnsupportedIterRange { .. } => "Type",
		_ => return format!("stock:{k:?}"),
	};
	class.into()
}
