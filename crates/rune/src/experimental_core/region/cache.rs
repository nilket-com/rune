//! P2a production region cache (feature `fresh-core-native`, x86-64 Linux only). UNSTABLE.
//!
//! Compiled regions live in a **thread-local** cache keyed by [`Code::id`], so `Program`,
//! `Code` and `Executor` stay `Send` and unchanged in meaning: a [`NativeRegion`] (raw
//! pointer, `!Send`/`!Sync`) never leaves the thread that loaded it. There is no unsafe
//! code here (the fresh core is `forbid(unsafe_code)`); only `NativeRegion`'s safe API is
//! used, and the only constructor of a loadable artifact stays `lower::compile`.
//!
//! # Life cycle of one entry
//!
//! - **Checkout.** The executor checks the entry for its code out of the cache for one
//!   invocation and puts it back when the invocation ends (an RAII [`Guard`]). While it is
//!   out, the cache keeps an explicit `CheckedOut` marker: a nested checkout of the same
//!   code on this thread runs interpreted and never compiles a duplicate. A build in
//!   progress keeps a `Building` marker with the same rule.
//! - **Build** (first invocation of a code on a thread). Streamed: per function, then per
//!   candidate header, analysis and at once compile, audit and load of an admitted region,
//!   each refusal recorded by kind. Every bound below is checked *before* the work it
//!   bounds. A panic inside `lower::compile` or `NativeRegion::load` is caught (best effort,
//!   only under `panic=unwind`) and recorded (`panicked`, `load_panicked`); any other
//!   unwinding drops the partial build (freeing loaded code and releasing its process
//!   reservations) and its `Building` marker, so nothing stays stuck.
//! - **Negative entries.** A code with no loaded region keeps a small `Empty` entry (counts
//!   only, no text): it is never re-analysed or recompiled while resident. When analysis
//!   admitted no region at all, the executor also remembers the code id (a property of the
//!   code alone), so later invocations skip the cache entirely.
//! - **Eviction and retry.** At most [`MAX_CACHED_PROGRAMS`] entries (positive or
//!   negative) per thread, least recently used first out; checked-out and building entries
//!   are never evicted. Evicting an entry drops its regions (`NativeRegion::drop` frees the
//!   executable memory). An evicted code is rebuilt from scratch at its next invocation:
//!   refusals are remembered only while the entry is resident.
//!
//! # Bounds (P3a; every value is one field of [`Limits`])
//!
//! - **Build is streamed per function.** Each function is scanned, analysed header by
//!   header, and every admitted region is compiled, audited and loaded before the next
//!   header is analysed. Once the per-program region or compile cap is reached nothing
//!   further is analysed (`skipped_after_caps`). Below the caps the loaded set and every
//!   refusal count equal the earlier batch build (the selection golden).
//! - **Per program:** functions attempted (every function reaching the gates is charged,
//!   [`MAX_PROGRAM_ATTEMPTED_FUNCTIONS`]), functions analysed, function length, candidate
//!   headers per function, an analysis-work ESTIMATE (the header scan's `len` is charged
//!   before the scan), compile attempts, loaded regions, machine-code bytes (the P0 1 MiB
//!   bound inside `lower::compile`) and retained metadata bytes.
//! - **Transient analysis bytes:** a HARD bound ([`MAX_TRANSIENT_ANALYSIS_BYTES`]) on the
//!   requested bytes of every Rune analysis buffer alive at once, checked before each
//!   allocation (see `region::budget`), resting on rune_alloc's pinned exact-capacity
//!   invariant (checked after every growth; a violation is an explicit analysis error,
//!   counted as `accounting_invariant`). It covers the header scan bitmap and list, the
//!   predecessor arcs, the function-level inference with its work list, every per-header
//!   scratch buffer and the admitted region until it is loaded. It does not cover
//!   allocator overhead, nor the compiler and audit transient memory: Cranelift's
//!   compilation heap (Cranelift allocates infallibly), the VCode text the machine audit
//!   reads (created and dropped inside `lower::compile`), the audit's temporaries and the
//!   compiled bytes until load. None of that is bounded by Rune; the compile peak is
//!   measured descriptively only. The JIT as a whole is therefore NOT hard-memory bounded.
//!   The fixpoints also have an operational transfer cutoff (`region::TRANSFER_FACTOR`).
//! - **Per thread:** cached programs, mapped executable bytes and retained metadata bytes.
//!   Metadata is our own structures by capacity plus a documented ESTIMATE per loaded region
//!   for the backend module's retained heap ([`MODULE_RESERVE_BYTES`]) and a per-thread
//!   ESTIMATE for the shared ISA and the cache's own entry table
//!   ([`thread_reserve_bytes`]). The estimates are not hard bounds on heap use.
//! - **Per process:** at most [`MAX_PROCESS_MAPPINGS`] loaded regions (one code allocation
//!   each) and [`MAX_PROCESS_MAPPED_BYTES`] requested mapping bytes, reserved by an RAII
//!   token that the loaded region owns (`budget::Token`) and released exactly once, after
//!   its mapping is freed. Three different quantities: the ALLOCATION COUNT (one anonymous
//!   `mmap` per loaded region, which this bound counts), the PAGE BYTES (`page_ceil(code)`
//!   per allocation, which `mapped_bytes` counts) and the kernel's VMA COUNT, which can be
//!   lower because the kernel merges adjacent anonymous mappings with equal protections.
//!   16,384 is our own resource policy, not a guarantee against the host's
//!   `vm.max_map_count`: that host value varies and every other mapping of the process
//!   consumes it too.
//! - **Page size:** the mapping arithmetic assumes 4 KiB pages. The system page size is
//!   queried once per process (`region::page::size`); any other value disables the tier
//!   (`page_size`).
//! - **Miri:** Miri can neither map generated code executable nor run it. Under `cfg(miri)`
//!   [`checkout`] refuses before it touches this thread's cache: no environment or page-size
//!   query, no analysis, no Cranelift compile and no mapping. The interpreter runs.
//!
//! No listing or diagnostic text is retained: refusal strings are dropped after
//! classification, and a production `Compiled` carries no listing at all (the CLIF texts
//! and the listing exist in test builds only; the VCode text the machine audit needs is
//! dropped inside `lower::compile` right after the audit).
//!
//! # Invalid exits
//!
//! A native exit that fails validation (`Refused::InvalidExit`, or `Refused::Window`) is an
//! internal invariant failure, not a language refusal. The invocation falls back to the
//! interpreter with the original state (validation precedes every write), the region is
//! poisoned (never entered again while resident), the thread's invalid-exit counter is
//! incremented, and one line is written to stderr. Debug builds additionally panic (a
//! `debug_assert!`); release builds do not. A measurement harness can treat any stderr output
//! as an invalid sample, so such an event is never counted as a success.
//!
//! # What is promised
//!
//! Language-visible execution state is unchanged by this tier: results, errors (kind,
//! location, operation), call chains, cleanup errors, store contents and remaining fuel are
//! those of the interpreter. The executor itself does change in non-visible ways (a code-id
//! hint), and the cache keeps counters.
use super::budget::{Budget, Held, Pool, Token};
use super::commit::{self, Refused, Room};
use super::{
	division_kind, scan_headers, DivisionKind, EntryClass, Exit, FunctionContext, Region,
	RegionRefusal, Stop, MAX_REGIONS_PER_FUNCTION, TRANSFER_FACTOR,
};
use crate::experimental_core::code::Code;
use crate::experimental_core::error::Result;
use crate::experimental_core::heap::Store;
use crate::experimental_core::value::Value;
use crate::experimental_native::{
	audit::Finding, boundary::NativeRegion, lower, NativeRefusal, MAX_PROGRAM_CODE_BYTES,
};
use core::cell::{Cell, RefCell};
use std::time::Instant;
use std::vec::Vec as StdVec;

/// Most cached entries (positive or negative) per thread.
pub(crate) const MAX_CACHED_PROGRAMS: usize = 16;
/// Most executable bytes mapped for loaded regions per thread, counted page-rounded.
pub(crate) const MAX_THREAD_MAPPED_BYTES: usize = 4 << 20;
/// Most retained region metadata bytes per thread (including the estimates).
pub(crate) const MAX_THREAD_METADATA_BYTES: usize = 4 << 20;
/// Most retained region metadata bytes per program (including the estimates).
pub(crate) const MAX_PROGRAM_METADATA_BYTES: usize = 1 << 20;
/// Most loaded regions per program.
pub(crate) const MAX_PROGRAM_REGIONS: usize = 64;
/// Most compile attempts per program (refused compiles cost as much as loaded ones).
pub(crate) const MAX_PROGRAM_COMPILES: usize = 128;
/// Most functions analysed per program; later functions get no regions.
pub(crate) const MAX_PROGRAM_ANALYSED_FUNCTIONS: usize = 1024;
/// Most functions attempted per program, whatever gate then skips them (a conservative
/// prototype choice, not an empirically optimal ceiling).
pub(crate) const MAX_PROGRAM_ATTEMPTED_FUNCTIONS: usize = 4096;
/// Functions longer than this get no regions.
pub(crate) const MAX_ANALYSED_FUNCTION_LEN: usize = 65_536;
/// Functions with more candidate headers than this get no regions (bounds the header
/// search, which is linear in the headers found per back edge).
pub(crate) const MAX_FUNCTION_CANDIDATE_HEADERS: usize = 256;
/// Analysis-work budget per program, in the units of [`analysis_work`] (an estimate of the
/// dominant terms, not a hard CPU bound).
pub(crate) const MAX_PROGRAM_ANALYSIS_WORK: u64 = 1 << 24;
/// Hard bound on the requested bytes of Rune analysis buffers alive at once during one
/// build (a conservative prototype choice).
pub(crate) const MAX_TRANSIENT_ANALYSIS_BYTES: usize = 8 << 20;
/// Most loaded regions (one code allocation each) in the whole process: our own policy,
/// not a guarantee against the host's `vm.max_map_count`.
pub(crate) const MAX_PROCESS_MAPPINGS: usize = 16_384;
/// Most requested mapping bytes (page-rounded) of loaded regions in the whole process.
pub(crate) const MAX_PROCESS_MAPPED_BYTES: usize = 64 << 20;
/// Consecutive tag or store entry refusals after which a region is no longer tried.
pub(crate) const COLD_AFTER_REFUSALS: u32 = 64;
/// The x86-64 Linux base page: each loaded region owns at least one private mapping. The
/// tier refuses to run when the system page size differs.
pub(crate) const PAGE_BYTES: usize = 4096;
/// ESTIMATE of the heap one loaded region's `JITModule` retains outside our metadata (its
/// declarations, memory provider and compiled-function table; the ISA is shared). Charged
/// to the program and thread metadata bounds per loaded region. Derived from one measured
/// configuration (a heap measurement) with a margin; NOT a proven upper bound.
pub(crate) const MODULE_RESERVE_BYTES: usize = 4096;
/// ESTIMATE of the heap of one thread's shared ISA (`lower::shared_isa`), charged once per
/// thread to the thread metadata bound. NOT a proven upper bound.
pub(crate) const ISA_RESERVE_BYTES: usize = 64 << 10;

/// Every bound of the tier. Production reads [`Limits::PRODUCTION`]; tests may swap in
/// scaled values for the current thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
	pub(crate) cached_programs: usize,
	pub(crate) thread_mapped_bytes: usize,
	pub(crate) thread_metadata_bytes: usize,
	pub(crate) program_metadata_bytes: usize,
	pub(crate) program_regions: usize,
	pub(crate) program_compiles: usize,
	/// At most [`MAX_PROGRAM_CODE_BYTES`], the bound `lower::compile` itself enforces.
	pub(crate) program_code_bytes: usize,
	pub(crate) program_analysed_functions: usize,
	pub(crate) program_attempted_functions: usize,
	pub(crate) function_len: usize,
	pub(crate) function_candidate_headers: usize,
	pub(crate) program_analysis_work: u64,
	pub(crate) transient_analysis_bytes: usize,
	pub(crate) transfer_factor: usize,
	pub(crate) process_mappings: usize,
	pub(crate) process_mapped_bytes: usize,
}

impl Limits {
	pub(crate) const PRODUCTION: Self = Self {
		cached_programs: MAX_CACHED_PROGRAMS,
		thread_mapped_bytes: MAX_THREAD_MAPPED_BYTES,
		thread_metadata_bytes: MAX_THREAD_METADATA_BYTES,
		program_metadata_bytes: MAX_PROGRAM_METADATA_BYTES,
		program_regions: MAX_PROGRAM_REGIONS,
		program_compiles: MAX_PROGRAM_COMPILES,
		program_code_bytes: MAX_PROGRAM_CODE_BYTES,
		program_analysed_functions: MAX_PROGRAM_ANALYSED_FUNCTIONS,
		program_attempted_functions: MAX_PROGRAM_ATTEMPTED_FUNCTIONS,
		function_len: MAX_ANALYSED_FUNCTION_LEN,
		function_candidate_headers: MAX_FUNCTION_CANDIDATE_HEADERS,
		program_analysis_work: MAX_PROGRAM_ANALYSIS_WORK,
		transient_analysis_bytes: MAX_TRANSIENT_ANALYSIS_BYTES,
		transfer_factor: TRANSFER_FACTOR,
		process_mappings: MAX_PROCESS_MAPPINGS,
		process_mapped_bytes: MAX_PROCESS_MAPPED_BYTES,
	};
}

/// The limits in force on this thread.
pub(crate) fn limits() -> Limits {
	#[cfg(test)]
	if let Some(limits) = LIMITS_FOR_TEST.with(|c| c.get()) {
		return limits;
	}

	Limits::PRODUCTION
}

/// The per-thread metadata charged before any entry: the shared ISA ESTIMATE and the entry
/// table at its largest capacity (it grows by exact reservation to at most
/// `cached_programs` entries).
pub(crate) fn thread_reserve_bytes(limits: &Limits) -> usize {
	limits
		.cached_programs
		.saturating_mul(core::mem::size_of::<Entry>())
		.saturating_add(ISA_RESERVE_BYTES)
}

/// The process-wide mapping reservations of the native tier.
static PROCESS: Pool = Pool::new();

/// Counts of one build. Plain numbers only: no text is retained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeSummary {
	pub(crate) functions: u32,
	/// Functions whose analysis ran (including those with no candidate header).
	pub(crate) analysed_functions: u32,
	pub(crate) skipped_length: u32,
	pub(crate) skipped_headers: u32,
	/// Skipped by the per-program attempted or analysed function count or the
	/// analysis-work budget.
	pub(crate) skipped_budget: u32,
	/// `analyze` returned `Err` (allocation or invalid code): no regions for that function.
	pub(crate) analysis_errors: u32,
	/// Candidate headers analysed.
	pub(crate) candidates: u32,
	/// Regions admitted by the analysis.
	pub(crate) regions: u32,
	/// Regions compiled, audited and loaded.
	pub(crate) loaded: u32,
	pub(crate) division_runtime: u32,
	pub(crate) division_const0: u32,
	pub(crate) division_constneg1: u32,
	/// `NativeRefusal::Size` (the P0 region and program code bounds).
	pub(crate) size: u32,
	pub(crate) codegen: u32,
	/// A panic inside `lower::compile`, caught (best effort).
	pub(crate) panicked: u32,
	pub(crate) audit_other: u32,
	/// `Isa`, `Shape` or `Verifier`, or a compiled slot count that does not match.
	pub(crate) other: u32,
	pub(crate) load_refused: u32,
	/// Refused by the per-program metadata bound.
	pub(crate) metadata: u32,
	/// Batch build: regions refused by [`MAX_PROGRAM_REGIONS`]. P3a (streamed): 1 when the
	/// region cap stopped the build with work left (`skipped_after_caps`), else 0.
	pub(crate) region_cap: u32,
	/// As `region_cap`, for [`MAX_PROGRAM_COMPILES`].
	pub(crate) compile_cap: u32,
	/// Refused by a per-thread bound (mapped or metadata bytes) after eviction.
	pub(crate) thread_bound: u32,
	/// Refused because the tier is disabled in this environment (`PERF_BUILDID_DIR`).
	pub(crate) environment: u32,
	/// P3a: functions that reached the gates (each is charged to
	/// [`MAX_PROGRAM_ATTEMPTED_FUNCTIONS`]).
	pub(crate) attempted_functions: u32,
	/// P3a: functions not analysed (or not finished) because the region or compile cap was
	/// reached; `region_cap`/`compile_cap` are then 1, naming the cap.
	pub(crate) skipped_after_caps: u32,
	/// P3a: functions or headers refused by the transient analysis-bytes bound.
	pub(crate) transient: u32,
	/// P3a: headers refused by the fixpoint transfer cutoff.
	pub(crate) steps: u32,
	/// P3a: loads refused by the process-wide mapping reservation.
	pub(crate) process_bound: u32,
	/// P3a: refused because the system page size is not [`PAGE_BYTES`].
	pub(crate) page_size: u32,
	/// P3a: a panic inside `NativeRegion::load`, caught; its mapping may be leaked, so its
	/// process reservation is kept (leaked) too.
	pub(crate) load_panicked: u32,
	/// P3a: the most transient analysis bytes charged at once (requested bytes).
	pub(crate) analysis_peak_bytes: u64,
	/// Analysis growths that failed rune_alloc's exact-capacity invariant (each one an
	/// analysis error, also counted in `analysis_errors` or `transient` by where it hit).
	pub(crate) accounting_invariant: u32,
	/// P3a: [`MODULE_RESERVE_BYTES`] per loaded region, charged as metadata (an estimate).
	pub(crate) reserved_bytes: u64,
	/// Machine-code bytes of the loaded regions.
	pub(crate) code_bytes: u64,
	/// Page-rounded executable bytes mapped for them.
	pub(crate) mapped_bytes: u64,
	/// Retained metadata bytes (regions and lookup tables), by capacity; the module
	/// estimate is in `reserved_bytes`.
	pub(crate) metadata_bytes: u64,
	pub(crate) analysis_ns: u64,
	pub(crate) compile_ns: u64,
	pub(crate) load_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
	Live,
	/// [`COLD_AFTER_REFUSALS`] consecutive tag or store refusals: no longer tried.
	Cold,
	/// An invalid exit was seen: never entered again.
	Poisoned,
}

/// Per-region counters, as read by tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegionCounters {
	pub(crate) function: u32,
	pub(crate) header: u32,
	pub(crate) state: State,
	pub(crate) entries: u64,
	/// Committed entries by exit status: Continue, Fuel, Retry, Replay (B1).
	pub(crate) exits: [u64; 4],
	pub(crate) refused_cancel: u64,
	pub(crate) refused_store: u64,
	pub(crate) refused_tag: u64,
	/// B1: the iterator entry check refused (counts toward `Cold` like a tag refusal).
	pub(crate) refused_iter: u64,
	/// B1: not enough frame or slot room for an inlined callee (does not count toward
	/// `Cold`: it depends on the call depth, not on the region).
	pub(crate) refused_room: u64,
	pub(crate) code_bytes: u32,
	pub(crate) compile_ns: u64,
	pub(crate) load_ns: u64,
	/// P3a: the requested mapping bytes this region's process reservation holds.
	pub(crate) reserved_mapping: usize,
}

/// One loaded region. The `Region` keeps only what entry and commit read (its nodes are
/// dropped after compile).
pub(crate) struct Ready {
	region: Region,
	native: NativeRegion,
	state: Cell<State>,
	entries: Cell<u64>,
	exits: [Cell<u64>; 4],
	refused_cancel: Cell<u64>,
	refused_store: Cell<u64>,
	refused_tag: Cell<u64>,
	refused_iter: Cell<u64>,
	refused_room: Cell<u64>,
	consecutive: Cell<u32>,
	code_bytes: u32,
	compile_ns: u64,
	load_ns: u64,
	/// The process-wide reservation of `native`'s mapping. Declared after `native`, so it
	/// is released only after the mapping is freed (fields drop in declaration order).
	reservation: Token<'static>,
}

/// The loaded regions of one code, checked out for one invocation at a time.
pub(crate) struct ProgramRegions {
	code: u32,
	/// `(function, start, end)` into `headers`, sorted by function.
	functions: StdVec<(u32, u32, u32)>,
	/// `(header pc, index into ready)`, grouped by function.
	headers: StdVec<(u32, u32)>,
	ready: StdVec<Ready>,
	summary: NativeSummary,
}

impl ProgramRegions {
	/// The header table of `function`; empty when it has no loaded region. Once per frame.
	#[inline]
	pub(crate) fn function(&self, function: u32) -> &[(u32, u32)] {
		match self.functions.binary_search_by_key(&function, |e| e.0) {
			Ok(i) => {
				let (_, start, end) = self.functions[i];
				self.headers
					.get(start as usize..end as usize)
					.unwrap_or(&[])
			}
			Err(_) => &[],
		}
	}

	/// The region whose header is `pc` in a table from [`Self::function`].
	#[inline]
	pub(crate) fn header(&self, table: &[(u32, u32)], pc: usize) -> Option<&Ready> {
		for &(header, index) in table {
			if header as usize == pc {
				return self.ready.get(index as usize);
			}
		}
		None
	}
}

std::thread_local! {
	/// This thread's invalid native exits (each poisoned a region).
	static INVALID_EXITS: Cell<u64> = const { Cell::new(0) };
}

/// The number of invalid native exits seen on this thread.
pub(crate) fn invalid_exits() -> u64 {
	INVALID_EXITS.with(|c| c.get())
}

impl Ready {
	/// Enters the region at its header (the caller checked the resume rule). `Ok(Some(_))`:
	/// committed, resume at `pc` with `fuel`. `Ok(None)`: nothing changed, the interpreter
	/// runs the header. `Err`: a write-back failed after the commit began (an invariant
	/// failure, unreachable by the entry proof): the region is poisoned, the invalid-exit
	/// counter incremented, and the caller raises the error at the header; it is never
	/// recovered as a fallback.
	#[inline]
	pub(crate) fn enter(
		&self,
		window: &mut [Value],
		store: &mut Store,
		cancel_pending: bool,
		fuel: usize,
		room: Room,
	) -> Result<Option<(usize, usize)>> {
		if self.state.get() != State::Live {
			return Ok(None);
		}

		let outcome = match commit::enter(
			&self.region,
			window,
			store,
			cancel_pending,
			fuel,
			room,
			|_, buf, record| self.call(buf, record),
		) {
			Ok(outcome) => outcome,
			Err(error) => {
				self.invalid(Refused::InvalidExit);
				return Err(error);
			}
		};

		match outcome {
			Ok(exited) => {
				self.entries.set(self.entries.get() + 1);
				let exits = &self.exits[exited.status as usize - 1];
				exits.set(exits.get() + 1);
				self.consecutive.set(0);
				Ok(Some((exited.pc, exited.fuel)))
			}
			Err(Refused::Cancel) => {
				// Not the region's fault; the interpreter consumes the request next.
				self.refused_cancel.set(self.refused_cancel.get() + 1);
				Ok(None)
			}
			Err(Refused::Room) => {
				self.refused_room.set(self.refused_room.get() + 1);
				Ok(None)
			}
			Err(refused @ (Refused::Store | Refused::Tag { .. } | Refused::Iter { .. })) => {
				let counter = match refused {
					Refused::Store => &self.refused_store,
					Refused::Iter { .. } => &self.refused_iter,
					_ => &self.refused_tag,
				};
				counter.set(counter.get() + 1);
				let consecutive = self.consecutive.get() + 1;
				self.consecutive.set(consecutive);
				if consecutive >= COLD_AFTER_REFUSALS {
					self.state.set(State::Cold);
				}
				Ok(None)
			}
			Err(refused @ (Refused::Window | Refused::InvalidExit)) => {
				self.invalid(refused);
				Ok(None)
			}
		}
	}

	/// The native call. A `None` (contract mismatch: nothing ran) returns status 0, which
	/// validation rejects before any write.
	fn call(&self, buf: &mut [i64], record: &mut super::abi::ExitRecord) -> u32 {
		let status = self.native.call(buf, record).unwrap_or(0);

		#[cfg(test)]
		if take_injected_invalid_exit() {
			record.status = 0;
			return 0;
		}

		status
	}

	#[cold]
	#[inline(never)]
	fn invalid(&self, refused: Refused) {
		self.state.set(State::Poisoned);
		INVALID_EXITS.with(|c| c.set(c.get() + 1));
		std::eprintln!(
			"rune fresh-core-native: INVALID native region exit ({refused:?}) in function {} at header pc {}: region poisoned, interpreter fallback with the original state",
			self.region.function,
			self.region.header
		);

		#[cfg(test)]
		let injected = INJECTED_INVALID_SEEN.with(|c| c.replace(false));
		#[cfg(not(test))]
		let injected = false;

		debug_assert!(
			injected,
			"invalid native region exit ({refused:?}) in function {} at header pc {}",
			self.region.function, self.region.header
		);
	}

	fn counters(&self) -> RegionCounters {
		RegionCounters {
			function: self.region.function,
			header: self.region.header,
			state: self.state.get(),
			entries: self.entries.get(),
			exits: [
				self.exits[0].get(),
				self.exits[1].get(),
				self.exits[2].get(),
				self.exits[3].get(),
			],
			refused_cancel: self.refused_cancel.get(),
			refused_store: self.refused_store.get(),
			refused_tag: self.refused_tag.get(),
			refused_iter: self.refused_iter.get(),
			refused_room: self.refused_room.get(),
			code_bytes: self.code_bytes,
			compile_ns: self.compile_ns,
			load_ns: self.load_ns,
			reserved_mapping: self.reservation.bytes(),
		}
	}
}

// ------------------------------------------------------------------------ the cache

enum Resident {
	/// A build in progress on this thread (in-flight marker).
	Building,
	/// Checked out by an invocation (in-flight marker).
	CheckedOut,
	Ready(ProgramRegions),
	/// No loaded region: counts only.
	Empty(NativeSummary),
}

struct Entry {
	code: u32,
	last_use: u64,
	/// Page-rounded executable bytes charged to this entry (also while in flight).
	mapped: usize,
	/// Retained metadata bytes charged to this entry (also while in flight).
	metadata: usize,
	resident: Resident,
}

struct Cache {
	entries: StdVec<Entry>,
	mapped: usize,
	metadata: usize,
	tick: u64,
	/// `None` until first use.
	enabled: Option<Enabled>,
}

/// Whether this thread may load native code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Enabled {
	Yes,
	/// `PERF_BUILDID_DIR` is set.
	Environment,
	/// The system page size is not [`PAGE_BYTES`].
	PageSize,
}

/// The system page size, queried once per process (safe: `region::page::size`).
fn page_size() -> usize {
	#[cfg(test)]
	if let Some(size) = PAGE_SIZE_FOR_TEST.with(|c| c.get()) {
		return size;
	}

	static PAGE_SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
	*PAGE_SIZE.get_or_init(region::page::size)
}

std::thread_local! {
	static CACHE: RefCell<Cache> = const {
		RefCell::new(Cache {
			entries: StdVec::new(),
			mapped: 0,
			metadata: 0,
			tick: 0,
			enabled: None,
		})
	};
}

impl Cache {
	#[inline]
	fn enabled(&mut self) -> Enabled {
		// cranelift-jit appends to /tmp/perf-<pid>.map (and unwraps) when this is set, so
		// the tier refuses to load anything then. Read once per thread. The mapping
		// arithmetic assumes 4 KiB pages: any other page size disables the tier.
		*self.enabled.get_or_insert_with(|| {
			if std::env::var_os("PERF_BUILDID_DIR").is_some() {
				Enabled::Environment
			} else if page_size() != PAGE_BYTES {
				Enabled::PageSize
			} else {
				Enabled::Yes
			}
		})
	}

	#[inline]
	fn position(&self, code: u32) -> Option<usize> {
		self.entries.iter().position(|e| e.code == code)
	}

	fn remove(&mut self, index: usize) -> Entry {
		let entry = self.entries.swap_remove(index);
		self.mapped -= entry.mapped;
		self.metadata -= entry.metadata;
		entry
	}

	/// Evicts the least recently used resident (not in-flight) entry other than `keep`.
	/// The removed entry is returned so the caller drops it after releasing the borrow.
	fn evict_lru(&mut self, keep: u32) -> Option<Entry> {
		let index = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, e)| e.code != keep)
			.filter(|(_, e)| matches!(e.resident, Resident::Ready(_) | Resident::Empty(_)))
			.min_by_key(|(_, e)| e.last_use)
			.map(|(i, _)| i)?;
		Some(self.remove(index))
	}

	/// Charges `mapped` and `metadata` bytes to the in-flight entry `code`, evicting least
	/// recently used entries as needed. `false`: the bound cannot be met.
	/// The per-thread base ([`thread_reserve_bytes`]) counts against the metadata bound
	/// before any entry does.
	fn reserve(
		&mut self,
		limits: &Limits,
		code: u32,
		mapped: usize,
		metadata: usize,
		evicted: &mut StdVec<Entry>,
	) -> bool {
		let base = thread_reserve_bytes(limits);
		loop {
			let fits = self
				.mapped
				.checked_add(mapped)
				.is_some_and(|m| m <= limits.thread_mapped_bytes)
				&& self
					.metadata
					.checked_add(metadata)
					.and_then(|m| m.checked_add(base))
					.is_some_and(|m| m <= limits.thread_metadata_bytes);
			if fits {
				break;
			}
			match self.evict_lru(code) {
				Some(entry) => evicted.push(entry),
				None => return false,
			}
		}
		let Some(index) = self.position(code) else {
			return false;
		};
		self.entries[index].mapped += mapped;
		self.entries[index].metadata += metadata;
		self.mapped += mapped;
		self.metadata += metadata;
		true
	}
}

/// What one invocation runs with. A short-lived value returned once per invocation:
/// boxing `Guard` would add an allocation to every native invocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Checkout {
	/// At least one loaded region: run the native-aware loop with these regions.
	Native(Guard),
	/// Run the plain loop. `no_regions`: the analysis of this code admitted no region at
	/// all (a property of the code), so the caller may skip the cache for it from now on.
	Interpret { no_regions: bool },
}

/// A checked-out entry; put back when dropped (also on unwind).
pub(crate) struct Guard {
	regions: Option<ProgramRegions>,
}

impl Guard {
	pub(crate) fn regions(&self) -> Option<&ProgramRegions> {
		self.regions.as_ref()
	}
}

impl Drop for Guard {
	#[inline]
	fn drop(&mut self) {
		let Some(regions) = self.regions.take() else {
			return;
		};
		// If the cache is gone (thread exit) or borrowed (impossible: no borrow outlives a
		// cache call), the regions are dropped here, which frees their code.
		let _ = CACHE.try_with(|cache| {
			if let Ok(mut cache) = cache.try_borrow_mut() {
				put_back(&mut cache, regions);
			}
		});
	}
}

#[inline]
fn put_back(cache: &mut Cache, regions: ProgramRegions) {
	cache.tick += 1;
	let tick = cache.tick;
	if let Some(index) = cache.position(regions.code) {
		let entry = &mut cache.entries[index];
		if matches!(entry.resident, Resident::CheckedOut) {
			entry.resident = Resident::Ready(regions);
			entry.last_use = tick;
			return;
		}
	}
	// No marker (cannot happen: markers are never evicted): drop the regions and any
	// charge that was left without an entry.
	drop(regions);
}

/// Removes a `Building` marker that a build left behind by unwinding.
struct BuildGuard {
	code: u32,
	armed: bool,
}

impl Drop for BuildGuard {
	fn drop(&mut self) {
		if !self.armed {
			return;
		}
		let removed = CACHE.try_with(|cache| {
			let mut cache = cache.try_borrow_mut().ok()?;
			let index = cache.position(self.code)?;
			if matches!(cache.entries[index].resident, Resident::Building) {
				Some(cache.remove(index))
			} else {
				None
			}
		});
		drop(removed);
	}
}

/// Checks out the regions of `code` for one invocation on this thread, building them at
/// the first invocation.
// The cache closure returns the final `Checkout` as its `Err` (see `Checkout`).
#[allow(clippy::result_large_err)]
#[inline]
pub(crate) fn checkout(code: &Code) -> Checkout {
	#[cfg(test)]
	{
		TEST_COUNTERS.with(|c| c.checkouts.set(c.checkouts.get() + 1));
		// Tests opt in, and the P1 test hook and the production tier never both run.
		if !PRODUCTION_FOR_TEST.with(|c| c.get()) || super::hook::is_installed() {
			return Checkout::Interpret { no_regions: false };
		}
	}

	// Miri: refuse here, before `Cache::enabled` queries the environment and the page size
	// and before any analysis, compile or mapping (module docs, "Miri").
	if cfg!(miri) {
		return Checkout::Interpret { no_regions: false };
	}

	let id = code.id();
	let mut evicted = None;
	let enabled = {
		let decided = CACHE.with(|cache| {
			let mut cache = cache.borrow_mut();
			cache.tick += 1;
			let tick = cache.tick;
			let enabled = cache.enabled();

			if let Some(index) = cache.position(id) {
				let entry = &mut cache.entries[index];
				entry.last_use = tick;
				return Err(match &mut entry.resident {
					Resident::Ready(_) => {
						let Resident::Ready(regions) =
							core::mem::replace(&mut entry.resident, Resident::CheckedOut)
						else {
							unreachable!()
						};
						Checkout::Native(Guard {
							regions: Some(regions),
						})
					}
					// In flight on this thread: run interpreted, never build a duplicate.
					Resident::Building | Resident::CheckedOut => {
						Checkout::Interpret { no_regions: false }
					}
					Resident::Empty(summary) => Checkout::Interpret {
						no_regions: summary.regions == 0,
					},
				});
			}

			if cache.entries.len() >= limits().cached_programs {
				match cache.evict_lru(id) {
					Some(entry) => evicted = Some(entry),
					// Every entry is in flight: run uncached.
					None => return Err(Checkout::Interpret { no_regions: false }),
				}
			}

			// Exact: the table's capacity never exceeds `cached_programs` (charged per thread).
			if cache.entries.try_reserve_exact(1).is_err() {
				return Err(Checkout::Interpret { no_regions: false });
			}

			cache.entries.push(Entry {
				code: id,
				last_use: tick,
				mapped: 0,
				metadata: 0,
				resident: Resident::Building,
			});
			Ok(enabled)
		});

		// Dropped outside the borrow: dropping regions runs `NativeRegion::drop`.
		drop(evicted.take());

		match decided {
			Ok(enabled) => enabled,
			Err(checkout) => return checkout,
		}
	};

	let mut marker = BuildGuard {
		code: id,
		armed: true,
	};
	let limits = limits();
	let regions = build(code, enabled, &limits);
	marker.armed = false;

	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let Some(index) = cache.position(id) else {
			// Unreachable (the marker is never evicted); the regions drop here.
			return Checkout::Interpret { no_regions: false };
		};
		let entry = &mut cache.entries[index];
		if regions.ready.is_empty() {
			let no_regions = regions.summary.regions == 0;
			entry.resident = Resident::Empty(regions.summary);
			Checkout::Interpret { no_regions }
		} else {
			entry.resident = Resident::CheckedOut;
			Checkout::Native(Guard {
				regions: Some(regions),
			})
		}
	})
}

/// Builds (or finds) the regions of `code` on this thread without running anything, and
/// returns the build summary. Crate-private: there is no public prepare API.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn prepare(code: &Code) -> Option<NativeSummary> {
	let checkout = checkout(code);
	drop(checkout);
	summary(code.id())
}

/// Drops the cached entry of `code` on this thread (its code is freed). `false` when it is
/// absent or in flight.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn evict(code: u32) -> bool {
	let removed = CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let index = cache.position(code)?;
		if matches!(
			cache.entries[index].resident,
			Resident::Ready(_) | Resident::Empty(_)
		) {
			Some(cache.remove(index))
		} else {
			None
		}
	});
	removed.is_some()
}

/// The build summary of the resident entry of `code` on this thread.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn summary(code: u32) -> Option<NativeSummary> {
	CACHE.with(|cache| {
		let cache = cache.borrow();
		let entry = &cache.entries[cache.position(code)?];
		match &entry.resident {
			Resident::Ready(regions) => Some(regions.summary),
			Resident::Empty(summary) => Some(*summary),
			Resident::Building | Resident::CheckedOut => None,
		}
	})
}

// ------------------------------------------------------------------------ the build

/// Retained metadata of one loaded region (its nodes are dropped after compile) plus its
/// two lookup-table rows, by capacity. The module estimate is charged separately
/// ([`MODULE_RESERVE_BYTES`]).
pub(crate) fn region_metadata(region: &Region) -> usize {
	region.slots.capacity() * core::mem::size_of::<u32>()
		+ region.entry.capacity() * core::mem::size_of::<EntryClass>()
		+ region.exits.capacity() * core::mem::size_of::<Exit>()
		+ core::mem::size_of::<Ready>()
		+ core::mem::size_of::<(u32, u32)>()
		+ core::mem::size_of::<(u32, u32, u32)>()
}

/// The analysis-work estimate of one function, charged before it is analysed: one pass
/// per candidate header plus the header search over the whole function, and the
/// function-level inference cells when that inference runs. An estimate of the dominant
/// terms, not a proven worst case of the fixpoints (those have the transfer cutoff).
pub(crate) fn analysis_work(len: usize, headers: usize, slots: usize) -> u64 {
	let len = len as u64;
	let passes = 1 + headers.min(MAX_REGIONS_PER_FUNCTION) as u64;
	let cells = len.saturating_mul(slots as u64);
	let inference = if cells <= super::MAX_FUNCTION_INFERENCE_CELLS as u64 {
		cells
	} else {
		0
	};
	len.saturating_mul(passes).saturating_add(inference)
}

fn elapsed_ns(start: Instant) -> u64 {
	u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn page_ceil(bytes: usize) -> usize {
	bytes.div_ceil(PAGE_BYTES).max(1) * PAGE_BYTES
}

/// The state of one streamed build.
struct Build<'l> {
	id: u32,
	limits: &'l Limits,
	enabled: Enabled,
	summary: NativeSummary,
	ready: StdVec<Ready>,
	program_bytes: usize,
	program_metadata: usize,
	compiles: usize,
}

impl Build<'_> {
	/// Whether the region or compile cap stops the build (checked before every header).
	fn capped(&mut self) -> bool {
		if self.enabled != Enabled::Yes {
			return false;
		}
		if self.ready.len() >= self.limits.program_regions {
			self.summary.region_cap = 1;
			return true;
		}
		if self.compiles >= self.limits.program_compiles {
			self.summary.compile_cap = 1;
			return true;
		}
		false
	}

	/// Compile, audit and load one admitted region; the batch build's second-loop body. `held`
	/// is the region's transient charge. It is released only after the buffers it stands
	/// for are freed: on success after the nodes are dropped (the retained slots, entry
	/// classes and exits are charged to metadata instead), on every other return after the
	/// region (a local, so it drops before the `held` parameter).
	fn load(&mut self, held: Held<'_>, region: Region) {
		let mut region = region;

		match self.enabled {
			Enabled::Yes => {}
			Enabled::Environment => {
				self.summary.environment += 1;
				return;
			}
			Enabled::PageSize => {
				self.summary.page_size += 1;
				return;
			}
		}

		let summary = &mut self.summary;
		let metadata = region_metadata(&region).saturating_add(MODULE_RESERVE_BYTES);
		if self
			.program_metadata
			.checked_add(metadata)
			.is_none_or(|m| m > self.limits.program_metadata_bytes)
		{
			summary.metadata += 1;
			return;
		}
		// Exact: the table's capacity is the loaded count, which `region_metadata` charges.
		if self.ready.try_reserve_exact(1).is_err() {
			summary.metadata += 1;
			return;
		}

		#[cfg(test)]
		injected_build_panic();

		self.compiles += 1;
		// `lower::compile` enforces the P0 program code bound at MAX_PROGRAM_CODE_BYTES; a
		// lower limit (tests) is applied by offsetting the bytes already compiled.
		let offset =
			MAX_PROGRAM_CODE_BYTES - self.limits.program_code_bytes.min(MAX_PROGRAM_CODE_BYTES);
		let program_bytes = self.program_bytes.saturating_add(offset);
		let started = Instant::now();
		let compiled = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
			#[cfg(test)]
			injected_compile_panic();
			lower::compile(&region, program_bytes)
		}));
		let compile_ns = elapsed_ns(started);
		summary.compile_ns += compile_ns;

		let compiled = match compiled {
			Err(_) => {
				summary.panicked += 1;
				return;
			}
			Ok(Err(refusal)) => {
				// Classified, then dropped with any text it carries.
				match refusal {
					NativeRefusal::Audit(Finding::Division(_)) => match division_kind(&region) {
						Some(DivisionKind::Const0) => summary.division_const0 += 1,
						Some(DivisionKind::ConstNeg1) => summary.division_constneg1 += 1,
						_ => summary.division_runtime += 1,
					},
					NativeRefusal::Size => summary.size += 1,
					NativeRefusal::Codegen(_) => summary.codegen += 1,
					NativeRefusal::Audit(_) => summary.audit_other += 1,
					NativeRefusal::Isa | NativeRefusal::Shape | NativeRefusal::Verifier(_) => {
						summary.other += 1
					}
				}
				return;
			}
			Ok(Ok(compiled)) => compiled,
		};

		if compiled.cells() != region.cells() {
			summary.other += 1;
			return;
		}

		let bytes = compiled.bytes().len();
		let mapped = page_ceil(bytes);
		let id = self.id;
		let mut evicted = StdVec::new();
		let reserved = CACHE.with(|cache| {
			cache
				.borrow_mut()
				.reserve(self.limits, id, mapped, metadata, &mut evicted)
		});
		drop(evicted);
		if !reserved {
			summary.thread_bound += 1;
			return;
		}

		// The process-wide reservation, taken before the mapping exists and kept by its
		// owner until after the mapping is freed.
		let Some(reservation) = PROCESS.reserve(
			mapped,
			self.limits.process_mappings,
			self.limits.process_mapped_bytes,
		) else {
			summary.process_bound += 1;
			unreserve(id, mapped, metadata);
			return;
		};

		let started = Instant::now();
		let loaded = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
			NativeRegion::load(&compiled)
		}));
		let load_ns = elapsed_ns(started);
		summary.load_ns += load_ns;
		drop(compiled);

		let native = match loaded {
			Ok(Ok(native)) => native,
			Ok(Err(_)) => {
				// `load` freed whatever it mapped: the reservation is released here.
				summary.load_refused += 1;
				drop(reservation);
				unreserve(id, mapped, metadata);
				return;
			}
			Err(_) => {
				// A panic inside cranelift-jit: its `JITModule` was dropped without
				// `free_memory`, which forgets (leaks) its mapping. The reservation is leaked
				// with it, so the process count never undercounts a live mapping.
				summary.load_panicked += 1;
				core::mem::forget(reservation);
				unreserve(id, mapped, metadata);
				return;
			}
		};

		// Entry and commit never read the nodes or the sites; the transient charge ends
		// with them.
		region.nodes = crate::alloc::Vec::new();
		region.sites = crate::alloc::Vec::new();
		drop(held);

		self.program_bytes += bytes;
		self.program_metadata += metadata;
		summary.loaded += 1;
		summary.code_bytes += bytes as u64;
		summary.mapped_bytes += mapped as u64;
		summary.metadata_bytes += (metadata - MODULE_RESERVE_BYTES) as u64;
		summary.reserved_bytes += MODULE_RESERVE_BYTES as u64;
		self.ready.push(Ready {
			region,
			native,
			state: Cell::new(State::Live),
			entries: Cell::new(0),
			exits: [Cell::new(0), Cell::new(0), Cell::new(0), Cell::new(0)],
			refused_cancel: Cell::new(0),
			refused_store: Cell::new(0),
			refused_tag: Cell::new(0),
			refused_iter: Cell::new(0),
			refused_room: Cell::new(0),
			consecutive: Cell::new(0),
			code_bytes: bytes as u32,
			compile_ns,
			load_ns,
			reservation,
		});
	}
}

/// Returns a thread reservation taken for a region that was not loaded.
fn unreserve(id: u32, mapped: usize, metadata: usize) {
	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		if let Some(index) = cache.position(id) {
			cache.entries[index].mapped -= mapped;
			cache.entries[index].metadata -= metadata;
			cache.mapped -= mapped;
			cache.metadata -= metadata;
		}
	});
}

/// Streams the build: per function, the gates, the header scan, then per candidate header
/// the analysis and at once the compile, audit and load of an admitted region. Every bound
/// is checked before the work it bounds, and every limit hit is an interpreter fallback.
/// Touches no store, no frontend and no `Program`.
///
/// Below the caps this equals the earlier batch build (analysis of every function, then
/// every admitted region in order): the order of compile attempts is function ascending,
/// then header ascending, in both. Differences, by design: (1) once a cap is reached the
/// rest is not analysed (`skipped_after_caps`; `region_cap`/`compile_cap` become flags);
/// (2) an analysis error in a header stops that function's later headers but keeps the
/// regions already loaded from it (the batch build dropped the whole function's regions; both are
/// fallbacks, and such errors are allocation failures or invalid code).
fn build(code: &Code, enabled: Enabled, limits: &Limits) -> ProgramRegions {
	let functions = code.functions();
	let mut b = Build {
		id: code.id(),
		limits,
		enabled,
		summary: NativeSummary {
			functions: u32::try_from(functions).unwrap_or(u32::MAX),
			..NativeSummary::default()
		},
		ready: StdVec::new(),
		program_bytes: 0,
		program_metadata: 0,
		compiles: 0,
	};

	#[cfg(test)]
	TEST_COUNTERS.with(|c| c.builds.set(c.builds.get() + 1));

	let started = Instant::now();
	let budget = Budget::new(limits.transient_analysis_bytes);
	let mut work = 0u64;

	'functions: for function in 0..functions as u32 {
		let remaining = || u32::try_from(functions - function as usize).unwrap_or(u32::MAX);
		if b.capped() {
			b.summary.skipped_after_caps += remaining();
			break;
		}
		if b.summary.attempted_functions as usize >= limits.program_attempted_functions {
			b.summary.skipped_budget += remaining();
			break;
		}
		b.summary.attempted_functions += 1;

		let Some(f) = code.function(function) else {
			continue;
		};
		if b.summary.analysed_functions as usize >= limits.program_analysed_functions {
			b.summary.skipped_budget += 1;
			continue;
		}
		if f.len() > limits.function_len {
			b.summary.skipped_length += 1;
			continue;
		}

		// The scan is charged (its `len` term of `analysis_work`) before it runs.
		let len = f.len() as u64;
		if work.saturating_add(len) > limits.program_analysis_work {
			b.summary.skipped_budget += 1;
			continue;
		}
		work += len;

		#[cfg(test)]
		TEST_COUNTERS.with(|c| c.scans.set(c.scans.get() + 1));

		let headers = match scan_headers(f, &budget) {
			Ok(headers) => headers,
			Err(Stop::Budget) => {
				b.summary.transient += 1;
				continue;
			}
			Err(Stop::Error(_)) => {
				b.summary.analysis_errors += 1;
				continue;
			}
		};
		if headers.len() > limits.function_candidate_headers {
			b.summary.skipped_headers += 1;
			continue;
		}
		let rest = analysis_work(f.len(), headers.len(), f.slots as usize).saturating_sub(len);
		if work.saturating_add(rest) > limits.program_analysis_work {
			b.summary.skipped_budget += 1;
			continue;
		}
		work += rest;
		b.summary.analysed_functions += 1;
		if headers.is_empty() {
			continue;
		}

		let context = match FunctionContext::new(code, function, &budget, limits.transfer_factor) {
			Ok(context) => context,
			Err(Stop::Budget) => {
				b.summary.transient += 1;
				continue;
			}
			Err(Stop::Error(_)) => {
				b.summary.analysis_errors += 1;
				continue;
			}
		};

		for &header in headers.iter().take(MAX_REGIONS_PER_FUNCTION) {
			if b.capped() {
				b.summary.skipped_after_caps += remaining();
				break 'functions;
			}
			b.summary.candidates += 1;

			#[cfg(test)]
			TEST_COUNTERS.with(|c| c.analyzed_headers.set(c.analyzed_headers.get() + 1));

			let (region, bytes) = match context.analyze_header(header) {
				Ok(Ok(found)) => found,
				Ok(Err(RegionRefusal::Steps)) => {
					b.summary.steps += 1;
					continue;
				}
				Ok(Err(_)) => continue,
				Err(Stop::Budget) => {
					b.summary.transient += 1;
					continue;
				}
				Err(Stop::Error(_)) => {
					// The failed header's buffers are already freed and released; the
					// function's later headers are not analysed, its loaded regions stay.
					b.summary.analysis_errors += 1;

					#[cfg(test)]
					TEST_COUNTERS.with(|c| {
						c.header_error_used.set(Some((
							budget.used(),
							headers.charged() + context.charged_for_test(),
						)))
					});

					break;
				}
			};
			let held = Held::adopt(&budget, bytes);
			b.summary.regions += 1;

			#[cfg(test)]
			TEST_COUNTERS.with(|c| {
				c.max_transfers
					.set(c.max_transfers.get().max(context.max_transfers()))
			});

			b.load(held, region);
		}
	}

	let mut summary = b.summary;
	summary.analysis_peak_bytes = budget.peak() as u64;
	summary.accounting_invariant = budget.violations();

	#[cfg(test)]
	TEST_COUNTERS.with(|c| c.build_end_used.set(Some(budget.used())));

	summary.analysis_ns = elapsed_ns(started).saturating_sub(summary.compile_ns + summary.load_ns);
	let ready = b.ready;

	// Lookup tables: by function, then by header pc. Exact capacities (`region_metadata`
	// charges one row of each per loaded region).
	let mut order: StdVec<(u32, u32, u32)> = ready
		.iter()
		.enumerate()
		.map(|(i, r)| (r.region.function, r.region.header, i as u32))
		.collect();
	order.sort_unstable();
	let mut functions: StdVec<(u32, u32, u32)> = StdVec::with_capacity(order.len());
	let mut headers: StdVec<(u32, u32)> = StdVec::with_capacity(order.len());
	for (function, header, index) in order {
		let at = headers.len() as u32;
		headers.push((header, index));
		match functions.last_mut() {
			Some(last) if last.0 == function => last.2 = at + 1,
			_ => functions.push((function, at, at + 1)),
		}
	}

	ProgramRegions {
		code: b.id,
		functions,
		headers,
		ready,
		summary,
	}
}

// --------------------------------------------------------------------- test seams

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestCounters {
	pub(crate) checkouts: Cell<u64>,
	pub(crate) builds: Cell<u64>,
	/// Candidate headers analysed by builds (P3a streaming tests).
	pub(crate) analyzed_headers: Cell<u64>,
	/// The most fixpoint transfers one admitted header used.
	pub(crate) max_transfers: Cell<usize>,
	/// Header scans run by builds.
	pub(crate) scans: Cell<u64>,
	/// At the last header analysis error: `(budget used, bytes the scan list and the
	/// function context hold)`; equal when the failed header retained nothing.
	pub(crate) header_error_used: Cell<Option<(usize, usize)>>,
	/// The budget's charge when the last build finished (0 when nothing leaked).
	pub(crate) build_end_used: Cell<Option<usize>>,
}

#[cfg(test)]
std::thread_local! {
	/// Test builds: the production tier is off unless a test turns it on.
	static PRODUCTION_FOR_TEST: Cell<bool> = const { Cell::new(false) };

	pub(crate) static TEST_COUNTERS: TestCounters = TestCounters::default();

	/// Countdown to an injected panic inside `lower::compile`'s catch_unwind.
	static INJECT_COMPILE_PANIC: Cell<Option<u32>> = const { Cell::new(None) };

	/// Countdown to an injected panic in the build, outside catch_unwind: it fires before
	/// the compile of the admitted region after that many.
	static INJECT_BUILD_PANIC: Cell<Option<u32>> = const { Cell::new(None) };

	/// Scaled limits for this thread.
	static LIMITS_FOR_TEST: Cell<Option<Limits>> = const { Cell::new(None) };

	/// The page size this thread's tier observes instead of the system's.
	static PAGE_SIZE_FOR_TEST: Cell<Option<usize>> = const { Cell::new(None) };

	/// Native calls to corrupt into an invalid exit.
	static INJECT_INVALID_EXIT: Cell<u32> = const { Cell::new(0) };

	/// Set when an injected invalid exit is consumed; read by `invalid`.
	static INJECTED_INVALID_SEEN: Cell<bool> = const { Cell::new(false) };
}

/// `size_of::<Ready>()` (the B1 golden's metadata delta).
#[cfg(test)]
pub(crate) fn ready_bytes_for_test() -> usize {
	core::mem::size_of::<Ready>()
}

#[cfg(test)]
pub(crate) fn enable_for_test(on: bool) {
	PRODUCTION_FOR_TEST.with(|c| c.set(on));
}

#[cfg(test)]
pub(crate) fn counters_for_test() -> (u64, u64) {
	TEST_COUNTERS.with(|c| (c.checkouts.get(), c.builds.get()))
}

/// Whether this thread's tier has queried the environment and the page size (decided
/// `Enabled`) since the last `clear_for_test`.
#[cfg(test)]
pub(crate) fn enabled_decided_for_test() -> bool {
	CACHE.with(|cache| cache.borrow().enabled.is_some())
}

#[cfg(test)]
pub(crate) fn inject_compile_panic_for_test(after: Option<u32>) {
	INJECT_COMPILE_PANIC.with(|c| c.set(after));
}

#[cfg(test)]
fn injected_compile_panic() {
	let fire = INJECT_COMPILE_PANIC.with(|c| match c.get() {
		Some(0) => {
			c.set(None);
			true
		}
		Some(n) => {
			c.set(Some(n - 1));
			false
		}
		None => false,
	});
	if fire {
		panic!("injected compile panic (test)");
	}
}

#[cfg(test)]
pub(crate) fn inject_build_panic_for_test() {
	INJECT_BUILD_PANIC.with(|c| c.set(Some(0)));
}

/// Arms an injected build panic before the compile of the admitted region after `after`.
#[cfg(test)]
pub(crate) fn inject_build_panic_after_for_test(after: Option<u32>) {
	INJECT_BUILD_PANIC.with(|c| c.set(after));
}

#[cfg(test)]
fn injected_build_panic() {
	let fire = INJECT_BUILD_PANIC.with(|c| match c.get() {
		Some(0) => {
			c.set(None);
			true
		}
		Some(n) => {
			c.set(Some(n - 1));
			false
		}
		None => false,
	});
	if fire {
		panic!("injected build panic (test)");
	}
}

#[cfg(test)]
pub(crate) fn analyzed_headers_for_test() -> u64 {
	TEST_COUNTERS.with(|c| c.analyzed_headers.get())
}

/// Takes `(header_error_used, build_end_used)` of the builds since the last call.
#[cfg(test)]
pub(crate) fn budget_used_for_test() -> (Option<(usize, usize)>, Option<usize>) {
	TEST_COUNTERS.with(|c| (c.header_error_used.take(), c.build_end_used.take()))
}

#[cfg(test)]
pub(crate) fn scans_for_test() -> u64 {
	TEST_COUNTERS.with(|c| c.scans.get())
}

#[cfg(test)]
pub(crate) fn max_transfers_for_test() -> usize {
	TEST_COUNTERS.with(|c| c.max_transfers.get())
}

/// Scaled limits for this thread (`None`: production).
#[cfg(test)]
pub(crate) fn set_limits_for_test(limits: Option<Limits>) {
	LIMITS_FOR_TEST.with(|c| c.set(limits));
}

/// The page size this thread's tier observes (`None`: the system's). Read when the
/// thread's cache decides whether it is enabled (`clear_for_test` resets that).
#[cfg(test)]
pub(crate) fn set_page_size_for_test(size: Option<usize>) {
	PAGE_SIZE_FOR_TEST.with(|c| c.set(size));
}

/// `(mappings, bytes)` reserved in the whole process.
#[cfg(test)]
pub(crate) fn process_reserved_for_test() -> (usize, usize) {
	PROCESS.current()
}

/// The capacity of this thread's entry table.
#[cfg(test)]
pub(crate) fn entry_capacity_for_test() -> usize {
	CACHE.with(|cache| cache.borrow().entries.capacity())
}

#[cfg(test)]
pub(crate) fn inject_invalid_exit_for_test(calls: u32) {
	INJECT_INVALID_EXIT.with(|c| c.set(calls));
}

#[cfg(test)]
fn take_injected_invalid_exit() -> bool {
	INJECT_INVALID_EXIT.with(|c| match c.get() {
		0 => false,
		n => {
			c.set(n - 1);
			INJECTED_INVALID_SEEN.with(|s| s.set(true));
			true
		}
	})
}

/// Per-region counters of the resident entry of `code` (tests).
#[cfg(test)]
pub(crate) fn regions_for_test(code: u32) -> Option<StdVec<RegionCounters>> {
	CACHE.with(|cache| {
		let cache = cache.borrow();
		match &cache.entries[cache.position(code)?].resident {
			Resident::Ready(regions) => Some(regions.ready.iter().map(Ready::counters).collect()),
			Resident::Empty(_) => Some(StdVec::new()),
			Resident::Building | Resident::CheckedOut => None,
		}
	})
}

/// `(entries, mapped bytes, metadata bytes, in-flight entries)` of this thread's cache.
#[cfg(test)]
pub(crate) fn cache_state_for_test() -> (usize, usize, usize, usize) {
	CACHE.with(|cache| {
		let cache = cache.borrow();
		let in_flight = cache
			.entries
			.iter()
			.filter(|e| matches!(e.resident, Resident::Building | Resident::CheckedOut))
			.count();
		(cache.entries.len(), cache.mapped, cache.metadata, in_flight)
	})
}

/// Whether `code` is resident (ready or empty) in this thread's cache.
#[cfg(test)]
pub(crate) fn resident_for_test(code: u32) -> bool {
	CACHE.with(|cache| {
		let cache = cache.borrow();
		cache.position(code).is_some_and(|i| {
			matches!(
				cache.entries[i].resident,
				Resident::Ready(_) | Resident::Empty(_)
			)
		})
	})
}

/// Drops every entry of this thread's cache and re-reads the environment at next use.
#[cfg(test)]
pub(crate) fn clear_for_test() {
	let entries = CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		cache.mapped = 0;
		cache.metadata = 0;
		cache.enabled = None;
		core::mem::take(&mut cache.entries)
	});
	drop(entries);
}

#[cfg(test)]
mod miri_fallback_tests;
