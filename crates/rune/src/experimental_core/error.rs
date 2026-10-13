//! Errors of the fresh core.
//!
//! An [`Error`] is a small `Copy` value, so reporting an allocation failure allocates
//! nothing. Compile-time refusals carry a [`Refusal`] category; run-time errors carry
//! an [`ErrorKind`], optional owned [`Detail`], and, when known, a source [`Location`].

pub use super::detail::{
	AdmittedMethod, BinarySymbol, Detail, FieldRef, IntTarget, IntValue, Message, MovedAccess,
	TypeName, UnarySymbol,
};

/// The class of an error.
///
/// Run-time classes are coarser than the stock VM's error kinds: several stock kinds map
/// to one class (a declared difference).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
	/// The program uses something outside the admitted subset (a compile-time refusal; see [`Error::refusal`]).
	Unsupported,
	/// An operand or value has the wrong type for the operation, including a missing display form.
	Type,
	/// Checked integer arithmetic overflowed or underflowed.
	Overflow,
	/// Integer division or remainder by zero.
	DivisionByZero,
	/// An index or field is not present: out of range or a missing object field.
	IndexOutOfBounds,
	/// A shift count is negative or not below the operand's bit width.
	InvalidShift,
	/// An `as` conversion between these types is not supported.
	InvalidCast,
	/// An integer operand does not fit the required integer type, including an index
	/// that cannot be represented as `usize`.
	IntegerCoercion,
	/// Formatted text exceeded its formatting limits.
	FormatLimit,
	/// A structural comparison exceeded the value walk depth (64, including the root).
	ValueDepth,
	/// Writing to stdout failed.
	Output,
	/// A memory reservation failed.
	Allocation,
	/// The store's entry or byte limit ([`Limits`](crate::experimental_core::heap::Limits)) would be exceeded.
	StoreCapacity,
	/// The store was closed.
	ClosedStore,
	/// A root or handle belongs to another store, or its entry no longer exists.
	InvalidHandle,
	/// A value is invalidated, or is mutated or released while an iteration over it is in
	/// progress.
	AccessConflict,
	/// The invocation ran out of fuel (one unit per charged instruction).
	FuelExhausted,
	/// Code or an entry request is invalid: an unknown function, a wrong arity or malformed code.
	InvalidCode,
	/// The call-frame or slot limit of the executor was reached.
	FrameCapacity,
	/// A cancellation requested with [`Executor::request_cancel`](crate::experimental_core::execute::Executor::request_cancel) was consumed.
	Cancelled,
}

/// Named compile-time refusal categories. The adapter consumes this enum,
/// never parses prose or guesses a category from an error message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
	/// A call with the wrong number of arguments.
	Arity,
	/// An assignment target outside the subset.
	Assignment,
	/// An `async` block.
	AsyncBlock,
	/// An `async fn`.
	AsyncFunction,
	/// An `.await` expression.
	Await,
	/// A byte-string literal.
	ByteStr,
	/// A call of an expression that is not a named function.
	Call,
	/// A closure.
	CallClosure,
	/// A condition form outside the subset.
	Condition,
	/// A call of a `const fn`.
	ConstCall,
	/// A constant collection. Reserved: not currently produced.
	ConstCollection,
	/// A cyclic constant. Reserved: not currently produced.
	ConstCycle,
	/// A `const fn` item. Reserved: not currently produced.
	ConstFn,
	/// Expressions nested deeper than the lowering limit.
	DepthLimit,
	/// A field access form outside the subset, such as one with generics.
	Field,
	/// A tuple field index that does not fit.
	FieldIndex,
	/// A format expression outside the admitted `format!`/`println!` forms.
	Format,
	/// Too many parts in a format string or template.
	FormatParts,
	/// A format specification outside the subset.
	FormatSpec,
	/// The Rune compiler reported an error before lowering; see the diagnostics.
	FrontendDiagnostic,
	/// A generator function (one that yields).
	GeneratorFunction,
	/// An `impl` item. Reserved: not currently produced.
	Impl,
	/// An import. Reserved: not currently produced.
	Import,
	/// A labelled block.
	LabelledBlock,
	/// A `break` or `continue` outside a loop it can reach.
	LoopControl,
	/// A macro other than the admitted std format macros. Reserved: not currently produced; such macros are refused through [`Refusal::FrontendDiagnostic`].
	Macro,
	/// A `match` expression.
	Match,
	/// A method call outside the subset.
	Method,
	/// A module item. Reserved: not currently produced.
	Module,
	/// A call of a native function, including `println!` when the context's printer is not the certified std stdio builtin.
	Native,
	/// An object constructor form outside the subset.
	ObjectConstructor,
	/// An operator outside the subset.
	Operator,
	/// A pattern outside the subset.
	Pattern,
	/// A range form outside the subset.
	Range,
	/// A `select` expression.
	Select,
	/// Too many sources, or a source too large to address.
	SourceLimit,
	/// A static item.
	Static,
	/// A type expression outside the subset.
	TypeExpr,
	/// The process-wide struct type table is full or a type name is too long (4,096
	/// identities, 256-byte names, 256 KiB of names per process, never reclaimed; see
	/// [`Nominal`](crate::experimental_core::value::Nominal)).
	TypeTable,
	/// A constant that cannot be resolved for the fresh core.
	UnresolvedConstant,
	/// A call of a function that is not part of the program.
	UnresolvedFunction,
	/// A variable that cannot be resolved.
	Variable,
	/// A `yield` expression.
	Yield,
}

impl Refusal {
	/// Stable category names, including the reviewed batch E vocabulary.
	pub const fn name(self) -> &'static str {
		match self {
			Self::Arity => "Arity",
			Self::Assignment => "Assignment",
			Self::AsyncBlock => "AsyncBlock",
			Self::AsyncFunction => "AsyncFunction",
			Self::Await => "Await",
			Self::ByteStr => "ByteStr",
			Self::Call => "Call",
			Self::CallClosure => "CallClosure",
			Self::Condition => "Condition",
			Self::ConstCall => "ConstCall",
			Self::ConstCollection => "ConstCollection",
			Self::ConstCycle => "ConstCycle",
			Self::ConstFn => "ConstFn",
			Self::DepthLimit => "DepthLimit",
			Self::Field => "Field",
			Self::FieldIndex => "FieldIndex",
			Self::Format => "Format",
			Self::FormatParts => "FormatParts",
			Self::FormatSpec => "FormatSpec",
			Self::FrontendDiagnostic => "FrontendDiagnostic",
			Self::GeneratorFunction => "GeneratorFunction",
			Self::Impl => "Impl",
			Self::Import => "Import",
			Self::LabelledBlock => "LabelledBlock",
			Self::LoopControl => "LoopControl",
			Self::Macro => "Macro",
			Self::Match => "Match",
			Self::Method => "Method",
			Self::Module => "Module",
			Self::Native => "Native",
			Self::ObjectConstructor => "ObjectConstructor",
			Self::Operator => "Operator",
			Self::Pattern => "Pattern",
			Self::Range => "Range",
			Self::Select => "Select",
			Self::SourceLimit => "SourceLimit",
			Self::Static => "Static",
			Self::TypeExpr => "TypeExpr",
			Self::TypeTable => "TypeTable",
			Self::UnresolvedConstant => "UnresolvedConstant",
			Self::UnresolvedFunction => "UnresolvedFunction",
			Self::Variable => "Variable",
			Self::Yield => "Yield",
		}
	}
}

/// A byte range in one source of a compiled program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Location {
	/// The source id, as assigned by [`Sources`](crate::Sources).
	pub source: u32,
	/// The revision passed to [`compile`](crate::experimental_core::frontend::compile).
	pub revision: u64,
	/// Start byte offset.
	pub start: u32,
	/// End byte offset (exclusive).
	pub end: u32,
}

/// An error of the fresh core: a compile-time refusal or a run-time error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
	/// The error class.
	pub kind: ErrorKind,
	/// Where the error happened, when known.
	pub location: Option<Location>,
	/// A short, static name of the operation that failed.
	pub operation: &'static str,
	/// The refusal category; `Some` exactly for compile-time refusals.
	pub refusal: Option<Refusal>,
	/// Owned language-error information; absent for generic resource errors.
	pub detail: Detail,
}

impl Error {
	/// An error of `kind` raised by `operation`, without a location.
	pub const fn new(kind: ErrorKind, operation: &'static str) -> Self {
		Self {
			kind,
			location: None,
			operation,
			refusal: None,
			detail: Detail::None,
		}
	}

	/// A compile-time refusal: kind [`ErrorKind::Unsupported`], operation the refusal's
	/// [`name`](Refusal::name).
	pub const fn refused(refusal: Refusal) -> Self {
		Self {
			kind: ErrorKind::Unsupported,
			location: None,
			operation: refusal.name(),
			refusal: Some(refusal),
			detail: Detail::None,
		}
	}

	/// Attach owned detail without changing the class, operation or location.
	pub const fn with_detail(mut self, detail: Detail) -> Self {
		self.detail = detail;
		self
	}

	/// Render without allocating. A foreign program never resolves constant field names.
	pub fn message<'a>(&'a self, program: &'a super::frontend::Program) -> Message<'a> {
		Message {
			error: self,
			program,
		}
	}

	/// The same error at `location`.
	pub fn at(mut self, location: Location) -> Self {
		self.location = Some(location);
		self
	}
}

impl From<crate::alloc::Error> for Error {
	fn from(_: crate::alloc::Error) -> Self {
		Self::new(ErrorKind::Allocation, "reserve")
	}
}

/// The result type of the fresh core.
pub type Result<T> = core::result::Result<T, Error>;
