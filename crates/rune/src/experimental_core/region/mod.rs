//! P0 native loop regions (feature `fresh-core-native`): eligibility, type inference, the
//! exit contract and a safe simulator of the native semantics. Everything here is safe
//! Rust under the fresh core's `forbid(unsafe_code)`; the Cranelift backend and the one
//! unsafe boundary live outside this module (`crate::experimental_native`).
//!
//! # Contract (summary; the full text is the P0 contract document)
//!
//! - **Region.** A natural loop whose header is a charged back-edge target, with no side
//!   entry, whose reached instructions are all `Literal` (`I64`/`Bool`), `Binary` or
//!   `AssignBinary` with an op in {Add, Sub, Mul, Div, Rem, Eq, Ne, Lt, Le, Gt, Ge} on
//!   proven `I64` operands, `Branch` on a proven `Bool`, or `Jump`. Anything else refuses
//!   the region; the interpreter runs it unchanged.
//! - **Entry** happens at the header, *before* the interpreter's own cancel/fuel step for
//!   that instruction, and only when no cancel is pending, the store is open with no queued
//!   work, every guarded live-in slot holds its inferred tag, and every other touched slot
//!   holds a value whose release is a no-op (`Store::is_slot_scalar`). Otherwise the
//!   interpreter executes the header normally: no state is changed by a refused entry.
//! - **Fuel.** The region charges exactly the interpreter's charged pcs, the header
//!   included, each once per arrival. At a charged pc with fuel 0 it exits `Fuel` with the
//!   pc *before* charging and fuel 0. A `Retry` exit restores the fuel the instruction had
//!   *before* its own charge, so the interpreter, which re-executes that instruction,
//!   charges it exactly once. The interpreter never consults the region at the pc it
//!   resumes at, so a header is never charged twice and a `Fuel` exit at the header cannot
//!   loop.
//! - **State commit.** Every exit describes the state *before* the exiting instruction
//!   (`Fuel`, `Retry`) or after the last executed one (`Continue`). Only slots whose
//!   written bit is set are written back, with their statically known tag; every other slot
//!   keeps its original value. The written bit is exact per path (a dynamic mask), and an
//!   exit whose possibly-written slot has no single scalar type refuses the region.
//! - **Errors** are never synthesized: a faulting op exits `Retry` and the interpreter
//!   re-executes it, producing the identical error, location, call chain and cleanup.
// Wired into execution (P2a) through `cache` on x86-64 Linux only; the simulator and the
// listing helpers are used by tests only.
#![cfg_attr(not(test), allow(dead_code))]
use super::arithmetic::Binary;
use super::code::{self, Function, Op};
use super::error::{Error, ErrorKind};
use super::value::Value;
use crate::alloc::Vec;
use budget::{AnalysisPod, BVec, Budget, Grow};

pub(crate) mod abi;
pub(crate) mod budget;
pub(crate) mod commit;
pub(crate) mod simulate;

#[cfg(all(feature = "fresh-core-native", target_arch = "x86_64", target_os = "linux"))]
pub(crate) mod cache;

#[cfg(test)]
pub(crate) mod hook;
#[cfg(test)]
mod tests;

/// Fallible-clone impls for the `Copy` lattice types (rune_alloc's resize needs them).
macro_rules! try_copy {
    ($($t:ty),*) => {$(
        impl crate::alloc::clone::TryClone for $t {
            fn try_clone(&self) -> Result<Self, crate::alloc::Error> {
                Ok(*self)
            }
        }
    )*};
}

try_copy!(SlotState, Scalar, FTy);

/// The analysis element types a budgeted vector may hold (all `Copy`, no owned heap).
macro_rules! analysis_pod {
	($($t:ty),*) => {$(
		impl AnalysisPod for $t {}
	)*};
}

analysis_pod!(SlotState, Option<Scalar>, FTy, Op, Exit, Node, EntryClass);

/// Most instructions in one region (the natural loop's body).
pub(crate) const MAX_REGION_INSTRUCTIONS: usize = 1024;
/// Most distinct frame slots one region may touch: the written mask is one `u64`.
pub(crate) const MAX_REGION_SLOTS: usize = 64;
/// Most exits of one region (bounded by 4 per instruction: fuel, retry, two edges).
pub(crate) const MAX_REGION_EXITS: usize = 4 * MAX_REGION_INSTRUCTIONS;
/// Most regions analysed in one function; later headers are not analysed.
pub(crate) const MAX_REGIONS_PER_FUNCTION: usize = 64;
/// Function-level inference runs only when `instructions * slots` stays below this.
pub(crate) const MAX_FUNCTION_INFERENCE_CELLS: usize = 1 << 22;

/// A scalar the native region holds unboxed in an `i64`: `I64` as itself, `Bool` as
/// exactly 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scalar {
	I64,
	Bool,
}

/// Value-type lattice of the region inference: `Bottom < {I64, Bool} < Top`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
	Bottom,
	I64,
	Bool,
	Top,
}

impl Ty {
	pub(crate) fn join(self, other: Self) -> Self {
		match (self, other) {
			(Self::Bottom, x) | (x, Self::Bottom) => x,
			(a, b) if a == b => a,
			_ => Self::Top,
		}
	}
	fn of(s: Scalar) -> Self {
		match s {
			Scalar::I64 => Self::I64,
			Scalar::Bool => Self::Bool,
		}
	}
}

/// Written-state lattice: has the region written this slot since entry?
/// `No` and `Yes` join to `Maybe` (path-dependent; tracked by the dynamic written mask).
/// Unreached program points carry no state at all (the implicit bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Written {
	No,
	Yes,
	Maybe,
}

impl Written {
	pub(crate) fn join(self, other: Self) -> Self {
		if self == other {
			self
		} else {
			Self::Maybe
		}
	}
}

/// Constant lattice for values written *inside* the region: `Bottom < Const(c) < Top`.
/// Entry values are never constants, whatever the function-level inference says, so a
/// runtime-dependent argument is never folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Konst {
	Bottom,
	Const(i64),
	Top,
}

impl Konst {
	pub(crate) fn join(self, other: Self) -> Self {
		match (self, other) {
			(Self::Bottom, x) | (x, Self::Bottom) => x,
			(a, b) if a == b => a,
			_ => Self::Top,
		}
	}
}

/// The abstract state of one touched slot at one program point. `ty` and `konst` describe
/// the value the region wrote on the paths where it wrote one (`Bottom` if none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotState {
	pub(crate) written: Written,
	pub(crate) ty: Ty,
	pub(crate) konst: Konst,
}

impl SlotState {
	const ENTRY: Self = Self {
		written: Written::No,
		ty: Ty::Bottom,
		konst: Konst::Bottom,
	};
	pub(crate) fn join(self, other: Self) -> Self {
		Self {
			written: self.written.join(other.written),
			ty: self.ty.join(other.ty),
			konst: self.konst.join(other.konst),
		}
	}
}

/// Exit status discriminants, shared with the native ABI ([`abi::ExitRecord::status`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum Status {
	/// Control left the region along an edge; resume at the edge's target, not yet
	/// executed or charged.
	Continue = 1,
	/// A charged pc found fuel 0; resume at that pc, before its charge, with fuel 0.
	Fuel = 2,
	/// An op would fault; resume at that pc with the fuel it had before its own charge.
	Retry = 3,
}

impl Status {
	pub(crate) fn from_u32(n: u32) -> Option<Self> {
		match n {
			1 => Some(Self::Continue),
			2 => Some(Self::Fuel),
			3 => Some(Self::Retry),
			_ => None,
		}
	}
}

/// One statically known exit site and its write-back map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exit {
	pub(crate) status: Status,
	/// The interpreter pc to resume at.
	pub(crate) pc: u32,
	/// Local slots written on every path to this exit.
	pub(crate) definite: u64,
	/// Local slots written on some but not all paths (the dynamic mask decides).
	pub(crate) maybe: u64,
	/// Among `definite | maybe`, the slots whose written value is statically a `Bool`.
	pub(crate) bools: u64,
	/// Among `definite | maybe`, the slots written as `I64` on some paths and `Bool` on
	/// others (temporaries reused across types): their tag comes from the dynamic bool
	/// mask ([`abi::ExitRecord::bools`]), exact per path like the written mask.
	pub(crate) dynamic: u64,
	/// A `Retry` at a charged pc: native code must report the pre-charge fuel.
	pub(crate) restore: bool,
}

/// How a touched slot is checked at entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryClass {
	/// A live-in: its tag must equal this scalar, and its payload is unboxed.
	Guard(Scalar),
	/// Written before any read on every path: its current value must be one whose release
	/// is a no-op (`Store::is_slot_scalar`); it is not read by the region.
	Scalar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
	Node(u32),
	Exit(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeOp {
	Literal {
		out: u32,
		value: i64,
		ty: Scalar,
	},
	/// Add, Sub, Mul, Div, Rem on I64. `retry` is the exit taken when the op would fault;
	/// `None` only for Div/Rem by an in-region constant outside {0, -1}, kept in `divisor`.
	Arith {
		out: u32,
		op: Binary,
		lhs: u32,
		rhs: u32,
		retry: Option<u32>,
		divisor: Option<i64>,
	},
	/// Eq, Ne, Lt, Le, Gt, Ge on I64, producing a Bool (0 or 1).
	Compare {
		out: u32,
		op: Binary,
		lhs: u32,
		rhs: u32,
	},
	Branch {
		condition: u32,
		yes: Target,
		no: Target,
	},
	Jump {
		target: Target,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Node {
	pub(crate) pc: u32,
	/// The `Fuel` exit of a charged pc.
	pub(crate) charged: Option<u32>,
	pub(crate) op: NodeOp,
	/// Fall-through successor of a non-control op.
	pub(crate) next: Option<Target>,
}

/// An admitted region. Local slot `i` is frame slot `slots[i]`.
pub(crate) struct Region {
	/// Slots whose dynamic bool bit the region maintains (the union of exits' `dynamic`).
	pub(crate) tracked: u64,
	pub(crate) function: u32,
	pub(crate) header: u32,
	pub(crate) header_node: u32,
	pub(crate) slots: Vec<u32>,
	pub(crate) entry: Vec<EntryClass>,
	/// Guards the function-level inference also proves (reported; still checked).
	pub(crate) proven: u64,
	pub(crate) nodes: Vec<Node>,
	pub(crate) exits: Vec<Exit>,
}

/// Why a candidate header did not produce a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionRefusal {
	/// The header is not a charged instruction (unreachable for validated code).
	NotCharged,
	/// The loop body exceeds [`MAX_REGION_INSTRUCTIONS`].
	Size,
	/// A body instruction other than the header has a predecessor outside the body
	/// (irreducible or multi-entry loop), or is the function entry.
	SideEntry { pc: u32 },
	/// A body instruction outside the admitted set.
	Op { pc: u32 },
	/// More than [`MAX_REGION_SLOTS`] slots touched.
	Slots,
	/// One live-in slot read with two different demanded types.
	LiveIn { pc: u32, slot: u32 },
	/// An operand whose type is not proven to be the one the op demands.
	Type { pc: u32 },
	/// More than [`MAX_REGION_EXITS`] exits.
	Exits,
	/// The analysis transient-bytes budget refused one of this header's allocations.
	Transient,
	/// The region fixpoint reached the [`TRANSFER_FACTOR`] cutoff.
	Steps,
}

fn alloc_err() -> Error {
	Error::new(ErrorKind::Allocation, "native region analysis")
}

fn invalid() -> Error {
	Error::new(ErrorKind::InvalidCode, "native region analysis")
}

const ARITH: [Binary; 5] = [
	Binary::Add,
	Binary::Sub,
	Binary::Mul,
	Binary::Div,
	Binary::Rem,
];
const COMPARE: [Binary; 6] = [
	Binary::Eq,
	Binary::Ne,
	Binary::Lt,
	Binary::Le,
	Binary::Gt,
	Binary::Ge,
];

/// Slot reads of an admitted op, with the type each read demands.
fn reads(op: Op) -> [Option<(u32, Scalar)>; 2] {
	match op {
		Op::Binary { lhs, rhs, .. } | Op::AssignBinary { lhs, rhs, .. } => {
			[Some((lhs, Scalar::I64)), Some((rhs, Scalar::I64))]
		}
		Op::Branch { condition, .. } => [Some((condition, Scalar::Bool)), None],
		_ => [None, None],
	}
}

/// The slot an admitted op writes.
fn write(op: Op) -> Option<u32> {
	match op {
		Op::Literal { out, .. } | Op::Binary { out, .. } | Op::AssignBinary { out, .. } => {
			Some(out)
		}
		_ => None,
	}
}

fn admitted(op: Op) -> bool {
	match op {
		Op::Literal { value, .. } => matches!(value, Value::I64(_) | Value::Bool(_)),
		Op::Binary { op, .. } | Op::AssignBinary { op, .. } => {
			ARITH.contains(&op) || COMPARE.contains(&op)
		}
		Op::Branch { .. } | Op::Jump { .. } => true,
		_ => false,
	}
}

fn binary_parts(op: Op) -> Option<(u32, Binary, u32, u32)> {
	match op {
		Op::Binary { out, op, lhs, rhs } | Op::AssignBinary { out, op, lhs, rhs } => {
			Some((out, op, lhs, rhs))
		}
		_ => None,
	}
}

fn successors(f: &Function, pc: usize) -> Result<[Option<usize>; 2], Error> {
	let mut out = [None, None];
	code::successors(f, pc, &mut out)?;
	Ok(out)
}

/// Why an analysis step stopped early: an error of the analysis itself (allocation or
/// invalid code), or the transient-bytes [`Budget`].
#[derive(Debug)]
pub(crate) enum Stop {
	Error(Error),
	Budget,
}

impl From<Error> for Stop {
	fn from(error: Error) -> Self {
		Self::Error(error)
	}
}

impl From<Grow> for Stop {
	fn from(grow: Grow) -> Self {
		match grow {
			Grow::Alloc => Self::Error(alloc_err()),
			Grow::Budget => Self::Budget,
			// Counted by `Budget::violations`; an analysis error like an allocation failure.
			Grow::Invariant => Self::Error(alloc_err()),
		}
	}
}

/// The fixpoint transfer cutoff factor: a region fixpoint may run at most
/// `TRANSFER_FACTOR * body * max(slots, 1)` transfers, and the function-level inference at
/// most `TRANSFER_FACTOR * instructions * max(slots, 1)`. This is an OPERATIONAL cutoff, not
/// a proof of the fixpoints' complexity. (A lattice-height argument suggests about
/// `body * (1 + 6 * slots)` region transfers and `len * (1 + 2 * slots)` function
/// transfers, below the cutoff, but that argument is not relied on.) Hitting it refuses the
/// region (`RegionRefusal::Steps`) or abandons the function-level inference (as past
/// [`MAX_FUNCTION_INFERENCE_CELLS`]); both fall back to the interpreter normally.
pub(crate) const TRANSFER_FACTOR: usize = 8;

fn stop_error(stop: Stop) -> Error {
	match stop {
		Stop::Error(error) => error,
		Stop::Budget => alloc_err(),
	}
}

/// `factor * a * max(b, 1)`, or no cutoff when that overflows.
pub(crate) fn cutoff(factor: usize, a: usize, b: usize) -> usize {
	factor
		.checked_mul(a)
		.and_then(|x| x.checked_mul(b.max(1)))
		.unwrap_or(usize::MAX)
}

/// Every back-edge target of `f`, in pc order: the candidate headers. One linear pass with
/// a bitmap; both buffers are charged to `budget` before they are allocated.
pub(crate) fn scan_headers<'b>(f: &Function, budget: &'b Budget) -> Result<BVec<'b, u32>, Stop> {
	let len = f.len();
	let mut seen: BVec<'b, bool> = BVec::new(budget);
	seen.resize(len, false)?;
	let mut count = 0usize;
	for pc in 0..len {
		for target in successors(f, pc)?.into_iter().flatten() {
			if target <= pc && !seen[target] {
				seen[target] = true;
				count += 1;
			}
		}
	}

	let mut headers: BVec<'b, u32> = BVec::with_capacity(budget, count)?;
	for (pc, &is_header) in seen.iter().enumerate() {
		if is_header {
			headers.push(u32::try_from(pc).map_err(|_| invalid())?)?;
		}
	}
	Ok(headers)
}

/// Analyses every candidate header of function `function` (at most
/// [`MAX_REGIONS_PER_FUNCTION`]) without a byte limit. `Err` is an allocation or
/// invalid-code failure of the analysis itself, which the caller treats as "no regions"
/// (interpreter fallback). Tests and the P1 hook use it; the production cache streams the
/// same steps ([`scan_headers`], [`FunctionContext::new`], [`FunctionContext::analyze_header`])
/// under its budget instead.
pub(crate) fn analyze(
	f: &Function,
	function: u32,
) -> Result<Vec<(u32, Result<Region, RegionRefusal>)>, Error> {
	let budget = Budget::unlimited();
	let headers = scan_headers(f, &budget).map_err(stop_error)?;
	let context = FunctionContext::new(f, function, &budget, TRANSFER_FACTOR).map_err(stop_error)?;
	let mut out = Vec::new();
	for &header in headers.iter().take(MAX_REGIONS_PER_FUNCTION) {
		let region = match context.analyze_header(header) {
			Ok(found) => found.map(|(region, _)| region),
			Err(Stop::Budget) => Err(RegionRefusal::Transient),
			Err(Stop::Error(error)) => return Err(error),
		};
		out.try_push((header, region)).map_err(|_| alloc_err())?;
	}
	Ok(out)
}

fn index_of(body: &[u32], pc: usize) -> Option<usize> {
	let pc = u32::try_from(pc).ok()?;
	body.binary_search(&pc).ok()
}

/// The function-level state every header of one function shares, built once per function:
/// the sorted predecessor arcs and the function-level type inference. Both are charged to
/// the budget for as long as the context lives.
pub(crate) struct FunctionContext<'f, 'b> {
	f: &'f Function,
	function: u32,
	budget: &'b Budget,
	/// `(target, source)` of every successor arc, sorted by target.
	edges: BVec<'b, (u32, u32)>,
	/// The function-level inference, `instructions * slots` cells, if it ran.
	types: Option<BVec<'b, FTy>>,
	factor: usize,
	/// The most fixpoint transfers one header used (tests and receipts).
	max_transfers: core::cell::Cell<usize>,
}

impl<'f, 'b> FunctionContext<'f, 'b> {
	/// Builds the predecessor arcs (exact capacity, counted first) and runs the
	/// function-level inference, every buffer charged before it is allocated. `factor` is
	/// the transfer cutoff factor ([`TRANSFER_FACTOR`] in production).
	pub(crate) fn new(f: &'f Function, function: u32, budget: &'b Budget, factor: usize) -> Result<Self, Stop> {
		let len = f.len();
		let mut count = 0usize;
		for pc in 0..len {
			count += successors(f, pc)?.into_iter().flatten().count();
		}

		let mut edges: BVec<'b, (u32, u32)> = BVec::with_capacity(budget, count)?;
		for pc in 0..len {
			for t in successors(f, pc)?.into_iter().flatten() {
				edges.push((t as u32, pc as u32))?;
			}
		}
		edges.sort_unstable();

		let types = function_types_in(f, budget, factor)?;

		Ok(Self {
			f,
			function,
			budget,
			edges,
			types,
			factor,
			max_transfers: core::cell::Cell::new(0),
		})
	}

	fn preds(&self, pc: u32) -> &[(u32, u32)] {
		let start = self.edges.partition_point(|e| e.0 < pc);
		let end = self.edges.partition_point(|e| e.0 <= pc);
		&self.edges[start..end]
	}

	/// The function-level type state at `header`, if that inference ran.
	fn static_at(&self, header: u32) -> Option<&[FTy]> {
		let n = self.f.slots as usize;
		self.types
			.as_ref()
			.map(|t| &t[header as usize * n..][..n])
	}

	/// The most fixpoint transfers any header of this context used so far.
	pub(crate) fn max_transfers(&self) -> usize {
		self.max_transfers.get()
	}

	/// The bytes this context itself holds charged (arcs and inference; tests).
	#[cfg(test)]
	pub(crate) fn charged_for_test(&self) -> usize {
		self.edges.charged() + self.types.as_ref().map_or(0, |t| t.charged())
	}

	/// One candidate header. On success, the region and the bytes of its four vectors
	/// (slots, entry, nodes, exits; by capacity), which stay charged to the budget: the
	/// caller adopts them (`budget::Held::adopt`). `Err(Stop::Budget)`: the transient-bytes
	/// budget refused an allocation (nothing of this header stays charged).
	pub(crate) fn analyze_header(&self, header: u32) -> Result<Result<(Region, usize), RegionRefusal>, Stop> {
		let f = self.f;
		let budget = self.budget;
		let function = self.function;
		let len = f.len();
		let h = header as usize;
		if h >= len {
			return Err(invalid().into());
		}
		if !f.instructions()[h].is_charged() {
			return Ok(Err(RegionRefusal::NotCharged));
		}

		// Forward reachability from the header, so the backward walk stays inside the part
		// of the function the header can reach (a side entry is then seen at the entered
		// node).
		let mut forward: BVec<'b, bool> = BVec::new(budget);
		forward.resize(len, false)?;
		let mut stack: BVec<'b, u32> = BVec::new(budget);
		forward[h] = true;
		stack.push(header)?;
		while let Some(pc) = stack.pop() {
			for q in successors(f, pc as usize)?.into_iter().flatten() {
				if !forward[q] {
					forward[q] = true;
					stack.push(q as u32)?;
				}
			}
		}

		// Natural loop: the header plus every header-reachable instruction that reaches a
		// back-edge source without passing through the header.
		let mut in_body: BVec<'b, bool> = BVec::new(budget);
		in_body.resize(len, false)?;
		in_body[h] = true;
		let mut size = 1usize;
		for &(_, source) in self.preds(header) {
			if source >= header && forward[source as usize] {
				stack.push(source)?;
			}
		}
		while let Some(pc) = stack.pop() {
			if in_body[pc as usize] {
				continue;
			}
			in_body[pc as usize] = true;
			size += 1;
			if size > MAX_REGION_INSTRUCTIONS {
				return Ok(Err(RegionRefusal::Size));
			}
			for &(_, source) in self.preds(pc) {
				if !in_body[source as usize] && forward[source as usize] {
					stack.push(source)?;
				}
			}
		}
		drop(stack);
		drop(forward);

		let mut body: BVec<'b, u32> = BVec::with_capacity(budget, size)?;
		for pc in 0..len {
			if in_body[pc] {
				body.push(pc as u32)?;
			}
		}

		// Side entries: every non-header body instruction is entered only from the body (so
		// the header dominates it), and the function entry (pc 0) is an entry of its own.
		for &pc in body.iter() {
			if pc == header {
				continue;
			}
			if pc == 0 || self.preds(pc).iter().any(|&(_, s)| !in_body[s as usize]) {
				return Ok(Err(RegionRefusal::SideEntry { pc }));
			}
		}
		drop(in_body);

		// Admission and touched slots.
		let mut ops: BVec<'b, Op> = BVec::with_capacity(budget, body.len())?;
		let mut slots: BVec<'b, u32> = BVec::new(budget);
		for &pc in body.iter() {
			let op = f.operation(pc as usize)?;
			if !admitted(op) {
				return Ok(Err(RegionRefusal::Op { pc }));
			}
			for slot in reads(op)
				.into_iter()
				.flatten()
				.map(|(s, _)| s)
				.chain(write(op))
			{
				if !slots.contains(&slot) {
					if slots.len() >= MAX_REGION_SLOTS {
						return Ok(Err(RegionRefusal::Slots));
					}
					slots.push(slot)?;
				}
			}
			ops.push(op)?;
		}
		slots.sort_unstable();
		let n = slots.len();
		let local = |slot: u32| slots.binary_search(&slot).map_err(|_| invalid());
		let header_index = index_of(&body, h).ok_or_else(invalid)?;

		// Test seam: an analysis error mid-header, with the body, ops and slots buffers
		// charged and alive. It can only produce an `Err`.
		#[cfg(test)]
		if take_injected_analysis_error() {
			return Err(invalid().into());
		}

		// Forward fixpoint over (written, written type, written constant) per slot. The
		// work list holds each body index at most once (the `in_work` bitmap), so its
		// capacity is the body length.
		let cells = body.len().checked_mul(n).ok_or(Stop::Budget)?;
		let mut states: BVec<'b, SlotState> = BVec::new(budget);
		states.resize(cells, SlotState::ENTRY)?;
		let mut reached: BVec<'b, bool> = BVec::new(budget);
		reached.resize(body.len(), false)?;
		reached[header_index] = true;
		let mut work: BVec<'b, usize> = BVec::with_capacity(budget, body.len())?;
		let mut in_work: BVec<'b, bool> = BVec::new(budget);
		in_work.resize(body.len(), false)?;
		work.push(header_index)?;
		in_work[header_index] = true;
		let mut scratch: BVec<'b, SlotState> = BVec::new(budget);
		scratch.resize(n, SlotState::ENTRY)?;
		let limit = cutoff(self.factor, body.len(), n);
		let mut transfers = 0usize;
		while let Some(i) = work.pop() {
			in_work[i] = false;
			transfers += 1;
			if transfers > limit {
				return Ok(Err(RegionRefusal::Steps));
			}
			scratch.copy_from_slice(&states[i * n..(i + 1) * n]);
			transfer(ops[i], &slots, &mut scratch)?;
			for q in successors(f, body[i] as usize)?.into_iter().flatten() {
				let Some(j) = index_of(&body, q) else {
					continue;
				};
				let target = &mut states[j * n..(j + 1) * n];
				let changed = if reached[j] {
					let mut changed = false;
					for (t, s) in target.iter_mut().zip(scratch.iter()) {
						let joined = t.join(*s);
						changed |= joined != *t;
						*t = joined;
					}
					changed
				} else {
					reached[j] = true;
					target.copy_from_slice(&scratch);
					true
				};
				if changed && !in_work[j] {
					in_work[j] = true;
					work.push(j)?;
				}
			}
		}
		if transfers > self.max_transfers.get() {
			self.max_transfers.set(transfers);
		}
		drop(work);
		drop(in_work);

		// Live-in demands: a read that can observe the entry value fixes that slot's guard.
		let mut demand: BVec<'b, Option<Scalar>> = BVec::new(budget);
		demand.resize(n, None)?;
		for i in (0..body.len()).filter(|&i| reached[i]) {
			for (slot, want) in reads(ops[i]).into_iter().flatten() {
				let l = local(slot)?;
				if states[i * n + l].written != Written::Yes {
					match demand[l] {
						None => demand[l] = Some(want),
						Some(d) if d == want => (),
						Some(_) => return Ok(Err(RegionRefusal::LiveIn { pc: body[i], slot })),
					}
				}
			}
		}
		let entry_ty = |l: usize| demand[l].map_or(Ty::Top, Ty::of);
		let current = |s: SlotState, l: usize| match s.written {
			Written::No => entry_ty(l),
			Written::Yes => s.ty,
			Written::Maybe => entry_ty(l).join(s.ty),
		};
		// Every read must see exactly the demanded scalar type.
		for i in (0..body.len()).filter(|&i| reached[i]) {
			for (slot, want) in reads(ops[i]).into_iter().flatten() {
				let l = local(slot)?;
				if current(states[i * n + l], l) != Ty::of(want) {
					return Ok(Err(RegionRefusal::Type { pc: body[i] }));
				}
			}
		}

		// Nodes (reached only) and exits.
		let mut node_of: BVec<'b, Option<u32>> = BVec::new(budget);
		node_of.resize(body.len(), None)?;
		let mut count = 0u32;
		for i in 0..body.len() {
			if reached[i] {
				node_of[i] = Some(count);
				count += 1;
			}
		}
		let mut exits: BVec<'b, Exit> = BVec::new(budget);
		let mut nodes: BVec<'b, Node> = BVec::new(budget);
		let mut push_exit = |status: Status,
		                     pc: u32,
		                     _at: u32,
		                     state: &[SlotState],
		                     restore: bool|
		 -> Result<Result<u32, RegionRefusal>, Stop> {
			let (mut definite, mut maybe, mut bools, mut dynamic) = (0u64, 0u64, 0u64, 0u64);
			for (l, s) in state.iter().enumerate() {
				let bit = 1u64 << l;
				match s.written {
					Written::No => continue,
					Written::Yes => definite |= bit,
					Written::Maybe => maybe |= bit,
				}
				// Written values are only I64 or Bool, so Top here means "I64 on some paths,
				// Bool on others", which the dynamic bool mask records exactly.
				match s.ty {
					Ty::Bool => bools |= bit,
					Ty::I64 => (),
					Ty::Top => dynamic |= bit,
					Ty::Bottom => return Err(invalid().into()),
				}
			}
			if exits.len() >= MAX_REGION_EXITS {
				return Ok(Err(RegionRefusal::Exits));
			}
			let id = exits.len() as u32;
			exits.push(Exit {
				status,
				pc,
				definite,
				maybe,
				bools,
				dynamic,
				restore,
			})?;
			Ok(Ok(id))
		};
		for i in (0..body.len()).filter(|&i| reached[i]) {
			let pc = body[i];
			let state = &states[i * n..(i + 1) * n];
			let charged_pc = f.instructions()[pc as usize].is_charged();
			let charged = if charged_pc {
				match push_exit(Status::Fuel, pc, pc, state, false)? {
					Ok(id) => Some(id),
					Err(refusal) => return Ok(Err(refusal)),
				}
			} else {
				None
			};
			// Arithmetic first: its Retry exit sees the state before the op.
			let mut arith = None;
			if let Some((out, op, lhs, rhs)) = binary_parts(ops[i]) {
				if ARITH.contains(&op) {
					let r = local(rhs)?;
					// A divisor is constant only when written by an in-region literal on
					// every path; entry values never are.
					let rs = state[r];
					let divisor = match (op, rs.written, rs.konst) {
						(Binary::Div | Binary::Rem, Written::Yes, Konst::Const(c))
							if c != 0 && c != -1 =>
						{
							Some(c)
						}
						_ => None,
					};
					let retry = if divisor.is_some() {
						None
					} else {
						match push_exit(Status::Retry, pc, pc, state, charged_pc)? {
							Ok(id) => Some(id),
							Err(refusal) => return Ok(Err(refusal)),
						}
					};
					arith = Some(NodeOp::Arith {
						out: local(out)? as u32,
						op,
						lhs: local(lhs)? as u32,
						rhs: r as u32,
						retry,
						divisor,
					});
				}
			}
			scratch.copy_from_slice(state);
			transfer(ops[i], &slots, &mut scratch)?;
			let mut target = |q: usize| -> Result<Result<Target, RegionRefusal>, Stop> {
				Ok(match index_of(&body, q) {
					Some(j) => Ok(Target::Node(node_of[j].ok_or_else(invalid)?)),
					None => {
						let q = u32::try_from(q).map_err(|_| invalid())?;
						push_exit(Status::Continue, q, pc, &scratch, false)?.map(Target::Exit)
					}
				})
			};
			macro_rules! target {
				($q:expr) => {
					match target($q)? {
						Ok(t) => t,
						Err(refusal) => return Ok(Err(refusal)),
					}
				};
			}
			let fall = pc as usize + 1;
			let (op, next) = match ops[i] {
				Op::Literal { out, value } => {
					let (value, ty) = match value {
						Value::I64(v) => (v, Scalar::I64),
						Value::Bool(b) => (i64::from(b), Scalar::Bool),
						_ => return Err(invalid().into()),
					};
					let out = local(out)? as u32;
					(NodeOp::Literal { out, value, ty }, Some(target!(fall)))
				}
				Op::Jump { target } => (
					NodeOp::Jump {
						target: target!(target as usize),
					},
					None,
				),
				Op::Branch { condition, yes, no } => {
					let yes = target!(yes as usize);
					let no = target!(no as usize);
					(
						NodeOp::Branch {
							condition: local(condition)? as u32,
							yes,
							no,
						},
						None,
					)
				}
				other => {
					let (out, op, lhs, rhs) = binary_parts(other).ok_or_else(invalid)?;
					let next = Some(target!(fall));
					match arith {
						Some(node) => (node, next),
						None => (
							NodeOp::Compare {
								out: local(out)? as u32,
								op,
								lhs: local(lhs)? as u32,
								rhs: local(rhs)? as u32,
							},
							next,
						),
					}
				}
			};
			nodes.push(Node {
				pc,
				charged,
				op,
				next,
			})?;
		}
		let mut entry: BVec<'b, EntryClass> = BVec::new(budget);
		let mut proven = 0u64;
		let static_at = self.static_at(header);
		for l in 0..n {
			let class = match demand[l] {
				Some(s) => {
					if let Some(at) = static_at {
						let want = match s {
							Scalar::I64 => FTy::I64,
							Scalar::Bool => FTy::Bool,
						};
						if at.get(slots[l] as usize) == Some(&want) {
							proven |= 1 << l;
						}
					}
					EntryClass::Guard(s)
				}
				None => EntryClass::Scalar,
			};
			entry.push(class)?;
		}
		let tracked = exits.iter().fold(0, |m, e| m | e.dynamic);
		let header_node = node_of[header_index].ok_or_else(invalid)?;

		let (slots, slots_bytes) = slots.into_parts();
		let (entry, entry_bytes) = entry.into_parts();
		let (nodes, nodes_bytes) = nodes.into_parts();
		let (exits, exits_bytes) = exits.into_parts();
		let region = Region {
			tracked,
			function,
			header,
			header_node,
			slots,
			entry,
			proven,
			nodes,
			exits,
		};
		Ok(Ok((region, slots_bytes + entry_bytes + nodes_bytes + exits_bytes)))
	}
}

#[cfg(test)]
std::thread_local! {
	/// Countdown to an injected analysis error: it fires in the analysis of the header after
	/// that many headers reached the injection point.
	static INJECT_ANALYSIS_ERROR: core::cell::Cell<Option<u32>> = const { core::cell::Cell::new(None) };
}

/// Arms an analysis error in the header analysis after `after` more headers reach the
/// injection point (`None` disarms). Tests only.
#[cfg(test)]
pub(crate) fn inject_analysis_error_for_test(after: Option<u32>) {
	INJECT_ANALYSIS_ERROR.with(|c| c.set(after));
}

#[cfg(test)]
fn take_injected_analysis_error() -> bool {
	INJECT_ANALYSIS_ERROR.with(|c| match c.get() {
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

/// The divisor kind of a region's first fallible Div/Rem: a slot written in the region
/// only by literals of one value is that constant, anything else is a runtime divisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DivisionKind {
	Runtime,
	Const0,
	ConstNeg1,
}

pub(crate) fn division_kind(region: &Region) -> Option<DivisionKind> {
	let rhs = region.nodes.iter().find_map(|n| match n.op {
		NodeOp::Arith {
			op: Binary::Div | Binary::Rem,
			rhs,
			retry: Some(_),
			..
		} => Some(rhs),
		_ => None,
	})?;
	let mut values = region.nodes.iter().filter_map(|n| match n.op {
		NodeOp::Literal { out, value, .. } if out == rhs => Some(value),
		_ => None,
	});
	let first = values.next();
	let single = first.filter(|&v| values.all(|w| w == v));
	Some(match single {
		Some(0) => DivisionKind::Const0,
		Some(-1) => DivisionKind::ConstNeg1,
		_ => DivisionKind::Runtime,
	})
}

/// Region transfer: writes set (Yes, type by op class, constant for literals). The result
/// type depends only on the op class; operand types are checked after the fixpoint.
fn transfer(op: Op, slots: &[u32], state: &mut [SlotState]) -> Result<(), Error> {
	let Some(out) = write(op) else {
		return Ok(());
	};
	let l = slots.binary_search(&out).map_err(|_| invalid())?;
	state[l] = match op {
		Op::Literal {
			value: Value::I64(v),
			..
		} => SlotState {
			written: Written::Yes,
			ty: Ty::I64,
			konst: Konst::Const(v),
		},
		Op::Literal {
			value: Value::Bool(b),
			..
		} => SlotState {
			written: Written::Yes,
			ty: Ty::Bool,
			konst: Konst::Const(i64::from(b)),
		},
		_ => {
			let (_, op, _, _) = binary_parts(op).ok_or_else(invalid)?;
			SlotState {
				written: Written::Yes,
				ty: if COMPARE.contains(&op) {
					Ty::Bool
				} else {
					Ty::I64
				},
				konst: Konst::Top,
			}
		}
	};
	Ok(())
}

/// Function-level type lattice: `Bottom < {Unit, Bool, I64} < Top`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FTy {
	Bottom,
	Unit,
	Bool,
	I64,
	Top,
}

impl FTy {
	pub(crate) fn join(self, other: Self) -> Self {
		match (self, other) {
			(Self::Bottom, x) | (x, Self::Bottom) => x,
			(a, b) if a == b => a,
			_ => Self::Top,
		}
	}
}

/// Whole-function forward inference from pc 0 (parameters `Top`, other slots `Unit`), over
/// the normal successor arcs only: run-time errors end the invocation and `Try`'s error arm
/// returns from the function, so no exceptional arc reaches another program point. Unreached
/// points stay `Bottom`. Returns `None` past [`MAX_FUNCTION_INFERENCE_CELLS`]. Unbudgeted
/// (tests); production runs [`function_types_in`] under its budget.
pub(crate) fn function_types(f: &Function) -> Result<Option<Vec<FTy>>, Error> {
	let budget = Budget::unlimited();
	let types = function_types_in(f, &budget, TRANSFER_FACTOR).map_err(stop_error)?;
	Ok(types.map(|types| types.into_parts().0))
}

/// [`function_types`] with every buffer charged to `budget` before it is allocated. The
/// work list holds each pc at most once (the `in_work` bitmap). Past the transfer cutoff
/// (`factor * len * max(slots, 1)`) the inference is abandoned (`None`), as past
/// [`MAX_FUNCTION_INFERENCE_CELLS`]: it only proves guards that are still checked.
pub(crate) fn function_types_in<'b>(
	f: &Function,
	budget: &'b Budget,
	factor: usize,
) -> Result<Option<BVec<'b, FTy>>, Stop> {
	let n = f.slots as usize;
	let len = f.len();
	if len
		.checked_mul(n)
		.is_none_or(|c| c > MAX_FUNCTION_INFERENCE_CELLS)
		|| len == 0
	{
		return Ok(None);
	}
	let mut states: BVec<'b, FTy> = BVec::new(budget);
	states.resize(len * n, FTy::Bottom)?;
	for (s, ty) in states[..n].iter_mut().enumerate() {
		*ty = if s < f.parameters as usize {
			FTy::Top
		} else {
			FTy::Unit
		};
	}
	let mut work: BVec<'b, usize> = BVec::with_capacity(budget, len)?;
	let mut in_work: BVec<'b, bool> = BVec::new(budget);
	in_work.resize(len, false)?;
	work.push(0)?;
	in_work[0] = true;
	let mut scratch: BVec<'b, FTy> = BVec::new(budget);
	scratch.resize(n, FTy::Bottom)?;
	let mut reached: BVec<'b, bool> = BVec::new(budget);
	reached.resize(len, false)?;
	reached[0] = true;
	let limit = cutoff(factor, len, n);
	let mut transfers = 0usize;
	while let Some(pc) = work.pop() {
		in_work[pc] = false;
		transfers += 1;
		if transfers > limit {
			return Ok(None);
		}
		scratch.copy_from_slice(&states[pc * n..(pc + 1) * n]);
		function_transfer(f.operation(pc)?, &mut scratch)?;
		for q in successors(f, pc)?.into_iter().flatten() {
			let target = &mut states[q * n..(q + 1) * n];
			let mut changed = !reached[q];
			reached[q] = true;
			for (t, s) in target.iter_mut().zip(scratch.iter()) {
				let joined = t.join(*s);
				changed |= joined != *t;
				*t = joined;
			}
			if changed && !in_work[q] {
				in_work[q] = true;
				work.push(q)?;
			}
		}
	}
	Ok(Some(states))
}

fn function_transfer(op: Op, s: &mut [FTy]) -> Result<(), Error> {
	let get = |s: &[FTy], n: u32| s.get(n as usize).copied().ok_or_else(invalid);
	let (out, ty) = match op {
		Op::Literal { out, value } => (
			out,
			match value {
				Value::Unit => FTy::Unit,
				Value::Bool(_) => FTy::Bool,
				Value::I64(_) => FTy::I64,
				_ => FTy::Top,
			},
		),
		Op::Binary { out, op, lhs, rhs } | Op::AssignBinary { out, op, lhs, rhs } => {
			let both = get(s, lhs)? == FTy::I64 && get(s, rhs)? == FTy::I64;
			(
				out,
				match (both, ARITH.contains(&op), COMPARE.contains(&op)) {
					(true, true, _) => FTy::I64,
					(true, _, true) => FTy::Bool,
					_ => FTy::Top,
				},
			)
		}
		Op::Copy { out, value } => (out, get(s, value)?),
		Op::Release { value } => (value, FTy::Unit),
		Op::Is { out, .. } => (out, FTy::Bool),
		Op::Cast { out, tag, .. } => (
			out,
			if tag == super::value::Type::I64 {
				FTy::I64
			} else {
				FTy::Top
			},
		),
		Op::Unary { out, value, .. } => {
			let v = get(s, value)?;
			(
				out,
				if matches!(v, FTy::I64 | FTy::Bool) {
					v
				} else {
					FTy::Top
				},
			)
		}
		Op::Template { out, .. }
		| Op::TupleField { out, .. }
		| Op::Record { out, .. }
		| Op::Field { out, .. }
		| Op::Variant { out, .. }
		| Op::Try { out, .. }
		| Op::CallValue { out, .. }
		| Op::Range { out, .. }
		| Op::IterStart { out, .. }
		| Op::IterNext { out, .. }
		| Op::String { out, .. }
		| Op::Clone { out, .. }
		| Op::Call { out, .. }
		| Op::List { out, .. }
		| Op::Get { out, .. }
		| Op::Len { out, .. } => (out, FTy::Top),
		Op::Print { .. }
		| Op::SetTupleField { .. }
		| Op::SetField { .. }
		| Op::Jump { .. }
		| Op::Branch { .. }
		| Op::Return { .. }
		| Op::Set { .. }
		| Op::Push { .. }
		| Op::Drop { .. } => return Ok(()),
	};
	*s.get_mut(out as usize).ok_or_else(invalid)? = ty;
	Ok(())
}
