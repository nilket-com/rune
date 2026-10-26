//! The execution core executor: an explicit-frame interpreter over a compiled
//! [`Program`]. It uses no legacy VM `Value`, `Vm` or protocol caller.
//!
//! # Output
//!
//! `println!` holds the process stdout lock while writing the text and newline.
//! There is no cumulative output cap or explicit flush; buffering follows Rust stdout.
//! Individual formatting calls remain bounded by their formatting limits. Earlier
//! output is not rolled back when a later write or language operation fails.

use super::{
	arithmetic,
	code::{CloneMode, Code, Op},
	error::{
		AdmittedMethod, BinarySymbol, Detail, Error, ErrorKind, FieldRef, IntTarget, IntValue,
		Location, Result, TypeName, UnarySymbol,
	},
	frontend::Program,
	heap::{FormatLimits, Store},
	value::{Root, Value},
};
use crate::alloc::{Box, Vec};

const MAX_FRAMES: usize = {
	let frames = isize::MAX as usize / core::mem::size_of::<Frame>();
	let locations = isize::MAX as usize / core::mem::size_of::<Location>();
	if frames < locations {
		frames
	} else {
		locations
	}
};
const INITIAL_FRAMES: usize = 8;
const INITIAL_SLOTS: usize = 64;

/// Logical invocation limits; bookkeeping grows with use. Allocator overgrowth
/// can exceed these limits physically. Payloads and output are separate.
/// Defaults allow platform-representable frame and slot buffers without reserving
/// them up front. Script calls and admitted callbacks use iterative heap frames.
/// These limits do not bound trusted Rust callbacks that create their own executor
/// or legacy VM and recurse through Rust outside this executor.
/// The optional native tier compiles on the invoking thread and needs an adequate
/// machine stack for the JIT compiler, independently of script call depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutorLimits {
	/// Maximum simultaneously published call frames.
	pub frames: usize,
	/// Maximum live value slots across all call frames.
	pub slots: usize,
}
impl Default for ExecutorLimits {
	fn default() -> Self {
		Self {
			frames: MAX_FRAMES,
			slots: MAX_SLOTS,
		}
	}
}
#[derive(Debug)]
struct CallSetupFailure {
	error: Error,
	cleanup_error: Option<Error>,
}
impl From<Error> for CallSetupFailure {
	fn from(error: Error) -> Self {
		Self {
			error,
			cleanup_error: None,
		}
	}
}

/// Entry arguments: host roots (owner-checked in `enter`, read in place) or
/// crate-internal values.
#[derive(Clone, Copy)]
enum Args<'a> {
	Roots(&'a [&'a Root]),
	#[cfg_attr(not(test), allow(dead_code))]
	Values(&'a [Value]),
	Slots {
		base: usize,
		indices: &'a [u32],
	},
}

impl Args<'_> {
	fn len(self) -> usize {
		match self {
			Self::Roots(roots) => roots.len(),
			Self::Values(values) => values.len(),
			Self::Slots { indices, .. } => indices.len(),
		}
	}

	fn get(self, slots: &[Value], i: usize) -> Value {
		match self {
			Self::Roots(roots) => roots[i].value,
			Self::Values(values) => values[i],
			Self::Slots { base, indices } => slots[base + indices[i] as usize],
		}
	}
}

const MAX_SLOTS: usize = isize::MAX as usize / core::mem::size_of::<Value>();

/// Native room is bounded by logical limits and actual frame, slot and
/// diagnostic capacities. Native execution and commit never grow them.
#[cfg(feature = "native-code")]
#[cfg_attr(
	not(any(test, all(target_arch = "x86_64", target_os = "linux"))),
	allow(dead_code)
)]
pub(crate) fn region_room(
	frames: usize,
	slots: usize,
	window_end: usize,
	limits: ExecutorLimits,
	capacities: (usize, usize, usize),
) -> super::region::commit::Room {
	if window_end != slots {
		return super::region::commit::Room {
			frames: 0,
			slots: 0,
		};
	}
	super::region::commit::Room {
		frames: limits
			.frames
			.min(capacities.0)
			.min(capacities.2)
			.saturating_sub(frames),
		slots: limits.slots.min(capacities.1).saturating_sub(slots),
	}
}

/// D14: a frame records only its callsite's pc in the caller; the Location is
/// derived on failure from the frame below it. `u32::MAX` is never a valid pc:
/// the Builder assigns pcs with `u32::try_from` and caps a function at
/// `MAX_CODE_BYTES / size_of::<Instruction>()` (1,048,576) instructions.
const NO_CALLSITE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Frame {
	function: u32,
	/// The pc of the Call in the caller (the frame directly below), or
	/// NO_CALLSITE for the entry frame.
	callsite_pc: u32,
	pc: usize,
	base: usize,
	out: Option<usize>,
}

/// A failed invocation. Plain source diagnostics only: this record cannot root a
/// script heap.
#[derive(Debug)]
pub struct Failure {
	numeric_context: Option<NumericContext>,
	/// The error that stopped the invocation.
	pub error: Error,
	/// Full dynamic missing-field key, if its best-effort diagnostic copy succeeded.
	pub missing_key: Option<Box<str>>,
	/// Complete panic text, copied before cleanup and independent of Store lifetime.
	pub panic_message: Option<Box<str>>,
	/// Complete native failure text, independent of legacy VM scratch and Store lifetime.
	pub host_message: Option<Box<str>>,
	/// The callsite locations of the active calls, innermost caller first; empty when the
	/// failure happened in the entry function or before it ran. An unresolved
	/// active caller is retained as [`Location::UNKNOWN`].
	pub call_chain: Vec<Location>,
	/// A second error raised while unwinding the invocation's frames, if any.
	pub cleanup_error: Option<Error>,
}

/// The exact receiver and associated-function identities of a failed protocol
/// lookup. This describes the root invocation error, never a cleanup error.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingInstanceFunction {
	/// Combined associated-function hash.
	pub hash: crate::Hash,
	/// Receiver type hash.
	pub instance: crate::Hash,
	target: TypeName,
}
impl MissingInstanceFunction {
	/// Receiver display name, using the same presentation as the diagnostic.
	pub fn instance_name(&self, _program: &Program) -> impl core::fmt::Display + '_ {
		self.target
	}
}

impl Failure {
	/// Inspect typed argument provenance independently of the logical frames.
	#[cold]
	#[inline(never)]
	pub fn invocation_context(&self) -> impl Iterator<Item = crate::runtime::InvocationContext> {
		self.error
			.detail
			.numeric_argument()
			.map(|(_, arg)| crate::runtime::InvocationContext::BadArgument {
				arg: usize::from(arg),
			})
			.into_iter()
	}

	/// Inspect the origin followed by callers from inner to outer.
	#[cold]
	#[inline(never)]
	pub fn logical_frames(&self) -> impl Iterator<Item = crate::runtime::LogicalFrame<'_>> {
		use crate::runtime::{LogicalFrame, LogicalFrameRole, LogicalLocation};
		core::iter::once((LogicalFrameRole::Origin, self.error.location))
			.chain(
				self.call_chain
					.iter()
					.copied()
					.map(|at| (LogicalFrameRole::Caller, Some(at))),
			)
			.map(|(role, location)| LogicalFrame {
				role,
				location: location.filter(|at| *at != Location::UNKNOWN).map(|at| {
					LogicalLocation {
						source_id: crate::SourceId::new(at.source),
						span: crate::ast::Span::new(at.start, at.end),
						unit: None,
					}
				}),
			})
	}

	/// Render the primary error and its logical invocation frames.
	#[cold]
	#[inline(never)]
	pub fn diagnostic<'a>(
		&'a self,
		program: &'a Program,
		sources: &'a crate::Sources,
	) -> impl core::fmt::Display + 'a {
		struct Diagnostic<'a>(&'a Failure, &'a Program, &'a crate::Sources);
		impl core::fmt::Display for Diagnostic<'_> {
			#[cold]
			#[inline(never)]
			fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
				write!(f, "{}", self.0.message(self.1))?;
				for frame in self.0.logical_frames() {
					crate::runtime::write_logical_frame(f, frame, self.2)?;
				}
				Ok(())
			}
		}
		Diagnostic(self, program, sources)
	}

	/// Return the missing instance function from the root error, if any.
	pub fn missing_instance_function(&self) -> Option<MissingInstanceFunction> {
		let Detail::MissingInstanceFunction {
			target,
			hash,
			instance,
		} = self.error.detail
		else {
			return None;
		};
		Some(MissingInstanceFunction {
			target,
			hash,
			instance,
		})
	}

	/// Render the error including owned dynamic key or panic text. Error::message
	/// alone renders these failures without their dynamic payload.
	pub fn message<'a>(&'a self, program: &'a Program) -> impl core::fmt::Display + 'a {
		FailureMessage {
			failure: self,
			program,
		}
	}
}
struct FailureMessage<'a> {
	failure: &'a Failure,
	program: &'a Program,
}
impl core::fmt::Display for FailureMessage<'_> {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		if let (Detail::MissingKey { target }, Some(key)) = (
			self.failure.error.detail,
			self.failure.missing_key.as_deref(),
		) {
			return write!(f, "Missing field `{key}` on `{target}`");
		}
		if let (Detail::PanicMessage, Some(message)) = (
			self.failure.error.detail,
			self.failure.panic_message.as_deref(),
		) {
			return write!(f, "Panicked: {message}");
		}
		if let Some(message) = self.failure.host_message.as_deref() {
			match self.failure.error.detail {
				Detail::HostMessage { .. }
				| Detail::HostExpectedType { .. }
				| Detail::HostExpectedNumber { .. } => return f.write_str(message),
				Detail::HostPanicked => return write!(f, "host panicked: {message}"),
				_ => (),
			}
		}
		if let Some(context) = self.failure.numeric_context {
			let error = &self.failure.error;
			let op = super::detail::numeric_symbol(context.op);
			if let Detail::Coerce { value, to } = error.detail {
				return crate::runtime::messages::shift_coerce(
					f,
					if context.rhs == TypeName::U64 {
						"u64"
					} else {
						"i64"
					},
					value,
					to,
					context.op,
				);
			}
			let reason = if error.detail == Detail::Underflow {
				"Numerical underflow"
			} else if error.kind == ErrorKind::DivisionByZero {
				"Division by zero"
			} else {
				"Numerical overflow"
			};
			return crate::runtime::messages::numeric_fault(
				f,
				reason,
				op,
				crate::runtime::messages::Operand(&context.lhs),
			);
		}
		core::fmt::Display::fmt(&self.failure.error.message(self.program), f)
	}
}

/// Runs functions of a [`Program`] against a [`Store`].
///
/// One executor can be invoked any number of times; it is left clean after success, a
/// run-time error, a fuel halt or a cancellation.
pub struct Executor {
	frames: Vec<Frame>,
	slots: Vec<Value>,
	diagnostics: Vec<Location>,
	canceled: bool,
	format_limits: FormatLimits,
	limits: ExecutorLimits,
	configured_host_runtime: Option<crate::sync::Arc<crate::runtime::RuntimeContext>>,
	// Integer tag so the cold-arm predicate combines both authorities before branching.
	runtime_format_guard: u8,
	active_host_runtime: Option<crate::sync::Arc<crate::runtime::RuntimeContext>>,
	pending_host_message: Option<Box<str>>,
	pending_host_cleanup: Option<Error>,
	#[cfg(test)]
	rollback_error: Option<Error>,
	/// Test-only, per executor: when set, `Print` appends here instead of stdout.
	#[cfg(test)]
	pub(crate) test_output: Option<std::vec::Vec<u8>>,
	/// Handshake after a successful real write, only in isolated tests.
	#[cfg(test)]
	pub(crate) test_after_print: Option<fn()>,
	#[cfg(test)]
	pub(crate) test_fail_key_copy: bool,
	/// P2a native tier: the last code id whose analysis admitted no region at all (a
	/// property of the code alone, valid on any thread); 0 is never a code id. Plain data,
	/// so `Executor` stays `Send`.
	#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
	no_region_code: u32,
}

fn error(kind: ErrorKind) -> Error {
	Error::new(kind, "execute")
}

// `Failure` carries a located error and its call chain by value; it is the public error
// type of `invoke`, so boxing it would change the API.
#[allow(clippy::result_large_err)]
impl Executor {
	/// A new executor with small initial bookkeeping buffers and default limits.
	pub fn new() -> Result<Self> {
		Self::with_limits(ExecutorLimits::default())
	}

	/// Configures logical limits without reserving their maxima. Initial and
	/// subsequent allocations can fail fallibly with Allocation.
	pub fn with_limits(limits: ExecutorLimits) -> Result<Self> {
		if limits.frames == 0
			|| limits.slots == 0
			|| limits
				.frames
				.checked_mul(core::mem::size_of::<Frame>())
				.is_none_or(|n| n > isize::MAX as usize)
			|| limits
				.frames
				.checked_mul(core::mem::size_of::<Location>())
				.is_none_or(|n| n > isize::MAX as usize)
			|| limits
				.slots
				.checked_mul(core::mem::size_of::<Value>())
				.is_none_or(|n| n > isize::MAX as usize)
		{
			return Err(error(ErrorKind::FrameCapacity));
		}
		Ok(Self {
			frames: Vec::try_with_capacity(limits.frames.min(INITIAL_FRAMES))?,
			slots: Vec::try_with_capacity(limits.slots.min(INITIAL_SLOTS))?,
			diagnostics: Vec::try_with_capacity(limits.frames.min(INITIAL_FRAMES))?,
			canceled: false,
			format_limits: FormatLimits::default(),
			limits,
			configured_host_runtime: None,
			runtime_format_guard: 0,
			active_host_runtime: None,
			pending_host_message: None,
			pending_host_cleanup: None,
			#[cfg(test)]
			rollback_error: None,
			#[cfg(test)]
			test_output: None,
			#[cfg(test)]
			test_after_print: None,
			#[cfg(test)]
			test_fail_key_copy: false,
			#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
			no_region_code: 0,
		})
	}

	/// Configured logical frame and slot limits.
	pub fn limits(&self) -> &ExecutorLimits {
		&self.limits
	}

	/// Retained executor bookkeeping, modeled from actual buffer capacities.
	/// This excludes allocator overhead, code, store payloads, formatting scratch
	/// and output; it is not a process peak or RSS ceiling.
	pub fn bookkeeping_bytes(&self) -> Result<usize> {
		let frames = self
			.frames
			.capacity()
			.checked_mul(core::mem::size_of::<Frame>());
		let slots = self
			.slots
			.capacity()
			.checked_mul(core::mem::size_of::<Value>());
		let diagnostics = self
			.diagnostics
			.capacity()
			.checked_mul(core::mem::size_of::<Location>());
		frames
			.and_then(|n| n.checked_add(slots?))
			.and_then(|n| n.checked_add(diagnostics?))
			.ok_or_else(|| error(ErrorKind::Overflow))
	}

	/// Replace the immutable host binding only while the executor is idle.
	/// Clearing it does not consume a pending cancellation.
	pub fn set_host_runtime(
		&mut self,
		runtime: Option<crate::sync::Arc<crate::runtime::RuntimeContext>>,
	) -> Result<()> {
		if !self.is_idle() || self.active_host_runtime.is_some() {
			return Err(error(ErrorKind::InvalidCode));
		}
		self.runtime_format_guard =
			u8::from(runtime.as_ref().is_some_and(|r| r.formatting.is_some()));
		self.configured_host_runtime = runtime;
		Ok(())
	}

	/// Requests cancellation. The next charged instruction of this executor consumes the
	/// request and fails its invocation with [`ErrorKind::Cancelled`]; a refusal before
	/// the first charged instruction leaves the request pending.
	pub fn request_cancel(&mut self) {
		self.canceled = true;
	}

	/// Limits used by each complete template/println formatting call.
	pub fn format_limits(&self) -> &FormatLimits {
		&self.format_limits
	}

	/// Sets template formatting limits without allocating. Growth follows actual
	/// traversal; raise bytes, nodes, depth and scratch independently as needed.
	pub fn set_format_limits(&mut self, limits: FormatLimits) {
		self.format_limits = limits;
	}

	/// Single-shot entry: one invocation on an executor that is then dropped.
	/// Literally `invoke`; there is no separate execution path.
	pub fn run(
		mut self,
		program: &Program,
		store: &mut Store,
		function: u32,
		args: &[&Root],
		fuel: usize,
	) -> core::result::Result<Root, Failure> {
		self.invoke(program, store, function, args, fuel)
	}

	/// One invocation on a prepared code object and store.
	///
	/// Internal proof-dependent entries return [`super::error::Refusal::Entry`]
	/// before argument transfer or any charged instruction. On Entry refusal,
	/// run stock from the untouched input; never retry after charged execution.
	/// The function, its
	/// arity and then every argument root (closed-store policy, store,
	/// generation) are checked before any argument is retained: a refused
	/// argument retains nothing. With default formatter authority, argument validation and transfer use no
	/// proportional scratch: arguments go straight from roots into the frame.
	/// Non-default authority first validates the reachable argument graph with
	/// fallible scratch bounded by its unique nodes. Unsafe nominal dispatch
	/// returns Entry before effects; scratch failure returns Allocation.
	/// Invocation setup can still reserve diagnostics after a previous failure. The returned root owns the
	/// frame's single return reference. The executor is left clean and may be
	/// invoked again after success, a run-time error, a fuel halt or
	/// cancellation; a pending cancellation survives a refusal before the first
	/// charged instruction and only that instruction consumes it.
	pub fn invoke(
		&mut self,
		program: &Program,
		store: &mut Store,
		function: u32,
		args: &[&Root],
		fuel: usize,
	) -> core::result::Result<Root, Failure> {
		#[cfg(test)]
		program.assert_entry_guard_for_test();
		// The former entry-policy branch also guards non-default formatting.
		// Both tags are cached at preparation; default authority with no internal
		// entry policy takes one branch and keeps the ordinary Code-only path.
		if (program.formatting.guard() | self.runtime_format_guard) != 0 {
			if let Err(error) = self.validate_format_entry(
				Some(program),
				(&program.code, function),
				program.formatting.as_ref(),
				store,
				args,
			) {
				return Err(Failure {
					numeric_context: None,
					error,
					missing_key: None,
					panic_message: None,
					host_message: None,
					call_chain: Vec::new(),
					cleanup_error: None,
				});
			}
		}
		self.invoke_checked(&program.code, store, function, args, fuel)
	}

	/// Test-only hand-built Code has no compile Context. Use the explicitly
	/// configured runtime's certificate, and run the same checked entry path.
	#[cfg(test)]
	pub(crate) fn invoke_code(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: &[&Root],
		fuel: usize,
	) -> core::result::Result<Root, Failure> {
		let authority = self
			.configured_host_runtime
			.as_ref()
			.and_then(|r| r.formatting.clone());
		if authority.is_some() {
			if let Err(error) =
				self.validate_format_entry(None, (code, function), authority.as_ref(), store, args)
			{
				return Err(Failure {
					numeric_context: None,
					error,
					missing_key: None,
					panic_message: None,
					host_message: None,
					call_chain: Vec::new(),
					cleanup_error: None,
				});
			}
		}
		self.invoke_checked(code, store, function, args, fuel)
	}

	fn invoke_checked(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: &[&Root],
		fuel: usize,
	) -> core::result::Result<Root, Failure> {
		// Keep the ordinary Code-only admission path independent of cold policy work.
		let admission = (|| {
			if store.is_closed() {
				return Err(error(ErrorKind::ClosedStore));
			}
			if !self.is_idle() || self.active_host_runtime.is_some() {
				return Err(error(ErrorKind::InvalidCode));
			}
			let entry = code
				.function(function)
				.ok_or_else(|| error(ErrorKind::InvalidCode))?;
			if args.len() != entry.parameters as usize {
				// Stock raises the same error when an embedder calls an entry
				// with the wrong count.
				let mut error = error(ErrorKind::ArgumentCount);
				error.detail = Detail::Arity {
					given: u32::try_from(args.len()).unwrap_or(u32::MAX),
					expected: entry.parameters,
				};
				return Err(error);
			}
			code.validate_hosts(self.configured_host_runtime.as_deref())?;
			Ok(())
		})();
		if let Err(error) = admission {
			return Err(Failure {
				numeric_context: None,
				error,
				missing_key: None,
				panic_message: None,
				host_message: None,
				call_chain: Vec::new(),
				cleanup_error: None,
			});
		}
		self.active_host_runtime = self.configured_host_runtime.clone();
		let result = {
			#[cfg(not(all(feature = "native-code", target_arch = "x86_64", target_os = "linux")))]
			let value = self.invoke_args(code, store, function, Args::Roots(args), fuel);
			#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
			let value = self.invoke_native(code, store, function, Args::Roots(args), fuel);
			value
		};
		self.active_host_runtime = None;
		result.map(|value| store.export(value))
	}

	#[cold]
	#[inline(never)]
	fn validate_format_entry(
		&self,
		program: Option<&Program>,
		entry: (&Code, u32),
		expected: Option<&crate::sync::Arc<crate::function::formatting::Certificate>>,
		store: &Store,
		args: &[&Root],
	) -> Result<()> {
		use super::error::Refusal;
		if program.is_some_and(|program| program.entry_refused(entry.1)) {
			return Err(Error::refused(Refusal::Entry));
		}
		// A default-authority program may arrive only for its internal entry policy.
		// Keep that policy check alone; ordinary admission still happens below.
		if expected.is_none() && self.runtime_format_guard == 0 {
			return Ok(());
		}
		// Guarded formatting repeats this read-only preflight so the ordinary
		// Code-only admission path remains independent of cold policy work.
		// Preserve used-host InvalidCode before formatter authority refusals.
		let (code, function) = entry;
		if store.is_closed() {
			return Err(error(ErrorKind::ClosedStore));
		}
		if !self.is_idle() || self.active_host_runtime.is_some() {
			return Err(error(ErrorKind::InvalidCode));
		}
		let entry = code
			.function(function)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		if args.len() != entry.parameters as usize {
			// Stock raises the same error when an embedder calls an entry
			// with the wrong count.
			let mut error = error(ErrorKind::ArgumentCount);
			error.detail = Detail::Arity {
				given: u32::try_from(args.len()).unwrap_or(u32::MAX),
				expected: entry.parameters,
			};
			return Err(error);
		}
		// Keep the ordinary validator's hot call graph unchanged. These immutable
		// inputs repeat its binding, signature, handler identity and absence checks.
		let runtime = self.configured_host_runtime.as_deref();
		let invalid = || Error::new(ErrorKind::InvalidCode, "validate code");
		let (bindings, has_absent) = code.host_validation_parts();
		for binding in bindings {
			let actual = runtime
				.and_then(|runtime| runtime.function(&binding.hash))
				.ok_or_else(invalid)?;
			if !binding.signature.valid() || !binding.handler.same_allocation(actual) {
				return Err(invalid());
			}
		}
		if has_absent {
			code.validate_absence(runtime)?;
		}

		if let Some(runtime) = self.configured_host_runtime.as_ref() {
			match (expected, runtime.formatting.as_ref()) {
				(Some(a), Some(b)) if crate::sync::Arc::ptr_eq(a, b) || a.equivalent(b) => {}
				(None, None) => {}
				_ => return Err(Error::refused(Refusal::Entry)),
			}
		}
		if let Some(certificate) = expected {
			store.validate_format_roots(args, certificate)?;
		}
		Ok(())
	}

	#[inline(never)]
	fn print_instruction(&mut self, store: &Store, value: Value) -> Result<()> {
		use std::io::Write;
		let text = store.text(value)?;
		// Test-only seam, scoped to this executor: same order, no stdout.
		#[cfg(test)]
		let captured = match self.test_output.as_mut() {
			Some(buffer) => {
				buffer.extend_from_slice(text.as_bytes());
				buffer.push(b'\n');
				true
			}
			None => false,
		};
		#[cfg(not(test))]
		let captured = false;

		// Hold one lock across the text and newline.
		if !captured {
			let stdout = std::io::stdout();
			let mut sink = stdout.lock();
			sink.write_all(text.as_bytes())
				.and_then(|_| sink.write_all(b"\n"))
				.map_err(|e| {
					error(ErrorKind::Output).with_detail(Detail::Io {
						kind: e.kind(),
						os: e.raw_os_error(),
					})
				})?;
			#[cfg(test)]
			if let Some(hook) = self.test_after_print {
				hook();
			}
		}
		Ok(())
	}

	#[inline(never)]
	#[allow(clippy::chunks_exact_to_as_chunks)]
	fn template_instruction(
		&mut self,
		store: &mut Store,
		function: &super::code::Function,
		instruction: &super::code::CachedInstruction,
		base: usize,
	) -> Result<()> {
		// Both references already exist at dispatch; no whole-Op temporary.
		let Op::Template {
			out,
			args_start,
			args_len,
		} = instruction.op
		else {
			return Err(error(ErrorKind::InvalidCode));
		};
		let end = args_start
			.checked_add(args_len)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		let parts = function
			.args
			.get(args_start as usize..end as usize)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		// D18: read the parts' slots here instead of capturing `get`; the
		// lookup and its error are identical, but `get` no longer escapes
		// into the out-of-line formatter.
		let slots = &self.slots;
		let text = store.format_template_specs(
			parts.chunks_exact(2).map(|pair| {
				let value = slots
					.get(base + pair[0] as usize)
					.copied()
					.ok_or_else(|| error(ErrorKind::InvalidCode))?;
				let (debug, spec) = super::format_spec::load(&function.args, pair[1])?;
				Ok((value, debug, spec))
			}),
			&self.format_limits,
		)?;
		let value = store.string_owned(text)?;
		self.put(store, base + out as usize, value)?;
		Ok(())
	}

	#[inline(never)]
	#[allow(clippy::chunks_exact_to_as_chunks)]
	fn record_instruction(
		&mut self,
		store: &mut Store,
		strings: &[crate::alloc::String],
		descriptors: &[u32],
		base: usize,
		out: u32,
		tag: super::value::Type,
	) -> Result<()> {
		let get = |n: u32| -> Result<Value> {
			self.slots
				.get(base + n as usize)
				.copied()
				.ok_or_else(|| error(ErrorKind::InvalidCode))
		};
		let mut fields = Vec::try_with_capacity(descriptors.len() / 2)?;

		for pair in descriptors.chunks_exact(2) {
			fields.try_push((strings[pair[0] as usize].as_str(), get(pair[1])?))?;
		}

		let value = store.record(tag, &fields)?;
		self.put(store, base + out as usize, value)?;
		Ok(())
	}

	#[inline(never)]
	fn match_variant_call(
		&mut self,
		store: &mut Store,
		function: &super::code::Function,
		base: usize,
		out: u32,
		args_start: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let end = args_start.checked_add(2).ok_or_else(invalid)?;
		let args = function
			.args
			.get(args_start as usize..end as usize)
			.ok_or_else(invalid)?;
		let read = |slot: u32| {
			if slot >= function.slots {
				return Err(invalid());
			}
			let index = base.checked_add(slot as usize).ok_or_else(invalid)?;
			self.slots.get(index).copied().ok_or_else(invalid)
		};
		let value = read(args[0])?;
		let Value::Type(super::value::Type::Struct(expected)) = read(args[1])? else {
			return Err(invalid());
		};
		let matches = match value {
			Value::Type(_) | Value::Function { .. } => false,
			_ => match store.tag(value)? {
				super::value::Type::Struct(actual) => {
					super::nominal::same_variant(actual, expected)
				}
				_ => false,
			},
		};
		if out >= function.slots {
			return Err(invalid());
		}
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		self.put(store, destination, Value::Bool(matches))
	}

	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	fn intrinsic(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		args_start: u32,
		args_len: u32,
	) -> Result<()> {
		if code.host(id).is_some() {
			return self.host_call(code, store, function, base, out, id, args_start, args_len);
		}
		if super::code::variant_test_id(id).is_some() {
			if args_len != 1 {
				return Err(error(ErrorKind::InvalidCode));
			}
			let slot = *function
				.args
				.get(args_start as usize)
				.ok_or_else(|| error(ErrorKind::InvalidCode))?;
			let value = self
				.slots
				.get(base + slot as usize)
				.copied()
				.ok_or_else(|| error(ErrorKind::InvalidCode))?;
			let matches =
				store.variant_matches(value, super::code::variant_test_id(id).unwrap())?;
			return self.put(store, base + out as usize, Value::Bool(matches));
		}
		self.fixed_intrinsic(code, store, function, base, out, id, args_start, args_len)
	}

	// Only a certified shared Call miss reaches this out-of-line intrinsic path.
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	fn fixed_intrinsic(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		args_start: u32,
		args_len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);

		let certificate = code.intrinsic(id).ok_or_else(invalid)?;
		if id == super::code::CLOSURE_CREATE {
			if args_len != certificate.arity {
				return Err(invalid());
			}
			let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
			let args = function
				.args
				.get(args_start as usize..end as usize)
				.ok_or_else(invalid)?;
			let read = |slot: u32| {
				let index = base.checked_add(slot as usize).ok_or_else(invalid)?;
				self.slots.get(index).copied().ok_or_else(invalid)
			};
			let Value::U64(function_id) = read(args[0])? else {
				return Err(invalid());
			};
			let function_id = u32::try_from(function_id).map_err(|_| invalid())?;
			let env = read(args[1])?;
			let target = code.function(function_id).ok_or_else(invalid)?;
			if (target.parameters as usize) < store.len(env)? {
				return Err(invalid());
			}
			let value = store.closure(code.id(), function_id, env)?;
			let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
			return self.put(store, destination, value);
		}
		if id == super::code::ANY_TYPE_OF || id == super::code::ANY_TYPE_NAME {
			return self.any_value_call(store, function, base, out, id, args_start, args_len);
		}
		if (0x8000_0060..=0x8000_007b).contains(&id) {
			return self.text_call(store, function, base, out, id, args_start, args_len);
		}
		if id == super::code::INSTANCE_METHOD {
			return self
				.instance_method_call(code, store, function, base, out, args_start, args_len);
		}
		if let Some(method) = crate::function::variant::Method::from_id(id) {
			return self.variant_method_call(
				code, store, function, base, out, method, args_start, args_len,
			);
		}
		if let Some(method) = crate::function::object::Method::from_id(id) {
			return self.object_method_call(
				code, store, function, base, out, method, args_start, args_len,
			);
		}
		if (super::code::SORT_BEGIN..=super::code::SORT_ORDERING).contains(&id) {
			if args_len != certificate.arity {
				return Err(invalid());
			}
			return self.sort_call(code, store, function, base, out, id, args_start, args_len);
		}
		if (super::code::RANGE_FROM..=super::code::ANY_READY).contains(&id) {
			if args_len != certificate.arity {
				return Err(invalid());
			}
			return self.small_forms_call(store, function, base, out, id, args_start, args_len);
		}
		if id == super::code::MATCH_VARIANT {
			if args_len != certificate.arity {
				return Err(invalid());
			}
			return self.match_variant_call(store, function, base, out, args_start);
		}
		if let Some(call) = crate::function::numeric::Call::from_id(id) {
			if args_len != certificate.arity {
				return Err(invalid());
			}
			return self.numeric_call(store, function, base, out, call, args_start, args_len);
		}
		if let Some(intrinsic) = crate::function::iter_terminal::Intrinsic::from_id(id) {
			return self.terminal_call(store, function, base, out, intrinsic, args_start, args_len);
		}
		if args_len != certificate.arity
			|| !matches!(id, super::code::PRINT_RAW | super::code::VARIANT_SUCCESS)
		{
			if matches!(
				id,
				super::code::VEC_NEW | super::code::VEC_WITH_CAPACITY | super::code::VEC_RESIZE
			) {
				return self
					.vec_constructor_call(store, function, base, out, id, args_start, args_len);
			}
			if id != super::code::CAPTURE_TAKE {
				return self
					.number_call(code, store, function, base, out, id, args_start, args_len);
			}
			return self.capture_take_call(store, function, base, out, id, args_start, args_len);
		}
		let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
		let args = function
			.args
			.get(args_start as usize..end as usize)
			.ok_or_else(invalid)?;
		let slot = *args.first().ok_or_else(invalid)?;
		if slot >= function.slots || out >= function.slots {
			return Err(invalid());
		}
		let index = base.checked_add(slot as usize).ok_or_else(invalid)?;
		let value = *self.slots.get(index).ok_or_else(invalid)?;
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		if id == super::code::VARIANT_SUCCESS {
			let success = store.variant_success(value)?;
			return self.put(store, destination, Value::Bool(success));
		}
		{
			let text = store.text(value).map_err(|mut e| {
				if e.kind == ErrorKind::Type && e.detail == Detail::None {
					if let Some(found) = store.diagnostic_type(value) {
						e = e.with_detail(Detail::ExpectedType {
							expected: TypeName::String,
							found,
						});
					}
				}
				e
			})?;
			#[cfg(test)]
			let captured = match self.test_output.as_mut() {
				Some(buffer) => {
					buffer.extend_from_slice(text.as_bytes());
					true
				}
				None => false,
			};
			#[cfg(not(test))]
			let captured = false;
			if !captured {
				use std::io::Write;
				let stdout = std::io::stdout();
				let mut sink = stdout.lock();
				sink.write_all(text.as_bytes()).map_err(|e| {
					error(ErrorKind::Output).with_detail(Detail::Io {
						kind: e.kind(),
						os: e.raw_os_error(),
					})
				})?;
				#[cfg(test)]
				if let Some(hook) = self.test_after_print {
					hook();
				}
			}
		}
		self.put(store, destination, Value::Unit)
	}

	#[cold]
	#[inline(never)]
	fn capture_take_call(
		&mut self,
		store: &mut Store,
		function: &super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		args_start: u32,
		args_len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		if id != super::code::CAPTURE_TAKE || args_len != 1 {
			return Err(invalid());
		}
		let slot = *function.args.get(args_start as usize).ok_or_else(invalid)?;
		let value = *self
			.slots
			.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
			.ok_or_else(invalid)?;
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		self.slots.get(destination).ok_or_else(invalid)?;
		let value = store.take_capture(value)?;
		self.put(store, destination, value)
	}

	// D21-derived execution logic preserved during extraction: the loops read fixed
	// (slot, flag) pairs from the argument table.
	#[allow(clippy::chunks_exact_to_as_chunks)]
	fn invoke_args<
		#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))] const NATIVE: bool,
	>(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: Args<'_>,
		mut fuel: usize,
		#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
		regions: Option<&super::region::cache::ProgramRegions>,
	) -> core::result::Result<Value, Failure> {
		let mut setup_cleanup_error = None;
		let mut missing_key = None;
		let mut panic_message = None;

		let result = (|| {
			if store.is_closed() {
				return Err(error(ErrorKind::ClosedStore));
			}

			if !self.frames.is_empty() || !self.slots.is_empty() {
				return Err(error(ErrorKind::InvalidCode));
			}

			self.enter(code, store, function, args, None).map_err(|f| {
				setup_cleanup_error = f.cleanup_error;
				f.error
			})?;
			'frames: loop {
				let frame = *self
					.frames
					.last()
					.ok_or_else(|| error(ErrorKind::InvalidCode))?;
				let function = code
					.function(frame.function)
					.ok_or_else(|| error(ErrorKind::InvalidCode))?;
				// Frame metadata and the function are stable until call or return.
				// No borrowed frame/slot reference crosses those mutations.
				let frame_index = self.frames.len() - 1;
				let mut pc = frame.pc;
				// P0 region simulator hook (tests with `native-code` only): never
				// consulted at the pc an exit resumes at.
				#[cfg(all(test, feature = "native-code"))]
				let mut region_resume = false;
				// P2a production tier: this frame's header table (empty in the NATIVE =
				// false instance) and the resume pc of the last region exit, which is never
				// a header consulted again before the next charged instruction.
				#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
				let table: &[(u32, u32)] = match regions {
					Some(regions) if NATIVE => regions.function(frame.function),
					_ => &[],
				};
				#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
				let mut resume_pc = usize::MAX;

				loop {
					let source_pc = pc;
					let instruction = match function.cached_ref(source_pc) {
						Some(instruction) => instruction,
						None => {
							// Validated code cannot reach this, but retain the checked
							// fetch and publish its pc before the outer cleanup.
							self.commit_pc(frame_index, pc)?;
							return Err(error(ErrorKind::InvalidCode));
						}
					};
					#[cfg(all(test, feature = "native-code"))]
					{
						let resume = core::mem::take(&mut region_resume);
						let end = frame.base + function.slots as usize;
						if instruction.charged && !resume && !cancel_hook_armed_for_test() {
							let room = region_room(
								self.frames.len(),
								self.slots.len(),
								end,
								self.limits,
								self.capacities(),
							);
							if let Some(window) = self.slots.get_mut(frame.base..end) {
								if let Some((next, left)) = super::region::hook::try_enter(
									code.id(),
									frame.function,
									source_pc,
									window,
									store,
									self.canceled,
									fuel,
									room,
								) {
									pc = next;
									fuel = left;
									region_resume = true;
									continue;
								}
							}
						}
					}
					// P2a production tier: at a charged instruction it did not just resume
					// at, before that instruction's own cancel and fuel step.
					#[cfg(all(
						feature = "native-code",
						target_arch = "x86_64",
						target_os = "linux"
					))]
					if NATIVE && instruction.charged {
						let resumed = core::mem::replace(&mut resume_pc, usize::MAX) == source_pc;
						#[cfg(test)]
						let armed = cancel_hook_armed_for_test();
						#[cfg(not(test))]
						let armed = false;
						if !resumed && !armed && !table.is_empty() {
							if let Some(ready) = regions.and_then(|r| r.header(table, source_pc)) {
								let end = frame.base + function.slots as usize;
								let room = region_room(
									self.frames.len(),
									self.slots.len(),
									end,
									self.limits,
									self.capacities(),
								);
								if let Some(window) = self.slots.get_mut(frame.base..end) {
									match ready.enter(window, store, self.canceled, fuel, room) {
										Ok(Some((next, left))) => {
											pc = next;
											fuel = left;
											resume_pc = next;
											continue;
										}
										Ok(None) => (),
										Err(e) => {
											// Unreachable by the entry proof (every released
											// value is a scalar in an open, idle store); raised
											// at the header like an instruction error.
											if let Some(current) = self.frames.get_mut(frame_index)
											{
												current.pc = pc;
											}
											return Err(e.at(function.locations[source_pc]));
										}
									}
								}
							}
						}
					}

					let mut transferred = false;
					let result = (|| {
						// Charge and cancellation ride an existing instruction, no Charge opcode.
						if instruction.charged {
							#[cfg(test)]
							if cancel_tick_for_test() {
								self.canceled = true;
							}

							if self.canceled {
								// The pending request is consumed here and only here.
								self.canceled = false;
								return Err(error(ErrorKind::Cancelled));
							}

							fuel = fuel
								.checked_sub(1)
								.ok_or_else(|| error(ErrorKind::FuelExhausted))?;
						}

						pc += 1;
						let get = |n: u32| -> Result<Value> {
							self.slots
								.get(frame.base + n as usize)
								.copied()
								.ok_or_else(|| error(ErrorKind::InvalidCode))
						};

						// D20: operands are bound per arm from the borrowed cached instruction (no
						// whole-Op temporary). Call and CallValue name their call here; the shared
						// Call body runs after the match, so CallValue's checks still precede
						// commit_pc and every retain/reserve/frame publication.
						let mut call = None;
						match instruction.op {
							Op::Template { .. } => {
								self.template_instruction(
									store,
									function,
									instruction,
									frame.base,
								)?;
							}

							Op::StringFrom { out, value } => {
								let value = store.string_from(get(value)?)?;
								self.put(store, frame.base + out as usize, value)?;
							}
							Op::Panic { value } => {
								// Full copy is required language payload, not best-effort evidence.
								panic_message = Some(copy_panic_message(store, get(value)?)?);
								return Err(
									error(ErrorKind::Panic).with_detail(Detail::PanicMessage)
								);
							}

							Op::Print { value } => {
								let value = get(value)?;
								self.print_instruction(store, value)?;
							}

							Op::Variant {
								out,
								tag,
								success,
								payload,
							} => {
								let value =
									store.variant(tag, success, payload.map(get).transpose()?)?;
								self.put(store, frame.base + out as usize, value)?;
							}

							Op::Try { out, value } => {
								let value = get(value)?;
								let (_, success, payload) = store.variant_parts(value)?;

								if success {
									let payload = payload.ok_or_else(|| error(ErrorKind::Type))?;
									store.retain(payload)?;
									self.put(store, frame.base + out as usize, payload)?;
								} else {
									store.retain(value)?;
									let out = frame.out;
									self.commit_pc(frame_index, pc)?;
									self.leave(store)?;
									transferred = true;

									if let Some(out) = out {
										self.put(store, out, value)?;
									} else {
										return Ok(Some(value));
									}
								}
							}

							Op::CallValue {
								out,
								value,
								args_start,
								args_len,
							} => {
								let called = get(value)?;
								match called {
									Value::Function {
										code: owner,
										id: callee,
									} => {
										// Keep the plain-function validation and dispatch path unchanged.
										if owner != code.id() {
											return Err(error(ErrorKind::Type));
										}
										let target = code
											.function(callee)
											.ok_or_else(|| error(ErrorKind::InvalidCode))?;
										if target.parameters != args_len {
											return Err(error(ErrorKind::Type));
										}
										call = Some((out, callee, args_start, args_len));
									}
									Value::Heap(_) => {
										self.call_closure(
											code,
											store,
											function,
											frame.base,
											frame_index,
											pc,
											source_pc,
											out,
											called,
											args_start,
											args_len,
										)
										.map_err(|failure| {
											setup_cleanup_error = failure.cleanup_error;
											failure.error
										})?;
										transferred = true;
									}
									_ => return Err(error(ErrorKind::Type)),
								}
							}

							Op::Range { out, start, end } => {
								let value = store.range_values(get(start)?, get(end)?)?;
								self.put(store, frame.base + out as usize, value)?;
							}

							Op::IterStart { out, value } => {
								let value = store.iterator(get(value)?)?;
								self.put(store, frame.base + out as usize, value)?;
							}

							Op::IterNext {
								out,
								iterator,
								none,
							} => {
								if let Some(value) = store.iterator_next(get(iterator)?)? {
									self.put(store, frame.base + out as usize, value)?;
								} else {
									pc = none as usize;
								}
							}

							Op::String { out, text } => {
								let v = store.string(
									function
										.strings
										.get(text as usize)
										.ok_or_else(|| error(ErrorKind::InvalidCode))?,
								)?;
								self.put(store, frame.base + out as usize, v)?;
							}

							Op::Clone { out, value, mode } => {
								let value = get(value)?;
								if mode == CloneMode::Associated
									&& !matches!(value, Value::Function { .. })
									&& !matches!(
										store.tag(value)?,
										super::value::Type::Vec
											| super::value::Type::Tuple | super::value::Type::Object
											| super::value::Type::String | super::value::Type::Unit
											| super::value::Type::I64 | super::value::Type::U64
											| super::value::Type::F64 | super::value::Type::Option
											| super::value::Type::Result | super::value::Type::Function
									) {
									return Err(error(ErrorKind::Type));
								}
								let v = store.clone_value(value)?;
								self.put(store, frame.base + out as usize, v)?;
							}

							Op::AssignBinary { out, op, lhs, rhs } => {
								// D21: borrow both operand slots (lhs then rhs, the same
								// InvalidCode order as `get`); two I64 values go to fast_i64
								// using their borrowed payloads, every other pair is copied and
								// takes the unchanged fallback. Both borrows end before `put`.
								let base = frame.base;
								let slots = &self.slots;
								let left = slots
									.get(base + lhs as usize)
									.ok_or_else(|| error(ErrorKind::InvalidCode))?;
								let right = slots
									.get(base + rhs as usize)
									.ok_or_else(|| error(ErrorKind::InvalidCode))?;
								let fast = match (left, right) {
									(&Value::I64(a), &Value::I64(b)) => {
										fast_i64(op, Value::I64(a), Value::I64(b))
									}
									_ => None,
								};

								let v = if let Some(result) = fast {
									result?
								} else {
									let (a, b) = (*left, *right);
									if op == arithmetic::Binary::Add
										&& store.tag(a)? == super::value::Type::String
									{
										store.append_string(a, b)?;
										store.retain(a)?;
										a
									} else {
										binary(store, op, a, b)?
									}
								};

								self.put(store, base + out as usize, v)?;
							}

							Op::Literal { out, value } => {
								self.put(store, frame.base + out as usize, value)?
							}

							Op::Copy { out, value } => {
								let v = get(value)?;
								store.retain(v)?;
								self.put(store, frame.base + out as usize, v)?;
							}

							Op::Release { value } => {
								self.put(store, frame.base + value as usize, Value::Unit)?
							}

							Op::Unary { out, op, value } => {
								let v = arithmetic::unary(op, get(value)?)?;
								self.put(store, frame.base + out as usize, v)?;
							}

							Op::Binary { out, op, lhs, rhs } => {
								// D21: as AssignBinary, without the string append arm.
								let base = frame.base;
								let slots = &self.slots;
								let left = slots
									.get(base + lhs as usize)
									.ok_or_else(|| error(ErrorKind::InvalidCode))?;
								let right = slots
									.get(base + rhs as usize)
									.ok_or_else(|| error(ErrorKind::InvalidCode))?;
								let fast = match (left, right) {
									(&Value::I64(a), &Value::I64(b)) => {
										fast_i64(op, Value::I64(a), Value::I64(b))
									}
									_ => None,
								};

								let v = if let Some(result) = fast {
									result?
								} else {
									binary(store, op, *left, *right)?
								};

								self.put(store, base + out as usize, v)?;
							}

							Op::Cast { out, value, tag } => {
								let v = arithmetic::cast(get(value)?, tag)?;
								self.put(store, frame.base + out as usize, v)?;
							}

							Op::Is {
								out,
								value,
								tag,
								negate,
							} => {
								let operand = get(value)?;
								let matches = match operand {
									Value::Function { .. } => tag == super::value::Type::Function,
									Value::Type(_) => false,
									_ => store.tag(operand)? == tag,
								};
								let v = Value::Bool(matches ^ negate);
								self.put(store, frame.base + out as usize, v)?;
							}
							Op::Jump { target } => pc = target as usize,
							Op::Branch { condition, yes, no } => {
								let Value::Bool(condition) = get(condition)? else {
									return Err(error(ErrorKind::Type));
								};

								pc = if condition { yes as usize } else { no as usize };
							}

							Op::Call {
								out,
								function: callee,
								args_start,
								args_len,
							} => call = Some((out, callee, args_start, args_len)),
							Op::Return { value } => {
								let value = get(value)?;
								store.retain(value)?;
								let out = frame.out;
								self.commit_pc(frame_index, pc)?;
								self.leave(store)?;
								transferred = true;

								if let Some(out) = out {
									self.put(store, out, value)?;
								} else {
									return Ok(Some(value));
								}
							}

							Op::Record {
								out,
								tag,
								args_start,
								args_len,
							} => {
								let descriptors = &function.args
									[args_start as usize..(args_start + args_len) as usize];
								self.record_instruction(
									store,
									&function.strings,
									descriptors,
									frame.base,
									out,
									tag,
								)?;
							}

							Op::Field { out, value, key } => {
								let value = store
									.field(get(value)?, function.strings[key as usize].as_str())?;
								self.put(store, frame.base + out as usize, value)?;
							}

							Op::FieldGetMut { out, value, key } => {
								let value = store.field_get_mut(
									get(value)?,
									function.strings[key as usize].as_str(),
								)?;
								self.put(store, frame.base + out as usize, value)?;
							}
							Op::FieldSetMut { value, key, new } => store.field_set_mut(
								get(value)?,
								function.strings[key as usize].as_str(),
								get(new)?,
							)?,
							Op::SetField { value, key, new } => store.set_field(
								get(value)?,
								function.strings[key as usize].as_str(),
								get(new)?,
							)?,
							Op::List {
								out,
								tag,
								args_start,
								args_len,
							} => {
								// No temporary proportional argument allocation: the slots themselves
								// are not necessarily contiguous, so reserve only the bounded list.
								let indices = &function.args
									[args_start as usize..args_start as usize + args_len as usize];
								let mut values = Vec::try_with_capacity(indices.len())?;

								for &i in indices {
									values.try_push(get(i)?)?;
								}

								let value = store.list(tag, &values)?;
								self.put(store, frame.base + out as usize, value)?;
							}

							Op::TupleFieldGetMut { out, value, index } => {
								let v =
									store.tuple_get_mut(get(value)?, index_value(get(index)?)?)?;
								self.put(store, frame.base + out as usize, v)?;
							}
							Op::TupleFieldSetMut { value, index, new } => store.tuple_set_mut(
								get(value)?,
								index_value(get(index)?)?,
								get(new)?,
							)?,
							Op::TupleField { out, value, index } => {
								let v = store.tuple_get(get(value)?, index_value(get(index)?)?)?;
								self.put(store, frame.base + out as usize, v)?;
							}
							Op::SetTupleField { value, index, new } => store.tuple_set(
								get(value)?,
								index_value(get(index)?)?,
								get(new)?,
							)?,
							Op::Get { out, value, index } => {
								let v = store.dynamic_get(get(value)?, get(index)?)?;
								self.put(store, frame.base + out as usize, v)?;
							}
							Op::Set { value, index, new } => {
								store.dynamic_set(get(value)?, get(index)?, get(new)?)?
							}

							Op::Len { out, value } => {
								if !matches!(
									store.tag(get(value)?)?,
									super::value::Type::Vec
										| super::value::Type::Tuple | super::value::Type::String
										| super::value::Type::Object | super::value::Type::Unit
								) {
									return Err(error(ErrorKind::Type));
								}

								let n = u64::try_from(store.len(get(value)?)?)
									.map_err(|_| error(ErrorKind::Overflow))?;
								self.put(store, frame.base + out as usize, Value::U64(n))?;
							}
							Op::Push { value, new } => {
								let value = get(value)?;
								if !matches!(value, Value::Heap(_))
									|| !matches!(
										store.tag(value)?,
										super::value::Type::Vec | super::value::Type::String
									) {
									return Err(error(ErrorKind::Type));
								}
								store.push(value, get(new)?)?;
							}
							Op::Drop { value } => store.invalidate(get(value)?)?,
						}

						if let Some((out, callee, args_start, args_len)) = call {
							self.commit_pc(frame_index, pc)?;
							match code.function(callee) {
								Some(callee_code) => {
									let source_args = &function.args[args_start as usize
										..args_start as usize + args_len as usize];
									self.push_call(
										store,
										callee_code.slots as usize,
										Args::Slots {
											base: frame.base,
											indices: source_args,
										},
										Frame {
											function: callee,
											pc: 0,
											base: 0,
											out: Some(frame.base + out as usize),
											callsite_pc: source_pc as u32,
										},
									)
									.map_err(|f| {
										setup_cleanup_error = f.cleanup_error;
										f.error
									})?;
									transferred = true;
								}
								None => self.intrinsic(
									code, store, function, frame.base, out, callee, args_start,
									args_len,
								)?,
							}
						}

						Ok(None)
					})();
					match result {
						Ok(Some(v)) => return Ok(v),
						Ok(None) if transferred => continue 'frames,
						Ok(None) => (),
						Err(e) => {
							// A failed return can already have popped this frame.
							// Never overwrite the parent's pc in that case. A failed
							// call can have reserved slots without publishing a frame.
							if let Some(current) = self.frames.get_mut(frame_index) {
								current.pc = pc;
							}

							let e = enrich_error(
								e,
								store,
								&instruction.op,
								code,
								frame.function,
								&self.slots,
								frame.base,
							);
							if matches!(e.detail, Detail::MissingKey { .. }) {
								if let Op::Get { index, .. } | Op::Set { index, .. } =
									instruction.op
								{
									if let Some(value) = self.slots.get(frame.base + index as usize)
									{
										#[cfg(test)]
										let fail = core::mem::take(&mut self.test_fail_key_copy);
										#[cfg(not(test))]
										let fail = false;
										missing_key = if fail {
											#[cfg(test)]
											{
												rune_alloc::limit::with(0, || {
													copy_missing_key(store, *value)
												})
												.call()
											}
											#[cfg(not(test))]
											{
												None
											}
										} else {
											copy_missing_key(store, *value)
										};
									}
								}
							}
							return Err(e.at(function.locations[source_pc]));
						}
					}
				}
			}
		})();
		// P1 tests only: the remaining fuel, compared across engines.
		#[cfg(all(test, feature = "native-code"))]
		super::region::hook::record_final_fuel(fuel);

		match result {
			Ok(value) => Ok(value),
			Err(mut failure) => {
				let numeric_context =
					invocation_numeric_context(&failure, code, store, &self.frames, &self.slots);
				self.resolve_failure_location(code, &mut failure);
				let mut cleanup_error = setup_cleanup_error.or(self.pending_host_cleanup.take());
				let mut host_message = self.pending_host_message.take();
				if failure.kind == ErrorKind::Panic
					&& failure.detail == Detail::PanicMessage
					&& panic_message.is_none()
				{
					panic_message = host_message.take();
				}

				self.collect_failure_chain(code);

				// Includes a partially filled callee reserve if retaining an arg failed.
				while let Some(value) = self.slots.pop() {
					if let Err(e) = store.release(value) {
						cleanup_error.get_or_insert(e);
					}
				}

				self.frames.clear();
				Err(Failure {
					numeric_context,
					error: failure,
					missing_key,
					panic_message,
					host_message,
					call_chain: core::mem::take(&mut self.diagnostics),
					cleanup_error,
				})
			}
		}
	}
	/// P2a: one invocation with this thread's loaded regions of `code`, if any. The regions
	/// are checked out of the thread-local cache for the invocation and put back after it
	/// (also on unwind); a code with no loaded region runs the `NATIVE = false` loop, whose
	/// body is the feature-off loop.
	#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
	fn invoke_native(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: Args<'_>,
		fuel: usize,
	) -> core::result::Result<Value, Failure> {
		use super::region::cache::{self, Checkout};

		if code.id() == self.no_region_code {
			return self.invoke_args::<false>(code, store, function, args, fuel, None);
		}

		match cache::checkout(code) {
			Checkout::Native(guard) => {
				let result =
					self.invoke_args::<true>(code, store, function, args, fuel, guard.regions());
				drop(guard);
				result
			}
			Checkout::Interpret { no_regions } => {
				if no_regions {
					self.no_region_code = code.id();
				}
				self.invoke_args::<false>(code, store, function, args, fuel, None)
			}
		}
	}

	/// Clean-state check for callers that repeat invocations: true when no
	/// frame, slot or pending diagnostic survives the last invocation.
	pub fn is_idle(&self) -> bool {
		self.frames.is_empty() && self.slots.is_empty() && self.diagnostics.is_empty()
	}

	#[cfg(any(
		test,
		all(feature = "native-code", target_arch = "x86_64", target_os = "linux")
	))]
	fn capacities(&self) -> (usize, usize, usize) {
		(
			self.frames.capacity(),
			self.slots.capacity(),
			self.diagnostics.capacity(),
		)
	}

	/// Validate and enter a capturing closure on the cold path. The ordinary
	/// `CallValue` arm and its shared call setup retain their original shape.
	#[cold]
	#[inline(never)]
	fn call_closure(
		&mut self,
		code: &super::code::Code,
		store: &mut Store,
		function: &super::code::Function,
		base: usize,
		frame_index: usize,
		pc: usize,
		source_pc: usize,
		out: u32,
		closure: Value,
		args_start: u32,
		args_len: u32,
	) -> core::result::Result<(), CallSetupFailure> {
		let (owner, callee, captures) = store.closure_parts(closure)?;
		if owner != code.id() {
			return Err(error(ErrorKind::Type).into());
		}
		let target = code
			.function(callee)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		let capture_len = captures.len();
		let capture_count =
			u32::try_from(capture_len).map_err(|_| error(ErrorKind::InvalidCode))?;
		let expected = capture_count
			.checked_add(args_len)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		if target.parameters != expected {
			return Err(error(ErrorKind::Type).into());
		}
		let end = args_start
			.checked_add(args_len)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		let indices = function
			.args
			.get(args_start as usize..end as usize)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;

		self.commit_pc(frame_index, pc)
			.map_err(CallSetupFailure::from)?;
		self.push_closure_call(
			store,
			target.slots as usize,
			closure,
			capture_len,
			base,
			indices,
			Frame {
				function: callee,
				pc: 0,
				base: 0,
				out: Some(base + out as usize),
				callsite_pc: source_pc as u32,
			},
		)
	}

	/// Rollback covers only this helper's unpublished window, preserving the
	/// caller and its diagnostic witnesses for the existing outer unwind.
	fn push_call(
		&mut self,
		store: &mut Store,
		slots: usize,
		args: Args<'_>,
		mut frame: Frame,
	) -> core::result::Result<(), CallSetupFailure> {
		let frames = self
			.frames
			.len()
			.checked_add(1)
			.ok_or_else(|| error(ErrorKind::FrameCapacity))?;
		let base = self.slots.len();
		let len = base
			.checked_add(slots)
			.ok_or_else(|| error(ErrorKind::FrameCapacity))?;
		if frames > self.limits.frames || len > self.limits.slots {
			return Err(error(ErrorKind::FrameCapacity).into());
		}
		if args.len() > slots {
			return Err(error(ErrorKind::InvalidCode).into());
		}
		// Reserve by additional length only when a physical buffer needs growth.
		// No borrowed slot survives growth; keep the fallible reserve order exact.
		if self.frames.len() >= self.frames.capacity()
			|| self.slots.capacity() < len
			|| self.diagnostics.capacity() < frames
		{
			self.frames
				.try_reserve(frames - self.frames.len())
				.map_err(Error::from)?;
			self.slots.try_reserve(len - base).map_err(Error::from)?;
			self.diagnostics
				.try_reserve(frames.saturating_sub(self.diagnostics.len()))
				.map_err(Error::from)?;
		}
		debug_assert!(self.diagnostics.capacity() >= frames);
		for i in 0..args.len() {
			let value = args.get(&self.slots, i);
			if let Err(error) = store.retain(value) {
				return Err(self.rollback_call(store, base, i, error));
			}
			if self.slots.push_within_capacity(value).is_err() {
				// Reservation makes this unreachable, but the unpublished owner
				// still precedes the published prefix in reverse-order cleanup.
				let cleanup_error = store.release(value).err();
				let mut failure = self.rollback_call(
					store,
					base,
					i,
					Error::new(ErrorKind::InvalidCode, "argument publication"),
				);
				failure.cleanup_error = cleanup_error.or(failure.cleanup_error);
				return Err(failure);
			}
		}
		// Published arguments already hold retained values. Only the unused
		// tail needs Unit initialization, without allocating after reservation.
		if let Err(error) = self.slots.try_resize_with(len, || Value::Unit) {
			return Err(self.rollback_call(store, base, args.len(), Error::from(error)));
		}
		frame.base = base;
		if self.frames.push_within_capacity(frame).is_err() {
			return Err(self.rollback_call(
				store,
				base,
				args.len(),
				Error::new(ErrorKind::InvalidCode, "frame publication"),
			));
		}
		debug_assert!(self.diagnostics.capacity() >= self.frames.len());
		Ok(())
	}

	/// Closure argument setup is deliberately cold: ordinary calls keep the
	/// infallible `Args::get` loop above, with no store lookup or closure branch.
	#[cold]
	#[inline(never)]
	fn push_closure_call(
		&mut self,
		store: &mut Store,
		slots: usize,
		closure: Value,
		capture_count: usize,
		base: usize,
		indices: &[u32],
		mut frame: Frame,
	) -> core::result::Result<(), CallSetupFailure> {
		let arg_count = capture_count
			.checked_add(indices.len())
			.ok_or_else(|| error(ErrorKind::FrameCapacity))?;
		let frames = self
			.frames
			.len()
			.checked_add(1)
			.ok_or_else(|| error(ErrorKind::FrameCapacity))?;
		let call_base = self.slots.len();
		let len = call_base
			.checked_add(slots)
			.ok_or_else(|| error(ErrorKind::FrameCapacity))?;
		if frames > self.limits.frames || len > self.limits.slots {
			return Err(error(ErrorKind::FrameCapacity).into());
		}
		if arg_count > slots {
			return Err(error(ErrorKind::InvalidCode).into());
		}
		if self.frames.len() >= self.frames.capacity()
			|| self.slots.capacity() < len
			|| self.diagnostics.capacity() < frames
		{
			self.frames
				.try_reserve(frames - self.frames.len())
				.map_err(Error::from)?;
			self.slots
				.try_reserve(len - call_base)
				.map_err(Error::from)?;
			self.diagnostics
				.try_reserve(frames.saturating_sub(self.diagnostics.len()))
				.map_err(Error::from)?;
		}
		debug_assert!(self.diagnostics.capacity() >= frames);
		for i in 0..arg_count {
			let value = if i < capture_count {
				match store.closure_capture(closure, i) {
					Ok(value) => value,
					Err(error) => return Err(self.rollback_call(store, call_base, i, error)),
				}
			} else {
				self.slots[base + indices[i - capture_count] as usize]
			};
			if let Err(error) = store.retain(value) {
				return Err(self.rollback_call(store, call_base, i, error));
			}
			if self.slots.push_within_capacity(value).is_err() {
				let cleanup_error = store.release(value).err();
				let mut failure = self.rollback_call(
					store,
					call_base,
					i,
					Error::new(ErrorKind::InvalidCode, "argument publication"),
				);
				failure.cleanup_error = cleanup_error.or(failure.cleanup_error);
				return Err(failure);
			}
		}
		if let Err(error) = self.slots.try_resize_with(len, || Value::Unit) {
			return Err(self.rollback_call(store, call_base, arg_count, Error::from(error)));
		}
		frame.base = call_base;
		if self.frames.push_within_capacity(frame).is_err() {
			return Err(self.rollback_call(
				store,
				call_base,
				arg_count,
				Error::new(ErrorKind::InvalidCode, "frame publication"),
			));
		}
		debug_assert!(self.diagnostics.capacity() >= self.frames.len());
		Ok(())
	}

	fn rollback_call(
		&mut self,
		store: &mut Store,
		base: usize,
		retained: usize,
		error: Error,
	) -> CallSetupFailure {
		let mut cleanup_error = None;
		for j in (base..base + retained).rev() {
			if let Err(e) = store.release(self.slots[j]) {
				cleanup_error.get_or_insert(e);
			}
		}
		#[cfg(test)]
		if let Some(e) = self.rollback_error.take() {
			cleanup_error.get_or_insert(e);
		}
		self.slots.truncate(base);
		CallSetupFailure {
			error,
			cleanup_error,
		}
	}

	fn enter(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: Args<'_>,
		out: Option<usize>,
	) -> core::result::Result<(), CallSetupFailure> {
		let f = code
			.function(function)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		if args.len() != f.parameters as usize {
			return Err(error(ErrorKind::InvalidCode).into());
		}

		// Host roots: every one owner-checked before any retain, no allocation.
		if let Args::Roots(roots) = args {
			store.import_all(roots.iter().copied())?;
		}

		self.push_call(
			store,
			f.slots as usize,
			args,
			Frame {
				function,
				pc: 0,
				base: 0,
				out,
				callsite_pc: NO_CALLSITE,
			},
		)
	}

	/// Publish only into the frame whose local pc is being executed.
	fn commit_pc(&mut self, frame_index: usize, pc: usize) -> Result<()> {
		self.frames
			.get_mut(frame_index)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?
			.pc = pc;
		Ok(())
	}

	#[inline(always)]
	fn put(&mut self, store: &mut Store, index: usize, value: Value) -> Result<()> {
		let slot = self
			.slots
			.get_mut(index)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		let old = core::mem::replace(slot, value);
		store.release_slot(old)
	}

	fn leave(&mut self, store: &mut Store) -> Result<()> {
		let frame = self
			.frames
			.pop()
			.ok_or_else(|| error(ErrorKind::InvalidCode))?;
		// D13: an all-scalar window in an open store with no queued work would
		// release nothing slot by slot; drop it in one step. Otherwise (any
		// heap value, closed store, queued work, base beyond len) the D4 loop.
		if store.scalar_release_is_noop()
			&& self
				.slots
				.get(frame.base..)
				.is_some_and(|window| window.iter().all(Store::is_slot_scalar))
		{
			#[cfg(test)]
			LEAVE_FAST_HITS.with(|c| c.set(c.get() + 1));
			self.slots.truncate(frame.base);
			return Ok(());
		}

		while self.slots.len() > frame.base {
			store.release_slot(
				self.slots
					.pop()
					.ok_or_else(|| error(ErrorKind::InvalidCode))?,
			)?;
		}

		Ok(())
	}
}

#[inline(never)]
fn copy_panic_message(store: &Store, value: Value) -> Result<Box<str>> {
	Ok(Box::<str>::try_from(store.text(value)?)?)
}

// Called only after an instruction fails, while its operand slots and constant pool
// still exist. Diagnostic lookup is best-effort: it never replaces the primary error.
fn copy_missing_key(store: &Store, value: Value) -> Option<Box<str>> {
	Box::<str>::try_from(store.text(value).ok()?).ok()
}

// Prove only the Vec IndexSet native argument wrapper on failure.
#[cold]
#[inline(never)]
fn moved_set_is_vector(store: &Store, value: Value) -> bool {
	matches!(store.tag(value), Ok(super::value::Type::Vec))
}

fn enrich_error(
	mut error: Error,
	store: &Store,
	op: &Op,
	code: &Code,
	function_id: u32,
	slots: &[Value],
	base: usize,
) -> Error {
	if let (Op::Set { value, .. }, Detail::Moved { access }) = (op, error.detail) {
		if let Some(&value) = slots.get(base + *value as usize) {
			if moved_set_is_vector(store, value) {
				error.detail = Detail::MovedArgument { access, arg: 0 };
			}
		}
		return error;
	}
	if let (Op::IterStart { .. } | Op::IterNext { .. }, Detail::Moved { access }) =
		(op, error.detail)
	{
		error.detail = Detail::IteratorArgumentMoved { access, arg: 0 };
		return error;
	}
	if matches!(error.detail, Detail::ReadExclusive | Detail::WriteExclusive)
		&& matches!(
			op,
			Op::Len { .. }
				| Op::Clone { .. }
				| Op::Push { .. }
				| Op::Set { .. }
				| Op::IterStart { .. }
				| Op::Template { .. }
		) {
		let access = if error.detail == Detail::ReadExclusive {
			super::error::MovedAccess::Read
		} else {
			super::error::MovedAccess::Write
		};
		error.detail = Detail::ExclusiveArgument { access, arg: 0 };
		return error;
	}
	if let Op::AssignBinary { lhs, .. } | Op::Push { value: lhs, .. } | Op::Set { value: lhs, .. } =
		*op
	{
		if error.detail == Detail::WriteBorrowed {
			if let Some(readers) = slots
				.get(base + lhs as usize)
				.and_then(|v| store.shared_readers(*v))
			{
				error.detail = Detail::SharedArgument { arg: 0, readers };
				return error;
			}
		}
	}
	if let Op::AssignBinary { op, .. } = *op {
		if let Detail::Binary { lhs, rhs, .. } = error.detail {
			error.detail = Detail::Binary {
				op: BinarySymbol::from_op(op, true),
				lhs,
				rhs,
			};
			return error;
		}
	}
	if error.detail != Detail::None
		|| !matches!(
			error.kind,
			ErrorKind::Type | ErrorKind::IndexOutOfBounds | ErrorKind::InvalidCast
		) {
		return error;
	}
	let detail =
		(|| -> Option<Detail> {
			let get = |n: u32| slots.get(base.checked_add(n as usize)?).copied();
			let ty = |n| store.diagnostic_type(get(n)?);
			let field = |key| FieldRef {
				code: code.id(),
				function: function_id,
				string: key,
			};
			let bounds = |value, index| {
				Some(Detail::OutOfRange {
					index: index_value(get(index)?).ok()?,
					length: store.len(get(value)?).ok()?,
				})
			};
			Some(match *op {
				Op::Binary { op, lhs, rhs, .. } | Op::AssignBinary { op, lhs, rhs, .. } => {
					let (lhs, rhs) = (ty(lhs)?, ty(rhs)?);
					if lhs == TypeName::String
						&& rhs != TypeName::String
						&& matches!(
							op,
							arithmetic::Binary::Add
								| arithmetic::Binary::Eq | arithmetic::Binary::Ne
								| arithmetic::Binary::Lt | arithmetic::Binary::Le
								| arithmetic::Binary::Gt | arithmetic::Binary::Ge
						) {
						Detail::NumericArgumentType {
							cause: super::error::HostCause::ExpectedType,
							arg: 1,
							expected: TypeName::String,
							found: rhs,
						}
					} else {
						Detail::Binary {
							op: BinarySymbol::from_op(op, false),
							lhs,
							rhs,
						}
					}
				}
				Op::Unary { op, value, .. } => Detail::Unary {
					op: match op {
						arithmetic::Unary::Neg => UnarySymbol::Neg,
						arithmetic::Unary::Not => UnarySymbol::Not,
					},
					operand: ty(value)?,
				},
				Op::Cast { value, tag, .. } => Detail::Cast {
					found: ty(value)?,
					to_numeric: matches!(
						tag,
						super::value::Type::I64 | super::value::Type::U64 | super::value::Type::F64
					),
				},
				Op::StringFrom { value, .. } | Op::Panic { value } => Detail::ExpectedType {
					expected: TypeName::String,
					found: ty(value)?,
				},
				Op::Branch { condition, .. } => Detail::ExpectedBool {
					found: ty(condition)?,
				},
				Op::CallValue {
					value, args_len, ..
				} => match get(value)? {
					Value::Function { code: owner, .. } if owner != code.id() => {
						Detail::ForeignFunction
					}
					Value::Function { id, .. } => Detail::Arity {
						given: args_len,
						expected: code.function(id)?.parameters,
					},
					Value::Heap(value) => match store.closure_parts(Value::Heap(value)).ok() {
						Some((owner, _, _)) if owner != code.id() => Detail::ForeignFunction,
						Some((_, id, captures)) => {
							let captured = u32::try_from(captures.len()).ok()?;
							let expected = code.function(id)?.parameters.checked_sub(captured)?;
							Detail::Arity {
								given: args_len,
								expected,
							}
						}
						None => Detail::NotCallable {
							found: store.diagnostic_type(Value::Heap(value))?,
						},
					},
					_ => Detail::NotCallable { found: ty(value)? },
				},
				Op::Range { start, end, .. } => {
					let bad = if matches!(get(start)?, Value::I64(_)) {
						end
					} else {
						start
					};
					Detail::ExpectedNumber { found: ty(bad)? }
				}
				Op::IterStart { value, .. } => {
					let target = ty(value)?;
					let instance = match target {
						TypeName::Struct(n) => super::nominal::hash_copy(n).map(crate::Hash::new),
						_ => target.builtin_hash(),
					};
					let Some(instance) = instance else {
						error.kind = ErrorKind::InvalidCode;
						return None;
					};
					Detail::MissingInstanceFunction {
						target,
						instance,
						hash: crate::Hash::associated_function(
							instance,
							&crate::runtime::Protocol::INTO_ITER,
						),
					}
				}
				Op::Try { value, .. } => Detail::TryOperand { found: ty(value)? },
				Op::Clone {
					value,
					mode: CloneMode::Associated,
					..
				}
				| Op::Len { value, .. }
				| Op::Push { value, .. } => {
					let method = match *op {
						Op::Clone { .. } => AdmittedMethod::Clone,
						Op::Len { .. } => AdmittedMethod::Len,
						_ => AdmittedMethod::Push,
					};
					let target = ty(value)?;
					let instance = match target {
						TypeName::Struct(n) => super::nominal::hash_copy(n).map(crate::Hash::new),
						_ => target.builtin_hash(),
					};
					let Some(instance) = instance else {
						error.kind = ErrorKind::InvalidCode;
						return None;
					};
					Detail::MissingInstanceFunction {
						target,
						instance,
						hash: crate::Hash::associated_function(instance, method.hash()),
					}
				}
				Op::Field { value, key, .. } | Op::FieldGetMut { value, key, .. } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						Detail::MissingField {
							target: ty(value)?,
							field: field(key),
						}
					} else {
						Detail::FieldOnNonRecord {
							target: ty(value)?,
							field: field(key),
						}
					}
				}
				Op::SetField { value, key, .. } | Op::FieldSetMut { value, key, .. } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						Detail::MissingField {
							target: ty(value)?,
							field: field(key),
						}
					} else {
						Detail::FieldSetUnsupported {
							target: ty(value)?,
							field: field(key),
						}
					}
				}
				Op::TupleField { value, index, .. } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						return bounds(value, index);
					}
					let index = index_value(get(index)?).ok()?;
					Detail::TupleIndexGet {
						target: ty(value)?,
						index,
					}
				}
				Op::SetTupleField { value, index, .. } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						return bounds(value, index);
					}
					Detail::TupleIndexSet { target: ty(value)? }
				}
				Op::Get { value, index, .. } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						return bounds(value, index);
					}
					let target = ty(value)?;
					if matches!(target, TypeName::Vec | TypeName::Tuple) {
						Detail::ExpectedNumber { found: ty(index)? }
					} else {
						Detail::IndexTarget {
							target,
							index: ty(index)?,
						}
					}
				}
				Op::Set { value, index, new } => {
					if error.kind == ErrorKind::IndexOutOfBounds {
						return bounds(value, index);
					}
					if !matches!(get(index)?, Value::I64(_) | Value::U64(_)) {
						Detail::ExpectedNumber { found: ty(index)? }
					} else {
						Detail::IndexSet {
							target: ty(value)?,
							index: ty(index)?,
							value: ty(new)?,
						}
					}
				}
				_ => return None,
			})
		})();
	if let Some(mut detail) = detail {
		if let (Op::AssignBinary { op, .. }, Detail::Binary { lhs, rhs, .. }) = (*op, detail) {
			detail = Detail::Binary {
				op: BinarySymbol::from_op(op, true),
				lhs,
				rhs,
			};
		}
		error.detail = detail;
	}
	if matches!(error.detail, Detail::Arity { .. }) {
		error.kind = ErrorKind::ArgumentCount;
	}
	error
}

pub(super) fn index_value(value: Value) -> Result<usize> {
	match value {
		Value::I64(n) => usize::try_from(n).map_err(|_| {
			error(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce {
				value: IntValue::of(value).unwrap(),
				to: IntTarget::Usize,
			})
		}),
		Value::U64(n) => usize::try_from(n).map_err(|_| {
			error(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce {
				value: IntValue::of(value).unwrap(),
				to: IntTarget::Usize,
			})
		}),
		_ => Err(error(ErrorKind::Type)),
	}
}

/// Exact signed-integer cases only; all other values/operations keep the
/// original string/coercion dispatch. No store access before result publication.
#[inline(always)]
fn fast_i64(op: arithmetic::Binary, a: Value, b: Value) -> Option<Result<Value>> {
	use arithmetic::Binary;
	let (Value::I64(a), Value::I64(b)) = (a, b) else {
		return None;
	};

	Some(match op {
		Binary::Add => a
			.checked_add(b)
			.map(Value::I64)
			.ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
		Binary::Sub => a.checked_sub(b).map(Value::I64).ok_or_else(|| {
			Error::new(ErrorKind::Overflow, "primitive operation").with_detail(Detail::Underflow)
		}),
		Binary::Mul => a
			.checked_mul(b)
			.map(Value::I64)
			.ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
		Binary::Div | Binary::Rem if b == 0 => {
			Err(Error::new(ErrorKind::DivisionByZero, "primitive operation"))
		}

		Binary::Div => a
			.checked_div(b)
			.map(Value::I64)
			.ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
		Binary::Rem => a
			.checked_rem(b)
			.map(Value::I64)
			.ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
		Binary::Eq => Ok(Value::Bool(a == b)),
		Binary::Ne => Ok(Value::Bool(a != b)),
		Binary::Lt => Ok(Value::Bool(a < b)),
		Binary::Le => Ok(Value::Bool(a <= b)),
		Binary::Gt => Ok(Value::Bool(a > b)),
		Binary::Ge => Ok(Value::Bool(a >= b)),
		Binary::BitAnd | Binary::BitOr | Binary::BitXor | Binary::Shl | Binary::Shr => return None,
	})
}

// D21-derived execution logic preserved during extraction: the string-pair branch keeps
// its heap-tag checks separate from the outer heap-pair test.
#[allow(clippy::collapsible_if)]
fn binary(store: &mut Store, op: arithmetic::Binary, a: Value, b: Value) -> Result<Value> {
	if let (Value::Heap(left), Value::Heap(right)) = (a, b) {
		if store.binary_heap_tag(left)? == super::value::Type::String
			&& store.binary_heap_tag(right)? == super::value::Type::String
		{
			if op == arithmetic::Binary::Add {
				return store.concat_string(a, b);
			}

			let left = store.text(a)?;
			let right = store.text(b)?;
			return arithmetic::compare(op, left == right, left < right, left > right);
		}
	}

	if matches!(
		op,
		arithmetic::Binary::Eq
			| arithmetic::Binary::Ne
			| arithmetic::Binary::Lt
			| arithmetic::Binary::Le
			| arithmetic::Binary::Gt
			| arithmetic::Binary::Ge
	) {
		return store.compare(op, a, b);
	}
	arithmetic::binary(op, a, b)
}

#[cfg(test)]
// `Failure` carries a located error and its call chain by value; it is the public error
// type of `invoke`, so boxing it would change the API.
#[allow(clippy::result_large_err)]
impl Executor {
	/// Actual (frames, slots, diagnostics) capacities at this point.
	pub(crate) fn capacities_for_test(&self) -> (usize, usize, usize) {
		(
			self.frames.capacity(),
			self.slots.capacity(),
			self.diagnostics.capacity(),
		)
	}

	/// Byte sizes of the three construction reservations, in reservation order.
	pub(crate) fn reservation_bytes_for_test() -> [usize; 3] {
		[
			INITIAL_FRAMES * core::mem::size_of::<Frame>(),
			INITIAL_SLOTS * core::mem::size_of::<Value>(),
			INITIAL_FRAMES * core::mem::size_of::<Location>(),
		]
	}
	/// The crate's own tests' direct entry: `invoke_args` with the feature off, and with the
	/// P2a tier the same checkout path as `invoke_code` (the tier is off in tests unless a
	/// test turns it on).
	fn invoke_args_for_test(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: Args<'_>,
		fuel: usize,
	) -> core::result::Result<Value, Failure> {
		#[cfg(not(all(feature = "native-code", target_arch = "x86_64", target_os = "linux")))]
		return self.invoke_args(code, store, function, args, fuel);
		#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
		return self.invoke_native(code, store, function, args, fuel);
	}

	/// Owner-free single-shot entry for the crate's own tests.
	pub(crate) fn run_values(
		mut self,
		code: &Code,
		store: &mut Store,
		function: u32,
		args: &[Value],
		fuel: usize,
	) -> core::result::Result<Value, Failure> {
		#[cfg(not(all(feature = "native-code", target_arch = "x86_64", target_os = "linux")))]
		return self.invoke_args(code, store, function, Args::Values(args), fuel);
		#[cfg(all(feature = "native-code", target_arch = "x86_64", target_os = "linux"))]
		return self.invoke_native(code, store, function, Args::Values(args), fuel);
	}
}

#[cfg(test)]
mod scalar_release_tests;

#[cfg(test)]
mod i64_fast_tests;

#[cfg(test)]
mod unit_retain_tests;

#[cfg(test)]
std::thread_local! {
	/// D13: per-thread count of leaves that took the bulk scalar fast path.
	static LEAVE_FAST_HITS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn leave_fast_hits_for_test() -> usize {
	LEAVE_FAST_HITS.with(|c| c.get())
}

#[cfg(test)]
std::thread_local! {
	/// D14 test-only: request cancellation at the Nth charged instruction of
	/// this thread's next invocation(s), so a nested charged frame can be
	/// canceled; there is no other way to cancel mid-invocation.
	static CANCEL_AT_CHARGE: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn cancel_at_charge_for_test(n: Option<usize>) {
	CANCEL_AT_CHARGE.with(|c| c.set(n));
}
/// Whether the D14 cancel-at-charge test hook is armed: a region would skip its ticks, so
/// the region hook stays out while it is.
#[cfg(all(test, feature = "native-code"))]
pub(crate) fn cancel_hook_armed_for_test() -> bool {
	CANCEL_AT_CHARGE.with(|c| c.get().is_some())
}

#[cfg(test)]
fn cancel_tick_for_test() -> bool {
	CANCEL_AT_CHARGE.with(|c| match c.get() {
		Some(0) => {
			c.set(None);
			true
		}

		Some(n) => {
			c.set(Some(n - 1));
			false
		}
		None => false,
	})
}

#[cfg(test)]
mod leave_fast_tests;

#[cfg(test)]
mod lazy_callsite_tests;

#[cfg(test)]
mod template_tests;

#[cfg(test)]
mod call_value_tests;

#[cfg(test)]
mod binary_peek_tests;

#[cfg(test)]
mod diagnostic_enrichment_tests {
	use super::super::{code::Builder, heap::Limits};
	use super::*;
	#[test]
	fn secondary_lookup_failure_preserves_the_primary_error() {
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 4096,
		})
		.unwrap();
		let stale = store.string("stale").unwrap();
		store.release(stale).unwrap();
		let code = Builder::default().finish().unwrap();
		let op = Op::Binary {
			out: 2,
			op: arithmetic::Binary::Add,
			lhs: 0,
			rhs: 1,
		};
		let slots = [stale, Value::I64(1), Value::Unit];
		for kind in [
			ErrorKind::Type,
			ErrorKind::InvalidHandle,
			ErrorKind::Allocation,
			ErrorKind::StoreCapacity,
			ErrorKind::Cancelled,
		] {
			let error = Error::new(kind, "original operation").at(Location {
				source: 9,
				revision: 81,
				start: 3,
				end: 7,
			});
			assert_eq!(enrich_error(error, &store, &op, &code, 0, &slots, 0), error);
		}
	}
}

#[cfg(test)]
mod growth_tests;

#[cfg(test)]
mod key_diagnostic_tests {
	use super::*;
	#[test]
	fn missing_key_copy_is_full_owned_and_allocation_failure_is_best_effort() {
		let mut store = Store::new(super::super::heap::Limits::default()).unwrap();
		let text = std::format!("{}終", "long-key".repeat(256));
		let key = store.string(&text).unwrap();
		let copy = copy_missing_key(&store, key).unwrap();
		assert_eq!(copy.as_ref(), text);
		assert!(rune_alloc::limit::with(0, || copy_missing_key(&store, key))
			.call()
			.is_none());
		store.release(key).unwrap();
		assert_eq!(copy.as_ref(), text);
	}
}

#[cfg(test)]
mod field_tests;

#[cfg(test)]
mod assertion_tests;

#[cfg(test)]
mod d1c_controls;

#[cfg(test)]
mod print_intrinsic_tests;

#[cfg(test)]
mod variant_intrinsic_tests;

mod host_bridge;
mod missing_methods;

mod diagnostic_locations;
mod iter_terminals;
mod number_methods;
mod numeric;
mod object_methods;
mod small_forms;
#[cfg(test)]
mod small_forms_tests;
mod sorting;
#[cfg(test)]
mod sorting_tests;
mod text_methods;
mod variant_methods;
mod vec_constructors;
mod vec_resize;

#[cfg(test)]
mod numeric_tests;

#[cfg(test)]
mod numeric_frontier_tests;

impl Executor {
	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	fn any_value_call(
		&mut self,
		store: &mut Store,
		f: &super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		len: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let end = start.checked_add(len).ok_or_else(invalid)?;
		let indices = f
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		if out >= f.slots {
			return Err(invalid());
		}
		let get = |i: usize| -> Result<Value> {
			let slot = *indices.get(i).ok_or_else(invalid)?;
			if slot >= f.slots {
				return Err(invalid());
			}
			self.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.copied()
				.ok_or_else(invalid)
		};
		let value = get(0)?;
		let result = if id == super::code::ANY_TYPE_OF {
			if len != 2 {
				return Err(invalid());
			}
			let tag = match value {
				Value::Type(_) => {
					let Value::Type(t) = get(1)? else {
						return Err(invalid());
					};
					t
				}
				Value::Function { .. } => super::value::Type::Function,
				_ => store.tag(value)?,
			};
			Value::Type(tag)
		} else {
			if len != 1 {
				return Err(invalid());
			}
			let ty = store.diagnostic_type(value).ok_or_else(invalid)?;
			let hash = match ty {
				TypeName::Struct(n) => {
					crate::Hash::new(super::nominal::hash_copy(n).ok_or_else(invalid)?)
				}
				_ => ty.builtin_hash().ok_or_else(invalid)?,
			};
			let registered = self.active_host_runtime.as_ref().and_then(|runtime| {
				runtime.constant(&crate::Hash::associated_function(
					hash,
					&crate::runtime::Protocol::INTO_TYPE_NAME,
				))
			});
			let name = if let Some(constant) = registered {
				crate::alloc::String::try_from(constant.as_string().map_err(|_| invalid())?)?
			} else {
				match ty {
					TypeName::Struct(n) => {
						let mut b = [0; 256];
						let len = super::nominal::type_name_copy(n, &mut b).ok_or_else(invalid)?;
						crate::alloc::String::try_from(
							core::str::from_utf8(&b[..len]).map_err(|_| invalid())?,
						)?
					}
					name => {
						let text = crate::alloc::fmt::try_format(format_args!("::{name}"))?;
						crate::alloc::String::try_from(text.split('<').next().ok_or_else(invalid)?)?
					}
				}
			};
			store.string_owned(name)?
		};
		self.put(
			store,
			base.checked_add(out as usize).ok_or_else(invalid)?,
			result,
		)
	}
}

#[derive(Clone, Copy, Debug)]
struct NumericContext {
	op: BinarySymbol,
	lhs: TypeName,
	rhs: TypeName,
}

#[cold]
#[inline(never)]
fn invocation_numeric_context(
	error: &Error,
	code: &Code,
	store: &Store,
	frames: &[Frame],
	slots: &[Value],
) -> Option<NumericContext> {
	if error.operation != "primitive operation"
		|| !matches!(
			error.kind,
			ErrorKind::Overflow | ErrorKind::DivisionByZero | ErrorKind::IntegerCoercion
		) {
		return None;
	}
	let frame = frames.last()?;
	let function = code.function(frame.function)?;
	let pc = frame.pc.checked_sub(1)?;
	if error.location != function.locations.get(pc).copied() {
		return None;
	}
	let (operator, lhs, rhs, assign) = match function.cached_ref(pc)?.op {
		Op::Binary { op, lhs, rhs, .. } => (op, lhs, rhs, false),
		Op::AssignBinary { op, lhs, rhs, .. } => (op, lhs, rhs, true),
		_ => return None,
	};
	let lhs = store.diagnostic_type(*slots.get(frame.base + lhs as usize)?)?;
	let rhs = store.diagnostic_type(*slots.get(frame.base + rhs as usize)?)?;
	if !matches!(lhs, TypeName::I64 | TypeName::U64)
		|| !matches!(rhs, TypeName::I64 | TypeName::U64)
	{
		return None;
	}
	match error.detail {
		Detail::None | Detail::Underflow
			if matches!(error.kind, ErrorKind::Overflow | ErrorKind::DivisionByZero) => {}
		Detail::Coerce {
			to: IntTarget::U32, ..
		} if matches!(operator, arithmetic::Binary::Shl | arithmetic::Binary::Shr) => (),
		_ => return None,
	}
	Some(NumericContext {
		op: BinarySymbol::from_op(operator, assign),
		lhs,
		rhs,
	})
}

#[cfg(test)]
mod d1e_frame_tests {
	use super::*;

	#[test]
	fn unresolved_origin_and_repeated_callers_are_retained() {
		let failure = Failure {
			numeric_context: None,
			error: Error::new(ErrorKind::InvalidCode, "contextless"),
			missing_key: None,
			panic_message: None,
			host_message: None,
			call_chain: {
				let mut chain = Vec::new();
				chain.try_push(Location::UNKNOWN).unwrap();
				chain.try_push(Location::UNKNOWN).unwrap();
				chain
			},
			cleanup_error: None,
		};
		let frames = failure
			.logical_frames()
			.collect::<rust_alloc::vec::Vec<_>>();
		assert_eq!(frames.len(), 3);
		assert!(frames.iter().all(|frame| frame.location.is_none()));
		assert_eq!(frames[0].role, crate::runtime::LogicalFrameRole::Origin);
		assert_eq!(frames[1].role, crate::runtime::LogicalFrameRole::Caller);
		assert_eq!(frames[2].role, crate::runtime::LogicalFrameRole::Caller);
	}
}

#[cfg(test)]
mod entry_authority_tests;
