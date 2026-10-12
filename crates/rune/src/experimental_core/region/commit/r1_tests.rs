//! R1 (rnx 0184): the canonical-entry contract R1 relies on, from the REAL entry fill.
//!
//! R1 stores the header IterNext's `out` at an exit as `select(written & bit, next - 1, 0)`.
//! Where the bit is clear that is 0, which is exactly the cell's entry value only because
//! the canonical entry buffer comes from `unbox` (the commit module's entry fill, the only
//! writer of the runner's buffer before the region runs): every `Scalar` cell is written 0,
//! whatever the window slot held and whatever the (reused) storage held. The simulator does
//! NOT establish this: it consumes the buffer it is given. The lowerer's own `iconst 0`
//! init is checked in `experimental_native::r1_tests`. Safe glue; Miri-able.
use super::super::b1_tests::{calls_kernel, outcome, range_sum};
use super::super::simulate::simulate;
use super::super::tests::{build, store};
use super::{unbox, ExitRecord, Room};
use crate::experimental_core::heap::Store;
use crate::experimental_core::region::{EntryClass, NodeOp, Region, Status, MAX_REGION_CELLS};
use crate::experimental_core::value::Value;
use std::format;
use std::vec::Vec as StdVec;

const ROOM: Room = Room {
	frames: 1024,
	slots: 65_536,
};

/// The header IterNext's `out` local (a `Scalar` cell in both regions here).
fn header_out(region: &Region) -> usize {
	let NodeOp::IterNext { out, .. } = region.nodes[region.header_node as usize].op else {
		panic!("header is not IterNext");
	};
	assert_eq!(region.entry[out as usize], EntryClass::Scalar);
	out as usize
}

/// The frame slot of local `l`.
fn slot(region: &Region, l: usize) -> usize {
	region.slots[l] as usize
}

/// Original (pre-entry) values of the `Scalar` slots: nonzero I64s at both ends, a Bool, an
/// F64, a Char, and Unit; every one passes `Store::is_slot_scalar`.
fn originals() -> [Value; 7] {
	[
		Value::I64(0x5eed),
		Value::I64(-1),
		Value::I64(i64::MIN),
		Value::I64(i64::MAX),
		Value::Bool(true),
		Value::F64(-2.5),
		Value::Unit,
	]
}

/// calls' main frame at its header over `start..end` (x, total, range, iterator, i,
/// temporary), with `original` in every `Scalar` slot.
fn calls_window(s: &mut Store, region: &Region, original: Value, start: i64, end: i64) -> StdVec<Value> {
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	let mut w = vec![Value::I64(end), Value::I64(7), range, it, Value::Unit, Value::Unit];
	for (l, class) in region.entry.iter().enumerate() {
		if *class == EntryClass::Scalar {
			w[slot(region, l)] = original;
		}
	}
	w
}

/// range_sum's frame at its header over `start..end` (a, b, s, range, iterator, i), with
/// `original` in every `Scalar` slot.
fn range_window(s: &mut Store, region: &Region, original: Value, start: i64, end: i64) -> StdVec<Value> {
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	let mut w = vec![Value::I64(start), Value::I64(end), Value::I64(3), range, it, Value::Unit];
	for (l, class) in region.entry.iter().enumerate() {
		if *class == EntryClass::Scalar {
			w[slot(region, l)] = original;
		}
	}
	w
}

/// Every `Scalar` cell of the filled buffer is 0, the header IterNext's `out` among them.
fn assert_scalar_cells_zero(region: &Region, buf: &[i64], what: &str) {
	for (l, class) in region.entry.iter().enumerate() {
		if *class == EntryClass::Scalar {
			assert_eq!(buf[l], 0, "{what}: Scalar cell {l} not 0 after the fill");
		}
	}
}

/// The REAL fill (`unbox`) writes 0 into the eligible `out` cell (and every `Scalar` cell)
/// before execution, with nonzero original slot values and storage that is dirty twice
/// over: first a sentinel in every cell, then the leftovers of a previous run in the same
/// storage (its `out` cell holding the last `i`, nonzero).
#[test]
fn r1_entry_fill_zeroes_the_iter_out_cell() {
	type Window = fn(&mut Store, &Region, Value, i64, i64) -> StdVec<Value>;
	let cases: [(&str, Region, Window); 2] = [
		("calls", outcome(&calls_kernel(), 0, 4).unwrap(), calls_window),
		("range_sum", outcome(&build(vec![range_sum()]), 0, 4).unwrap(), range_window),
	];
	let mut fills = 0;
	for (name, region, window) in &cases {
		let out = header_out(region);
		let cells = region.cells();
		for original in originals() {
			let what = format!("{name} original={original:?}");
			let mut s = store();
			let w = window(&mut s, region, original, 3, 9);
			assert_eq!(w[slot(region, out)], original, "{what}: the out slot holds the original value");

			// Dirty storage: a sentinel in every cell.
			let mut storage = [0x5a5a_5a5a_5a5a_5a5a_i64; MAX_REGION_CELLS];
			let (fuel, iter) = unbox(region, &w, &s, false, 10_000, ROOM, &mut storage).unwrap();
			assert!(iter.is_some(), "{what}");
			assert_scalar_cells_zero(region, &storage[..cells], &what);
			assert_eq!(storage[out], 0, "{what}: out cell after the first fill");
			fills += 1;

			// Run it (the simulator only consumes the buffer), so the same storage now holds
			// the last `i` (8) in the out cell, then fill the SAME storage again.
			let mut record = ExitRecord::new(fuel);
			let status = simulate(region, &mut storage[..cells], &mut record);
			assert_eq!(status, Status::Continue as u32, "{what}");
			assert_eq!(storage[out], 8, "{what}: the run left the last i in the out cell");
			let (_, _) = unbox(region, &w, &s, false, 10_000, ROOM, &mut storage).unwrap();
			assert_scalar_cells_zero(region, &storage[..cells], &what);
			assert_eq!(storage[out], 0, "{what}: out cell after refilling reused storage");
			fills += 1;
		}
	}
	std::println!("R1\tentry_fill\tregions={}\tfills={fills}", cases.len());
}
