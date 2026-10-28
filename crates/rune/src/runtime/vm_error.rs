use core::convert::Infallible;
use core::fmt;

use rust_alloc::boxed::Box;

use crate::alloc::error::CustomError;
use crate::alloc::{self, String};
use crate::runtime::unit::{BadInstruction, BadJump};
use crate::sync::Arc;
use crate::{vm_error, Any, Hash, Item, ItemBuf};

use super::{
	AccessError, AnyObjError, AnyObjErrorKind, AnySequenceTakeError, BoxedPanic, CallFrame,
	DynArgsUsed, ExecutionState, Panic, Protocol, SliceError, StackError, StaticString, StoreError,
	StoreErrorKind, TypeInfo, TypeOf, Unit, Vm, VmHaltInfo,
};

macro_rules! from_new {
    (
        $($ty:ty { $($from:ty),* $(,)? })*
    ) => {
        $($(
            impl From<$from> for $ty {
                #[inline]
                fn from(error: $from) -> Self {
                    Self::new(error)
                }
            }
        )*)*
    }
}

vm_error!(VmError);

/// A virtual machine error which includes tracing information.
pub struct VmError {
	inner: Box<VmErrorInner>,
}

/// Structured information about a missing instance function at the root failure.
///
/// Identity is independent of the rendered diagnostic text.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingInstanceFunction<'a> {
	/// The hash of the missing instance function.
	pub hash: Hash,
	/// The receiver's type hash, for identity checks.
	pub instance: Hash,
	receiver: &'a TypeInfo,
}

impl MissingInstanceFunction<'_> {
	/// Display the receiver's name for a diagnostic.
	///
	/// This is presentation only and may change. Use [`Self::instance`] for
	/// identity checks rather than parsing this name.
	pub fn instance_name(&self) -> impl fmt::Display + '_ {
		super::messages::DiagnosticType(self.receiver)
	}
}

impl VmError {
	/// Inspect typed argument context without conflating it with caller frames.
	#[cold]
	#[inline(never)]
	pub fn invocation_context(&self) -> impl Iterator<Item = super::InvocationContext> + '_ {
		self.inner
			.chain
			.iter()
			.filter_map(|entry| match entry.kind {
				VmErrorKind::BadArgument { arg } => {
					Some(super::InvocationContext::BadArgument { arg })
				}
				_ => None,
			})
	}

	/// Inspect the origin followed by active callers, retaining unresolved frames.
	/// Each frozen callback window is resolved through its own unit.
	#[cold]
	#[inline(never)]
	pub fn logical_frames(&self) -> impl Iterator<Item = super::LogicalFrame<'_>> {
		let mut origin = true;
		let frames = self.inner.stacktrace.iter().flat_map(|window| {
			let debug = window.unit.debug_info();
			core::iter::once(debug.and_then(|d| d.instruction_at(window.ip)))
				.chain(window.frames.iter().rev().map(move |frame| {
					// A saved pc is the return address. Debug instruction boundaries,
					// rather than a guessed instruction width, identify its call.
					debug.and_then(|d| {
						let (ip, inst) = d
							.instructions
							.iter()
							.filter(|(ip, _)| **ip < frame.ip)
							.max_by_key(|(ip, _)| **ip)?;
						let (_, width) = window.unit.instruction_at(*ip).ok()??;
						// A missing call's debug entry must not resolve to an older
						// instruction merely because it precedes the return address.
						(ip.checked_add(width) == Some(frame.ip)).then_some(inst)
					})
				}))
				.map(move |inst| {
					inst.map(|inst| super::LogicalLocation {
						source_id: inst.source_id,
						span: inst.span,
						unit: Some(window.unit.as_ref()),
					})
				})
		});
		// Even a contextless error has an unresolved origin.
		let empty = self.inner.stacktrace.is_empty();
		frames
			.chain(core::iter::once(None).take(usize::from(empty)))
			.map(move |location| {
				let role = if core::mem::replace(&mut origin, false) {
					super::LogicalFrameRole::Origin
				} else {
					super::LogicalFrameRole::Caller
				};
				super::LogicalFrame { role, location }
			})
	}

	/// Render the primary error and its logical invocation frames.
	#[cold]
	#[inline(never)]
	pub fn diagnostic<'a>(&'a self, sources: &'a crate::Sources) -> impl fmt::Display + 'a {
		struct Diagnostic<'a>(&'a VmError, &'a crate::Sources);
		impl fmt::Display for Diagnostic<'_> {
			#[cold]
			#[inline(never)]
			fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "{}", self.0)?;
				for frame in self.0.logical_frames() {
					super::write_logical_frame(f, frame, self.1)?;
				}
				Ok(())
			}
		}
		Diagnostic(self, sources)
	}

	/// Inspect a missing instance function at the root failure without parsing text.
	///
	/// Auxiliary errors are not searched. A panic quoting a missing-function
	/// diagnostic therefore does not acquire that diagnostic's identity.
	pub fn missing_instance_function(&self) -> Option<MissingInstanceFunction<'_>> {
		let VmErrorKind::MissingInstanceFunction { hash, instance } = &self.inner.error.kind else {
			return None;
		};
		Some(MissingInstanceFunction {
			hash: *hash,
			instance: instance.type_hash(),
			receiver: instance,
		})
	}

	#[inline]
	pub(crate) fn new<E>(error: E) -> Self
	where
		VmErrorKind: From<E>,
	{
		Self {
			inner: Box::new(VmErrorInner {
				error: VmErrorAt {
					#[cfg(feature = "emit")]
					index: 0,
					kind: VmErrorKind::from(error),
				},
				chain: rust_alloc::vec::Vec::new(),
				stacktrace: rust_alloc::vec::Vec::new(),
				numeric_context: None,
			}),
		}
	}

	/// Construct an error containing a panic.
	#[inline]
	pub fn panic<D>(message: D) -> Self
	where
		D: 'static + BoxedPanic,
	{
		Self::from(Panic::custom(message))
	}

	/// Construct an expectation error. The actual type received is `actual`,
	/// but we expected `E`.
	#[inline]
	pub fn expected<E>(actual: TypeInfo) -> Self
	where
		E: ?Sized + TypeOf,
	{
		Self::from(VmErrorKind::expected::<E>(actual))
	}

	/// Get the location where the error happened.
	#[inline]
	pub fn at(&self) -> &VmErrorAt {
		&self.inner.error
	}

	/// Get the top-level error.
	#[inline]
	pub fn error(&self) -> &VmErrorAt {
		&self.inner.error
	}

	/// Get the full backtrace of errors and their corresponding instructions.
	#[inline]
	pub fn chain(&self) -> &[VmErrorAt] {
		&self.inner.chain
	}

	/// Get the full backtrace of errors and their corresponding instructions.
	#[inline]
	pub fn stacktrace(&self) -> &[VmErrorLocation] {
		&self.inner.stacktrace
	}

	/// Construct an overflow error.
	#[inline]
	pub fn overflow() -> Self {
		Self::from(VmErrorKind::Overflow)
	}

	/// Construct context exclusively after a checked primitive has failed.
	#[cold]
	#[inline(never)]
	pub(crate) fn numeric_failure(
		kind: VmErrorKind,
		op: &'static str,
		operand: &'static str,
	) -> Self {
		let operand = match &kind {
			VmErrorKind::ValueToIntegerCoercionError { from, .. } => match from.kind {
				VmIntegerReprKind::Signed(_) => "i64",
				VmIntegerReprKind::Unsigned(_) => "u64",
				_ => operand,
			},
			_ => operand,
		};
		let mut error = Self::new(kind);
		error.inner.numeric_context = Some((op, operand));
		error
	}

	/// Get the first error location.
	#[inline]
	pub fn first_location(&self) -> Option<&VmErrorLocation> {
		self.inner.stacktrace.first()
	}

	#[inline]
	pub(crate) fn into_kind(self) -> VmErrorKind {
		self.inner.error.kind
	}

	/// Name the source operator at its instruction boundary, without relabeling
	/// an already-located failure from a nested protocol call or auxiliary error.
	pub(crate) fn with_operator(mut self, protocol: &'static str, symbol: &'static str) -> Self {
		if self.inner.stacktrace.is_empty() {
			if let VmErrorKind::UnsupportedBinaryOperation { op, .. } = &mut self.inner.error.kind {
				if *op == protocol {
					*op = symbol;
				}
			}
		}
		self
	}

	/// Apply the given frame to the current result.
	pub(crate) fn with_vm<T>(result: Result<T, Self>, vm: &Vm) -> Result<T, Self> {
		match result {
			Ok(ok) => Ok(ok),
			Err(mut err) => {
				#[cfg(feature = "execution")]
				let origin = err.capture_origin(vm);
				err.inner.stacktrace.push(VmErrorLocation {
					unit: vm.unit().clone(),
					ip: vm.last_ip(),
					frames: vm.call_frames().to_vec(),
					#[cfg(feature = "execution")]
					origin: Some(origin),
				});

				Err(err)
			}
		}
	}

	/// Add auxiliary errors if appropriate.
	#[inline]
	pub(crate) fn with_error<E>(mut self, error: E) -> Self
	where
		VmErrorKind: From<E>,
	{
		#[cfg(feature = "emit")]
		let index = self.inner.stacktrace.len();

		self.inner.chain.push(VmErrorAt {
			#[cfg(feature = "emit")]
			index,
			kind: VmErrorKind::from(error),
		});

		self
	}
}

impl fmt::Display for VmError {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if let Some((op, operand)) = self.inner.numeric_context {
			let reason = match &self.inner.error.kind {
				VmErrorKind::Overflow => Some("Numerical overflow"),
				VmErrorKind::Underflow => Some("Numerical underflow"),
				VmErrorKind::DivideByZero => Some("Division by zero"),
				VmErrorKind::ValueToIntegerCoercionError { from, to }
					if matches!(op, "<<" | ">>" | "<<=" | ">>=") =>
				{
					let source = match from.kind {
						VmIntegerReprKind::Signed(_) => Some("i64"),
						VmIntegerReprKind::Unsigned(_) => Some("u64"),
						_ => None,
					};
					if let Some(source) = source {
						return super::messages::shift_coerce(f, source, from, to, op);
					}
					None
				}
				_ => None,
			};
			if let Some(reason) = reason {
				return super::messages::numeric_fault(f, reason, op, operand);
			}
		}

		if let VmErrorKind::AccessError { error } = &self.inner.error.kind {
			let arg = if error.is_moved() {
				None
			} else {
				self.inner.chain.iter().find_map(|entry| match &entry.kind {
					VmErrorKind::BadArgument { arg } => Some(*arg),
					_ => None,
				})
			};
			return error.fmt_subject(f, arg);
		}
		if let VmErrorKind::MissingInstanceFunction { hash, instance } = &self.inner.error.kind {
			let name_hash = Hash::associated_function(instance.type_hash(), *hash);
			let target = super::messages::DiagnosticType(instance);
			if name_hash == Protocol::INTO_ITER.hash {
				return write!(f, "Cannot iterate over `{target}`");
			}
			let name = self
				.first_location()
				.and_then(|location| location.unit.debug_info())
				.and_then(|debug| debug.ident_for_hash(name_hash));
			return match name {
				Some(name) => write!(f, "Type `{target}` has no method named `{name}`"),
				None => write!(f, "Type `{target}` has no matching method"),
			};
		}
		self.inner.error.fmt(f)
	}
}

impl fmt::Debug for VmError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("VmError")
			.field("error", &self.inner.error)
			.field("chain", &self.inner.chain)
			.field("stacktrace", &self.inner.stacktrace)
			.finish()
	}
}

impl core::error::Error for VmError {}

/// A single unit producing errors.
#[non_exhaustive]
pub struct VmErrorLocation {
	/// Associated unit.
	pub unit: Arc<Unit>,
	/// Frozen instruction pointer.
	pub ip: usize,
	/// All lower call frames before the unwind trigger point
	pub frames: rust_alloc::vec::Vec<CallFrame>,
	#[cfg(feature = "execution")]
	pub(crate) origin: Option<(
		crate::experimental_core::error::ErrorOrigin,
		Option<&'static str>,
	)>,
}

impl fmt::Debug for VmErrorLocation {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("VmErrorLocation")
			.field("unit", &self.unit)
			.field("ip", &self.ip)
			.field("frames", &self.frames)
			.finish()
	}
}

#[derive(Debug)]
#[non_exhaustive]
pub struct VmErrorAt {
	/// Index into the backtrace which contains information of what caused this error.
	#[cfg(feature = "emit")]
	index: usize,
	/// The kind of error.
	kind: VmErrorKind,
}

impl VmErrorAt {
	/// Get the instruction which caused the error.
	#[cfg(feature = "emit")]
	pub(crate) fn index(&self) -> usize {
		self.index
	}

	#[cfg(any(feature = "emit", feature = "execution"))]
	pub(crate) fn kind(&self) -> &VmErrorKind {
		&self.kind
	}
}

impl fmt::Display for VmErrorAt {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.kind.fmt(f)
	}
}

#[non_exhaustive]
pub(crate) struct VmErrorInner {
	pub(crate) error: VmErrorAt,
	pub(crate) chain: rust_alloc::vec::Vec<VmErrorAt>,
	pub(crate) stacktrace: rust_alloc::vec::Vec<VmErrorLocation>,
	numeric_context: Option<(&'static str, &'static str)>,
}

/// A result produced by the virtual machine.
#[deprecated = "Use `Result<T, VmError>` directly instead."]
pub type VmResult<T> = Result<T, VmError>;

impl<E> From<E> for VmError
where
	VmErrorKind: From<E>,
{
	#[inline]
	fn from(error: E) -> Self {
		Self::new(error)
	}
}

impl<E> From<CustomError<E>> for VmError
where
	VmError: From<E>,
{
	#[inline]
	fn from(error: CustomError<E>) -> Self {
		match error {
			CustomError::Custom(error) => Self::from(error),
			CustomError::Error(error) => VmError::new(error),
		}
	}
}

impl<const N: usize> From<[VmErrorKind; N]> for VmError {
	fn from(kinds: [VmErrorKind; N]) -> Self {
		let mut it = kinds.into_iter();

		let Some(first) = it.next() else {
			return VmError::panic("Cannot construct an empty collection of errors");
		};

		let mut chain = rust_alloc::vec::Vec::with_capacity(it.len());

		for kind in it {
			chain.push(VmErrorAt {
				#[cfg(feature = "emit")]
				index: 0,
				kind,
			});
		}

		Self {
			inner: Box::new(VmErrorInner {
				error: VmErrorAt {
					#[cfg(feature = "emit")]
					index: 0,
					kind: first,
				},
				chain,
				stacktrace: rust_alloc::vec::Vec::new(),
				numeric_context: None,
			}),
		}
	}
}

impl From<Panic> for VmErrorKind {
	#[inline]
	fn from(reason: Panic) -> Self {
		VmErrorKind::Panic { reason }
	}
}

impl From<ExpectedType> for VmErrorKind {
	#[inline]
	fn from(expected: ExpectedType) -> Self {
		VmErrorKind::ExpectedType {
			expected: expected.expected,
			actual: expected.actual,
		}
	}
}

/// A expected error.
pub struct ExpectedType {
	pub(crate) expected: TypeInfo,
	pub(crate) actual: TypeInfo,
}

impl ExpectedType {
	/// Construct an expected error.
	pub(crate) fn new<T>(actual: TypeInfo) -> Self
	where
		T: ?Sized + TypeOf,
	{
		Self {
			expected: T::type_info(),
			actual,
		}
	}
}

vm_error!(RuntimeError);

/// An opaque simple runtime error.
#[cfg_attr(test, derive(PartialEq))]
pub struct RuntimeError {
	error: Box<VmErrorKind>,
}

impl RuntimeError {
	#[inline]
	pub(crate) fn new<E>(error: E) -> Self
	where
		VmErrorKind: From<E>,
	{
		Self {
			error: Box::new(VmErrorKind::from(error)),
		}
	}

	#[inline]
	pub(crate) fn into_vm_error_kind(self) -> VmErrorKind {
		*self.error
	}

	/// Construct an error containing a panic.
	#[inline]
	pub fn panic<D>(message: D) -> Self
	where
		D: 'static + BoxedPanic,
	{
		Self::new(Panic::custom(message))
	}

	/// Bad argument count.
	pub fn bad_argument_count(actual: usize, expected: usize) -> Self {
		Self::new(VmErrorKind::BadArgumentCount { actual, expected })
	}

	/// Construct an expected error.
	pub fn expected<T>(actual: TypeInfo) -> Self
	where
		T: ?Sized + TypeOf,
	{
		Self::new(VmErrorKind::ExpectedType {
			expected: T::type_info(),
			actual,
		})
	}

	/// Construct an expected error from any.
	pub(crate) fn expected_any<T>(actual: TypeInfo) -> Self
	where
		T: Any,
	{
		Self::new(VmErrorKind::ExpectedType {
			expected: TypeInfo::any::<T>(),
			actual,
		})
	}

	/// Construct an expected any error.
	pub(crate) fn expected_any_obj(actual: TypeInfo) -> Self {
		Self::new(VmErrorKind::ExpectedAny { actual })
	}

	/// Indicate that a constant constructor is missing.
	pub(crate) fn missing_constant_constructor(hash: Hash) -> Self {
		Self::new(VmErrorKind::MissingConstantConstructor { hash })
	}

	pub(crate) fn expected_empty(actual: TypeInfo) -> Self {
		Self::new(VmErrorKind::ExpectedEmpty { actual })
	}

	pub(crate) fn expected_tuple(actual: TypeInfo) -> Self {
		Self::new(VmErrorKind::ExpectedTuple { actual })
	}

	pub(crate) fn expected_struct(actual: TypeInfo) -> Self {
		Self::new(VmErrorKind::ExpectedStruct { actual })
	}
}

impl core::error::Error for RuntimeError {}

impl fmt::Display for RuntimeError {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.error.fmt(f)
	}
}

impl fmt::Debug for RuntimeError {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.error.fmt(f)
	}
}

impl From<VmError> for RuntimeError {
	#[inline]
	fn from(error: VmError) -> Self {
		Self::new(error.into_kind())
	}
}

impl From<Infallible> for RuntimeError {
	#[inline]
	fn from(error: Infallible) -> Self {
		match error {}
	}
}

from_new! {
	RuntimeError {
		alloc::Error,
		alloc::alloc::AllocError,
		AccessError,
		AnySequenceTakeError,
		AnyObjError,
		StackError,
		VmErrorKind,
		ExpectedType,
	}
}

/// The kind of error encountered.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
#[doc(hidden)]
pub(crate) enum VmErrorKind {
	AllocError {
		error: alloc::Error,
	},
	AccessError {
		error: AccessError,
	},
	StackError {
		error: StackError,
	},
	SliceError {
		error: SliceError,
	},
	BadInstruction {
		error: BadInstruction,
	},
	BadJump {
		error: BadJump,
	},
	DynArgsUsed {
		error: DynArgsUsed,
	},
	Panic {
		reason: Panic,
	},
	NoRunningVm,
	Halted {
		halt: VmHaltInfo,
	},
	Overflow,
	Underflow,
	DivideByZero,
	MissingEntry {
		item: ItemBuf,
		hash: Hash,
	},
	MissingEntryHash {
		hash: Hash,
	},
	MissingFunction {
		hash: Hash,
	},
	MissingContextFunction {
		hash: Hash,
	},
	NotOwned {
		type_info: TypeInfo,
	},
	MissingProtocolFunction {
		protocol: &'static Protocol,
		instance: TypeInfo,
	},
	MissingInstanceFunction {
		hash: Hash,
		instance: TypeInfo,
	},
	IpOutOfBounds {
		ip: usize,
		length: usize,
	},
	UnsupportedBinaryOperation {
		op: &'static str,
		lhs: TypeInfo,
		rhs: TypeInfo,
	},
	UnsupportedUnaryOperation {
		op: &'static str,
		operand: TypeInfo,
	},
	MissingStaticString {
		slot: usize,
	},
	MissingStaticBytes {
		slot: usize,
	},
	MissingStaticObjectKeys {
		slot: usize,
	},
	MissingDropSet {
		set: usize,
	},
	MissingGlobals {
		slot: usize,
		name: Option<ItemBuf>,
	},
	BadGlobalSlot {
		slot: usize,
		name: Option<ItemBuf>,
	},
	UninitializedGlobal {
		slot: usize,
		name: Option<ItemBuf>,
	},
	MissingRtti {
		hash: Hash,
	},
	BadArgumentCount {
		actual: usize,
		expected: usize,
	},
	/// A walk over a value descended more deeply than it is allowed to.
	///
	/// Comparing, ordering, hashing and formatting a value all recurse into the
	/// values it is made of, so a graph which nests too deeply is reported
	/// rather than overflowing the stack.
	MaxValueDepth {
		max: usize,
	},
	/// A value was nested more deeply than a constant is allowed to be.
	///
	/// A `ConstValue` is built, walked, cloned and dropped by recursing over
	/// it, and the value it is built from is produced by evaluation, so how
	/// deep it is has nothing to do with how deep the source was.
	MaxConstDepth {
		max: usize,
	},
	/// A constant is made of more values than it is allowed to be.
	MaxConstSize {
		max: usize,
	},
	/// The array a constant is stored as does not describe a tree.
	///
	/// A constant is one array of nodes in which every subtree is a contiguous
	/// run, so an array in which a node claims children it does not have is not
	/// a constant. Building one here cannot produce such an array; reading one
	/// back from somewhere else can.
	MalformedConstValue,
	/// Executions were nested more deeply than they are allowed to be.
	///
	/// A call which a native function performs is driven by a machine of its
	/// own, which costs a native frame, so a script which nests them too deeply
	/// is reported rather than overflowing the stack.
	MaxExecutionDepth {
		max: usize,
	},
	BadEnvironmentCount {
		actual: usize,
		expected: usize,
	},
	BadArgument {
		arg: usize,
	},
	UnsupportedIndexSet {
		target: TypeInfo,
		index: TypeInfo,
		value: TypeInfo,
	},
	UnsupportedIndexGet {
		target: TypeInfo,
		index: TypeInfo,
	},
	UnsupportedTupleIndexGet {
		target: TypeInfo,
		index: usize,
	},
	UnsupportedTupleIndexSet {
		target: TypeInfo,
	},
	UnsupportedObjectSlotIndexGet {
		target: TypeInfo,
		field: Arc<StaticString>,
	},
	UnsupportedObjectSlotIndexSet {
		target: TypeInfo,
		field: Arc<StaticString>,
	},
	UnsupportedIs {
		value: TypeInfo,
		test_type: TypeInfo,
	},
	UnsupportedAs {
		value: TypeInfo,
	},
	UnsupportedAsTarget {
		value: TypeInfo,
	},
	UnsupportedCallFn {
		actual: TypeInfo,
	},
	MissingIndex {
		target: TypeInfo,
	},
	MissingIndexInteger {
		target: TypeInfo,
		index: VmIntegerRepr,
	},
	MissingIndexKey {
		target: TypeInfo,
	},
	OutOfRange {
		index: VmIntegerRepr,
		length: VmIntegerRepr,
	},
	UnsupportedTryOperand {
		actual: TypeInfo,
	},
	UnsupportedIterRangeInclusive {
		start: TypeInfo,
		end: TypeInfo,
	},
	UnsupportedIterRangeFrom {
		start: TypeInfo,
	},
	UnsupportedIterRange {
		start: TypeInfo,
		end: TypeInfo,
	},
	UnsupportedIterNextOperand {
		actual: TypeInfo,
	},
	ExpectedType {
		expected: TypeInfo,
		actual: TypeInfo,
	},
	ExpectedAny {
		actual: TypeInfo,
	},
	ExpectedNumber {
		actual: TypeInfo,
	},
	ExpectedEmpty {
		actual: TypeInfo,
	},
	ExpectedTuple {
		actual: TypeInfo,
	},
	ExpectedStruct {
		actual: TypeInfo,
	},
	MissingConstantConstructor {
		hash: Hash,
	},
	ValueToIntegerCoercionError {
		from: VmIntegerRepr,
		to: &'static str,
	},
	IntegerToValueCoercionError {
		from: VmIntegerRepr,
		to: &'static str,
	},
	ExpectedTupleLength {
		actual: usize,
		expected: usize,
	},
	ExpectedVecLength {
		actual: usize,
		expected: usize,
	},
	ConstNotSupported {
		actual: TypeInfo,
	},
	MissingInterfaceEnvironment,
	ExpectedExitedExecutionState {
		actual: ExecutionState,
	},
	GeneratorComplete,
	FutureCompleted,
	// Used in rune-macros.
	MissingVariant {
		name: String,
	},
	MissingField {
		target: TypeInfo,
		field: String,
	},
	MissingVariantName,
	MissingStructField {
		target: &'static str,
		name: &'static str,
	},
	MissingTupleIndex {
		target: &'static str,
		index: usize,
	},
	ExpectedVariant {
		actual: TypeInfo,
	},
	UnsupportedObjectFieldGet {
		target: TypeInfo,
	},
	IllegalFloatComparison {
		lhs: f64,
		rhs: f64,
	},
	IllegalFloatOperation {
		value: f64,
	},
	MissingCallFrame,
	IllegalFormat,
}

impl fmt::Display for VmErrorKind {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			VmErrorKind::AllocError { error } => error.fmt(f),
			VmErrorKind::AccessError { error } => error.fmt(f),
			VmErrorKind::StackError { error } => error.fmt(f),
			VmErrorKind::SliceError { error } => error.fmt(f),
			VmErrorKind::BadInstruction { error } => error.fmt(f),
			VmErrorKind::BadJump { error } => error.fmt(f),
			VmErrorKind::DynArgsUsed { error } => error.fmt(f),
			VmErrorKind::Panic { reason } => write!(f, "Panicked: {reason}"),
			VmErrorKind::NoRunningVm => write!(f, "No running virtual machines"),
			// A halt says what the machine did, and each of the three is a
			// different thing to have happened: one is the host's own limit
			// doing what it was set to do, and the other two are a function
			// being called as if it were something it is not.
			VmErrorKind::Halted { halt } => match halt {
				VmHaltInfo::Limited => write!(f, "Exhausted the budget it was given"),
				VmHaltInfo::Yielded => write!(
					f,
					"Yielded, but was called as if it could not - a generator has to be \
                     driven through an execution"
				),
				VmHaltInfo::Awaited => write!(
					f,
					"Awaited, but was called as if it could not - an asynchronous function \
                     has to be called asynchronously"
				),
			},
			VmErrorKind::Overflow => write!(f, "Numerical overflow"),
			VmErrorKind::Underflow => write!(f, "Numerical underflow"),
			VmErrorKind::DivideByZero => write!(f, "Division by zero"),
			VmErrorKind::MissingEntry { item, hash } => {
				write!(f, "Missing entry `{item}` with hash `{hash}`")
			}
			VmErrorKind::MissingEntryHash { hash } => {
				write!(f, "Missing entry with hash `{hash}`")
			}
			VmErrorKind::MissingFunction { hash } => {
				write!(f, "Missing function with hash `{hash}`")
			}
			VmErrorKind::MissingContextFunction { hash } => {
				write!(f, "Missing context function with hash `{hash}`")
			}
			VmErrorKind::NotOwned { type_info } => {
				let type_info = super::messages::DiagnosticType(type_info);
				write!(f, "Cannot use owned operations for {type_info}")
			}
			VmErrorKind::MissingProtocolFunction { protocol, instance } => {
				let instance = super::messages::DiagnosticType(instance);
				if protocol.hash == Protocol::DISPLAY_FMT.hash {
					write!(f, "`{instance}` cannot be formatted with `{{}}`")
				} else {
					write!(f, "Missing protocol function `{protocol}` for `{instance}`")
				}
			}
			VmErrorKind::MissingInstanceFunction { hash, instance } => {
				let target = super::messages::DiagnosticType(instance);
				if Hash::associated_function(instance.type_hash(), *hash)
					== Protocol::INTO_ITER.hash
				{
					write!(f, "Cannot iterate over `{target}`")
				} else {
					write!(f, "Type `{target}` has no matching method")
				}
			}
			VmErrorKind::IpOutOfBounds { ip, length } => write!(
				f,
				"Instruction pointer `{ip}` is out-of-bounds `0-{length}`",
			),
			VmErrorKind::UnsupportedBinaryOperation { op, lhs, rhs } => {
				let lhs = super::messages::Operand(lhs);
				let rhs = super::messages::Operand(rhs);
				super::messages::binary_unsupported(f, op, lhs, rhs)
			}
			VmErrorKind::UnsupportedUnaryOperation { op, operand } => {
				let operand = super::messages::Operand(operand);
				super::messages::unary_unsupported(f, op, operand)
			}
			VmErrorKind::MissingStaticString { slot } => {
				write!(f, "Static string slot {slot} does not exist")
			}
			VmErrorKind::MissingStaticBytes { slot } => {
				write!(f, "Static bytes slot {slot} does not exist")
			}
			VmErrorKind::MissingStaticObjectKeys { slot } => {
				write!(f, "Static object keys slot {slot} does not exist")
			}
			VmErrorKind::MissingDropSet { set } => {
				write!(f, "Static drop set {set} does not exist")
			}
			VmErrorKind::MissingGlobals { slot, name } => {
				write!(f, "No storage has been configured for the static ")?;
				fmt_global(f, *slot, name.as_deref())
			}
			VmErrorKind::BadGlobalSlot { slot, name } => {
				write!(f, "The configured storage has no slot for the static ")?;
				fmt_global(f, *slot, name.as_deref())
			}
			VmErrorKind::UninitializedGlobal { slot, name } => {
				write!(f, "Reading uninitialized static ")?;
				fmt_global(f, *slot, name.as_deref())
			}
			VmErrorKind::MissingRtti { hash } => {
				write!(f, "Missing runtime information for type with hash `{hash}`")
			}
			VmErrorKind::BadArgumentCount { actual, expected } => {
				super::messages::arity(f, actual, expected)
			}
			VmErrorKind::MaxValueDepth { max } => {
				write!(f, "Value is nested too deeply to walk, limit is {max}")
			}
			VmErrorKind::MaxConstSize { max } => {
				write!(f, "Value is too large to be a constant, limit is {max}")
			}
			VmErrorKind::MalformedConstValue => {
				write!(f, "Constant value is not a tree")
			}
			VmErrorKind::MaxConstDepth { max } => {
				write!(
					f,
					"Value is nested too deeply to be a constant, limit is {max}"
				)
			}
			VmErrorKind::MaxExecutionDepth { max } => {
				write!(f, "Executions are nested too deeply, limit is {max}")
			}
			VmErrorKind::BadEnvironmentCount { actual, expected } => write!(
				f,
				"Wrong environment size `{actual}`, expected `{expected}`",
			),
			VmErrorKind::BadArgument { arg } => write!(f, "Bad argument #{arg}"),
			VmErrorKind::UnsupportedIndexSet {
				target,
				index,
				value,
			} => {
				let target = super::messages::DiagnosticType(target);
				let index = super::messages::DiagnosticType(index);
				let value = super::messages::DiagnosticType(value);
				super::messages::index_set(f, target, index, value)
			}
			VmErrorKind::UnsupportedIndexGet { target, index } => {
				let target = super::messages::DiagnosticType(target);
				let index = super::messages::DiagnosticType(index);
				super::messages::index_get(f, target, index)
			}
			VmErrorKind::UnsupportedTupleIndexGet { target, index } => {
				let target = super::messages::DiagnosticType(target);
				super::messages::tuple_get(f, target, index)
			}
			VmErrorKind::UnsupportedTupleIndexSet { target } => {
				let target = super::messages::DiagnosticType(target);
				super::messages::tuple_set(f, target)
			}
			VmErrorKind::UnsupportedObjectSlotIndexGet { target, field } => {
				let target = super::messages::DiagnosticType(target);
				super::messages::field_get(f, target, field)
			}
			VmErrorKind::UnsupportedObjectSlotIndexSet { target, field } => {
				let target = super::messages::DiagnosticType(target);
				write!(f, "Field `{field}` not available to set on `{target}`")
			}
			VmErrorKind::UnsupportedIs { value, test_type } => {
				let value = super::messages::DiagnosticType(value);
				let test_type = super::messages::DiagnosticType(test_type);
				write!(f, "Operation `{value} is {test_type}` is not supported")
			}
			VmErrorKind::UnsupportedAs { value } => {
				let value = super::messages::DiagnosticType(value);
				super::messages::cast_source(f, value)
			}
			// The type being converted to is not something the machine can name
			// - all it has is a hash, which tells whoever wrote it nothing the
			// source in front of them does not already say. What `as` can
			// convert to is the half which is worth saying.
			VmErrorKind::UnsupportedAsTarget { value } => {
				let value = super::messages::DiagnosticType(value);
				super::messages::cast_target(f, value)
			}
			VmErrorKind::UnsupportedCallFn { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				super::messages::not_callable(f, actual)
			}
			VmErrorKind::MissingIndex { target } => {
				let target = super::messages::DiagnosticType(target);
				write!(f, "Type `{target}` missing index")
			}
			VmErrorKind::MissingIndexInteger { target, index } => {
				let target = super::messages::DiagnosticType(target);
				write!(f, "Type `{target}` missing integer index `{index}`")
			}
			VmErrorKind::MissingIndexKey { target } => {
				let target = super::messages::DiagnosticType(target);
				write!(f, "Type `{target}` missing index")
			}
			VmErrorKind::OutOfRange { index, length } => {
				super::messages::out_of_range(f, index, length)
			}
			VmErrorKind::UnsupportedTryOperand { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				super::messages::try_operand(f, actual)
			}
			VmErrorKind::UnsupportedIterRangeInclusive { start, end } => {
				let start = super::messages::DiagnosticType(start);
				let end = super::messages::DiagnosticType(end);
				write!(f, "Cannot build an iterator out of {start}..={end}")
			}
			VmErrorKind::UnsupportedIterRangeFrom { start } => {
				let start = super::messages::DiagnosticType(start);
				write!(f, "Cannot build an iterator out of {start}..")
			}
			VmErrorKind::UnsupportedIterRange { start, end } => {
				let start = super::messages::DiagnosticType(start);
				let end = super::messages::DiagnosticType(end);
				write!(f, "Cannot build an iterator out of {start}..{end}")
			}
			VmErrorKind::UnsupportedIterNextOperand { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Type `{actual}` is not supported as iter-next operand")
			}
			VmErrorKind::ExpectedType { expected, actual } => {
				let expected = super::messages::DiagnosticType(expected);
				let actual = super::messages::DiagnosticType(actual);
				super::messages::expected_type(f, expected, actual)
			}
			VmErrorKind::ExpectedAny { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Expected `Any` type, but found `{actual}`")
			}
			VmErrorKind::ExpectedNumber { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				super::messages::expected_number(f, actual)
			}
			VmErrorKind::ExpectedEmpty { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Expected empty, but found `{actual}`")
			}
			VmErrorKind::ExpectedTuple { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Expected tuple, but found `{actual}`")
			}
			VmErrorKind::ExpectedStruct { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Expected struct, but found `{actual}`")
			}
			VmErrorKind::MissingConstantConstructor { hash } => {
				write!(f, "Missing constant constructor for type with hash {hash}")
			}
			VmErrorKind::ValueToIntegerCoercionError { from, to } => {
				super::messages::coerce(f, from, to)
			}
			VmErrorKind::IntegerToValueCoercionError { from, to } => {
				write!(f, "Failed to convert integer `{from}` to value `{to}`")
			}
			VmErrorKind::ExpectedTupleLength { actual, expected } => write!(
				f,
				"Expected a tuple of length `{expected}`, but found one with length `{actual}`",
			),
			VmErrorKind::ExpectedVecLength { actual, expected } => write!(
				f,
				"Expected a vector of length `{expected}`, but found one with length `{actual}`",
			),
			VmErrorKind::ConstNotSupported { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Type `{actual}` can't be converted to a constant value")
			}
			VmErrorKind::MissingInterfaceEnvironment => {
				write!(f, "Missing interface environment")
			}
			VmErrorKind::ExpectedExitedExecutionState { actual } => {
				write!(f, "Expected exited execution state, but was {actual}")
			}
			VmErrorKind::GeneratorComplete => {
				write!(f, "Cannot resume a generator that has completed")
			}
			VmErrorKind::FutureCompleted => write!(f, "Future already completed"),
			VmErrorKind::MissingVariant { name } => write!(f, "No variant matching `{name}`"),
			VmErrorKind::MissingField { target, field } => {
				let target = super::messages::DiagnosticType(target);
				super::messages::missing_field(f, target, field)
			}
			VmErrorKind::MissingVariantName => {
				write!(f, "missing variant name in runtime information")
			}
			VmErrorKind::MissingStructField { target, name } => write!(
				f,
				"missing dynamic field for struct field `{target}::{name}`",
			),
			VmErrorKind::MissingTupleIndex { target, index } => write!(
				f,
				"missing dynamic index #{index} in tuple struct `{target}`",
			),
			VmErrorKind::ExpectedVariant { actual } => {
				let actual = super::messages::DiagnosticType(actual);
				write!(f, "Expected an enum variant, but got `{actual}`")
			}
			VmErrorKind::UnsupportedObjectFieldGet { target } => {
				let target = super::messages::DiagnosticType(target);
				write!(
					f,
					"The object field get operation is not supported on `{target}`",
				)
			}
			VmErrorKind::IllegalFloatComparison { lhs, rhs } => {
				write!(
					f,
					"Cannot perform a comparison of the floats {lhs} and {rhs}",
				)
			}
			VmErrorKind::IllegalFloatOperation { value } => {
				write!(f, "Cannot perform operation on float `{value}`")
			}
			VmErrorKind::MissingCallFrame => {
				write!(f, "Missing call frame for internal vm call")
			}
			VmErrorKind::IllegalFormat => {
				write!(f, "Value cannot be formatted")
			}
		}
	}
}

impl From<Infallible> for VmErrorKind {
	#[inline]
	fn from(error: Infallible) -> Self {
		match error {}
	}
}

impl<E> From<StoreError<E>> for VmErrorKind
where
	VmErrorKind: From<E>,
{
	#[inline]
	fn from(value: StoreError<E>) -> Self {
		match value.into_kind() {
			StoreErrorKind::Stack(error) => VmErrorKind::StackError { error },
			StoreErrorKind::Error(error) => VmErrorKind::from(error),
			StoreErrorKind::Alloc(error) => VmErrorKind::AllocError { error },
		}
	}
}

impl From<RuntimeError> for VmErrorKind {
	#[inline]
	fn from(value: RuntimeError) -> Self {
		value.into_vm_error_kind()
	}
}

impl From<AnySequenceTakeError> for VmErrorKind {
	#[inline]
	fn from(value: AnySequenceTakeError) -> Self {
		match value {
			AnySequenceTakeError::Access(error) => Self::from(error),
			AnySequenceTakeError::Alloc(error) => Self::from(error),
		}
	}
}

impl From<AnyObjError> for VmErrorKind {
	#[inline]
	fn from(value: AnyObjError) -> Self {
		match value.into_kind() {
			AnyObjErrorKind::Alloc(error) => Self::from(error),
			AnyObjErrorKind::Cast(expected, actual) => VmErrorKind::ExpectedType {
				expected: TypeInfo::any_type_info(expected),
				actual,
			},
			AnyObjErrorKind::AccessError(error) => Self::from(error),
			AnyObjErrorKind::NotOwned(type_info) => VmErrorKind::NotOwned { type_info },
		}
	}
}

impl From<AccessError> for VmErrorKind {
	#[inline]
	fn from(error: AccessError) -> Self {
		VmErrorKind::AccessError { error }
	}
}

impl From<StackError> for VmErrorKind {
	#[inline]
	fn from(error: StackError) -> Self {
		VmErrorKind::StackError { error }
	}
}

impl From<SliceError> for VmErrorKind {
	#[inline]
	fn from(error: SliceError) -> Self {
		VmErrorKind::SliceError { error }
	}
}

impl From<BadInstruction> for VmErrorKind {
	#[inline]
	fn from(error: BadInstruction) -> Self {
		VmErrorKind::BadInstruction { error }
	}
}

impl From<BadJump> for VmErrorKind {
	#[inline]
	fn from(error: BadJump) -> Self {
		VmErrorKind::BadJump { error }
	}
}

impl From<DynArgsUsed> for VmErrorKind {
	#[inline]
	fn from(error: DynArgsUsed) -> Self {
		VmErrorKind::DynArgsUsed { error }
	}
}

impl From<alloc::Error> for VmErrorKind {
	#[inline]
	fn from(error: alloc::Error) -> Self {
		VmErrorKind::AllocError { error }
	}
}

impl From<alloc::alloc::AllocError> for VmErrorKind {
	#[inline]
	fn from(error: alloc::alloc::AllocError) -> Self {
		VmErrorKind::AllocError {
			error: error.into(),
		}
	}
}

impl VmErrorKind {
	/// Bad argument.
	pub(crate) fn bad_argument(arg: usize) -> Self {
		Self::BadArgument { arg }
	}

	/// Construct an expected error.
	pub(crate) fn expected<T>(actual: TypeInfo) -> Self
	where
		T: ?Sized + TypeOf,
	{
		Self::ExpectedType {
			expected: T::type_info(),
			actual,
		}
	}
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
enum VmIntegerReprKind {
	Signed(i128),
	Unsigned(u128),
	Isize(isize),
	Usize(usize),
}

/// A type-erased integer representation.
#[derive(Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct VmIntegerRepr {
	kind: VmIntegerReprKind,
}

impl VmIntegerRepr {
	#[inline]
	fn new(kind: VmIntegerReprKind) -> Self {
		Self { kind }
	}
}

macro_rules! impl_from {
    ($($variant:ident => [$($ty:ty),* $(,)?]),* $(,)?) => {
        $($(
            impl From<$ty> for VmIntegerRepr {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self::new(VmIntegerReprKind::$variant(From::from(value)))
                }
            }
        )*)*
    };
}

impl_from! {
	Signed => [i8, i16, i32, i64, i128],
	Unsigned => [u8, u16, u32, u64, u128],
	Isize => [isize],
	Usize => [usize],
}

impl fmt::Display for VmIntegerRepr {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match &self.kind {
			VmIntegerReprKind::Signed(value) => value.fmt(f),
			VmIntegerReprKind::Unsigned(value) => value.fmt(f),
			VmIntegerReprKind::Isize(value) => value.fmt(f),
			VmIntegerReprKind::Usize(value) => value.fmt(f),
		}
	}
}

impl fmt::Debug for VmIntegerRepr {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.kind.fmt(f)
	}
}

/// Format a static item for diagnostics, using its name if debug information
/// was available and falling back to its slot if it wasn't.
fn fmt_global(f: &mut fmt::Formatter<'_>, slot: usize, name: Option<&Item>) -> fmt::Result {
	match name {
		Some(name) => write!(f, "`{name}` (slot {slot})"),
		None => write!(f, "in slot {slot}"),
	}
}

#[cfg(test)]
mod logical_frame_tests {
	use super::*;

	#[test]
	#[cfg(feature = "execution")]
	fn origin_snapshot_checks_binding_and_hash_at_capture() {
		use crate::experimental_core::error::ErrorOrigin;
		let context = crate::Context::with_default_modules().unwrap();
		let mut sources = crate::Sources::new();
		sources
			.insert(
				crate::Source::new("origin.rn", "pub fn main() { 1.missing_method() }").unwrap(),
			)
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let retained = vm.context().clone();
		let weak = Arc::downgrade(&retained);
		let before = weak.strong_count();
		let mut error = vm.call(["main"], ()).unwrap_err();
		assert_eq!(error.origin(), ErrorOrigin::Script);
		assert_eq!(
			weak.strong_count(),
			before,
			"error must not retain the context"
		);
		let mut other = crate::Context::with_default_modules().unwrap();
		let mut module = crate::Module::new();
		module
			.associated_function("missing_method", |_: i64| 1i64)
			.unwrap();
		other.install(module).unwrap();
		*vm.context_mut() = Arc::try_new(other.runtime().unwrap()).unwrap();
		assert_eq!(error.capture_origin(&vm).0, ErrorOrigin::Unknown);
		assert_eq!(
			error.origin(),
			ErrorOrigin::Script,
			"snapshot must not change with context"
		);
		*vm.context_mut() = retained.clone();
		if let VmErrorKind::MissingInstanceFunction { hash, .. } = &mut error.inner.error.kind {
			*hash = crate::Hash::EMPTY;
		} else {
			panic!("expected missing instance function");
		}
		assert_eq!(error.capture_origin(&vm).0, ErrorOrigin::Unknown);
		assert_eq!(
			error.origin(),
			ErrorOrigin::Script,
			"snapshot must not change with error mutation"
		);
		drop(vm);
		drop(retained);
		assert_eq!(
			weak.strong_count(),
			0,
			"snapshot must not keep context alive"
		);
		assert_eq!(error.origin(), ErrorOrigin::Script);
		error.inner.stacktrace[0].origin = None;
		assert_eq!(error.origin(), ErrorOrigin::Unknown);
	}

	#[test]
	#[cfg(feature = "execution")]
	fn error_windows_keep_independent_origin_snapshots_without_context_cycles() {
		let first = crate::Context::with_default_modules().unwrap();
		let second = crate::Context::with_default_modules().unwrap();
		let mut a = crate::Sources::new();
		let mut b = crate::Sources::new();
		a.insert(crate::Source::new("a.rn", "pub fn main() { 1 }").unwrap())
			.unwrap();
		b.insert(crate::Source::new("b.rn", "pub fn main() { 2 }").unwrap())
			.unwrap();
		let vm_a = crate::prepare(&mut a)
			.with_context(&first)
			.build_vm()
			.unwrap();
		let vm_b = crate::prepare(&mut b)
			.with_context(&second)
			.build_vm()
			.unwrap();
		let a_weak = Arc::downgrade(vm_a.context());
		let b_weak = Arc::downgrade(vm_b.context());
		let error = VmError::with_vm::<()>(Err(VmError::panic("inner")), &vm_a).unwrap_err();
		let first_snapshot = error.inner.stacktrace[0].origin;
		let second_snapshot = error.capture_origin(&vm_b);
		let error = VmError::with_vm::<()>(Err(error), &vm_b).unwrap_err();
		assert_eq!(error.inner.stacktrace.len(), 2);
		assert_eq!(error.inner.stacktrace[0].origin, first_snapshot);
		assert_eq!(error.inner.stacktrace[1].origin, Some(second_snapshot));
		assert_eq!(
			(error.origin(), error.origin_instruction()),
			first_snapshot.unwrap()
		);
		drop(vm_a);
		drop(vm_b);
		assert_eq!(a_weak.strong_count(), 0);
		assert_eq!(b_weak.strong_count(), 0);
		assert_eq!(
			(error.origin(), error.origin_instruction()),
			first_snapshot.unwrap()
		);
	}

	#[test]
	fn d1e_unresolved_window_retains_each_frame() {
		let mut error = VmError::panic("unresolved");
		error.inner.stacktrace.push(VmErrorLocation {
			unit: Arc::try_new(Unit::default()).unwrap(),
			ip: 0,
			#[cfg(feature = "execution")]
			origin: None,
			frames: rust_alloc::vec![CallFrame {
                ip: 0,
                top: 0,
                isolated: super::super::Isolated::None,
                out: super::super::Output::discard(),
            }; 2],
		});
		let frames = error.logical_frames().collect::<rust_alloc::vec::Vec<_>>();
		#[cfg(feature = "execution")]
		assert_eq!(
			error.origin(),
			crate::experimental_core::error::ErrorOrigin::Unknown
		);
		assert_eq!(frames.len(), 3);
		assert!(frames.iter().all(|frame| frame.location.is_none()));
		assert_eq!(rust_alloc::format!("{}", error.diagnostic(&crate::Sources::new())),
            "Panicked: unresolved\n  at <unknown>\n  called from <unknown>\n  called from <unknown>");
	}
}
