//! Fail-closed semantic classes shared by both execution engines.
use super::error::{Detail, Error, ErrorKind};
use crate::runtime::{VmError, VmErrorKind, VmHaltInfo};

/// A reviewed user-visible failure class.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
	/// Type, operation or callable mismatch. Equality also requires matching
	/// observable context and applicable typed detail; known origins must agree.
	TypeMismatch,
	/// Missing index or field.
	IndexOutOfBounds,
	/// Arithmetic overflow or underflow.
	Overflow,
	/// Integer division by zero.
	DivisionByZero,
	/// NaN ordering is undefined.
	IllegalFloatComparison,
	/// Value-walk depth limit.
	ValueDepth,
	/// Callable arity mismatch.
	ArgumentCount,
	/// Script panic.
	Panic,
	/// Access to a moved or borrowed value.
	AccessConflict,
	/// Integer conversion cannot represent the value.
	IntegerCoercion,
	/// Cast source or target is invalid.
	InvalidCast,
	/// Execution budget exhausted.
	BudgetExhausted,
	/// Memory allocation failed.
	Allocation,
}

/// Proven origin of the primary error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorOrigin {
	/// Failure occurred in a reviewed script operation or unbound associated lookup.
	Script,
	/// Failure occurred in a uniquely resolved native function.
	Host,
	/// The recorded evidence is insufficient to distinguish script from host.
	Unknown,
}

/// An explicit result; only matching `Class` values may be cross-compared.
///
/// ```compile_fail
/// use rune::experimental_core::error::ErrorProjection;
/// let _ = ErrorProjection::EngineOnly == ErrorProjection::EngineOnly;
/// ```
#[derive(Clone, Copy, Debug)]
pub enum ErrorProjection {
	/// A reviewed common semantic class.
	Class(ErrorClass),
	/// An execution-core resource or embedding boundary without a stock equivalent.
	EngineOnly,
	/// A kind whose common semantics have not yet been reviewed.
	Unclassified(&'static str),
}

/// The reviewed class, or an unclassified identity, retained for a host failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostClass {
	/// A reviewed common class.
	Reviewed(ErrorClass),
	/// A legacy kind whose common semantics are not yet reviewed.
	Unclassified(&'static str),
}

impl ErrorProjection {
	/// Compare reviewed classes only; compare applicable context and details separately.
	pub fn same_class(self, other: Self) -> bool {
		matches!((self, other), (Self::Class(a), Self::Class(b)) if a == b)
	}
}

impl VmError {
	/// Determine origin from the innermost recorded VM window and its own context.
	/// Unknown is never evidence of script/host equality.
	pub fn origin(&self) -> ErrorOrigin {
		self.origin_evidence().0
	}

	/// Return the faulting instruction name, when recorded, for unknown-origin audits.
	pub fn origin_instruction(&self) -> Option<&'static str> {
		self.origin_evidence().1
	}

	fn origin_evidence(&self) -> (ErrorOrigin, Option<&'static str>) {
		self.first_location()
			.and_then(|at| at.origin)
			.unwrap_or((ErrorOrigin::Unknown, None))
	}

	pub(crate) fn capture_origin(
		&self,
		vm: &crate::runtime::Vm,
	) -> (ErrorOrigin, Option<&'static str>) {
		use crate::runtime::inst::Kind;
		let unit = vm.unit();
		let context = vm.context();
		let Ok(Some((instruction, _))) = unit.instruction_at(vm.last_ip()) else {
			return (ErrorOrigin::Unknown, None);
		};
		match instruction.kind {
			Kind::Allocate { .. } => (ErrorOrigin::Unknown, Some("Allocate")),
			Kind::Not { .. } => (ErrorOrigin::Unknown, Some("Not")),
			Kind::Neg { .. } => (ErrorOrigin::Unknown, Some("Neg")),
			Kind::Closure { .. } => (ErrorOrigin::Unknown, Some("Closure")),
			Kind::CallOffset { .. } => (ErrorOrigin::Unknown, Some("CallOffset")),
			Kind::Call { hash, .. } => {
				let host = context.function(&hash).is_some() && unit.function(&hash).is_none();
				(
					if host {
						ErrorOrigin::Host
					} else {
						ErrorOrigin::Unknown
					},
					Some("Call"),
				)
			}
			Kind::CallAssociated { hash, .. } => {
				let script = self.missing_instance_function().is_some_and(|missing| {
					let combined = crate::Hash::associated_function(missing.instance, hash);
					combined == missing.hash
						&& context.function(&combined).is_none()
						&& unit.function(&combined).is_none()
				});
				(
					if script {
						ErrorOrigin::Script
					} else {
						ErrorOrigin::Unknown
					},
					Some("CallAssociated"),
				)
			}
			Kind::LoadInstanceFn { .. } => (ErrorOrigin::Unknown, Some("LoadInstanceFn")),
			Kind::CallFn { .. } => (ErrorOrigin::Unknown, Some("CallFn")),
			Kind::IndexGet { .. } => (ErrorOrigin::Unknown, Some("IndexGet")),
			Kind::TupleIndexSet { .. } => (ErrorOrigin::Unknown, Some("TupleIndexSet")),
			Kind::TupleIndexGetAt { .. } => (ErrorOrigin::Unknown, Some("TupleIndexGetAt")),
			Kind::ObjectIndexSet { .. } => (ErrorOrigin::Unknown, Some("ObjectIndexSet")),
			Kind::ObjectIndexGetAt { .. } => (ErrorOrigin::Unknown, Some("ObjectIndexGetAt")),
			Kind::IndexSet { .. } => (ErrorOrigin::Unknown, Some("IndexSet")),
			Kind::Await { .. } => (ErrorOrigin::Unknown, Some("Await")),
			Kind::Select { .. } => (ErrorOrigin::Unknown, Some("Select")),
			Kind::LoadFn { .. } => (ErrorOrigin::Unknown, Some("LoadFn")),
			Kind::Store { .. } => (ErrorOrigin::Unknown, Some("Store")),
			Kind::Copy { .. } => (ErrorOrigin::Unknown, Some("Copy")),
			Kind::Move { .. } => (ErrorOrigin::Unknown, Some("Move")),
			Kind::Drop { .. } => (ErrorOrigin::Unknown, Some("Drop")),
			Kind::Swap { .. } => (ErrorOrigin::Unknown, Some("Swap")),
			Kind::Return { .. } => (ErrorOrigin::Unknown, Some("Return")),
			Kind::ReturnUnit => (ErrorOrigin::Unknown, Some("ReturnUnit")),
			Kind::Jump { .. } => (ErrorOrigin::Unknown, Some("Jump")),
			Kind::JumpIf { .. } => (ErrorOrigin::Unknown, Some("JumpIf")),
			Kind::JumpIfNot { .. } => (ErrorOrigin::Unknown, Some("JumpIfNot")),
			Kind::Vec { .. } => (ErrorOrigin::Unknown, Some("Vec")),
			Kind::Tuple1 { .. } => (ErrorOrigin::Unknown, Some("Tuple1")),
			Kind::Tuple2 { .. } => (ErrorOrigin::Unknown, Some("Tuple2")),
			Kind::Tuple3 { .. } => (ErrorOrigin::Unknown, Some("Tuple3")),
			Kind::Tuple4 { .. } => (ErrorOrigin::Unknown, Some("Tuple4")),
			Kind::Tuple { .. } => (ErrorOrigin::Unknown, Some("Tuple")),
			Kind::Environment { .. } => (ErrorOrigin::Unknown, Some("Environment")),
			Kind::Object { .. } => (ErrorOrigin::Unknown, Some("Object")),
			Kind::Range { .. } => (ErrorOrigin::Unknown, Some("Range")),
			Kind::Struct { .. } => (ErrorOrigin::Unknown, Some("Struct")),
			Kind::ConstConstruct { .. } => (ErrorOrigin::Unknown, Some("ConstConstruct")),
			Kind::String { .. } => (ErrorOrigin::Unknown, Some("String")),
			Kind::Bytes { .. } => (ErrorOrigin::Unknown, Some("Bytes")),
			Kind::GlobalGet { .. } => (ErrorOrigin::Unknown, Some("GlobalGet")),
			Kind::GlobalSet { .. } => (ErrorOrigin::Unknown, Some("GlobalSet")),
			Kind::StringConcat { .. } => (ErrorOrigin::Unknown, Some("StringConcat")),
			Kind::Format { .. } => (ErrorOrigin::Unknown, Some("Format")),
			Kind::Try { .. } => (ErrorOrigin::Unknown, Some("Try")),
			Kind::EqChar { .. } => (ErrorOrigin::Unknown, Some("EqChar")),
			Kind::EqSigned { .. } => (ErrorOrigin::Unknown, Some("EqSigned")),
			Kind::EqUnsigned { .. } => (ErrorOrigin::Unknown, Some("EqUnsigned")),
			Kind::EqBool { .. } => (ErrorOrigin::Unknown, Some("EqBool")),
			Kind::EqString { .. } => (ErrorOrigin::Unknown, Some("EqString")),
			Kind::EqBytes { .. } => (ErrorOrigin::Unknown, Some("EqBytes")),
			Kind::MatchType { .. } => (ErrorOrigin::Unknown, Some("MatchType")),
			Kind::MatchSequence { .. } => (ErrorOrigin::Unknown, Some("MatchSequence")),
			Kind::MatchObject { .. } => (ErrorOrigin::Unknown, Some("MatchObject")),
			Kind::Yield { .. } => (ErrorOrigin::Unknown, Some("Yield")),
			Kind::YieldUnit { .. } => (ErrorOrigin::Unknown, Some("YieldUnit")),
			Kind::Op { .. } => (ErrorOrigin::Unknown, Some("Op")),
			Kind::Arithmetic { .. } => (ErrorOrigin::Unknown, Some("Arithmetic")),
			Kind::Bitwise { .. } => (ErrorOrigin::Unknown, Some("Bitwise")),
			Kind::Shift { .. } => (ErrorOrigin::Unknown, Some("Shift")),
			Kind::AssignArithmetic { .. } => (ErrorOrigin::Unknown, Some("AssignArithmetic")),
			Kind::AssignBitwise { .. } => (ErrorOrigin::Unknown, Some("AssignBitwise")),
			Kind::AssignShift { .. } => (ErrorOrigin::Unknown, Some("AssignShift")),
			Kind::IterNext { .. } => (ErrorOrigin::Unknown, Some("IterNext")),
			Kind::Panic { .. } => (ErrorOrigin::Script, Some("Panic")),
		}
	}

	/// Project the primary legacy error without parsing diagnostic text.
	/// Unreviewed kinds fail closed; callers must also compare origin and applicable details.
	pub fn error_class(&self) -> ErrorProjection {
		use ErrorClass as C;
		use ErrorProjection::{Class, Unclassified};
		match self.error().kind() {
			VmErrorKind::AllocError { .. } => Class(C::Allocation),
			VmErrorKind::AccessError { .. } => Class(C::AccessConflict),
			VmErrorKind::StackError { .. } => Unclassified("StackError"),
			VmErrorKind::SliceError { .. } => Unclassified("SliceError"),
			VmErrorKind::BadInstruction { .. } => Unclassified("BadInstruction"),
			VmErrorKind::BadJump { .. } => Unclassified("BadJump"),
			VmErrorKind::DynArgsUsed { .. } => Unclassified("DynArgsUsed"),
			VmErrorKind::Panic { .. } => Class(C::Panic),
			VmErrorKind::NoRunningVm => Unclassified("NoRunningVm"),
			VmErrorKind::Halted {
				halt: VmHaltInfo::Limited,
			} => Class(C::BudgetExhausted),
			VmErrorKind::Halted {
				halt: VmHaltInfo::Yielded,
			} => Unclassified("Halted::Yielded"),
			VmErrorKind::Halted {
				halt: VmHaltInfo::Awaited,
			} => Unclassified("Halted::Awaited"),
			VmErrorKind::Overflow => Class(C::Overflow),
			VmErrorKind::Underflow => Class(C::Overflow),
			VmErrorKind::DivideByZero => Class(C::DivisionByZero),
			VmErrorKind::MissingEntry { .. } => Unclassified("MissingEntry"),
			VmErrorKind::MissingEntryHash { .. } => Unclassified("MissingEntryHash"),
			VmErrorKind::MissingFunction { .. } => Unclassified("MissingFunction"),
			VmErrorKind::MissingContextFunction { .. } => Unclassified("MissingContextFunction"),
			VmErrorKind::NotOwned { .. } => Unclassified("NotOwned"),
			VmErrorKind::MissingProtocolFunction { .. } => Class(C::TypeMismatch),
			VmErrorKind::MissingInstanceFunction { .. } => Class(C::TypeMismatch),
			VmErrorKind::IpOutOfBounds { .. } => Unclassified("IpOutOfBounds"),
			VmErrorKind::UnsupportedBinaryOperation { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedUnaryOperation { .. } => Class(C::TypeMismatch),
			VmErrorKind::MissingStaticString { .. } => Unclassified("MissingStaticString"),
			VmErrorKind::MissingStaticBytes { .. } => Unclassified("MissingStaticBytes"),
			VmErrorKind::MissingStaticObjectKeys { .. } => Unclassified("MissingStaticObjectKeys"),
			VmErrorKind::MissingDropSet { .. } => Unclassified("MissingDropSet"),
			VmErrorKind::MissingGlobals { .. } => Unclassified("MissingGlobals"),
			VmErrorKind::BadGlobalSlot { .. } => Unclassified("BadGlobalSlot"),
			VmErrorKind::UninitializedGlobal { .. } => Unclassified("UninitializedGlobal"),
			VmErrorKind::MissingRtti { .. } => Unclassified("MissingRtti"),
			VmErrorKind::BadArgumentCount { .. } => Class(C::ArgumentCount),
			VmErrorKind::MaxValueDepth { .. } => Class(C::ValueDepth),
			VmErrorKind::MaxConstDepth { .. } => Unclassified("MaxConstDepth"),
			VmErrorKind::MaxConstSize { .. } => Unclassified("MaxConstSize"),
			VmErrorKind::MalformedConstValue => Unclassified("MalformedConstValue"),
			VmErrorKind::MaxExecutionDepth { .. } => Unclassified("MaxExecutionDepth"),
			VmErrorKind::BadEnvironmentCount { .. } => Unclassified("BadEnvironmentCount"),
			VmErrorKind::BadArgument { .. } => Unclassified("BadArgument"),
			VmErrorKind::UnsupportedIndexSet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIndexGet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedTupleIndexGet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedTupleIndexSet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedObjectSlotIndexGet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedObjectSlotIndexSet { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIs { .. } => Unclassified("UnsupportedIs"),
			VmErrorKind::UnsupportedAs { .. } => Class(C::InvalidCast),
			VmErrorKind::UnsupportedAsTarget { .. } => Class(C::InvalidCast),
			VmErrorKind::UnsupportedCallFn { .. } => Class(C::TypeMismatch),
			VmErrorKind::MissingIndex { .. } => Class(C::IndexOutOfBounds),
			VmErrorKind::MissingIndexInteger { .. } => Class(C::IndexOutOfBounds),
			VmErrorKind::MissingIndexKey { .. } => Unclassified("MissingIndexKey"),
			VmErrorKind::OutOfRange { .. } => Class(C::IndexOutOfBounds),
			VmErrorKind::UnsupportedTryOperand { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIterRangeInclusive { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIterRangeFrom { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIterRange { .. } => Class(C::TypeMismatch),
			VmErrorKind::UnsupportedIterNextOperand { .. } => {
				Unclassified("UnsupportedIterNextOperand")
			}
			VmErrorKind::ExpectedType { .. } => Class(C::TypeMismatch),
			VmErrorKind::ExpectedAny { .. } => Unclassified("ExpectedAny"),
			VmErrorKind::ExpectedNumber { .. } => Class(C::TypeMismatch),
			VmErrorKind::ExpectedEmpty { .. } => Unclassified("ExpectedEmpty"),
			VmErrorKind::ExpectedTuple { .. } => Unclassified("ExpectedTuple"),
			VmErrorKind::ExpectedStruct { .. } => Unclassified("ExpectedStruct"),
			VmErrorKind::MissingConstantConstructor { .. } => {
				Unclassified("MissingConstantConstructor")
			}
			VmErrorKind::ValueToIntegerCoercionError { .. } => Class(C::IntegerCoercion),
			VmErrorKind::IntegerToValueCoercionError { .. } => {
				Unclassified("IntegerToValueCoercionError")
			}
			VmErrorKind::ExpectedTupleLength { .. } => Class(C::TypeMismatch),
			VmErrorKind::ExpectedVecLength { .. } => Unclassified("ExpectedVecLength"),
			VmErrorKind::ConstNotSupported { .. } => Unclassified("ConstNotSupported"),
			VmErrorKind::MissingInterfaceEnvironment => Unclassified("MissingInterfaceEnvironment"),
			VmErrorKind::ExpectedExitedExecutionState { .. } => {
				Unclassified("ExpectedExitedExecutionState")
			}
			VmErrorKind::GeneratorComplete => Unclassified("GeneratorComplete"),
			VmErrorKind::FutureCompleted => Unclassified("FutureCompleted"),
			VmErrorKind::MissingVariant { .. } => Unclassified("MissingVariant"),
			VmErrorKind::MissingField { .. } => Class(C::IndexOutOfBounds),
			VmErrorKind::MissingVariantName => Unclassified("MissingVariantName"),
			VmErrorKind::MissingStructField { .. } => Unclassified("MissingStructField"),
			VmErrorKind::MissingTupleIndex { .. } => Unclassified("MissingTupleIndex"),
			VmErrorKind::ExpectedVariant { .. } => Unclassified("ExpectedVariant"),
			VmErrorKind::UnsupportedObjectFieldGet { .. } => {
				Unclassified("UnsupportedObjectFieldGet")
			}
			VmErrorKind::IllegalFloatComparison { .. } => Class(C::IllegalFloatComparison),
			VmErrorKind::IllegalFloatOperation { .. } => Unclassified("IllegalFloatOperation"),
			VmErrorKind::MissingCallFrame => Unclassified("MissingCallFrame"),
			VmErrorKind::IllegalFormat => Unclassified("IllegalFormat"),
		}
	}
}

impl Error {
	/// Project this error without parsing diagnostic text.
	/// Engine-only and unclassified results never compare equal across engines.
	pub fn error_class(&self) -> ErrorProjection {
		use ErrorClass as C;
		use ErrorProjection::{Class, EngineOnly, Unclassified};
		match self.kind {
			ErrorKind::Type => Class(C::TypeMismatch),
			ErrorKind::IllegalFloatComparison => Class(C::IllegalFloatComparison),
			ErrorKind::Overflow => Class(C::Overflow),
			ErrorKind::DivisionByZero => Class(C::DivisionByZero),
			ErrorKind::Panic => Class(C::Panic),
			ErrorKind::IndexOutOfBounds => Class(C::IndexOutOfBounds),
			ErrorKind::InvalidCast => Class(C::InvalidCast),
			ErrorKind::IntegerCoercion => Class(C::IntegerCoercion),
			ErrorKind::ValueDepth => Class(C::ValueDepth),
			ErrorKind::Allocation => Class(C::Allocation),
			ErrorKind::AccessConflict => Class(C::AccessConflict),
			ErrorKind::FuelExhausted => Class(C::BudgetExhausted),
			ErrorKind::ArgumentCount => Class(C::ArgumentCount),
			ErrorKind::InvalidShift => EngineOnly,
			ErrorKind::FormatLimit => EngineOnly,
			ErrorKind::Output => EngineOnly,
			ErrorKind::StoreCapacity => EngineOnly,
			ErrorKind::ClosedStore => EngineOnly,
			ErrorKind::InvalidHandle => EngineOnly,
			ErrorKind::InvalidCode => EngineOnly,
			ErrorKind::FrameCapacity => EngineOnly,
			ErrorKind::Cancelled => EngineOnly,
			ErrorKind::Unsupported => Unclassified("Unsupported"),
			ErrorKind::Host => match self.detail {
				Detail::HostExpectedType { .. } | Detail::HostExpectedNumber { .. } => {
					Class(C::TypeMismatch)
				}
				Detail::HostMessage {
					class: HostClass::Reviewed(class),
					..
				} => Class(class),
				Detail::HostMessage {
					class: HostClass::Unclassified(name),
					..
				} => Unclassified(name),
				_ => Unclassified("Host"),
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate as rune;
	use crate::runtime::{AccessError, Panic, Protocol, StaticString, TypeInfo};
	use crate::sync::Arc;

	#[derive(crate::Any)]
	#[rune(item = ::origin_controls)]
	struct OriginIndex;

	fn info() -> TypeInfo {
		TypeInfo::named::<i64>()
	}
	fn field() -> Arc<StaticString> {
		Arc::try_new(StaticString::new("field").unwrap()).unwrap()
	}

	fn live_error(context: &crate::Context, text: &str) -> VmError {
		let mut sources = crate::Sources::new();
		sources
			.insert(crate::Source::new("origin.rn", text).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(context)
			.build_vm()
			.unwrap();
		vm.call(["main"], ()).unwrap_err()
	}

	#[test]
	fn origin_uses_recorded_context_for_script_and_native_errors() {
		let mut context = crate::Context::with_default_modules().unwrap();
		let mut module = crate::Module::with_item(["origin_controls"]).unwrap();
		module.ty::<OriginIndex>().unwrap();
		module
			.function("indexer", || {
				crate::runtime::Value::new(OriginIndex).unwrap()
			})
			.build()
			.unwrap();
		module
			.associated_function(
				&Protocol::INDEX_GET,
				|_: &OriginIndex, _: i64| -> Result<i64, VmError> {
					Err(VmError::panic("index protocol host failure"))
				},
			)
			.unwrap();
		module
			.function("integer", |value: i64| value)
			.build()
			.unwrap();
		module
			.raw_function("fail", |_: &mut dyn crate::runtime::Memory, _, _, _| {
				Err(VmError::panic("host failure"))
			})
			.build()
			.unwrap();
		context.install(module).unwrap();
		let missing = live_error(&context, "pub fn main() { 1.missing_method() }");
		assert_eq!(missing.origin_instruction(), Some("CallAssociated"));
		assert_eq!(missing.origin(), ErrorOrigin::Script);
		let argument = live_error(&context, "pub fn main() { origin_controls::integer(true) }");
		assert_eq!(argument.origin_instruction(), Some("Call"));
		assert_eq!(argument.origin(), ErrorOrigin::Host);
		let returned = live_error(&context, "pub fn main() { origin_controls::fail() }");
		assert_eq!(returned.origin_instruction(), Some("Call"));
		assert_eq!(returned.origin(), ErrorOrigin::Host);
		let index = live_error(&context, "pub fn main() { let v = [1]; let i = 9; v[i] }");
		assert_eq!(index.origin_instruction(), Some("IndexGet"));
		assert_eq!(index.origin(), ErrorOrigin::Unknown);
		let protocol = live_error(
			&context,
			"pub fn main() { let v = origin_controls::indexer(); let i = 0; v[i] }",
		);
		assert_eq!(protocol.origin_instruction(), Some("IndexGet"));
		assert_eq!(protocol.origin(), ErrorOrigin::Unknown);
		assert!(std::format!("{protocol}").contains("index protocol host failure"));
		assert_eq!(VmError::panic("without VM").origin(), ErrorOrigin::Unknown);
	}

	#[test]
	fn every_reviewed_legacy_variant_projects_to_its_class() {
		use ErrorClass as C;
		use VmErrorKind as K;
		let rows = [
			(
				K::AllocError {
					error: crate::alloc::Error::CapacityOverflow,
				},
				C::Allocation,
			),
			(
				K::AccessError {
					error: AccessError::moved_read(),
				},
				C::AccessConflict,
			),
			(
				K::Panic {
					reason: Panic::custom(std::string::String::from("panic")),
				},
				C::Panic,
			),
			(
				K::Halted {
					halt: VmHaltInfo::Limited,
				},
				C::BudgetExhausted,
			),
			(K::Overflow, C::Overflow),
			(K::Underflow, C::Overflow),
			(K::DivideByZero, C::DivisionByZero),
			(
				K::MissingProtocolFunction {
					protocol: &Protocol::DISPLAY_FMT,
					instance: info(),
				},
				C::TypeMismatch,
			),
			(
				K::MissingInstanceFunction {
					hash: crate::Hash::new(1),
					instance: info(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedBinaryOperation {
					op: "+",
					lhs: info(),
					rhs: info(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedUnaryOperation {
					op: "-",
					operand: info(),
				},
				C::TypeMismatch,
			),
			(
				K::BadArgumentCount {
					actual: 1,
					expected: 2,
				},
				C::ArgumentCount,
			),
			(K::MaxValueDepth { max: 64 }, C::ValueDepth),
			(
				K::UnsupportedIndexSet {
					target: info(),
					index: info(),
					value: info(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedIndexGet {
					target: info(),
					index: info(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedTupleIndexGet {
					target: info(),
					index: 0,
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedTupleIndexSet { target: info() },
				C::TypeMismatch,
			),
			(
				K::UnsupportedObjectSlotIndexGet {
					target: info(),
					field: field(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedObjectSlotIndexSet {
					target: info(),
					field: field(),
				},
				C::TypeMismatch,
			),
			(K::UnsupportedAs { value: info() }, C::InvalidCast),
			(K::UnsupportedAsTarget { value: info() }, C::InvalidCast),
			(K::UnsupportedCallFn { actual: info() }, C::TypeMismatch),
			(K::MissingIndex { target: info() }, C::IndexOutOfBounds),
			(
				K::MissingIndexInteger {
					target: info(),
					index: 1i64.into(),
				},
				C::IndexOutOfBounds,
			),
			(
				K::OutOfRange {
					index: 1i64.into(),
					length: 2i64.into(),
				},
				C::IndexOutOfBounds,
			),
			(K::UnsupportedTryOperand { actual: info() }, C::TypeMismatch),
			(
				K::UnsupportedIterRangeInclusive {
					start: info(),
					end: info(),
				},
				C::TypeMismatch,
			),
			(
				K::UnsupportedIterRangeFrom { start: info() },
				C::TypeMismatch,
			),
			(
				K::UnsupportedIterRange {
					start: info(),
					end: info(),
				},
				C::TypeMismatch,
			),
			(
				K::ExpectedType {
					expected: info(),
					actual: info(),
				},
				C::TypeMismatch,
			),
			(K::ExpectedNumber { actual: info() }, C::TypeMismatch),
			(
				K::ValueToIntegerCoercionError {
					from: 1i64.into(),
					to: "u8",
				},
				C::IntegerCoercion,
			),
			(
				K::ExpectedTupleLength {
					actual: 1,
					expected: 2,
				},
				C::TypeMismatch,
			),
			(
				K::MissingField {
					target: info(),
					field: crate::alloc::String::try_from("field").unwrap(),
				},
				C::IndexOutOfBounds,
			),
			(
				K::IllegalFloatComparison {
					lhs: f64::NAN,
					rhs: 0.0,
				},
				C::IllegalFloatComparison,
			),
		];
		for (kind, class) in rows {
			let error = VmError::new(kind);
			assert!(
				error
					.error_class()
					.same_class(ErrorProjection::Class(class)),
				"{error:?}"
			);
		}
		for (halt, expected) in [
			(VmHaltInfo::Yielded, "Halted::Yielded"),
			(VmHaltInfo::Awaited, "Halted::Awaited"),
		] {
			assert!(
				matches!(VmError::new(K::Halted { halt }).error_class(), ErrorProjection::Unclassified(name) if name == expected)
			);
		}
		assert!(matches!(
			VmError::new(K::MissingCallFrame).error_class(),
			ErrorProjection::Unclassified("MissingCallFrame")
		));
	}

	#[test]
	fn every_fresh_kind_has_explicit_fail_closed_projection() {
		use ErrorClass as C;
		use ErrorKind as K;
		use ErrorProjection::{Class, EngineOnly, Unclassified};
		for (kind, expected) in [
			(K::Type, Class(C::TypeMismatch)),
			(K::IllegalFloatComparison, Class(C::IllegalFloatComparison)),
			(K::Overflow, Class(C::Overflow)),
			(K::DivisionByZero, Class(C::DivisionByZero)),
			(K::Panic, Class(C::Panic)),
			(K::IndexOutOfBounds, Class(C::IndexOutOfBounds)),
			(K::InvalidCast, Class(C::InvalidCast)),
			(K::IntegerCoercion, Class(C::IntegerCoercion)),
			(K::ValueDepth, Class(C::ValueDepth)),
			(K::Allocation, Class(C::Allocation)),
			(K::AccessConflict, Class(C::AccessConflict)),
			(K::FuelExhausted, Class(C::BudgetExhausted)),
			(K::ArgumentCount, Class(C::ArgumentCount)),
			(K::InvalidShift, EngineOnly),
			(K::FormatLimit, EngineOnly),
			(K::Output, EngineOnly),
			(K::StoreCapacity, EngineOnly),
			(K::ClosedStore, EngineOnly),
			(K::InvalidHandle, EngineOnly),
			(K::InvalidCode, EngineOnly),
			(K::FrameCapacity, EngineOnly),
			(K::Cancelled, EngineOnly),
			(K::Unsupported, Unclassified("Unsupported")),
			(K::Host, Unclassified("Host")),
		] {
			let actual = Error::new(kind, "test").error_class();
			assert!(
				match (actual, expected) {
					(Class(a), Class(b)) => a == b,
					(EngineOnly, EngineOnly) => true,
					(Unclassified(a), Unclassified(b)) => a == b,
					_ => false,
				},
				"{kind:?}"
			);
		}
		let mapped = Error::new(K::Host, "test").with_detail(Detail::HostMessage {
			cause: super::super::error::HostCause::Other,
			arg: None,
			class: HostClass::Reviewed(C::DivisionByZero),
		});
		assert!(mapped.error_class().same_class(Class(C::DivisionByZero)));
		let expected_type = Error::new(K::Host, "test").with_detail(Detail::HostExpectedType {
			expected: crate::Hash::new(1),
			actual: crate::Hash::new(2),
			arg: Some(0),
		});
		let expected_number = Error::new(K::Host, "test").with_detail(Detail::HostExpectedNumber {
			actual: crate::Hash::new(2),
			arg: Some(0),
		});
		assert!(expected_type
			.error_class()
			.same_class(Class(C::TypeMismatch)));
		assert!(expected_number
			.error_class()
			.same_class(Class(C::TypeMismatch)));
		let unclassified = Error::new(K::Host, "test").with_detail(Detail::HostMessage {
			cause: super::super::error::HostCause::Other,
			arg: None,
			class: HostClass::Unclassified("MissingCallFrame"),
		});
		assert!(matches!(
			unclassified.error_class(),
			Unclassified("MissingCallFrame")
		));
		assert!(Class(C::Overflow).same_class(Class(C::Overflow)));
		assert!(!Class(C::Overflow).same_class(Class(C::TypeMismatch)));
		assert!(!EngineOnly.same_class(EngineOnly));
		assert!(!Unclassified("x").same_class(Unclassified("x")));
	}
}
