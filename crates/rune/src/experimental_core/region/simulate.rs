//! The exit simulator: a safe Rust model of exactly what the native region function does
//! to `buf` and the [`ExitRecord`]. The Cranelift lowering follows the same node walk.
use super::abi::ExitRecord;
use super::{NodeOp, Region, Scalar, Target};
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

/// Runs `region` from its header over `buf` (one `i64` per local slot) with the entry
/// fuel in `record.fuel`; fills `record` and returns the status. A contract violation
/// (which validated regions cannot produce) returns status 0, which the commit step
/// rejects as an invalid exit.
pub(crate) fn simulate(region: &Region, buf: &mut [i64], record: &mut ExitRecord) -> u32 {
	if buf.len() != region.slots.len() {
		return 0;
	}
	let mut fuel = record.fuel;
	let mut written = 0u64;
	// Dynamic tags, maintained for the region's tracked slots only.
	let mut bools = 0u64;
	let tracked = region.tracked;
	let tag = |bools: &mut u64, out: u32, is_bool: bool| {
		let bit = (1u64 << out) & tracked;
		if is_bool {
			*bools |= bit;
		} else {
			*bools &= !bit;
		}
	};
	let mut node = region.header_node as usize;
	let finish = |record: &mut ExitRecord, exit: u32, fuel: u64, written: u64, bools: u64| -> u32 {
		let Some(e) = region.exits.get(exit as usize) else {
			return 0;
		};
		record.fuel = fuel;
		record.written = written;
		record.bools = bools;
		record.status = e.status as u32;
		record.exit = exit;
		record.pc = e.pc;
		record.status
	};
	loop {
		let Some(n) = region.nodes.get(node) else {
			return 0;
		};
		let before = fuel;
		if let Some(exit) = n.charged {
			if fuel == 0 {
				return finish(record, exit, 0, written, bools);
			}
			fuel -= 1;
		}
		let next = match n.op {
			NodeOp::Literal { out, value, ty } => {
				buf[out as usize] = value;
				written |= 1 << out;
				tag(&mut bools, out, ty == Scalar::Bool);
				n.next
			}
			NodeOp::Arith {
				out,
				op,
				lhs,
				rhs,
				retry,
				..
			} => match arith(op, buf[lhs as usize], buf[rhs as usize]) {
				Some(v) => {
					buf[out as usize] = v;
					written |= 1 << out;
					tag(&mut bools, out, false);
					n.next
				}
				None => match retry {
					Some(exit) => return finish(record, exit, before, written, bools),
					None => return 0,
				},
			},
			NodeOp::Compare { out, op, lhs, rhs } => {
				buf[out as usize] = compare(op, buf[lhs as usize], buf[rhs as usize]);
				written |= 1 << out;
				tag(&mut bools, out, true);
				n.next
			}
			NodeOp::Branch { condition, yes, no } => Some(if buf[condition as usize] != 0 {
				yes
			} else {
				no
			}),
			NodeOp::Jump { target } => Some(target),
		};
		match next {
			Some(Target::Node(i)) => node = i as usize,
			Some(Target::Exit(exit)) => return finish(record, exit, fuel, written, bools),
			None => return 0,
		}
	}
}
