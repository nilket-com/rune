//! Owned, allocation-free details for language errors.
use super::value::Nominal;

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeName {
	Unit,
	Bool,
	I64,
	U64,
	F64,
	Char,
	String,
	Vec,
	Tuple,
	Object,
	Option,
	Result,
	Range,
	/// Reserved for a future admitted inclusive range; currently refused by lowering.
	RangeInclusive,
	VecIter,
	RangeIter,
	UnsignedRangeIter,
	CharRangeIter,
	Function,
	Struct(Nominal),
	Type,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinarySymbol {
	Add,
	Sub,
	Mul,
	Div,
	Rem,
	Eq,
	Ne,
	Lt,
	Le,
	Gt,
	Ge,
	BitAnd,
	BitOr,
	BitXor,
	Shl,
	Shr,
	AddAssign,
	SubAssign,
	MulAssign,
	DivAssign,
	RemAssign,
	BitAndAssign,
	BitOrAssign,
	BitXorAssign,
	ShlAssign,
	ShrAssign,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnarySymbol {
	Neg,
	Not,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntTarget {
	I64,
	U64,
	U32,
	Usize,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntValue {
	pub negative: bool,
	pub magnitude: u64,
}

/// A constant field name belonging to exactly one compiled program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldRef {
	pub(crate) code: u32,
	pub(crate) function: u32,
	pub(crate) string: u32,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmittedMethod {
	Len,
	Push,
	Clone,
}

/// The access that encountered an explicitly invalidated payload.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovedAccess {
	Read,
	Write,
	Take,
}

impl AdmittedMethod {
	// Stock: hir/lowering2.rs builds Hash::ident(ident), then assemble.rs
	// emits CallAssociated. These are literal identities, never success-path work.
	pub(crate) fn hash(self) -> crate::Hash {
		match self {
			Self::Len => crate::Hash::new(0x6215b706d11e40e5),
			Self::Push => crate::Hash::new(0x7ddb8003b8ab1457),
			Self::Clone => crate::Hash::new(0xa4c75da413f782ae),
		}
	}
}

/// Primary native error identity for a certified host failure.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCause {
	ExpectedNumber,
	ExpectedType,
	Access,
	IntegerCoercion,
	MissingEnvironment,
	Other,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
	#[default]
	None,
	/// Stdout write failure. OS codes retain their exact message; custom errors
	/// retain only their kind, not their allocated custom payload.
	Io {
		kind: std::io::ErrorKind,
		os: Option<i32>,
	},
	Binary {
		op: BinarySymbol,
		lhs: TypeName,
		rhs: TypeName,
	},
	Unary {
		op: UnarySymbol,
		operand: TypeName,
	},
	ExpectedBool {
		found: TypeName,
	},
	ValueDepth,
	ExpectedType {
		expected: TypeName,
		found: TypeName,
	},
	ComparisonNotIterable {
		found: TypeName,
	},
	StringOperand {
		found: TypeName,
	},
	Coerce {
		value: IntValue,
		to: IntTarget,
	},
	ShiftWidth,
	/// A mutation was refused because iteration holds a read lease.
	WriteBorrowed,
	OutOfRange {
		index: usize,
		length: usize,
	},
	/// The full dynamic key is owned by Failure, when its diagnostic copy succeeds.
	MissingKey {
		target: TypeName,
	},
	MissingIndex {
		target: TypeName,
	},
	MissingStringSlice,
	/// The full panic message is owned by Failure, outside this Copy payload.
	PanicMessage,
	UnsupportedIterRange {
		start: TypeName,
		end: TypeName,
	},
	MissingInteger {
		target: TypeName,
		index: usize,
	},
	MissingField {
		target: TypeName,
		field: FieldRef,
	},
	FieldSetUnsupported {
		target: TypeName,
		field: FieldRef,
	},
	FieldOnNonRecord {
		target: TypeName,
		field: FieldRef,
	},
	NotCallable {
		found: TypeName,
	},
	Arity {
		given: u32,
		expected: u32,
	},
	ForeignFunction,
	Moved {
		access: MovedAccess,
	},
	ExpectedChar {
		found: TypeName,
	},
	NoCloneProtocol {
		found: TypeName,
	},
	Cast {
		found: TypeName,
		to_numeric: bool,
	},
	Underflow,
	MissingInstanceFunction {
		target: TypeName,
		hash: crate::Hash,
		instance: crate::Hash,
	},
	NotIterable {
		found: TypeName,
	},
	TryOperand {
		found: TypeName,
	},
	NoDisplay {
		found: TypeName,
	},
	ExpectedNumber {
		found: TypeName,
	},
	IndexTarget {
		target: TypeName,
		index: TypeName,
	},
	IndexSet {
		target: TypeName,
		index: TypeName,
		value: TypeName,
	},
	TupleIndexGet {
		target: TypeName,
		index: usize,
	},
	TupleIndexSet {
		target: TypeName,
	},
	/// Typed host primary and native BadArgument index; text is owned by Failure.
	HostMessage {
		cause: HostCause,
		arg: Option<u8>,
	},
	/// Typed scalar builtin conversion context. Allocates only through diagnostics, never success.
	NumericArgumentType {
		cause: HostCause,
		arg: u8,
		expected: TypeName,
		found: TypeName,
	},
	NumericArgumentNumber {
		cause: HostCause,
		arg: u8,
		found: TypeName,
	},
	NumericArgumentCoerce {
		cause: HostCause,
		arg: u8,
		value: IntValue,
		to: IntTarget,
	},
	/// A Rust unwind contained at the synchronous host boundary.
	HostPanicked,
}

impl Detail {
	/// Argument index retained by a scalar builtin's typed conversion failure.
	pub fn numeric_argument(self) -> Option<(HostCause, u8)> {
		match self {
			Self::NumericArgumentType { cause, arg, .. }
			| Self::NumericArgumentNumber { cause, arg, .. }
			| Self::NumericArgumentCoerce { cause, arg, .. } => Some((cause, arg)),
			_ => None,
		}
	}
}

use super::{
	error::{Error, ErrorKind},
	frontend::Program,
	value::{Type, Value},
};
use crate::runtime::messages;
use core::fmt;

impl TypeName {
	/// Exact stock builtin type identity. Nominal identities are resolved only
	/// on a failing protocol path through the C2 interner.
	pub(crate) fn builtin_hash(self) -> Option<crate::Hash> {
		use crate::runtime as rt;
		use crate::TypeHash;
		Some(match self {
			Self::Unit | Self::Tuple => rt::OwnedTuple::HASH,
			Self::Bool => bool::HASH,
			Self::I64 => i64::HASH,
			Self::U64 => u64::HASH,
			Self::F64 => f64::HASH,
			Self::Char => char::HASH,
			Self::String => crate::alloc::String::HASH,
			Self::Vec => rt::Vec::HASH,
			Self::Object => rt::Object::HASH,
			Self::Option => Option::<rt::Value>::HASH,
			Self::Result => core::result::Result::<rt::Value, rt::Value>::HASH,
			Self::Range => rt::Range::HASH,
			Self::RangeInclusive => rt::RangeInclusive::HASH,
			Self::VecIter => rt::slice::Iter::HASH,
			Self::RangeIter => rt::range::RangeIter::<i64>::HASH,
			Self::UnsignedRangeIter => rt::range::RangeIter::<u64>::HASH,
			Self::CharRangeIter => rt::range::RangeIter::<char>::HASH,
			Self::Function => rt::Function::HASH,
			Self::Type => rt::Type::HASH,
			Self::Struct(_) => return None,
		})
	}
	pub(crate) fn primitive(value: Value) -> Option<Self> {
		Some(match value {
			Value::Function { .. } => Self::Function,
			Value::Type(_) => Self::Type,
			_ => Self::from_type(value.primitive_type()?),
		})
	}

	pub(crate) fn from_type(tag: Type) -> Self {
		match tag {
			Type::Unit => Self::Unit,
			Type::Bool => Self::Bool,
			Type::I64 => Self::I64,
			Type::U64 => Self::U64,
			Type::F64 => Self::F64,
			Type::Char => Self::Char,
			Type::String => Self::String,
			Type::Vec => Self::Vec,
			Type::Tuple => Self::Tuple,
			Type::Object => Self::Object,
			Type::Struct(n) => Self::Struct(n),
			Type::Option => Self::Option,
			Type::Result => Self::Result,
			Type::Range => Self::Range,
			Type::Iterator => Self::VecIter,
		}
	}
}

impl fmt::Display for TypeName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Unit => "std::tuple::Tuple",
			Self::Bool => "std::bool",
			Self::I64 => "std::i64",
			Self::U64 => "std::u64",
			Self::F64 => "std::f64",
			Self::Char => "std::char",
			Self::String => "std::string::String",
			Self::Vec => "std::vec::Vec",
			Self::Tuple => "std::tuple::Tuple",
			Self::Object => "std::object::Object",
			Self::Option => "std::option::Option",
			Self::Result => "std::result::Result",
			Self::Range => "std::ops::Range",
			Self::RangeInclusive => "std::ops::RangeInclusive",
			Self::VecIter => "std::slice::Iter",
			Self::RangeIter => "std::ops::RangeIter<std::i64>",
			Self::UnsignedRangeIter => "std::ops::RangeIter<std::u64>",
			Self::CharRangeIter => "std::ops::RangeIter<std::char>",
			Self::Function => "std::ops::Function",
			Self::Type => "std::any::Type",
			Self::Struct(n) => {
				let mut buffer = [0; 256];
				return match super::nominal::name_copy(*n, &mut buffer) {
					Some(len) => {
						let name = core::str::from_utf8(&buffer[..len]).map_err(|_| fmt::Error)?;
						crate::runtime::messages::DiagnosticType(name).fmt(f)
					}
					None => f.write_str("<struct>"),
				};
			}
		})
	}
}

impl BinarySymbol {
	pub(crate) fn from_op(op: super::arithmetic::Binary, assign: bool) -> Self {
		use super::arithmetic::Binary as B;
		match (op, assign) {
			(B::Add, true) => Self::AddAssign,
			(B::Sub, true) => Self::SubAssign,
			(B::Mul, true) => Self::MulAssign,
			(B::Div, true) => Self::DivAssign,
			(B::Rem, true) => Self::RemAssign,
			(B::BitAnd, true) => Self::BitAndAssign,
			(B::BitOr, true) => Self::BitOrAssign,
			(B::BitXor, true) => Self::BitXorAssign,
			(B::Shl, true) => Self::ShlAssign,
			(B::Shr, true) => Self::ShrAssign,
			(B::Add, _) => Self::Add,
			(B::Sub, _) => Self::Sub,
			(B::Mul, _) => Self::Mul,
			(B::Div, _) => Self::Div,
			(B::Rem, _) => Self::Rem,
			(B::Eq, _) => Self::Eq,
			(B::Ne, _) => Self::Ne,
			(B::Lt, _) => Self::Lt,
			(B::Le, _) => Self::Le,
			(B::Gt, _) => Self::Gt,
			(B::Ge, _) => Self::Ge,
			(B::BitAnd, _) => Self::BitAnd,
			(B::BitOr, _) => Self::BitOr,
			(B::BitXor, _) => Self::BitXor,
			(B::Shl, _) => Self::Shl,
			(B::Shr, _) => Self::Shr,
		}
	}
}

impl fmt::Display for BinarySymbol {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Add => "+",
			Self::Sub => "-",
			Self::Mul => "*",
			Self::Div => "/",
			Self::Rem => "%",
			Self::Eq => "==",
			Self::Ne => "!=",
			Self::Lt => "<",
			Self::Le => "<=",
			Self::Gt => ">",
			Self::Ge => ">=",
			Self::BitAnd => "&",
			Self::BitOr => "|",
			Self::BitXor => "^",
			Self::Shl => "<<",
			Self::Shr => ">>",
			Self::AddAssign => "+=",
			Self::SubAssign => "-=",
			Self::MulAssign => "*=",
			Self::DivAssign => "/=",
			Self::RemAssign => "%=",
			Self::BitAndAssign => "&=",
			Self::BitOrAssign => "|=",
			Self::BitXorAssign => "^=",
			Self::ShlAssign => "<<=",
			Self::ShrAssign => ">>=",
		})
	}
}

impl IntValue {
	pub(crate) fn of(value: Value) -> Option<Self> {
		Some(match value {
			Value::I64(n) => Self {
				negative: n < 0,
				magnitude: n.unsigned_abs(),
			},
			Value::U64(n) => Self {
				negative: false,
				magnitude: n,
			},
			_ => return None,
		})
	}
}

impl fmt::Display for IntValue {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.negative {
			f.write_str("-")?;
		}
		self.magnitude.fmt(f)
	}
}

impl fmt::Display for IntTarget {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::I64 => "i64",
			Self::U64 => "u64",
			Self::U32 => "u32",
			Self::Usize => "usize",
		})
	}
}

impl FieldRef {
	fn resolve(self, program: &Program) -> Option<&str> {
		if self.code != program.code.id() {
			return None;
		}
		Some(
			program
				.code
				.function(self.function)?
				.strings
				.get(self.string as usize)?
				.as_str(),
		)
	}
}

/// A borrowed allocation-free rendering of an owned error.
pub struct Message<'a> {
	pub(crate) error: &'a Error,
	pub(crate) program: &'a Program,
}

impl fmt::Display for Message<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		use Detail::*;
		match self.error.detail {
			Io { kind, os } => {
				let error = match os {
					Some(os) => std::io::Error::from_raw_os_error(os),
					Option::None => std::io::Error::from(kind),
				};
				write!(f, "Panicked: failed printing to stdout: {error}")
			}
			Binary { op, lhs, rhs } => messages::binary_unsupported(f, op, lhs, rhs),
			Unary { op, operand } => messages::unary_unsupported(
				f,
				match op {
					UnarySymbol::Neg => "-",
					UnarySymbol::Not => "!",
				},
				operand,
			),
			ExpectedBool { found } => messages::expected_type(f, TypeName::Bool, found),
			ValueDepth => f.write_str("Value is nested too deeply to walk, limit is 64"),
			ExpectedType { expected, found } => messages::expected_type(f, expected, found),
			NumericArgumentType {
				expected, found, ..
			} => messages::expected_type(f, expected, found),
			NumericArgumentNumber { found, .. } => messages::expected_number(f, found),
			NumericArgumentCoerce { value, to, .. } => messages::coerce(f, value, to),
			ComparisonNotIterable { found } => {
				write!(f, "Missing protocol function `INTO_ITER` for `{found}`")
			}
			StringOperand { found } => messages::expected_type(f, TypeName::String, found),
			Coerce { value, to } => messages::coerce(f, value, to),
			WriteBorrowed => f.write_str("Cannot write, the value is being read from"),
			ShiftWidth => f.write_str("Numerical overflow"),
			Underflow => f.write_str("Numerical underflow"),
			OutOfRange { index, length } => messages::out_of_range(f, index, length),
			MissingIndex { target } => write!(f, "Type `{target}` missing index"),
			MissingStringSlice => f.write_str("Panicked: missing string slice"),
			PanicMessage => f.write_str("Panicked"),
			HostMessage { .. } => f.write_str("Host function failed"),
			HostPanicked => f.write_str("host panicked"),
			UnsupportedIterRange { start, end } => {
				write!(f, "Cannot build an iterator out of {start}..{end}")
			}
			MissingKey { target } => write!(f, "Missing field on `{target}`"),
			MissingInteger { target, index } => {
				write!(f, "Type `{target}` missing integer index `{index}`")
			}
			MissingField { target, field } => match field.resolve(self.program) {
				Some(key) => messages::missing_field(f, target, key),
				Option::None => write!(f, "Missing field on `{target}`"),
			},
			FieldSetUnsupported { target, field } => match field.resolve(self.program) {
				Some(key) => write!(f, "Field `{key}` not available to set on `{target}`"),
				Option::None => write!(f, "Field not available to set on `{target}`"),
			},
			FieldOnNonRecord { target, field } => match field.resolve(self.program) {
				Some(key) => messages::field_get(f, target, key),
				Option::None => write!(f, "Field not available on `{target}`"),
			},
			NotCallable { found } => messages::not_callable(f, found),
			Arity { given, expected } => messages::arity(f, given, expected),
			Moved { access } => write!(
				f,
				"Cannot {}, the value has been moved",
				match access {
					MovedAccess::Read => "read",
					MovedAccess::Write => "write",
					MovedAccess::Take => "take",
				}
			),
			ExpectedChar { found } => write!(f, "Expected type `std::char` but found `{found}`"),
			NoCloneProtocol { found } => {
				write!(f, "Missing protocol function `CLONE` for `{found}`")
			}
			ForeignFunction => f.write_str("Function value belongs to another program"),
			Cast {
				found,
				to_numeric: true,
			} => messages::cast_source(f, found),
			Cast {
				found,
				to_numeric: false,
			} => messages::cast_target(f, found),
			MissingInstanceFunction { target, hash, .. } => {
				write!(f, "Missing instance function `{hash}` for `{target}`")
			}
			NotIterable { found } => write!(f, "Type `{found}` is not iterable"),
			TryOperand { found } => messages::try_operand(f, found),
			NoDisplay { found } => {
				write!(f, "Missing protocol function `DISPLAY_FMT` for `{found}`")
			}
			ExpectedNumber { found } => messages::expected_number(f, found),
			IndexTarget { target, index } => messages::index_get(f, target, index),
			IndexSet {
				target,
				index,
				value,
			} => messages::index_set(f, target, index, value),
			TupleIndexGet { target, index } => messages::tuple_get(f, target, index),
			TupleIndexSet { target } => messages::tuple_set(f, target),
			None => match self.error.kind {
				ErrorKind::Overflow => f.write_str("Numerical overflow"),
				ErrorKind::DivisionByZero => f.write_str("Division by zero"),
				kind => write!(f, "{}: {kind:?}", self.error.operation),
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::super::{
		code::{Builder, Function, Op},
		error::{Location, Result},
		execute::Failure,
	};
	use super::*;
	use std::string::ToString;

	fn program() -> Program {
		let mut f = Function::new(1, 0).unwrap();
		f.string("missing\nkey").unwrap();
		f.push(Op::Return { value: 0 }, Location::default())
			.unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		Program::for_diagnostics_test(b.finish().unwrap())
	}

	#[test]
	fn owned_layout_stays_within_budget_including_io_payload() {
		use core::mem::{align_of, size_of};
		assert!(size_of::<Detail>() <= 32);
		assert_eq!(
			[
				size_of::<Detail>(),
				size_of::<Error>(),
				size_of::<Failure>()
			],
			[32, 88, 248]
		);
		assert!(align_of::<Detail>() <= 8);
		std::println!(
			"Detail={} align={} Error={} Result<Value>={} Result<()>={} Failure={}",
			size_of::<Detail>(),
			align_of::<Detail>(),
			size_of::<Error>(),
			size_of::<Result<Value>>(),
			size_of::<Result<()>>(),
			size_of::<Failure>()
		);
	}

	#[test]
	fn io_details_preserve_os_errors_and_generalize_custom_payloads() {
		let p = program();
		for original in [
			std::io::Error::from_raw_os_error(32),
			std::io::Error::other("custom payload intentionally omitted"),
		] {
			let os = original.raw_os_error();
			let generic = std::io::Error::from(original.kind());
			let expected = std::format!(
				"Panicked: failed printing to stdout: {}",
				if os.is_some() { &original } else { &generic }
			);
			let error = Error::new(ErrorKind::Output, "execute").with_detail(Detail::Io {
				kind: original.kind(),
				os,
			});
			assert_eq!(error.message(&p).to_string(), expected);
		}
	}

	#[test]
	fn field_identity_checks_all_indices_and_never_roots_the_store() {
		let p = program();
		let other = program();
		let field = FieldRef {
			code: p.code.id(),
			function: 0,
			string: 0,
		};
		let error =
			Error::new(ErrorKind::IndexOutOfBounds, "store").with_detail(Detail::MissingField {
				target: TypeName::Object,
				field,
			});
		assert_eq!(
			error.message(&p).to_string(),
			"Missing field `missing\nkey` on `std::object::Object`"
		);
		assert_eq!(
			error.message(&other).to_string(),
			"Missing field on `std::object::Object`"
		);
		for field in [
			FieldRef {
				function: u32::MAX,
				..field
			},
			FieldRef {
				string: u32::MAX,
				..field
			},
		] {
			let e = error.with_detail(Detail::MissingField {
				target: TypeName::Object,
				field,
			});
			assert_eq!(
				e.message(&p).to_string(),
				"Missing field on `std::object::Object`"
			);
		}
	}

	#[test]
	fn pure_rendering_preserves_integer_extremes_and_error_metadata() {
		let p = program();
		for (value, to, expected) in [
			(
				Value::I64(i64::MIN),
				IntTarget::Usize,
				"Failed to convert value `-9223372036854775808` to integer `usize`",
			),
			(
				Value::U64(u64::MAX),
				IntTarget::I64,
				"Failed to convert value `18446744073709551615` to integer `i64`",
			),
		] {
			let original =
				Error::new(ErrorKind::IntegerCoercion, "primitive operation").at(Location {
					source: 4,
					revision: 83,
					start: 9,
					end: 17,
				});
			let error = original.with_detail(Detail::Coerce {
				value: IntValue::of(value).unwrap(),
				to,
			});
			assert_eq!(error.message(&p).to_string(), expected);
			assert_eq!(
				(error.kind, error.operation, error.location, error.refusal),
				(
					original.kind,
					original.operation,
					original.location,
					original.refusal
				)
			);
		}
	}
	#[test]
	fn iterator_names_match_stock_types_and_classify_validated_payloads() {
		use super::super::heap::{Limits, Store};
		assert_eq!(
			TypeName::RangeIter.to_string(),
			crate::runtime::messages::DiagnosticType(&crate::runtime::TypeInfo::any::<
				crate::runtime::range::RangeIter<i64>,
			>())
			.to_string()
		);
		assert_eq!(
			TypeName::VecIter.to_string(),
			crate::runtime::messages::DiagnosticType(&crate::runtime::TypeInfo::any::<
				crate::runtime::slice::Iter,
			>())
			.to_string()
		);
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 4096,
		})
		.unwrap();
		let range = store.range(0, 2).unwrap();
		let iterator = store.iterator(range).unwrap();
		assert_eq!(store.diagnostic_type(iterator), Some(TypeName::RangeIter));
		store.release(iterator).unwrap();
		store.release(range).unwrap();
		let value = store.string("stale").unwrap();
		store.release(value).unwrap();
		assert_eq!(store.diagnostic_type(value), None);
	}
	#[test]
	fn nominal_diagnostic_names_shorten_only_the_presentation() {
		let rooted = "::display_d3::P";
		let nominal = super::super::nominal::intern(0xd3d3_0185_0000_0001, rooted).unwrap();
		assert_eq!(TypeName::Struct(nominal).to_string(), "display_d3::P");
		let mut buffer = [0; 256];
		let len = super::super::nominal::name_copy(nominal, &mut buffer).unwrap();
		assert_eq!(&buffer[..len], rooted.as_bytes());
	}
}

#[cfg(test)]
mod iteration_hash_tests {
	use super::*;
	use crate::runtime as rt;
	#[test]
	fn builtin_receiver_hashes_match_stock_values() {
		let rows = [
			(TypeName::Unit, rt::Value::unit()),
			(TypeName::Bool, crate::to_value(true).unwrap()),
			(TypeName::I64, crate::to_value(1i64).unwrap()),
			(TypeName::U64, crate::to_value(1u64).unwrap()),
			(TypeName::F64, crate::to_value(1f64).unwrap()),
			(TypeName::Char, crate::to_value('x').unwrap()),
			(
				TypeName::String,
				crate::to_value(crate::alloc::String::try_from("x").unwrap()).unwrap(),
			),
			(TypeName::Vec, rt::Value::new(rt::Vec::new()).unwrap()),
			(TypeName::Tuple, crate::to_value((1i64,)).unwrap()),
			(TypeName::Object, rt::Value::new(rt::Object::new()).unwrap()),
			(TypeName::Option, crate::to_value(Some(1i64)).unwrap()),
			(
				TypeName::Result,
				crate::to_value(core::result::Result::<i64, i64>::Ok(1)).unwrap(),
			),
			(TypeName::Range, crate::to_value(1i64..3i64).unwrap()),
			(
				TypeName::RangeInclusive,
				crate::to_value(1i64..=3i64).unwrap(),
			),
			(
				TypeName::Type,
				crate::to_value(rt::Type::new(crate::Hash::EMPTY)).unwrap(),
			),
		];
		for (name, stock) in rows {
			assert_eq!(name.builtin_hash(), Some(stock.type_hash()), "{name:?}");
		}
	}
}
