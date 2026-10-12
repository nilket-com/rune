//! The exit simulator: a safe Rust model of exactly what the native region function does
//! to `buf` and the [`ExitRecord`]. The Cranelift lowering follows the same node walk.
use super::abi::ExitRecord;
use super::{
	NodeOp, Region, Scalar, Status, Target, MAX_INLINE_SLOTS, MAX_REGION_CELLS, MAX_REGION_SLOTS,
	VIRTUAL_BASE,
};
use crate::experimental_core::arithmetic::Binary;

/// I64 arithmetic of the admitted ops; `None` where the interpreter would raise an error
/// (overflow, division by zero, `MIN / -1`, `MIN % -1`).
pub(crate) fn arith(op: Binary, a: i64, b: i64) -> Option<i64> {
	match op {
		Binary::Add => a.checked_add(b),
		Binary::Sub => a.checked_sub(b),
		Binary::Mul => a.checked_mul(b),
		Binary::Div => a.checked_div(b),
		Binary::Rem => a.checked_rem(b),
		_ => None,
	}
}

pub(crate) fn compare(op: Binary, a: i64, b: i64) -> i64 {
	i64::from(match op {
		Binary::Eq => a == b,
		Binary::Ne => a != b,
		Binary::Lt => a < b,
		Binary::Le => a <= b,
		Binary::Gt => a > b,
		_ => a >= b,
	})
}

/// Runs `region` from its header over `buf` (one `i64` per local slot, then, with an
/// iterator, `next` and `end`) with the entry fuel in `record.fuel`; fills `record` and
/// returns the status. A contract violation (which validated regions cannot produce)
/// returns status 0, which the commit step rejects as an invalid exit.
pub(crate) fn simulate(region: &Region, buf: &mut [i64], record: &mut ExitRecord) -> u32 {
	run(region, buf, record, &mut None)
}

/// B1 test trace: the state at every `Enter` (the pre-Call checkpoint) and the number of
/// `Enter` and `Ret` nodes executed.
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Trace {
	pub(crate) snapshots: std::vec::Vec<Snapshot>,
	pub(crate) enters: usize,
	pub(crate) rets: usize,
}

/// The runner's caller state immediately before a Call: `buf[0..=n]` (the caller locals
/// and `next`), the written and dynamic tag masks, and the fuel.
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
	pub(crate) cells: std::vec::Vec<i64>,
	pub(crate) written: u64,
	pub(crate) bools: u64,
	pub(crate) units: u64,
	pub(crate) fuel: u64,
}

/// [`simulate`] that also records the [`Trace`] (tests: the Replay checkpoint check).
#[cfg(test)]
pub(crate) fn simulate_traced(region: &Region, buf: &mut [i64], record: &mut ExitRecord) -> (u32, Trace) {
	let mut trace = Some(Trace::default());
	let status = run(region, buf, record, &mut trace);
	(status, trace.unwrap_or_default())
}

#[cfg(test)]
type Tracer = Option<Trace>;
#[cfg(not(test))]
type Tracer = Option<core::convert::Infallible>;

fn run(region: &Region, buf: &mut [i64], record: &mut ExitRecord, trace: &mut Tracer) -> u32 {
	let n = region.slots.len();
	let cells = region.cells();
	if n > MAX_REGION_SLOTS || cells > MAX_REGION_CELLS || buf.len() != cells {
		return 0;
	}
	#[cfg(not(test))]
	let _ = trace;
	let mut fuel = record.fuel;
	let mut written = 0u64;
	// Dynamic tags, maintained for the region's tracked slots only: the type of the value
	// last written, Bool in `bools`, Unit in `units` (P2b), I64 in neither.
	let mut tags = (0u64, 0u64);
	let tracked = region.tracked;
	// B1: the inlined callee's virtual locals and the checkpoint fuel. Virtual locals live
	// here only: never in `buf`, never in a mask.
	let mut virt = [0i64; MAX_INLINE_SLOTS];
	let mut checkpoint = 0u64;
	let mut node = region.header_node as usize;
	let finish = |record: &mut ExitRecord, buf: &[i64], exit: u32, fuel: u64, written: u64, tags: (u64, u64)| -> u32 {
		let _ = buf;
		let Some(e) = region.exits.get(exit as usize) else {
			return 0;
		};
		record.fuel = fuel;
		record.written = written;
		record.bools = tags.0;
		record.units = tags.1;
		record.status = e.status as u32;
		record.exit = exit;
		record.pc = e.pc;
		record.status
	};
	// The fuel an exit reports: Fuel 0, Retry the fuel before the op's own charge, Replay
	// the checkpoint (the fuel before the Call), Continue the current fuel.
	let exit_fuel = |exit: u32, fuel: u64, before: u64, checkpoint: u64| -> Option<u64> {
		let e = region.exits.get(exit as usize)?;
		Some(match e.status {
			Status::Fuel => 0,
			Status::Retry => before,
			Status::Replay => checkpoint,
			Status::Continue => fuel,
		})
	};
	loop {
		let Some(nd) = region.nodes.get(node) else {
			return 0;
		};
		let before = fuel;
		if let Some(exit) = nd.charged {
			if fuel == 0 {
				let Some(f) = exit_fuel(exit, 0, before, checkpoint) else {
					return 0;
				};
				return finish(record, buf, exit, f, written, tags);
			}
			fuel -= 1;
		}
		// A read of a caller local or a virtual local.
		let read = |buf: &[i64], virt: &[i64; MAX_INLINE_SLOTS], l: u32| -> Option<i64> {
			if l >= VIRTUAL_BASE {
				virt.get((l - VIRTUAL_BASE) as usize).copied()
			} else if (l as usize) < n {
				Some(buf[l as usize])
			} else {
				None
			}
		};
		// A write: a virtual local writes `virt` only; a caller local writes `buf[l]`, its
		// written bit and its tags.
		macro_rules! write {
			($l:expr, $v:expr, $ty:expr) => {{
				let (l, v, ty): (u32, i64, Scalar) = ($l, $v, $ty);
				if l >= VIRTUAL_BASE {
					let Some(slot) = virt.get_mut((l - VIRTUAL_BASE) as usize) else {
						return 0;
					};
					*slot = v;
				} else if (l as usize) < n {
					buf[l as usize] = v;
					written |= 1u64 << l;
					let bit = (1u64 << l) & tracked;
					tags.0 &= !bit;
					tags.1 &= !bit;
					match ty {
						Scalar::Bool => tags.0 |= bit,
						Scalar::Unit => tags.1 |= bit,
						Scalar::I64 => (),
					}
				} else {
					return 0;
				}
			}};
		}
		macro_rules! get {
			($l:expr) => {
				match read(buf, &virt, $l) {
					Some(v) => v,
					None => return 0,
				}
			};
		}
		let next = match nd.op {
			NodeOp::Literal { out, value, ty } => {
				write!(out, value, ty);
				nd.next
			}
			NodeOp::Copy { out, src, ty } => {
				let v = get!(src);
				write!(out, v, ty);
				nd.next
			}
			NodeOp::Arith {
				out,
				op,
				lhs,
				rhs,
				retry,
				..
			} => match arith(op, get!(lhs), get!(rhs)) {
				Some(v) => {
					write!(out, v, Scalar::I64);
					nd.next
				}
				None => match retry {
					Some(exit) => {
						let Some(f) = exit_fuel(exit, fuel, before, checkpoint) else {
							return 0;
						};
						return finish(record, buf, exit, f, written, tags);
					}
					None => return 0,
				},
			},
			NodeOp::Compare { out, op, lhs, rhs } => {
				let v = compare(op, get!(lhs), get!(rhs));
				write!(out, v, Scalar::Bool);
				nd.next
			}
			NodeOp::Branch { condition, yes, no } => Some(if get!(condition) != 0 { yes } else { no }),
			NodeOp::Jump { target } => Some(target),
			NodeOp::IterNext { out, none } => {
				if cells != n + 2 {
					return 0;
				}
				let (next, end) = (buf[n], buf[n + 1]);
				if next >= end {
					Some(none)
				} else {
					let Some(after) = next.checked_add(1) else {
						// Unreachable: next < end.
						return 0;
					};
					buf[n] = after;
					write!(out, next, Scalar::I64);
					nd.next
				}
			}
			NodeOp::Enter { slots, .. } => {
				if slots as usize > MAX_INLINE_SLOTS {
					return 0;
				}
				checkpoint = fuel;
				virt = [0; MAX_INLINE_SLOTS];
				#[cfg(test)]
				if let Some(t) = trace.as_mut() {
					t.enters += 1;
					let mut cells_now = std::vec::Vec::with_capacity(n + 1);
					cells_now.extend_from_slice(&buf[..n]);
					cells_now.push(if cells == n + 2 { buf[n] } else { 0 });
					t.snapshots.push(Snapshot {
						cells: cells_now,
						written,
						bools: tags.0,
						units: tags.1,
						fuel,
					});
				}
				nd.next
			}
			NodeOp::Arg { param, src } => {
				let v = get!(src);
				let Some(slot) = virt.get_mut(param as usize) else {
					return 0;
				};
				*slot = v;
				nd.next
			}
			NodeOp::Ret { out, src, ty } => {
				let v = get!(src);
				if out >= VIRTUAL_BASE {
					return 0;
				}
				write!(out, v, ty);
				#[cfg(test)]
				if let Some(t) = trace.as_mut() {
					t.rets += 1;
				}
				nd.next
			}
			#[cfg(test)]
			NodeOp::ForceReplay { zero, positive } => Some(if checkpoint == 0 { zero } else { positive }),
		};
		match next {
			Some(Target::Node(i)) => node = i as usize,
			Some(Target::Exit(exit)) => {
				let Some(f) = exit_fuel(exit, fuel, before, checkpoint) else {
					return 0;
				};
				return finish(record, buf, exit, f, written, tags);
			}
			None => return 0,
		}
	}
}
