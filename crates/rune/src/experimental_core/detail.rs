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
	Ordering,
	String,
	Vec,
	Tuple,
	Object,
	Option,
	Result,
	Range,
	RangeInclusive,
	VecIter,
	RangeIter,
	UnsignedRangeIter,
	CharRangeIter,
	Function,
	Struct(Nominal),
	Type,
	RangeFrom,
	RangeFull,
	RangeToInclusive,
	RangeTo,
	RangeInclusiveIter,
	UnsignedRangeInclusiveIter,
	CharRangeInclusiveIter,
	RangeFromIter,
	UnsignedRangeFromIter,
	CharRangeFromIter,
	OptionIter,
	StringSplit,
	CharSplit,
	Chars,
	Skip,
	Take,
	ObjectIter,
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
	I32,
	Usize,
}

/// An integer operand exactly as the script supplied it.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntValue {
	Signed(i64),
	Unsigned(u64),
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

/// Failure while importing a certified dynamic host output.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostDynamicReason {
	ExternalAlias,
	Unsupported,
	Cycle,
	OutputAccess,
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
	/// Stock's `protocol_into_iter` failing on a value without `INTO_ITER`.
	MissingIntoIter {
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
	/// Taking a value was refused because iteration holds a read lease.
	TakeBorrowed,
	SharedArgument {
		arg: u8,
		readers: u32,
	},
	ReadExclusive,
	WriteExclusive,
	TakeExclusive,
	ExclusiveArgument {
		access: MovedAccess,
		arg: u8,
	},
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
	UnsupportedIterRangeInclusive {
		start: TypeName,
		end: TypeName,
	},
	UnsupportedIterRangeFrom {
		start: TypeName,
	},
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
	/// Exact type hashes copied from a native conversion error, never from its text.
	HostExpectedType {
		expected: crate::Hash,
		actual: crate::Hash,
		arg: Option<u8>,
	},
	HostExpectedNumber {
		actual: crate::Hash,
		arg: Option<u8>,
	},
	/// A Rust unwind contained at the synchronous host boundary.
	HostPanicked,
	/// No argument index: the native call succeeded and its output was rejected.
	HostDynamic {
		reason: HostDynamicReason,
		type_hash: Option<crate::Hash>,
	},
	/// IEEE operands retained without losing NaN payload or sign bits.
	FloatComparison {
		lhs_bits: u64,
		rhs_bits: u64,
	},
	/// Native String argument read failed after the method operands were evaluated.
	CmpArgumentAccess {
		arg: u8,
		access: MovedAccess,
	},
	/// A moved read at an actual formatting protocol argument boundary.
	FormatArgumentMoved {
		arg: u8,
		access: MovedAccess,
	},
	/// A moved access at the iterator conversion or next protocol boundary.
	IteratorArgumentMoved {
		arg: u8,
		access: MovedAccess,
	},
	/// A moved access at a proven native argument boundary.
	MovedArgument {
		arg: u8,
		access: MovedAccess,
	},
	/// Taking a native argument was refused because iteration holds a read lease.
	TakeSharedArgument {
		arg: u8,
		readers: u32,
	},
	/// A native tuple conversion found the wrong number of elements.
	TupleLength {
		actual: usize,
		expected: usize,
	},
	/// Stock Vec constructor layout exceeds the allocator addressable limit.
	CapacityOverflow,
}

impl Detail {
	/// Native argument index retained by a scalar builtin or certified host conversion failure.
	pub fn numeric_argument(self) -> Option<(HostCause, u8)> {
		match self {
			Self::NumericArgumentType { cause, arg, .. }
			| Self::NumericArgumentNumber { cause, arg, .. }
			| Self::NumericArgumentCoerce { cause, arg, .. } => Some((cause, arg)),
			Self::SharedArgument { arg, .. }
			| Self::TakeSharedArgument { arg, .. }
			| Self::FormatArgumentMoved { arg, .. }
			| Self::IteratorArgumentMoved { arg, .. }
			| Self::MovedArgument { arg, .. }
			| Self::CmpArgumentAccess { arg, .. }
			| Self::ExclusiveArgument { arg, .. } => Some((HostCause::Access, arg)),
			Self::HostExpectedType { arg: Some(arg), .. } => Some((HostCause::ExpectedType, arg)),
			Self::HostExpectedNumber { arg: Some(arg), .. } => {
				Some((HostCause::ExpectedNumber, arg))
			}
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
			Self::Ordering => core::cmp::Ordering::HASH,
			Self::String => crate::alloc::String::HASH,
			Self::Vec => rt::Vec::HASH,
			Self::Object => rt::Object::HASH,
			Self::Option => Option::<rt::Value>::HASH,
			Self::Result => core::result::Result::<rt::Value, rt::Value>::HASH,
			Self::Range => rt::Range::HASH,
			Self::CharRangeFromIter => rt::range_from::RangeFromIter::<char>::HASH,
			Self::UnsignedRangeFromIter => rt::range_from::RangeFromIter::<u64>::HASH,
			Self::RangeFromIter => rt::range_from::RangeFromIter::<i64>::HASH,
			Self::CharRangeInclusiveIter => rt::range_inclusive::RangeInclusiveIter::<char>::HASH,
			Self::UnsignedRangeInclusiveIter => {
				rt::range_inclusive::RangeInclusiveIter::<u64>::HASH
			}
			Self::RangeInclusiveIter => rt::range_inclusive::RangeInclusiveIter::<i64>::HASH,
			Self::RangeTo => rt::RangeTo::HASH,
			Self::RangeToInclusive => rt::RangeToInclusive::HASH,
			Self::RangeFull => rt::RangeFull::HASH,
			Self::RangeFrom => rt::RangeFrom::HASH,
			Self::RangeInclusive => rt::RangeInclusive::HASH,
			Self::VecIter => rt::slice::Iter::HASH,
			Self::ObjectIter => rt::object::RuneIter::HASH,
			Self::OptionIter => crate::modules::option::Iter::HASH,
			Self::StringSplit => crate::modules::string::Split::<crate::alloc::String>::HASH,
			Self::CharSplit => crate::modules::string::Split::<char>::HASH,
			Self::Chars => crate::modules::string::Chars::HASH,
			Self::Skip => crate::modules::iter::Skip::HASH,
			Self::Take => crate::modules::iter::Take::HASH,
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
			Type::Ordering => Self::Ordering,
			Type::String => Self::String,
			Type::Vec => Self::Vec,
			Type::Tuple => Self::Tuple,
			Type::Object => Self::Object,
			Type::Struct(n) => Self::Struct(n),
			Type::Option => Self::Option,
			Type::Result => Self::Result,
			Type::Range => Self::Range,
			Type::RangeTo => Self::RangeTo,
			Type::RangeToInclusive => Self::RangeToInclusive,
			Type::RangeInclusive => Self::RangeInclusive,
			Type::RangeFull => Self::RangeFull,
			Type::RangeFrom => Self::RangeFrom,
			Type::Iterator => Self::VecIter,
			Type::Function => Self::Function,
		}
	}
}

impl crate::runtime::messages::OperandType for TypeName {
	fn fmt_operand(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let name = match self {
			Self::I64 => "i64",
			Self::U64 => "u64",
			Self::F64 => "f64",
			Self::Bool => "bool",
			Self::Char => "char",
			Self::String => "String",
			Self::Vec => "Vec",
			_ => return fmt::Display::fmt(self, f),
		};
		f.write_str(name)
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
			Self::Ordering => "std::cmp::Ordering",
			Self::String => "std::string::String",
			Self::Vec => "std::vec::Vec",
			Self::Tuple => "std::tuple::Tuple",
			Self::Object => "std::object::Object",
			Self::Option => "std::option::Option",
			Self::Result => "std::result::Result",
			Self::Range => "std::ops::Range",
			Self::CharRangeFromIter => "std::ops::RangeFromIter<std::char>",
			Self::UnsignedRangeFromIter => "std::ops::RangeFromIter<std::u64>",
			Self::RangeFromIter => "std::ops::RangeFromIter<std::i64>",
			Self::CharRangeInclusiveIter => "std::ops::RangeInclusiveIter<std::char>",
			Self::UnsignedRangeInclusiveIter => "std::ops::RangeInclusiveIter<std::u64>",
			Self::RangeInclusiveIter => "std::ops::RangeInclusiveIter<std::i64>",
			Self::RangeTo => "std::ops::RangeTo",
			Self::RangeToInclusive => "std::ops::RangeToInclusive",
			Self::RangeFull => "std::ops::RangeFull",
			Self::RangeFrom => "std::ops::RangeFrom",
			Self::RangeInclusive => "std::ops::RangeInclusive",
			Self::VecIter => "std::slice::Iter",
			Self::ObjectIter => "std::object::Iter",
			Self::OptionIter => "std::option::Iter",
			Self::StringSplit => "std::string::Split<std::string::String>",
			Self::CharSplit => "std::string::Split<std::char>",
			Self::Chars => "std::string::Chars",
			Self::Skip => "std::iter::Skip",
			Self::Take => "std::iter::Take",
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
			Value::I64(n) => Self::Signed(n),
			Value::U64(n) => Self::Unsigned(n),
			_ => return None,
		})
	}

	/// The value as one wide integer; every `i64` and `u64` fits.
	pub(crate) fn wide(self) -> i128 {
		match self {
			Self::Signed(n) => i128::from(n),
			Self::Unsigned(n) => i128::from(n),
		}
	}
}

impl fmt::Display for IntValue {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Signed(n) => n.fmt(f),
			Self::Unsigned(n) => n.fmt(f),
		}
	}
}

impl fmt::Display for IntTarget {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::I64 => "i64",
			Self::U64 => "u64",
			Self::U32 => "u32",
			Self::I32 => "i32",
			Self::Usize => "usize",
		})
	}
}

impl FieldRef {
	pub(crate) fn resolve(self, program: &Program) -> Option<&str> {
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

/// A borrowed rendering of an owned error.
pub struct Message<'a> {
	pub(crate) error: &'a Error,
	pub(crate) program: &'a Program,
}

pub(super) fn numeric_symbol(op: BinarySymbol) -> &'static str {
	match op {
		BinarySymbol::Add => "+",
		BinarySymbol::Sub => "-",
		BinarySymbol::Mul => "*",
		BinarySymbol::Div => "/",
		BinarySymbol::Rem => "%",
		BinarySymbol::Shl => "<<",
		BinarySymbol::Shr => ">>",
		BinarySymbol::AddAssign => "+=",
		BinarySymbol::SubAssign => "-=",
		BinarySymbol::MulAssign => "*=",
		BinarySymbol::DivAssign => "/=",
		BinarySymbol::RemAssign => "%=",
		BinarySymbol::ShlAssign => "<<=",
		BinarySymbol::ShrAssign => ">>=",
		_ => "",
	}
}

fn access_message(
	f: &mut fmt::Formatter<'_>,
	access: MovedAccess,
	arg: Option<u8>,
	state: &str,
) -> fmt::Result {
	let operation = match access {
		MovedAccess::Read => "read",
		MovedAccess::Write => "write",
		MovedAccess::Take => "take",
	};
	f.write_str("Cannot ")?;
	f.write_str(operation)?;
	f.write_str(" ")?;
	match arg {
		Some(0) => f.write_str("the receiver")?,
		Some(arg) => write!(f, "argument {arg}")?,
		None => f.write_str("the value")?,
	}
	write!(f, ": {state}")
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
			Binary { op, lhs, rhs } => messages::binary_unsupported(
				f,
				op,
				messages::Operand(&lhs),
				messages::Operand(&rhs),
			),
			Unary { op, operand } => messages::unary_unsupported(
				f,
				match op {
					UnarySymbol::Neg => "-",
					UnarySymbol::Not => "!",
				},
				messages::Operand(&operand),
			),
			ExpectedBool { found } => messages::expected_type(f, TypeName::Bool, found),
			CapacityOverflow => f.write_str("Capacity overflow"),
			ValueDepth => f.write_str("Value is nested too deeply to walk, limit is 64"),
			ExpectedType { expected, found } => messages::expected_type(f, expected, found),
			NumericArgumentType {
				expected, found, ..
			} => messages::expected_type(f, expected, found),
			NumericArgumentNumber { found, .. } => messages::expected_number(f, found),
			NumericArgumentCoerce { value, to, .. } => messages::coerce(f, value, to),
			MissingIntoIter { found } => {
				write!(f, "Missing protocol function `INTO_ITER` for `{found}`")
			}
			StringOperand { found } => messages::expected_type(f, TypeName::String, found),
			Coerce { value, to } => messages::coerce(f, value, to),
			WriteBorrowed => {
				access_message(f, MovedAccess::Write, Option::None, "it is being read")
			}
			TakeBorrowed => access_message(f, MovedAccess::Take, Option::None, "it is being read"),
			SharedArgument { arg, .. } => {
				access_message(f, MovedAccess::Write, Some(arg), "it is being read")
			}
			TakeSharedArgument { arg, .. } => {
				access_message(f, MovedAccess::Take, Some(arg), "it is being read")
			}
			ExclusiveArgument { access, arg } => {
				access_message(f, access, Some(arg), "it is being written")
			}
			ReadExclusive => {
				access_message(f, MovedAccess::Read, Option::None, "it is being written")
			}
			WriteExclusive => {
				access_message(f, MovedAccess::Write, Option::None, "it is being written")
			}
			TakeExclusive => {
				access_message(f, MovedAccess::Take, Option::None, "it is being written")
			}
			ShiftWidth => f.write_str("Numerical overflow"),
			Underflow => f.write_str("Numerical underflow"),
			OutOfRange { index, length } => messages::out_of_range(f, index, length),
			MissingIndex { target } => write!(f, "Type `{target}` missing index"),
			MissingStringSlice => f.write_str("Panicked: missing string slice"),
			PanicMessage => f.write_str("Panicked"),
			HostMessage { .. } | HostExpectedType { .. } | HostExpectedNumber { .. } => {
				f.write_str("Host function failed")
			}
			HostPanicked => f.write_str("host panicked"),
			HostDynamic { reason, type_hash } => {
				use HostDynamicReason as R;
				match reason {
					R::ExternalAlias => f.write_str(
						"host output contract violation: externally aliased or borrowed value",
					),
					R::Unsupported => {
						f.write_str(
							"host output contract violation: value outside the importable domain",
						)?;
						if let Some(hash) = type_hash {
							write!(f, " of type {hash}")?;
						}
						Ok(())
					}
					R::Cycle => f.write_str("host output contract violation: cyclic value"),
					R::OutputAccess => {
						f.write_str("host output contract violation: unreadable value")?;
						if let Some(hash) = type_hash {
							write!(f, " of type {hash}")?;
						}
						Ok(())
					}
				}
			}
			FloatComparison { lhs_bits, rhs_bits } => core::fmt::Display::fmt(
				&crate::runtime::VmErrorKind::IllegalFloatComparison {
					lhs: f64::from_bits(lhs_bits),
					rhs: f64::from_bits(rhs_bits),
				},
				f,
			),
			UnsupportedIterRangeInclusive { start, end } => {
				write!(f, "Cannot build an iterator out of {start}..={end}")
			}
			UnsupportedIterRangeFrom { start } => {
				write!(f, "Cannot build an iterator out of {start}..")
			}
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
			Moved { access }
			| FormatArgumentMoved { access, .. }
			| IteratorArgumentMoved { access, .. }
			| MovedArgument { access, .. }
			| CmpArgumentAccess { access, .. } => {
				access_message(f, access, Option::None, "it has been moved")
			}
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
			MissingInstanceFunction {
				target,
				hash,
				instance,
			} => {
				let name_hash = crate::Hash::associated_function(instance, hash);
				if name_hash == crate::runtime::Protocol::INTO_ITER.hash {
					return write!(f, "Cannot iterate over `{target}`");
				}
				let mut name = Option::None;
				if let Some(at) = self.error.location {
					if let Some(source) = self
						.program
						.sources()
						.iter()
						.find(|source| source.id == at.source && source.revision == at.revision)
					{
						let mut lexer =
							crate::parse::Lexer::new(&source.text, crate::SourceId::EMPTY, false)
								.without_processing();
						let mut after_dot = false;
						for _ in 0..=source.text.len() {
							let Ok(Some(token)) = lexer.next() else {
								break;
							};
							match token.kind {
								crate::ast::Kind::Whitespace
								| crate::ast::Kind::Comment
								| crate::ast::Kind::MultilineComment(true) => continue,
								crate::ast::Kind::MultilineComment(false) => break,
								_ => {}
							}
							if after_dot && matches!(token.kind, crate::ast::Kind::Ident(..)) {
								if let Some(candidate) = source.text.get(token.span.range()) {
									if crate::Hash::associated_function(
										instance,
										crate::Hash::ident(candidate),
									) == hash
									{
										name = Some(candidate);
										break;
									}
								}
							}
							after_dot = matches!(token.kind, crate::ast::Kind::Dot);
						}
					}
				}
				match name {
					Some(name) => write!(f, "Type `{target}` has no method named `{name}`"),
					Option::None => write!(f, "Type `{target}` has no matching method"),
				}
			}
			NotIterable { found } => write!(f, "Type `{found}` is not iterable"),
			TryOperand { found } => messages::try_operand(f, found),
			NoDisplay { found } => {
				write!(f, "`{found}` cannot be formatted with `{{}}`")
			}
			ExpectedNumber { found } => messages::expected_number(f, found),
			TupleLength { actual, expected } => write!(
				f,
				"Expected a tuple of length `{expected}`, but found one with length `{actual}`"
			),
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
			[32, 88, 272]
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
		assert_eq!(
			TypeName::OptionIter.to_string(),
			crate::runtime::messages::DiagnosticType(&crate::runtime::TypeInfo::any::<
				crate::modules::option::Iter,
			>())
			.to_string()
		);
		assert_eq!(
			TypeName::OptionIter.builtin_hash(),
			Some(<crate::modules::option::Iter as crate::TypeHash>::HASH)
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
		let option = store
			.variant(Type::Option, true, Some(Value::I64(7)))
			.unwrap();
		let iter = store.iterator(option).unwrap();
		assert_eq!(store.diagnostic_type(iter), Some(TypeName::OptionIter));
		store.release(iter).unwrap();
		store.release(option).unwrap();
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

#[cfg(test)]
#[path = "tests/method_diagnostics_controls.rs"]
mod method_diagnostics_tests;
