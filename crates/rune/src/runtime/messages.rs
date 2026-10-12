//! Allocation-free diagnostic text shared by the stock and fresh executors.
use core::fmt::{self, Display};

pub(crate) fn binary_unsupported(
	f: &mut fmt::Formatter<'_>,
	op: impl Display,
	lhs: impl Display,
	rhs: impl Display,
) -> fmt::Result {
	write!(
		f,
		"Unsupported binary operation `{op}` on `{lhs}` and `{rhs}`"
	)
}

pub(crate) fn unary_unsupported(
	f: &mut fmt::Formatter<'_>,
	op: impl Display,
	operand: impl Display,
) -> fmt::Result {
	write!(f, "Unsupported unary operation `{op}` on `{operand}`")
}

pub(crate) fn expected_type(
	f: &mut fmt::Formatter<'_>,
	expected: impl Display,
	actual: impl Display,
) -> fmt::Result {
	write!(f, "Expected type `{expected}` but found `{actual}`")
}

pub(crate) fn expected_number(f: &mut fmt::Formatter<'_>, actual: impl Display) -> fmt::Result {
	write!(f, "Expected number type, but found `{actual}`")
}

pub(crate) fn coerce(
	f: &mut fmt::Formatter<'_>,
	from: impl Display,
	to: impl Display,
) -> fmt::Result {
	write!(f, "Failed to convert value `{from}` to integer `{to}`")
}

pub(crate) fn out_of_range(
	f: &mut fmt::Formatter<'_>,
	index: impl Display,
	length: impl Display,
) -> fmt::Result {
	write!(
		f,
		"Index out of bounds, the length is `{length}` but the index is `{index}`"
	)
}

pub(crate) fn missing_field(
	f: &mut fmt::Formatter<'_>,
	target: impl Display,
	field: impl Display,
) -> fmt::Result {
	write!(f, "Missing field `{field}` on `{target}`")
}

pub(crate) fn field_get(
	f: &mut fmt::Formatter<'_>,
	target: impl Display,
	field: impl Display,
) -> fmt::Result {
	write!(f, "Field `{field}` not available on `{target}`")
}

pub(crate) fn index_get(
	f: &mut fmt::Formatter<'_>,
	target: impl Display,
	index: impl Display,
) -> fmt::Result {
	write!(
		f,
		"The index get operation `{target}[{index}]` is not supported"
	)
}

pub(crate) fn index_set(
	f: &mut fmt::Formatter<'_>,
	target: impl Display,
	index: impl Display,
	value: impl Display,
) -> fmt::Result {
	write!(
		f,
		"The index set operation `{target}[{index}] = {value}` is not supported"
	)
}

pub(crate) fn tuple_get(
	f: &mut fmt::Formatter<'_>,
	target: impl Display,
	index: impl Display,
) -> fmt::Result {
	write!(
		f,
		"The tuple index get {index} operation is not supported on `{target}`"
	)
}

pub(crate) fn tuple_set(f: &mut fmt::Formatter<'_>, target: impl Display) -> fmt::Result {
	write!(
		f,
		"The tuple index set operation is not supported on `{target}`"
	)
}

pub(crate) fn not_callable(f: &mut fmt::Formatter<'_>, actual: impl Display) -> fmt::Result {
	write!(
		f,
		"Type `{actual}` cannot be called since it's not a function"
	)
}

pub(crate) fn arity(
	f: &mut fmt::Formatter<'_>,
	actual: impl Display,
	expected: impl Display,
) -> fmt::Result {
	write!(f, "Wrong number of arguments {actual}, expected {expected}")
}

pub(crate) fn try_operand(f: &mut fmt::Formatter<'_>, actual: impl Display) -> fmt::Result {
	write!(f, "Type `{actual}` is not supported as try operand")
}

pub(crate) fn cast_source(f: &mut fmt::Formatter<'_>, value: impl Display) -> fmt::Result {
	write!(f, "Type `{value}` cannot be converted with `as`, which only converts between `i64`, `u64` and `f64`")
}

pub(crate) fn cast_target(f: &mut fmt::Formatter<'_>, value: impl Display) -> fmt::Result {
	write!(f, "Type `{value}` cannot be converted with `as` to that type, which only converts to `i64`, `u64` or `f64`")
}
