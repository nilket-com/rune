//! P0 native loop regions (feature `fresh-core-native`): eligibility, type inference, the
//! exit contract and a safe simulator of the native semantics. Everything here is safe
//! Rust under the fresh core's `forbid(unsafe_code)`; the Cranelift backend and the one
//! unsafe boundary live outside this module (`crate::experimental_native`).
//!
//! # Contract (summary; the full text is the P0 contract document)
//!
//! - **Region.** A natural loop whose header is a charged back-edge target, with no side
//!   entry, whose reached instructions are all `Literal` (`I64`/`Bool`/`Unit`), `Binary` or
//!   `AssignBinary` with an op in {Add, Sub, Mul, Div, Rem, Eq, Ne, Lt, Le, Gt, Ge} on
//!   proven `I64` operands, `Branch` on a proven `Bool`, `Jump`, `Release` (writes `Unit`),
//!   or `Copy` (P2b) from a source written in the region on every path with a concrete
//!   `I64`/`Bool`/`Unit` type in the final fixpoint (otherwise `CopySource`). Anything else
//!   refuses the region; the interpreter runs it unchanged.
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
//!   written bit is set are written back, with their statically known tag or, for a slot
//!   whose written type differs by path, the tag of the dynamic bool and unit masks; every
//!   other slot keeps its original value. The written bit is exact per path (a dynamic mask).
//! - **Ownership (P2b).** Every value a region writes is `I64`, `Bool` or `Unit`, and every
//!   old value it replaces is a scalar checked at entry (or written by the region), in an
//!   open store with no queued work. So every `retain` and `release_slot` the interpreter
//!   would run inside the region is `Ok(())` with no store access; the region performs none,
//!   and the commit runs one `release_slot` per written slot, the interpreter's last `put`.
//! - **Errors** are never synthesized: a faulting op exits `Retry` and the interpreter
//!   re-executes it, producing the identical error, location, call chain and cleanup.
// Wired into execution (P2a) through `cache` on x86-64 Linux only; the simulator and the
// listing helpers are used by tests only.
#![cfg_attr(not(test), allow(dead_code))]
use super::arithmetic::Binary;
use super::code::{self, Code, Function, Op};
use super::error::{Error, ErrorKind};
use super::value::Value;
use crate::alloc::Vec;
use budget::{AnalysisPod, BVec, Budget, Grow};

pub(crate) mod abi;
pub(crate) mod budget;
pub(crate) mod commit;
pub(crate) mod simulate;

#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
pub(crate) mod cache;

#[cfg(test)]
pub(crate) mod b1_tests;
#[cfg(test)]
pub(crate) mod hook;
#[cfg(test)]
pub(crate) mod p2b_tests;
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

analysis_pod!(
	SlotState,
	Option<Scalar>,
	FTy,
	Op,
	Exit,
	Node,
	EntryClass,
	Site,
	SiteAnalysis
);

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
/// B1: most `i64` cells of one entry buffer: the touched slots plus the iterator's `next`
/// and `end` (`buf[n]`, `buf[n + 1]`). Checked on its own, before any buffer is filled or
/// any code is lowered: the 64-slot limit does not by itself leave room for two more.
pub(crate) const MAX_REGION_CELLS: usize = MAX_REGION_SLOTS + 2;
/// B1: most nodes of one region (the body plus every inlined expansion).
pub(crate) const MAX_REGION_NODES: usize = 2048;
/// B1: most instructions of an inlined callee.
pub(crate) const MAX_INLINE_INSTRUCTIONS: usize = 16;
/// B1: most frame slots of an inlined callee (its virtual locals).
pub(crate) const MAX_INLINE_SLOTS: usize = 16;
/// B1: most inlined call sites per region.
pub(crate) const MAX_INLINE_SITES: usize = 16;
/// B1: the first virtual local. A node operand `l >= VIRTUAL_BASE` names the inlined
/// callee's slot `l - VIRTUAL_BASE`: it lives in the runner only (never in `buf`, never in a
/// mask, never written back). Caller locals are `0..n`, `n <= 64`.
pub(crate) const VIRTUAL_BASE: u32 = MAX_REGION_SLOTS as u32;

/// A scalar the native region holds unboxed in an `i64`: `I64` as itself, `Bool` as
/// exactly 0 or 1, `Unit` (P2b) as exactly 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scalar {
	I64,
	Bool,
	Unit,
}

/// Value-type lattice of the region inference: `Bottom < {I64, Bool, Unit} < Top`. A
/// written `Top` is a per-path mix of I64, Bool and Unit, recorded exactly by the dynamic
/// bool and unit masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
	Bottom,
	I64,
	Bool,
	Unit,
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
			Scalar::Unit => Self::Unit,
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
	/// B1: the single charge or an op of an inlined callee would stop. Resume AT the Call pc
	/// (not yet executed; never charged, by the ChargedCall rule) with the caller state and
	/// fuel the interpreter had immediately before the Call: every callee effect and its one
	/// charge are undone; every effect of earlier nodes of this entry, including IterNext's
	/// increment, is kept. The interpreter then executes the Call itself.
	Replay = 4,
}

impl Status {
	pub(crate) fn from_u32(n: u32) -> Option<Self> {
		match n {
			1 => Some(Self::Continue),
			2 => Some(Self::Fuel),
			3 => Some(Self::Retry),
			4 => Some(Self::Replay),
			_ => None,
		}
	}
}

/// B1: why a Replay exit is taken. No variant is test-specific: production validation is
/// the same in every build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplayReason {
	/// The callee's single charge (at its pc 0, run once per call) found fuel 0: the
	/// checkpoint fuel is 0.
	CalleeFuel,
	/// A callee op would fault, after the single charge succeeded: checkpoint fuel >= 1.
	CalleeFault,
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
	/// Among `definite | maybe`, the slots written with different types (I64, Bool, Unit)
	/// on different paths (temporaries reused across types): their tag comes from the
	/// dynamic bool and unit masks ([`abi::ExitRecord::bools`], [`abi::ExitRecord::units`]),
	/// exact per path like the written mask.
	pub(crate) dynamic: u64,
	/// A `Retry` at a charged pc: native code must report the pre-charge fuel.
	pub(crate) restore: bool,
	/// P2b: among `definite | maybe`, the slots whose written value is statically `Unit`.
	/// `units`, `bools` and `dynamic` are pairwise disjoint.
	pub(crate) units: u64,
	/// B1: `Some` iff `status == Replay`.
	pub(crate) replay: Option<ReplayReason>,
	/// B1: the IterNext none-edge exit (`next` is then exhausted).
	pub(crate) iter_done: bool,
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
	/// B1, the header only: `next >= end` goes to `none` (nothing written); otherwise `out`
	/// := `next` (I64), `next += 1` (cannot overflow: `next < end`), then `next` (the node's
	/// fall-through). `next` and `end` are the runner's `buf[n]` and `buf[n + 1]`.
	IterNext {
		out: u32,
		none: Target,
	},
	/// B1, at the Call pc (uncharged): checkpoint := fuel; the site's virtual locals
	/// `VIRTUAL_BASE..VIRTUAL_BASE + slots` := 0 (the callee frame's `Unit` slots).
	Enter {
		site: u32,
		slots: u32,
	},
	/// B1: virtual local `VIRTUAL_BASE + param` := caller local `src`.
	Arg {
		param: u32,
		src: u32,
	},
	/// B1, one per reachable callee Return: caller local `out` := virtual local `src`,
	/// tagged `ty` (the only caller write of an expansion); `next` is the caller's
	/// fall-through after the Call.
	Ret {
		out: u32,
		src: u32,
		ty: Scalar,
	},
	/// B1 TEST PROTOCOL ONLY (absent in non-test builds; inserted after `Enter` by the
	/// hook's test transform): `checkpoint == 0` goes to `zero` (the site's real CalleeFuel
	/// exit), otherwise to `positive` (the site's real CalleeFault exit when it has one).
	#[cfg(test)]
	ForceReplay {
		zero: Target,
		positive: Target,
	},
	/// `ty` may be `Unit` (value 0): `Literal Unit` and `Release` both lower to this.
	Literal {
		out: u32,
		value: i64,
		ty: Scalar,
	},
	/// P2b: `out = src`, with the source's static type (the CopySource rule guarantees one).
	Copy {
		out: u32,
		src: u32,
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
	/// B1: the header's iterator, when the header is an `IterNext` over a range (its slot
	/// is not a region slot; its `next` and `end` are `buf[n]`, `buf[n + 1]`).
	pub(crate) iter: Option<IterSpec>,
	/// B1: the most frame slots any inlined callee reserves (0 without a call): the slot
	/// room entry requires.
	pub(crate) callee_slots: u32,
	/// B1: the inlined call sites, in pc order (analysis data: dropped with the nodes after
	/// compile).
	pub(crate) sites: Vec<Site>,
}

impl Region {
	/// The entry buffer's cells: one per touched slot, plus `next` and `end`.
	pub(crate) fn cells(&self) -> usize {
		self.slots.len() + if self.iter.is_some() { 2 } else { 0 }
	}
}

/// B1: the header IterNext's iterator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IterSpec {
	/// Frame slot that holds the iterator handle at the header.
	pub(crate) slot: u32,
}

/// B1: one inlined call site of a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Site {
	/// The caller pc of the Call.
	pub(crate) pc: u32,
	pub(crate) callee: u32,
	/// The site's `Enter` node.
	pub(crate) enter: u32,
	/// The expansion's nodes: `first..end` (Args, then one node per reachable callee pc).
	pub(crate) first: u32,
	pub(crate) end: u32,
	/// The site's Replay exits.
	pub(crate) fuel_exit: u32,
	pub(crate) fault_exit: Option<u32>,
	pub(crate) slots: u32,
	pub(crate) parameters: u32,
	pub(crate) ret: Scalar,
}

/// B1: why an `IterNext` refused its region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IterReason {
	/// Not the header.
	NotHeader,
	/// A second IterNext after the header's.
	Second,
}

/// B1: why a Call was not inlined (its region is refused).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineReason {
	/// The Call itself is charged (its cached or encoded flag).
	ChargedCall,
	/// More than [`MAX_INLINE_SITES`] sites.
	Sites,
	/// More than [`MAX_INLINE_INSTRUCTIONS`] callee instructions.
	Size,
	/// More than [`MAX_INLINE_SLOTS`] callee slots.
	Slots,
	/// The callee calls (it is not a leaf).
	NotLeaf,
	/// A callee op outside the inlinable set (every heap op, so no store access).
	Op { callee_pc: u32 },
	/// A callee charge other than exactly one at pc 0 (cached or encoded flag).
	Charged { callee_pc: u32 },
	/// A callee edge back to pc 0 (its one charge would run more than once per call).
	Pc0Reentry { callee_pc: u32 },
	/// A callee edge that is not forward.
	Loop,
	/// One parameter demanded with two types.
	ParamConflict,
	/// A callee operand not proven to be the demanded type.
	Type { callee_pc: u32 },
	/// A callee Copy whose source is not written on every path with one scalar type.
	CopySource { callee_pc: u32 },
	/// A callee Return whose value is not written on every path with one scalar type.
	ReturnSource { callee_pc: u32 },
	/// Reachable Returns of different types.
	ReturnType,
	/// No reachable Return.
	NoReturn,
}

/// What one admitted callee contributes to its caller's analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InlineSummary {
	pub(crate) callee: u32,
	pub(crate) slots: u32,
	pub(crate) parameters: u32,
	/// Per parameter; `None` = never typed-read (the argument is still touched).
	pub(crate) demands: [Option<Scalar>; MAX_INLINE_SLOTS],
	pub(crate) ret: Scalar,
	/// Some reachable callee Arith keeps a fault exit.
	pub(crate) faults: bool,
	/// The expansion's node count: one Arg per parameter plus one node per reachable pc.
	pub(crate) nodes: u32,
}

/// A site during analysis (budgeted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SiteAnalysis {
	pub(crate) pc: u32,
	pub(crate) summary: InlineSummary,
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
	/// P2b: a `Copy` whose source is not written in the region on every path with one
	/// concrete type (I64, Bool or Unit) in the final fixpoint.
	CopySource { pc: u32 },
	/// B1: an `IterNext` other than the header's.
	Iter { pc: u32, reason: IterReason },
	/// B1: a node other than the header IterNext names the iterator's frame slot (read,
	/// write, Copy, Release, Branch, any Call argument or out), or `out == iterator`.
	IterSlot { pc: u32 },
	/// B1: a Call that cannot be inlined.
	Inline { pc: u32, reason: InlineReason },
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
		Op::Literal { out, .. }
		| Op::Binary { out, .. }
		| Op::AssignBinary { out, .. }
		| Op::Copy { out, .. } => Some(out),
		Op::Release { value } => Some(value),
		_ => None,
	}
}

/// The source slot of a Copy: not a typed demand (it never fixes a live-in guard), but it
/// must be written in the region on every path with a concrete scalar type (P2b rule).
fn copy_source(op: Op) -> Option<u32> {
	match op {
		Op::Copy { value, .. } => Some(value),
		_ => None,
	}
}

fn admitted(op: Op) -> bool {
	match op {
		Op::Literal { value, .. } => {
			matches!(value, Value::I64(_) | Value::Bool(_) | Value::Unit)
		}
		Op::Copy { .. } | Op::Release { .. } => true,
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

/// Analyses every candidate header of function `function` of `code` (at most
/// [`MAX_REGIONS_PER_FUNCTION`]) without a byte limit. `Err` is an allocation or
/// invalid-code failure of the analysis itself, which the caller treats as "no regions"
/// (interpreter fallback). Tests and the P1 hook use it; the production cache streams the
/// same steps ([`scan_headers`], [`FunctionContext::new`], [`FunctionContext::analyze_header`])
/// under its budget instead. B1: `code` is the immutable code the executor runs, so an
/// inlined callee is the executed callee.
pub(crate) fn analyze(
	code: &Code,
	function: u32,
) -> Result<Vec<(u32, Result<Region, RegionRefusal>)>, Error> {
	let budget = Budget::unlimited();
	let f = code.function(function).ok_or_else(invalid)?;
	let headers = scan_headers(f, &budget).map_err(stop_error)?;
	let context =
		FunctionContext::new(code, function, &budget, TRANSFER_FACTOR).map_err(stop_error)?;
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
	/// B1: the whole code, for the callees of inlined calls.
	code: &'f Code,
	f: &'f Function,
	function: u32,
	budget: &'b Budget,
	/// `(target, source)` of every successor arc, sorted by target.
	edges: BVec<'b, (u32, u32)>,
	/// The function-level inference, `instructions * slots` cells, if it ran.
	types: Option<BVec<'b, FTy>>,
	factor: usize,
	/// The most fixpoint transfers one header used (tests and diagnostics).
	max_transfers: core::cell::Cell<usize>,
}

impl<'f, 'b> FunctionContext<'f, 'b> {
	/// Builds the predecessor arcs (exact capacity, counted first) and runs the
	/// function-level inference of function `function` of `code`, every buffer charged
	/// before it is allocated. `factor` is the transfer cutoff factor ([`TRANSFER_FACTOR`]
	/// in production).
	pub(crate) fn new(
		code: &'f Code,
		function: u32,
		budget: &'b Budget,
		factor: usize,
	) -> Result<Self, Stop> {
		let f = code.function(function).ok_or_else(invalid)?;
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
			code,
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
		self.types.as_ref().map(|t| &t[header as usize * n..][..n])
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

	/// One candidate header. On success, the region and the bytes of its five vectors
	/// (slots, entry, nodes, exits, sites; by capacity), which stay charged to the budget:
	/// the caller adopts them (`budget::Held::adopt`). `Err(Stop::Budget)`: the
	/// transient-bytes budget refused an allocation (nothing of this header stays charged).
	pub(crate) fn analyze_header(
		&self,
		header: u32,
	) -> Result<Result<(Region, usize), RegionRefusal>, Stop> {
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

		// Admission and touched slots (the interpreter footprint; the header IterNext's iterator
		// operand is not a slot). B1: the header IterNext over a range, and inlined calls,
		// whose summaries (and their typing buffers, freed per site) are built here.
		let mut ops: BVec<'b, Op> = BVec::with_capacity(budget, body.len())?;
		let mut slots: BVec<'b, u32> = BVec::new(budget);
		let mut iter: Option<IterSpec> = None;
		let mut sites: BVec<'b, SiteAnalysis> = BVec::new(budget);
		for &pc in body.iter() {
			let op = f.operation(pc as usize)?;
			let mut header_iterator = None;
			match op {
				Op::IterNext { iterator, .. } => {
					if pc == header && iter.is_none() {
						iter = Some(IterSpec { slot: iterator });
						header_iterator = Some(iterator);
					} else if iter.is_some() {
						return Ok(Err(RegionRefusal::Iter {
							pc,
							reason: IterReason::Second,
						}));
					} else {
						return Ok(Err(RegionRefusal::Iter {
							pc,
							reason: IterReason::NotHeader,
						}));
					}
				}
				Op::Call { .. } => {
					if sites.charged() == 0 {
						sites = BVec::with_capacity(budget, MAX_INLINE_SITES)?;
					}
					match inline_summary(self.code, f, pc, op, sites.len(), budget)? {
						Ok(typing) => {
							note_inline_charged(sites.charged() + typing.charged());
							sites.push(SiteAnalysis {
								pc,
								summary: typing.summary,
							})?;
						}
						Err(reason) => return Ok(Err(RegionRefusal::Inline { pc, reason })),
					}
				}
				_ => {
					if !admitted(op) {
						return Ok(Err(RegionRefusal::Op { pc }));
					}
				}
			}
			let (args, fixed) = footprint(f, op)?;
			let mut skip = header_iterator;
			for slot in args.iter().copied().chain(fixed.into_iter().flatten()) {
				// Exactly the header's iterator OPERAND is skipped (once); an `out` naming
				// the same slot stays touched and is refused below.
				if skip == Some(slot) {
					skip = None;
					continue;
				}
				if !slots.contains(&slot) {
					if slots.len() >= MAX_REGION_SLOTS {
						return Ok(Err(RegionRefusal::Slots));
					}
					slots.push(slot)?;
				}
			}
			ops.push(op)?;
		}
		// B1: slot aliases of the iterator are refused statically: any footprint, other than
		// the header IterNext's iterator operand, that names its slot.
		if let Some(spec) = iter {
			for (i, &pc) in body.iter().enumerate() {
				let (args, fixed) = footprint(f, ops[i])?;
				let named = args
					.iter()
					.copied()
					.chain(fixed.into_iter().flatten())
					.filter(|&s| s == spec.slot)
					.count();
				let allowed = usize::from(pc == header);
				if named > allowed {
					return Ok(Err(RegionRefusal::IterSlot { pc }));
				}
			}
			if slots.contains(&spec.slot) {
				return Err(invalid().into());
			}
		}
		slots.sort_unstable();
		let n = slots.len();
		let local = |slot: u32| slots.binary_search(&slot).map_err(|_| invalid());
		let header_index = index_of(&body, h).ok_or_else(invalid)?;
		let site_of = |pc: u32| -> Result<usize, Stop> {
			sites
				.binary_search_by_key(&pc, |s| s.pc)
				.map_err(|_| invalid().into())
		};
		let ret_of = |op: Op, pc: u32| -> Result<Option<Scalar>, Stop> {
			Ok(match op {
				Op::Call { .. } => Some(sites[site_of(pc)?].summary.ret),
				_ => None,
			})
		};

		// Test seam: an analysis error mid-header, with the body, ops and slots buffers
		// charged and alive (and, in B1, the site summaries). It can only produce an `Err`.
		#[cfg(test)]
		if take_injected_analysis_error() {
			return Err(invalid().into());
		}

		// Forward fixpoint over (written, written type, written constant) per slot. The
		// work list holds each body index at most once (the `in_work` bitmap), so its
		// capacity is the body length. B1: IterNext's write is on its some edge only (the
		// none edge carries the state before it).
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
		let mut pre: BVec<'b, SlotState> = BVec::new(budget);
		if iter.is_some() {
			pre.resize(n, SlotState::ENTRY)?;
		}
		let limit = cutoff(self.factor, body.len(), n);
		let mut transfers = 0usize;
		while let Some(i) = work.pop() {
			in_work[i] = false;
			transfers += 1;
			if transfers > limit {
				return Ok(Err(RegionRefusal::Steps));
			}
			let is_iter = matches!(ops[i], Op::IterNext { .. });
			if is_iter {
				pre.copy_from_slice(&states[i * n..(i + 1) * n]);
			}
			scratch.copy_from_slice(&states[i * n..(i + 1) * n]);
			transfer_at(ops[i], &slots, &mut scratch, ret_of(ops[i], body[i])?)?;
			for (k, q) in successors(f, body[i] as usize)?.into_iter().enumerate() {
				let Some(q) = q else {
					continue;
				};
				let Some(j) = index_of(&body, q) else {
					continue;
				};
				// IterNext's successors are [pc + 1 (some), none].
				let source: &[SlotState] = if is_iter && k == 1 { &pre } else { &scratch };
				let target = &mut states[j * n..(j + 1) * n];
				let changed = if reached[j] {
					let mut changed = false;
					for (t, s) in target.iter_mut().zip(source.iter()) {
						let joined = t.join(*s);
						changed |= joined != *t;
						*t = joined;
					}
					changed
				} else {
					reached[j] = true;
					target.copy_from_slice(source);
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
		drop(pre);

		// Every typed read of body index `i`: the op's own, plus a Call's arguments whose
		// parameter the callee demands (an unread parameter's argument is touched only).
		let typed_reads =
			|i: usize,
			 each: &mut dyn FnMut(u32, Scalar) -> Result<Option<RegionRefusal>, Stop>|
			 -> Result<Option<RegionRefusal>, Stop> {
				for (slot, want) in reads(ops[i]).into_iter().flatten() {
					if let Some(refusal) = each(slot, want)? {
						return Ok(Some(refusal));
					}
				}
				if let Op::Call {
					args_start,
					args_len,
					..
				} = ops[i]
				{
					let summary = sites[site_of(body[i])?].summary;
					for (p, &slot) in call_args(f, args_start, args_len)?.iter().enumerate() {
						if let Some(want) = summary.demands.get(p).copied().flatten() {
							if let Some(refusal) = each(slot, want)? {
								return Ok(Some(refusal));
							}
						}
					}
				}
				Ok(None)
			};

		// Live-in demands: a read that can observe the entry value fixes that slot's guard.
		let mut demand: BVec<'b, Option<Scalar>> = BVec::new(budget);
		demand.resize(n, None)?;
		for i in (0..body.len()).filter(|&i| reached[i]) {
			let refusal = typed_reads(i, &mut |slot, want| {
				let l = local(slot)?;
				if states[i * n + l].written != Written::Yes {
					match demand[l] {
						None => demand[l] = Some(want),
						Some(d) if d == want => (),
						Some(_) => return Ok(Some(RegionRefusal::LiveIn { pc: body[i], slot })),
					}
				}
				Ok(None)
			})?;
			if let Some(refusal) = refusal {
				return Ok(Err(refusal));
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
			let refusal = typed_reads(i, &mut |slot, want| {
				let l = local(slot)?;
				Ok((current(states[i * n + l], l) != Ty::of(want))
					.then_some(RegionRefusal::Type { pc: body[i] }))
			})?;
			if let Some(refusal) = refusal {
				return Ok(Err(refusal));
			}
		}

		// P2b: every reached Copy's source, in the FINAL fixpoint state before the Copy, is
		// written in the region on every path with one concrete scalar type. (The fixpoint
		// is complete here: the cutoff returned `Steps` above otherwise.)
		for i in (0..body.len()).filter(|&i| reached[i]) {
			if let Some(src) = copy_source(ops[i]) {
				let s = states[i * n + local(src)?];
				if s.written != Written::Yes || !matches!(s.ty, Ty::I64 | Ty::Bool | Ty::Unit) {
					return Ok(Err(RegionRefusal::CopySource { pc: body[i] }));
				}
			}
		}

		// Nodes (reached only) and exits. B1: the caller's reached body nodes are numbered
		// first; each site's expansion follows, in site order.
		let mut node_of: BVec<'b, Option<u32>> = BVec::new(budget);
		node_of.resize(body.len(), None)?;
		let mut count = 0u32;
		for i in 0..body.len() {
			if reached[i] {
				node_of[i] = Some(count);
				count += 1;
			}
		}
		let mut total = count as usize;
		for site in sites.iter() {
			total += site.summary.nodes as usize;
		}
		if total > MAX_REGION_NODES {
			return Ok(Err(RegionRefusal::Size));
		}
		let mut exits: BVec<'b, Exit> = BVec::new(budget);
		// Exact capacity when the region inlines (computed before allocating); unchanged
		// growth otherwise.
		let mut nodes: BVec<'b, Node> = if sites.is_empty() {
			BVec::new(budget)
		} else {
			BVec::with_capacity(budget, total)?
		};
		let mut placed: BVec<'b, Site> = BVec::new(budget);
		if !sites.is_empty() {
			placed = BVec::with_capacity(budget, sites.len())?;
		}
		note_inline_charged(
			sites.charged()
				+ placed.charged()
				+ (total - count as usize) * core::mem::size_of::<Node>(),
		);
		let mut ret_targets = [Target::Exit(u32::MAX); MAX_INLINE_SITES];
		let mut push_exit = |status: Status,
		                     pc: u32,
		                     _at: u32,
		                     state: &[SlotState],
		                     restore: bool,
		                     replay: Option<ReplayReason>,
		                     iter_done: bool|
		 -> Result<Result<u32, RegionRefusal>, Stop> {
			let (mut definite, mut maybe, mut bools, mut dynamic) = (0u64, 0u64, 0u64, 0u64);
			let mut units = 0u64;
			for (l, s) in state.iter().enumerate() {
				let bit = 1u64 << l;
				match s.written {
					Written::No => continue,
					Written::Yes => definite |= bit,
					Written::Maybe => maybe |= bit,
				}
				// Written values are only I64, Bool or Unit, so Top here is a per-path mix of
				// them, which the dynamic bool and unit masks record exactly.
				match s.ty {
					Ty::Bool => bools |= bit,
					Ty::Unit => units |= bit,
					Ty::I64 => (),
					Ty::Top => dynamic |= bit,
					Ty::Bottom => return Err(invalid().into()),
				}
			}
			debug_assert!(units & bools == 0 && (units | bools) & dynamic == 0);
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
				units,
				replay,
				iter_done,
			})?;
			Ok(Ok(id))
		};
		let mut first = count;
		for i in (0..body.len()).filter(|&i| reached[i]) {
			let pc = body[i];
			let state = &states[i * n..(i + 1) * n];
			let charged_pc = f.instructions()[pc as usize].is_charged();
			let charged = if charged_pc {
				match push_exit(Status::Fuel, pc, pc, state, false, None, false)? {
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
					let divisor = const_divisor(op, state[r]);
					let retry = if divisor.is_some() {
						None
					} else {
						match push_exit(Status::Retry, pc, pc, state, charged_pc, None, false)? {
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
			// B1: a Call's Replay exits carry the state BEFORE the Call (the checkpoint).
			let mut replay_exits = None;
			if let Op::Call { .. } = ops[i] {
				let k = site_of(pc)?;
				let fuel_exit = match push_exit(
					Status::Replay,
					pc,
					pc,
					state,
					false,
					Some(ReplayReason::CalleeFuel),
					false,
				)? {
					Ok(id) => id,
					Err(refusal) => return Ok(Err(refusal)),
				};
				let fault_exit = if sites[k].summary.faults {
					match push_exit(
						Status::Replay,
						pc,
						pc,
						state,
						false,
						Some(ReplayReason::CalleeFault),
						false,
					)? {
						Ok(id) => Some(id),
						Err(refusal) => return Ok(Err(refusal)),
					}
				} else {
					None
				};
				replay_exits = Some((k, fuel_exit, fault_exit));
			}
			scratch.copy_from_slice(state);
			transfer_at(ops[i], &slots, &mut scratch, ret_of(ops[i], pc)?)?;
			let mut target = |q: usize,
			                  from: &[SlotState],
			                  iter_done: bool|
			 -> Result<Result<Target, RegionRefusal>, Stop> {
				Ok(match index_of(&body, q) {
					Some(j) => Ok(Target::Node(node_of[j].ok_or_else(invalid)?)),
					None => {
						let q = u32::try_from(q).map_err(|_| invalid())?;
						push_exit(Status::Continue, q, pc, from, false, None, iter_done)?
							.map(Target::Exit)
					}
				})
			};
			macro_rules! target {
				($q:expr) => {
					target!($q, &scratch, false)
				};
				($q:expr, $from:expr, $done:expr) => {
					match target($q, $from, $done)? {
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
						Value::Unit => (0, Scalar::Unit),
						_ => return Err(invalid().into()),
					};
					let out = local(out)? as u32;
					(NodeOp::Literal { out, value, ty }, Some(target!(fall)))
				}
				// `put(value, Value::Unit)`: the same write as a Unit literal.
				Op::Release { value } => (
					NodeOp::Literal {
						out: local(value)? as u32,
						value: 0,
						ty: Scalar::Unit,
					},
					Some(target!(fall)),
				),
				Op::Copy { out, value } => {
					// The source's final state; the CopySource check admitted only these.
					let ty = match state[local(value)?].ty {
						Ty::I64 => Scalar::I64,
						Ty::Bool => Scalar::Bool,
						Ty::Unit => Scalar::Unit,
						_ => return Err(invalid().into()),
					};
					(
						NodeOp::Copy {
							out: local(out)? as u32,
							src: local(value)? as u32,
							ty,
						},
						Some(target!(fall)),
					)
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
				// B1: the some edge carries the write; the none edge the state before it.
				Op::IterNext { out, none, .. } => {
					let some = target!(fall);
					let none = target!(none as usize, state, true);
					(
						NodeOp::IterNext {
							out: local(out)? as u32,
							none,
						},
						Some(some),
					)
				}
				// B1: the Enter of a site; its Ret nodes continue at the fall-through.
				Op::Call { .. } => {
					let (k, fuel_exit, fault_exit) = replay_exits.ok_or_else(invalid)?;
					let summary = sites[k].summary;
					ret_targets[k] = target!(fall);
					let enter = node_of[i].ok_or_else(invalid)?;
					placed.push(Site {
						pc,
						callee: summary.callee,
						enter,
						first,
						end: first + summary.nodes,
						fuel_exit,
						fault_exit,
						slots: summary.slots,
						parameters: summary.parameters,
						ret: summary.ret,
					})?;
					first += summary.nodes;
					(
						NodeOp::Enter {
							site: k as u32,
							slots: summary.slots,
						},
						Some(Target::Node(placed[k].first)),
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

		// B1: each site's expansion, after the caller nodes, in site order: the Args, then
		// one node per reachable callee pc (a Return becomes a Ret). The callee's typing is
		// recomputed (deterministic; the same pass as admission), its buffers freed per site.
		for (k, site) in placed.iter().enumerate() {
			let summary = sites[k].summary;
			let callee = self.code.function(summary.callee).ok_or_else(invalid)?;
			let Ok(typing) = callee_typing(callee, summary.callee, budget)? else {
				return Err(invalid().into());
			};
			if typing.summary != summary {
				return Err(invalid().into());
			}
			note_inline_charged(
				sites.charged()
					+ placed.charged()
					+ typing.charged()
					+ (total - count as usize) * core::mem::size_of::<Node>(),
			);
			let Op::Call {
				out,
				args_start,
				args_len,
				..
			} = f.operation(site.pc as usize)?
			else {
				return Err(invalid().into());
			};
			let out = local(out)? as u32;
			let args = call_args(f, args_start, args_len)?;
			let base = site.first + summary.parameters;
			// The node of each reachable callee pc.
			let mut node_at = [u32::MAX; MAX_INLINE_INSTRUCTIONS];
			let mut at = base;
			// Indexed on purpose: both tables are indexed by callee pc, and an index past
			// either is a bug that must panic, not end the loop early.
			#[allow(clippy::needless_range_loop)]
			for q in 0..callee.len() {
				if typing.reached[q] {
					node_at[q] = at;
					at += 1;
				}
			}
			if at != site.end {
				return Err(invalid().into());
			}
			let to = |q: usize| -> Result<Target, Stop> {
				match node_at.get(q) {
					Some(&j) if j != u32::MAX => Ok(Target::Node(j)),
					_ => Err(invalid().into()),
				}
			};
			let v = |slot: u32| VIRTUAL_BASE + slot;
			for (p, &slot) in args.iter().enumerate() {
				let next = if p + 1 < args.len() {
					site.first + p as u32 + 1
				} else {
					base
				};
				nodes.push(Node {
					pc: site.pc,
					charged: None,
					op: NodeOp::Arg {
						param: p as u32,
						src: local(slot)? as u32,
					},
					next: Some(Target::Node(next)),
				})?;
			}
			for q in (0..callee.len()).filter(|&q| typing.reached[q]) {
				let (op, next) = match callee.operation(q)? {
					Op::Literal { out: o, value } => {
						let (value, ty) = match value {
							Value::I64(x) => (x, Scalar::I64),
							Value::Bool(b) => (i64::from(b), Scalar::Bool),
							Value::Unit => (0, Scalar::Unit),
							_ => return Err(invalid().into()),
						};
						(
							NodeOp::Literal {
								out: v(o),
								value,
								ty,
							},
							Some(to(q + 1)?),
						)
					}
					Op::Release { value } => (
						NodeOp::Literal {
							out: v(value),
							value: 0,
							ty: Scalar::Unit,
						},
						Some(to(q + 1)?),
					),
					Op::Copy { out: o, value } => {
						let ty = match typing.state(q, value).ty {
							Ty::I64 => Scalar::I64,
							Ty::Bool => Scalar::Bool,
							Ty::Unit => Scalar::Unit,
							_ => return Err(invalid().into()),
						};
						(
							NodeOp::Copy {
								out: v(o),
								src: v(value),
								ty,
							},
							Some(to(q + 1)?),
						)
					}
					Op::Jump { target } => (
						NodeOp::Jump {
							target: to(target as usize)?,
						},
						None,
					),
					Op::Branch { condition, yes, no } => (
						NodeOp::Branch {
							condition: v(condition),
							yes: to(yes as usize)?,
							no: to(no as usize)?,
						},
						None,
					),
					Op::Return { value } => (
						NodeOp::Ret {
							out,
							src: v(value),
							ty: summary.ret,
						},
						Some(ret_targets[k]),
					),
					other => {
						let (o, binary, lhs, rhs) = binary_parts(other).ok_or_else(invalid)?;
						let op = if ARITH.contains(&binary) {
							let divisor = const_divisor(binary, typing.state(q, rhs));
							NodeOp::Arith {
								out: v(o),
								op: binary,
								lhs: v(lhs),
								rhs: v(rhs),
								retry: if divisor.is_some() {
									None
								} else {
									Some(site.fault_exit.ok_or_else(invalid)?)
								},
								divisor,
							}
						} else {
							NodeOp::Compare {
								out: v(o),
								op: binary,
								lhs: v(lhs),
								rhs: v(rhs),
							}
						};
						(op, Some(to(q + 1)?))
					}
				};
				nodes.push(Node {
					pc: site.pc,
					// The callee's single charge, at its pc 0 (C1, C2).
					charged: (q == 0).then_some(site.fuel_exit),
					op,
					next,
				})?;
			}
		}
		drop(scratch);
		if nodes.len() != total {
			return Err(invalid().into());
		}
		// The structural checks (`check_parts`) on the finished parts, before they are handed
		// over (an error releases every charge with the buffers).
		check_parts(&nodes, &exits, &placed, n)?;

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
							// Unreachable: no admitted read demands Unit (P2b).
							Scalar::Unit => FTy::Unit,
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
		let callee_slots = placed.iter().map(|s| s.slots).max().unwrap_or(0);

		let (slots, slots_bytes) = slots.into_parts();
		let (entry, entry_bytes) = entry.into_parts();
		let (nodes, nodes_bytes) = nodes.into_parts();
		let (exits, exits_bytes) = exits.into_parts();
		let (sites, sites_bytes) = placed.into_parts();
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
			iter,
			callee_slots,
			sites,
		};
		Ok(Ok((
			region,
			slots_bytes + entry_bytes + nodes_bytes + exits_bytes + sites_bytes,
		)))
	}
}

// ------------------------------------------------------------------------- B1

/// The interpreter footprint of an admitted body op: every frame
/// slot the interpreter reads, writes, retains or releases for it, a Call's arguments
/// (used or not) included. The touched set, the IterSlot check and the read passes all
/// derive from it. Returns the Call's argument slots and up to three other slots.
fn footprint(f: &Function, op: Op) -> Result<(&[u32], [Option<u32>; 3]), Error> {
	Ok(match op {
		Op::Literal { out, .. } => (&[], [Some(out), None, None]),
		Op::Binary { out, lhs, rhs, .. } | Op::AssignBinary { out, lhs, rhs, .. } => {
			(&[], [Some(lhs), Some(rhs), Some(out)])
		}
		Op::Branch { condition, .. } => (&[], [Some(condition), None, None]),
		Op::Copy { out, value } => (&[], [Some(value), Some(out), None]),
		Op::Release { value } => (&[], [Some(value), None, None]),
		Op::IterNext { out, iterator, .. } => (&[], [Some(iterator), Some(out), None]),
		Op::Call {
			out,
			args_start,
			args_len,
			..
		} => (call_args(f, args_start, args_len)?, [Some(out), None, None]),
		_ => (&[], [None, None, None]),
	})
}

fn call_args(f: &Function, start: u32, len: u32) -> Result<&[u32], Error> {
	let end = start.checked_add(len).ok_or_else(invalid)?;
	f.args.get(start as usize..end as usize).ok_or_else(invalid)
}

#[cfg(test)]
std::thread_local! {
	static CHARGE_FLAGS_INSPECTED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
	/// The most bytes the inline buffers of one header held charged at once (B1-23).
	static INLINE_CHARGED_PEAK: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Callee charge flags read by [`callee_charge_shape`] on this thread (tests).
#[cfg(test)]
pub(crate) fn charge_flags_inspected_for_test() -> usize {
	CHARGE_FLAGS_INSPECTED.with(|c| c.get())
}

#[cfg(test)]
pub(crate) fn reset_inline_charged_for_test() {
	INLINE_CHARGED_PEAK.with(|c| c.set(0));
}

#[cfg(test)]
pub(crate) fn inline_charged_peak_for_test() -> usize {
	INLINE_CHARGED_PEAK.with(|c| c.get())
}

#[cfg(test)]
fn note_inline_charged(bytes: usize) {
	INLINE_CHARGED_PEAK.with(|c| c.set(c.get().max(bytes)));
}

#[cfg(not(test))]
fn note_inline_charged(_: usize) {}

/// C1 and C2 of the charge shape: the callee's only charge is at
/// pc 0 and pc 0 runs once per call. Reads every instruction's ACTUAL flags, the one the
/// interpreter consults (`cached_ref(p).charged`) and the encoded one; never infers them
/// from the instruction count or acyclicity.
pub(crate) fn callee_charge_shape(callee: &Function) -> Result<Result<(), InlineReason>, Error> {
	for p in 0..callee.len() {
		let cached = callee.cached_ref(p).ok_or_else(invalid)?.charged;
		let encoded = callee.instructions()[p].is_charged();

		#[cfg(test)]
		CHARGE_FLAGS_INSPECTED.with(|c| c.set(c.get() + 1));

		if cached != encoded || cached != (p == 0) {
			return Ok(Err(InlineReason::Charged {
				callee_pc: p as u32,
			}));
		}
	}

	for p in 0..callee.len() {
		if successors(callee, p)?.into_iter().flatten().any(|q| q == 0) {
			return Ok(Err(InlineReason::Pc0Reentry {
				callee_pc: p as u32,
			}));
		}
	}

	Ok(Ok(()))
}

/// The callee ops an inlined callee may contain: no call, no heap op (so no store access).
fn inlinable(op: Op) -> bool {
	match op {
		Op::Literal { value, .. } => matches!(value, Value::I64(_) | Value::Bool(_) | Value::Unit),
		Op::Binary { op, .. } | Op::AssignBinary { op, .. } => {
			ARITH.contains(&op) || COMPARE.contains(&op)
		}
		Op::Branch { .. }
		| Op::Jump { .. }
		| Op::Copy { .. }
		| Op::Release { .. }
		| Op::Return { .. } => true,
		_ => false,
	}
}

/// The identity slot list of a callee (`0..slots`), so the region transfer applies to it.
fn identity_slots(slots: u32) -> [u32; MAX_INLINE_SLOTS] {
	let mut out = [0u32; MAX_INLINE_SLOTS];
	for (i, s) in out.iter_mut().enumerate().take(slots as usize) {
		*s = i as u32;
	}
	out
}

/// One callee's typing pass (check 8 of `inline_summary`), in pc order (topological: the
/// callee is a forward DAG). `states[q * slots..]` is the state BEFORE pc `q`; budgeted.
struct CalleeTyping<'b> {
	states: BVec<'b, SlotState>,
	reached: BVec<'b, bool>,
	summary: InlineSummary,
}

impl CalleeTyping<'_> {
	fn charged(&self) -> usize {
		self.states.charged() + self.reached.charged()
	}

	fn state(&self, q: usize, slot: u32) -> SlotState {
		self.states[q * self.summary.slots as usize + slot as usize]
	}
}

/// Checks 1 to 8 of inline admission for the Call `op` at caller pc `pc`, in order;
/// the first failure is the refusal. On success, the callee's typing pass (its buffers
/// charged to `budget` before allocation).
fn inline_summary<'b>(
	code: &Code,
	f: &Function,
	pc: u32,
	op: Op,
	sites: usize,
	budget: &'b Budget,
) -> Result<Result<CalleeTyping<'b>, InlineReason>, Stop> {
	let Op::Call {
		function: g,
		args_len,
		..
	} = op
	else {
		return Err(invalid().into());
	};

	// 1. The Call itself is uncharged, by both flags.
	let cached = f.cached_ref(pc as usize).ok_or_else(invalid)?.charged;
	if cached || f.instructions()[pc as usize].is_charged() {
		return Ok(Err(InlineReason::ChargedCall));
	}

	// 2. Sites.
	if sites >= MAX_INLINE_SITES {
		return Ok(Err(InlineReason::Sites));
	}

	// 3. Size and slots (bound the cost of 4..8; not evidence for the fuel rule).
	let callee = code.function(g).ok_or_else(invalid)?;
	if callee.parameters != args_len {
		return Err(invalid().into());
	}
	if callee.len() > MAX_INLINE_INSTRUCTIONS {
		return Ok(Err(InlineReason::Size));
	}
	if callee.slots as usize > MAX_INLINE_SLOTS {
		return Ok(Err(InlineReason::Slots));
	}
	let len = callee.len();

	// 4. A leaf.
	for q in 0..len {
		if matches!(callee.operation(q)?, Op::Call { .. } | Op::CallValue { .. }) {
			return Ok(Err(InlineReason::NotLeaf));
		}
	}

	// 5. Inlinable ops only.
	for q in 0..len {
		if !inlinable(callee.operation(q)?) {
			return Ok(Err(InlineReason::Op {
				callee_pc: q as u32,
			}));
		}
	}

	// 6. The charge shape, from the actual flags.
	if let Err(reason) = callee_charge_shape(callee)? {
		return Ok(Err(reason));
	}

	// 7. A forward DAG.
	for q in 0..len {
		if successors(callee, q)?.into_iter().flatten().any(|t| t <= q) {
			return Ok(Err(InlineReason::Loop));
		}
	}

	// 8. The typing pass.
	callee_typing(callee, g, budget)
}

fn callee_typing<'b>(
	callee: &Function,
	g: u32,
	budget: &'b Budget,
) -> Result<Result<CalleeTyping<'b>, InlineReason>, Stop> {
	let len = callee.len();
	let k = callee.slots as usize;
	let params = callee.parameters as usize;
	let ident = identity_slots(callee.slots);
	let ident = &ident[..k];

	let mut states: BVec<'b, SlotState> = BVec::new(budget);
	states.resize(len * k, SlotState::ENTRY)?;
	let mut reached: BVec<'b, bool> = BVec::new(budget);
	reached.resize(len, false)?;
	let mut demand: BVec<'b, Option<Scalar>> = BVec::new(budget);
	demand.resize(params, None)?;
	note_inline_charged(states.charged() + reached.charged() + demand.charged());

	// The callee frame: parameters from the arguments (not written by the callee), every
	// other slot `Unit` (`reserve_frame`).
	for (slot, st) in states[..k].iter_mut().enumerate() {
		*st = if slot < params {
			SlotState::ENTRY
		} else {
			SlotState {
				written: Written::Yes,
				ty: Ty::Unit,
				konst: Konst::Top,
			}
		};
	}
	reached[0] = true;
	let mut scratch = [SlotState::ENTRY; MAX_INLINE_SLOTS];
	for q in 0..len {
		if !reached[q] {
			continue;
		}
		scratch[..k].copy_from_slice(&states[q * k..(q + 1) * k]);
		transfer(callee.operation(q)?, ident, &mut scratch[..k])?;
		for t in successors(callee, q)?.into_iter().flatten() {
			let target = &mut states[t * k..(t + 1) * k];
			if reached[t] {
				for (a, b) in target.iter_mut().zip(scratch[..k].iter()) {
					*a = a.join(*b);
				}
			} else {
				reached[t] = true;
				target.copy_from_slice(&scratch[..k]);
			}
		}
	}

	// Parameter demands: a typed read of a slot not written on every path can see only a
	// parameter (every other slot starts written).
	for q in (0..len).filter(|&q| reached[q]) {
		for (slot, want) in reads(callee.operation(q)?).into_iter().flatten() {
			if states[q * k + slot as usize].written != Written::Yes {
				let Some(d) = demand.get_mut(slot as usize) else {
					return Err(invalid().into());
				};
				match *d {
					None => *d = Some(want),
					Some(have) if have == want => (),
					Some(_) => return Ok(Err(InlineReason::ParamConflict)),
				}
			}
		}
	}
	let entry_ty = |slot: usize| demand.get(slot).copied().flatten().map_or(Ty::Top, Ty::of);
	let current = |s: SlotState, slot: usize| match s.written {
		Written::No => entry_ty(slot),
		Written::Yes => s.ty,
		Written::Maybe => entry_ty(slot).join(s.ty),
	};
	for q in (0..len).filter(|&q| reached[q]) {
		for (slot, want) in reads(callee.operation(q)?).into_iter().flatten() {
			if current(states[q * k + slot as usize], slot as usize) != Ty::of(want) {
				return Ok(Err(InlineReason::Type {
					callee_pc: q as u32,
				}));
			}
		}
	}
	let concrete = |s: SlotState| -> Option<Scalar> {
		if s.written != Written::Yes {
			return None;
		}
		match s.ty {
			Ty::I64 => Some(Scalar::I64),
			Ty::Bool => Some(Scalar::Bool),
			Ty::Unit => Some(Scalar::Unit),
			_ => None,
		}
	};
	for q in (0..len).filter(|&q| reached[q]) {
		if let Some(src) = copy_source(callee.operation(q)?) {
			if concrete(states[q * k + src as usize]).is_none() {
				return Ok(Err(InlineReason::CopySource {
					callee_pc: q as u32,
				}));
			}
		}
	}
	let mut ret = None;
	let mut faults = false;
	for q in (0..len).filter(|&q| reached[q]) {
		let op = callee.operation(q)?;
		if let Op::Return { value } = op {
			let Some(ty) = concrete(states[q * k + value as usize]) else {
				return Ok(Err(InlineReason::ReturnSource {
					callee_pc: q as u32,
				}));
			};
			match ret {
				None => ret = Some(ty),
				Some(have) if have == ty => (),
				Some(_) => return Ok(Err(InlineReason::ReturnType)),
			}
		}
		if let Some((_, binary, _, rhs)) = binary_parts(op) {
			if ARITH.contains(&binary)
				&& const_divisor(binary, states[q * k + rhs as usize]).is_none()
			{
				faults = true;
			}
		}
	}
	let Some(ret) = ret else {
		return Ok(Err(InlineReason::NoReturn));
	};

	let mut demands = [None; MAX_INLINE_SLOTS];
	demands[..params].copy_from_slice(&demand);
	let reached_count = reached.iter().filter(|&&r| r).count();
	Ok(Ok(CalleeTyping {
		states,
		reached,
		summary: InlineSummary {
			callee: g,
			slots: callee.slots,
			parameters: callee.parameters,
			demands,
			ret,
			faults,
			nodes: (params + reached_count) as u32,
		},
	}))
}

/// A Div/Rem divisor that is a constant written in the analysed code on every path, outside
/// {0, -1} (entry values never are): no fault exit.
fn const_divisor(op: Binary, rhs: SlotState) -> Option<i64> {
	match (op, rhs.written, rhs.konst) {
		(Binary::Div | Binary::Rem, Written::Yes, Konst::Const(c)) if c != 0 && c != -1 => Some(c),
		_ => None,
	}
}

/// The region transfer extended to B1's body ops: IterNext's write on its some edge and a
/// Call's write of the callee's return type (`ret`).
fn transfer_at(
	op: Op,
	slots: &[u32],
	state: &mut [SlotState],
	ret: Option<Scalar>,
) -> Result<(), Error> {
	let (out, ty) = match op {
		Op::IterNext { out, .. } => (out, Scalar::I64),
		Op::Call { out, .. } => (out, ret.ok_or_else(invalid)?),
		_ => return transfer(op, slots, state),
	};
	let l = slots.binary_search(&out).map_err(|_| invalid())?;
	state[l] = SlotState {
		written: Written::Yes,
		ty: Ty::of(ty),
		konst: Konst::Top,
	};
	Ok(())
}

/// The structural checks of an expansion (exactly one charged node per site, the
/// callee pc 0 node, carrying the site's CalleeFuel exit) and of virtual confinement (
/// no node between `Enter` and a `Ret` defines a caller local, a mask or `next`; every
/// expansion edge stays inside the expansion except `Ret`'s). Any violation is an analysis
/// error (interpreter fallback).
pub(crate) fn check_expansion(region: &Region) -> Result<(), Error> {
	check_parts(
		&region.nodes,
		&region.exits,
		&region.sites,
		region.slots.len(),
	)
}

fn check_parts(nodes: &[Node], exits: &[Exit], sites: &[Site], n: usize) -> Result<(), Error> {
	let n = n as u32;
	let fail = |ok: bool| if ok { Ok(()) } else { Err(invalid()) };
	let caller_end = sites.first().map_or(nodes.len() as u32, |s| s.first);
	let virt = |l: u32, slots: u32| l >= VIRTUAL_BASE && l - VIRTUAL_BASE < slots;

	// Caller nodes: caller locals only, no Arg or Ret, Enter only at a site.
	for (i, node) in nodes.iter().enumerate().take(caller_end as usize) {
		let locals: &[u32] = match node.op {
			NodeOp::Literal { out, .. } | NodeOp::IterNext { out, .. } => &[out][..],
			NodeOp::Copy { out, src, .. } => &[out, src][..],
			NodeOp::Arith { out, lhs, rhs, .. } | NodeOp::Compare { out, lhs, rhs, .. } => {
				&[out, lhs, rhs][..]
			}
			NodeOp::Branch { condition, .. } => &[condition][..],
			NodeOp::Jump { .. } => &[][..],
			NodeOp::Enter { site, .. } => {
				let s = sites.get(site as usize).ok_or_else(invalid)?;
				fail(s.enter == i as u32 && node.charged.is_none())?;
				&[][..]
			}
			NodeOp::Arg { .. } | NodeOp::Ret { .. } => return Err(invalid()),

			#[cfg(test)]
			NodeOp::ForceReplay { .. } => &[][..],
		};
		fail(locals.iter().all(|&l| l < n))?;
	}

	let mut expected_first = caller_end;
	for (k, site) in sites.iter().enumerate() {
		fail(
			site.first == expected_first
				&& site.first <= site.end
				&& site.end as usize <= nodes.len(),
		)?;
		expected_first = site.end;
		let enter = nodes.get(site.enter as usize).ok_or_else(invalid)?;
		fail(
			enter.op
				== NodeOp::Enter {
					site: k as u32,
					slots: site.slots,
				} && enter.charged.is_none(),
		)?;
		fail(site.slots as usize <= MAX_INLINE_SLOTS && site.parameters <= site.slots)?;

		// The site's exits: Replay at the Call pc, never restored, never iter_done.
		let replay_ok = |e: u32, reason: ReplayReason| {
			exits.get(e as usize).is_some_and(|x| {
				x.status == Status::Replay
					&& x.replay == Some(reason)
					&& x.pc == site.pc
					&& !x.restore && !x.iter_done
			})
		};
		fail(replay_ok(site.fuel_exit, ReplayReason::CalleeFuel))?;
		if let Some(e) = site.fault_exit {
			fail(replay_ok(e, ReplayReason::CalleeFault))?;
		}
		let inside = |t: Target| matches!(t, Target::Node(j) if j >= site.first && j < site.end);
		let site_exit = |e: u32| e == site.fuel_exit || Some(e) == site.fault_exit;
		fail(enter.next == Some(Target::Node(site.first)))?;

		let mut charged = 0;
		let mut ret_target = None;
		for (i, node) in nodes[site.first as usize..site.end as usize]
			.iter()
			.enumerate()
		{
			let at_pc0 = i as u32 == site.parameters;
			if let Some(e) = node.charged {
				fail(at_pc0 && e == site.fuel_exit)?;
				charged += 1;
			}
			match node.op {
				NodeOp::Arg { param, src } => {
					fail((i as u32) < site.parameters && param == i as u32 && src < n)?;
					fail(node.next.is_some_and(inside))?;
				}
				NodeOp::Ret { out, src, .. } => {
					fail(out < n && virt(src, site.slots))?;
					// One caller target for every Ret of the site, outside the expansion.
					let next = node.next.ok_or_else(invalid)?;
					fail(!inside(next))?;
					fail(*ret_target.get_or_insert(next) == next)?;
				}
				NodeOp::Literal { out, .. } => {
					fail(virt(out, site.slots) && node.next.is_some_and(inside))?;
				}
				NodeOp::Copy { out, src, .. } => {
					fail(
						virt(out, site.slots)
							&& virt(src, site.slots)
							&& node.next.is_some_and(inside),
					)?;
				}
				NodeOp::Arith {
					out,
					lhs,
					rhs,
					retry,
					..
				} => {
					fail(virt(out, site.slots) && virt(lhs, site.slots) && virt(rhs, site.slots))?;
					fail(retry.is_none_or(site_exit) && node.next.is_some_and(inside))?;
				}
				NodeOp::Compare { out, lhs, rhs, .. } => {
					fail(virt(out, site.slots) && virt(lhs, site.slots) && virt(rhs, site.slots))?;
					fail(node.next.is_some_and(inside))?;
				}
				NodeOp::Branch { condition, yes, no } => {
					fail(
						virt(condition, site.slots)
							&& inside(yes) && inside(no)
							&& node.next.is_none(),
					)?;
				}
				NodeOp::Jump { target } => fail(inside(target) && node.next.is_none())?,
				_ => return Err(invalid()),
			}
		}
		fail(charged == 1)?;

		// No node outside this expansion charges the site's CalleeFuel exit.
		for (i, node) in nodes.iter().enumerate() {
			let own = (i as u32) >= site.first && (i as u32) < site.end;
			fail(own || node.charged != Some(site.fuel_exit))?;
		}
	}
	fail(expected_first as usize == nodes.len() || sites.is_empty())?;
	Ok(())
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
	// P2b: a divisor slot that is also a Copy out is a runtime divisor (no constant flows
	// through Copy), and its Unit writes (Release, Literal Unit) are not divisor values.
	if region
		.nodes
		.iter()
		.any(|n| matches!(n.op, NodeOp::Copy { out, .. } if out == rhs))
	{
		return Some(DivisionKind::Runtime);
	}
	let mut values = region.nodes.iter().filter_map(|n| match n.op {
		NodeOp::Literal { out, value, ty } if out == rhs && ty != Scalar::Unit => Some(value),
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
/// type depends only on the op class, except for Copy, whose result is the source's type
/// when the source is written on every path and Top otherwise (monotone in the source
/// state); operand types and Copy sources are checked after the fixpoint.
fn transfer(op: Op, slots: &[u32], state: &mut [SlotState]) -> Result<(), Error> {
	let Some(out) = write(op) else {
		return Ok(());
	};
	let l = slots.binary_search(&out).map_err(|_| invalid())?;
	state[l] = match op {
		Op::Literal {
			value: Value::Unit, ..
		}
		| Op::Release { .. } => SlotState {
			written: Written::Yes,
			ty: Ty::Unit,
			// Never a divisor constant: a Unit cannot be read by arithmetic (Type refusal).
			konst: Konst::Top,
		},
		Op::Copy { value, .. } => {
			let src = state[slots.binary_search(&value).map_err(|_| invalid())?];
			SlotState {
				written: Written::Yes,
				// Monotone: Yes keeps the source type; anything below Yes gives Top, which the
				// CopySource check after the fixpoint refuses.
				ty: if src.written == Written::Yes {
					src.ty
				} else {
					Ty::Top
				},
				// No constant through Copy: the divisor rule stays "written by an in-region
				// literal on every path".
				konst: Konst::Top,
			}
		}
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
		| Op::TupleFieldGetMut { out, .. }
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
		| Op::TupleFieldSetMut { .. }
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
