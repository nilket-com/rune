//! Allocation-free diagnostic text shared by the stock and fresh executors.
use core::fmt::{self, Display, Write};

/// Shorten type paths only when presenting a runtime diagnostic.
///
/// The public type and item displays retain their rooted identities. This
/// adapter also handles generic arguments split across formatting writes.
pub(crate) struct DiagnosticType<'a, T: ?Sized + Display>(pub(crate) &'a T);

impl<T: ?Sized + Display> Display for DiagnosticType<'_, T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut writer = TypeWriter {
			out: f,
			at_start: true,
			pending_colon: false,
		};
		fmt::write(&mut writer, format_args!("{}", self.0))?;
		if writer.pending_colon {
			writer.out.write_char(':')?;
		}
		Ok(())
	}
}

struct TypeWriter<'a, 'b> {
	out: &'a mut fmt::Formatter<'b>,
	at_start: bool,
	pending_colon: bool,
}

impl Write for TypeWriter<'_, '_> {
	fn write_str(&mut self, text: &str) -> fmt::Result {
		for c in text.chars() {
			if self.pending_colon {
				self.pending_colon = false;
				self.at_start = false;
				if c == ':' {
					continue;
				}
				self.out.write_char(':')?;
			}
			if self.at_start && c == ':' {
				self.pending_colon = true;
				continue;
			}
			self.out.write_char(c)?;
			self.at_start = matches!(c, '<' | '(' | '[' | ',' | ' ');
		}
		Ok(())
	}
}

/// Diagnostic operand spelling, independent of type identity.
pub(crate) trait OperandType {
	fn fmt_operand(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}
pub(crate) struct Operand<'a, T: ?Sized>(pub(crate) &'a T);
impl<T: ?Sized + OperandType> Display for Operand<'_, T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt_operand(f)
	}
}
impl OperandType for crate::runtime::TypeInfo {
	fn fmt_operand(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		use crate::TypeHash;
		let hash = self.type_hash();
		let name = if hash == i64::HASH {
			"i64"
		} else if hash == u64::HASH {
			"u64"
		} else if hash == f64::HASH {
			"f64"
		} else if hash == bool::HASH {
			"bool"
		} else if hash == char::HASH {
			"char"
		} else if hash == crate::alloc::String::HASH {
			"String"
		} else if hash == crate::runtime::Vec::HASH {
			"Vec"
		} else {
			return DiagnosticType(self).fmt(f);
		};
		f.write_str(name)
	}
}

pub(crate) fn numeric_fault(
	f: &mut fmt::Formatter<'_>,
	reason: &str,
	op: &str,
	operand: impl Display,
) -> fmt::Result {
	let action = match op {
		"+" | "+=" => "addition",
		"-" | "-=" => "subtraction",
		"*" | "*=" => "multiplication",
		"/" | "/=" => "division",
		"%" | "%=" => "remainder",
		"<<" | "<<=" => "left shift",
		">>" | ">>=" => "right shift",
		_ => return f.write_str(reason),
	};
	write!(f, "{reason} in `{operand}` {action} (`{op}`)")
}

pub(crate) fn shift_coerce(
	f: &mut fmt::Formatter<'_>,
	from: impl Display,
	value: impl Display,
	to: impl Display,
	op: impl Display,
) -> fmt::Result {
	write!(
		f,
		"Cannot convert `{from}` value `{value}` to `{to}` for `{op}`"
	)
}

pub(crate) fn binary_unsupported(
	f: &mut fmt::Formatter<'_>,
	op: impl Display,
	lhs: impl Display,
	rhs: impl Display,
) -> fmt::Result {
	write!(f, "Cannot apply `{op}` to `{lhs}` and `{rhs}`")
}

pub(crate) fn unary_unsupported(
	f: &mut fmt::Formatter<'_>,
	op: impl Display,
	operand: impl Display,
) -> fmt::Result {
	write!(f, "Cannot apply `{op}` to `{operand}`")
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate as rune;
	use crate::runtime::{AnyObjError, AnyObjErrorKind, TypeInfo, TypeOf, VmErrorKind};
	use crate::Hash;
	use std::string::ToString;

	struct Chunks<'a>(&'a str, usize);

	impl Display for Chunks<'_> {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			f.write_str(&self.0[..self.1])?;
			f.write_str("")?;
			f.write_str(&self.0[self.1..])
		}
	}

	#[test]
	fn diagnostic_paths_stream_across_utf8_boundaries() {
		for (input, expected) in [
			("::std::i64", "std::i64"),
			(
				"::std::ops::RangeIter<::std::i64>",
				"std::ops::RangeIter<std::i64>",
			),
			("::a::B<::c::D, ::e::F<::g::H>>", "a::B<c::D, e::F<g::H>>"),
			("(::a::B, [::c::D])", "(a::B, [c::D])"),
			("::été::名<::λ::β>", "été::名<λ::β>"),
			("a::b", "a::b"),
			("P", "P"),
			(":", ":"),
			(":x", ":x"),
			("::x:", "x:"),
			("::::x", "::x"),
			("::x::::y", "x::::y"),
		] {
			for split in (0..=input.len()).filter(|&i| input.is_char_boundary(i)) {
				assert_eq!(DiagnosticType(&Chunks(input, split)).to_string(), expected);
			}
		}
	}

	#[test]
	fn diagnostic_paths_propagate_inner_and_sink_failures() {
		struct InnerFailure;
		impl Display for InnerFailure {
			fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				f.write_str("::std::")?;
				Err(fmt::Error)
			}
		}
		let mut text = std::string::String::new();
		assert!(fmt::write(&mut text, format_args!("{}", DiagnosticType(&InnerFailure))).is_err());

		struct SinkFailure;
		impl Write for SinkFailure {
			fn write_str(&mut self, _: &str) -> fmt::Result {
				Err(fmt::Error)
			}
		}
		// The second case fails while flushing the final pending colon.
		for input in ["::std::i64", ":"] {
			assert!(
				fmt::write(&mut SinkFailure, format_args!("{}", DiagnosticType(input))).is_err()
			);
		}
	}

	#[test]
	fn diagnostic_paths_leave_public_type_and_item_identities_rooted() {
		let ty = TypeInfo::named::<i64>();
		let item = crate::item!(::std::i64);
		assert_eq!(ty.to_string(), "::std::i64");
		assert_eq!(item.to_string(), "::std::i64");
		assert_eq!(ty.type_hash(), Hash::type_hash(item));
		assert_eq!(DiagnosticType(&ty).to_string(), "std::i64");
		assert_eq!(ty.to_string(), "::std::i64");
	}

	#[test]
	fn diagnostic_paths_only_shorten_type_operands() {
		let field = crate::alloc::String::try_from("::std::user::field").unwrap();
		let error = VmErrorKind::MissingField {
			target: TypeInfo::any::<crate::runtime::Object>(),
			field,
		};
		assert_eq!(
			error.to_string(),
			"Missing field `::std::user::field` on `std::object::Object`"
		);
		let hash = Hash::new(42);
		let error = VmErrorKind::MissingInstanceFunction {
			hash,
			instance: TypeInfo::named::<i64>(),
		};
		assert_eq!(error.to_string(), "Type `std::i64` has no matching method");
		let error = AnyObjError::new(AnyObjErrorKind::Cast(
			<bool as TypeOf>::STATIC_TYPE_INFO,
			TypeInfo::named::<i64>(),
		));
		assert_eq!(
			error.to_string(),
			"Failed to cast `std::i64` to `std::bool`"
		);
		let error = AnyObjError::new(AnyObjErrorKind::NotOwned(TypeInfo::named::<i64>()));
		assert_eq!(
			error.to_string(),
			"Cannot use owned operations for std::i64"
		);
	}
	#[test]
	fn missing_instance_identity_comes_only_from_the_root_kind() {
		use crate::runtime::VmError;
		let instance = || TypeInfo::any::<crate::runtime::Vec>();
		let hash = Hash::associated_function(instance().type_hash(), "frobnicate");
		let missing = || VmErrorKind::MissingInstanceFunction {
			hash,
			instance: instance(),
		};
		let error = VmError::new(missing());
		let info = error.missing_instance_function().unwrap();
		assert_eq!(info.hash, hash);
		assert_eq!(info.instance, instance().type_hash());
		assert_eq!(info.instance_name().to_string(), "std::vec::Vec");
		let quoted = error.to_string();
		assert!(VmError::panic(quoted).missing_instance_function().is_none());
		assert!(VmError::panic("outer")
			.with_error(missing())
			.missing_instance_function()
			.is_none());
	}
}
