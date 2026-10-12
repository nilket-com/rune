//! B1 commit tests (rnx 0184): `validate` and `apply` driven directly (private to the
//! commit module). The iterator checks V3 with the widened progress check, the Replay
//! fuel rules, the M1-before-M2 order and the no-mutation property of every refusal,
//! compared with the state immediately BEFORE validation. Safe glue; Miri-able.
use super::super::b1_tests::{calls_kernel, outcome, range_sum};
use super::super::simulate::simulate;
use super::super::tests::{build, store};
use super::{
	apply, inject_m1_failure_for_test, inject_m2_failure_for_test, iter_progress_ok, unbox, validate,
	ExitRecord, IterEntry, Refused, Room,
};
use crate::experimental_core::error::ErrorKind;
use crate::experimental_core::heap::Store;
use crate::experimental_core::region::{Region, ReplayReason, Status, MAX_REGION_CELLS};
use crate::experimental_core::value::Value;
use std::vec::Vec as StdVec;

const ROOM: Room = Room {
	frames: 1024,
	slots: 65_536,
};

fn range_region() -> Region {
	outcome(&build(vec![range_sum()]), 0, 4).unwrap()
}

fn calls_region() -> Region {
	outcome(&calls_kernel(), 0, 4).unwrap()
}

/// range_sum's frame at its header over `start..end`: a, b, s, range, iterator, i.
fn range_window(s: &mut Store, start: i64, end: i64) -> StdVec<Value> {
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	vec![Value::I64(start), Value::I64(end), Value::I64(0), range, it, Value::Unit]
}

/// calls' main frame at its header: x, total, range, iterator, i, temporary.
fn calls_window(s: &mut Store, total: i64, start: i64, end: i64) -> StdVec<Value> {
	let range = s.range(start, end).unwrap();
	let it = s.iterator(range).unwrap();
	vec![Value::I64(end), Value::I64(total), range, it, Value::Unit, Value::Unit]
}

/// Entry steps then the simulator: the buffer, the record, the returned status and the
/// iterator entry, with nothing committed.
struct Run {
	storage: [i64; MAX_REGION_CELLS],
	cells: usize,
	record: ExitRecord,
	returned: u32,
	entry_fuel: u64,
	iter: Option<IterEntry>,
}

fn run(region: &Region, window: &[Value], store: &Store, fuel: usize) -> Run {
	let mut storage = [0i64; MAX_REGION_CELLS];
	let (entry_fuel, iter) = unbox(region, window, store, false, fuel, ROOM, &mut storage).unwrap();
	let cells = region.cells();
	let mut record = ExitRecord::new(entry_fuel);
	let returned = simulate(region, &mut storage[..cells], &mut record);
	Run {
		storage,
		cells,
		record,
		returned,
		entry_fuel,
		iter,
	}
}

fn snapshot(window: &[Value], store: &Store) -> (StdVec<Value>, std::string::String) {
	(window.to_vec(), store.state_for_test())
}

// ------------------------------------------------------------------- B1-14

#[test]
fn b1_commit_validates_iter_cells_before_any_write() {
	let region = range_region();
	let n = region.slots.len();
	type Forge = fn(usize, &mut [i64], &mut ExitRecord);
	let forgeries: [(&str, i64, i64, usize, Forge); 6] = [
		("next below the entry next", 0, 10, 5, |n, b, _| b[n] = -1),
		("next above end", 0, 10, 5, |n, b, _| b[n] = 11),
		("end cell changed", 0, 10, 5, |n, b, _| b[n + 1] = 9),
		("iter_done with next < end", 0, 3, 100, |n, b, _| b[n] = 2),
		// Fuel 5 with a 0..10 range: two iterations are paid, ten claimed.
		("progress above consumed fuel", 0, 10, 5, |n, b, _| b[n] = 10),
		("next moved from an exhausted entry", 7, 7, 5, |n, b, _| b[n] = 8),
	];
	for (name, start, end, fuel, forge) in forgeries {
		let mut s = store();
		let keep = s.string("keep").unwrap();
		let mut w = range_window(&mut s, start, end);
		let mut r = run(&region, &w, &s, fuel);
		forge(n, &mut r.storage[..r.cells], &mut r.record);
		let before = snapshot(&w, &s);
		let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit), "{name}");
		assert_eq!(snapshot(&w, &s), before, "{name}: nothing mutated");
		// The honest run of the same entry commits.
		let honest = run(&region, &w, &s, fuel);
		let plan = validate(
			&region,
			&w,
			&s,
			honest.entry_fuel,
			honest.iter,
			&honest.storage[..honest.cells],
			&honest.record,
			honest.returned,
		)
		.unwrap_or_else(|e| panic!("{name}: honest record refused {e:?}"));
		apply(plan, &mut w, &mut s).unwrap().unwrap();
		s.release(keep).unwrap();
	}
}

// ------------------------------------------------------------------- B1-15

#[test]
fn b1_iter_progress_widened_boundaries() {
	let (min, max) = (i64::MIN, i64::MAX);
	// Direct.
	assert!(iter_progress_ok(min, max, u64::MAX, 0), "(a) 2^64 - 1 <= consumed");
	assert!(!iter_progress_ok(min, max, u64::MAX, 1), "(b) one below");
	assert!(iter_progress_ok(min, min, 0, 0), "(c) progress 0");
	assert!(iter_progress_ok(5, 5, 0, 0), "(d) reversed, unchanged");
	assert!(!iter_progress_ok(5, 4, 100, 0), "(d) negative progress");
	assert!(iter_progress_ok(0, 0, 0, 0), "(e) empty");
	assert!(!iter_progress_ok(0, 1, 0, 0), "(e) empty, moved without fuel");
	assert!(!iter_progress_ok(0, max, 3, 0), "(f) large progress, small fuel");
	assert!(!iter_progress_ok(min, 0, 10, 0), "(f)");
	assert!(!iter_progress_ok(0, 0, 0, u64::MAX), "(g) record fuel above entry fuel");
	assert!(!iter_progress_ok(max - 1, min, u64::MAX, 0), "(h) the old wrap case");

	// Through `validate`, with records built for each case against the real payload.
	let region = range_region();
	let n = region.slots.len();
	let exit_of = |pred: fn(&super::super::Exit) -> bool| region.exits.iter().position(pred).unwrap() as u32;
	let done = exit_of(|e| e.iter_done);
	let fuel_exit = exit_of(|e| e.status == Status::Fuel);
	// (start, end, next, entry fuel, record fuel, exit, accepted)
	let cases: [(&str, i64, i64, i64, u64, u64, u32, bool); 12] = [
		("a", min, max, max, u64::MAX, 0, done, true),
		("b", min, max, max, u64::MAX, 1, done, false),
		("c", min, max, min, 5, 0, fuel_exit, true),
		("d next=5", 5, 2, 5, 5, 0, fuel_exit, true),
		("d next=4", 5, 2, 4, 5, 0, fuel_exit, false),
		("d next=6", 5, 2, 6, 5, 0, fuel_exit, false),
		("e next=0", 0, 0, 0, 5, 0, done, true),
		("e next=1", 0, 0, 1, 5, 0, done, false),
		("f (0, MAX)", 0, max, max, 3, 0, done, false),
		("f (MIN, 0)", min, 0, 0, 10, 0, done, false),
		("g", 0, 10, 0, 0, u64::MAX, fuel_exit, false),
		("h", max - 1, max, min, u64::MAX, 0, fuel_exit, false),
	];
	for (name, start, end, next, entry_fuel, record_fuel, exit, accepted) in cases {
		let mut s = store();
		let w = range_window(&mut s, start, end);
		let e = region.exits[exit as usize];
		let mut buf = [0i64; MAX_REGION_CELLS];
		buf[n] = next;
		buf[n + 1] = end;
		let mut record = ExitRecord::new(entry_fuel);
		record.fuel = record_fuel;
		record.written = e.definite;
		record.status = e.status as u32;
		record.exit = exit;
		record.pc = e.pc;
		let iter = Some(IterEntry {
			value: w[4],
			next: start,
			end,
		});
		let before = snapshot(&w, &s);
		let plan = validate(&region, &w, &s, entry_fuel, iter, &buf[..n + 2], &record, record.status);
		assert_eq!(plan.is_ok(), accepted, "({name})");
		drop(plan);
		assert_eq!(snapshot(&w, &s), before, "({name})");
	}
}

// ------------------------------------------------------------------- B1-16

#[test]
fn b1_replay_record_validation() {
	let region = calls_region();
	let n = region.slots.len();
	let (fuel_exit, fault_exit) = (2u32, 3u32);
	assert_eq!(region.exits[2].replay, Some(ReplayReason::CalleeFuel));
	assert_eq!(region.exits[3].replay, Some(ReplayReason::CalleeFault));

	// An honest CalleeFuel checkpoint: entry fuel 1 pays the header; the callee's charge
	// finds 0. The checkpoint is the state before the Call: i written, `next` advanced.
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	let r = run(&region, &w, &s, 1);
	assert_eq!(r.returned, Status::Replay as u32);
	assert_eq!((r.record.exit, r.record.fuel, r.record.pc), (fuel_exit, 0, 5));
	assert_eq!(r.storage[n], 1, "the first IterNext's increment is kept");

	type Forge = fn(&mut [i64], &mut ExitRecord) -> u32;
	let forgeries: [(&str, Forge); 8] = [
		("CalleeFuel with fuel 1", |_, r| {
			r.fuel = 1;
			r.status
		}),
		("status 3 at a Replay exit", |_, r| {
			r.status = Status::Retry as u32;
			r.status
		}),
		("returned differs", |_, r| r.status + 1),
		("Replay pc other than the Call pc", |_, r| {
			r.pc = 6;
			r.status
		}),
		("fuel above entry", |_, r| {
			r.fuel = 2;
			r.status
		}),
		("written bit past n", |_, r| {
			r.written |= 1 << 3;
			r.status
		}),
		("definite slot not written", |_, r| {
			r.written &= !0b010;
			r.status
		}),
		("status 4 at a non-Replay exit", |_, r| {
			r.exit = 1;
			r.pc = 9;
			r.status
		}),
	];
	for (name, forge) in forgeries {
		let mut r = run(&region, &w, &s, 1);
		let returned = forge(&mut r.storage[..r.cells], &mut r.record);
		let before = snapshot(&w, &s);
		let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit), "{name}");
		assert_eq!(snapshot(&w, &s), before, "{name}");
	}

	// CalleeFault with fuel 0 (an honest fault needs fuel >= 1 at the checkpoint).
	let mut r = run(&region, &w, &s, 1);
	r.record.exit = fault_exit;
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned);
	assert_eq!(plan.err(), Some(Refused::InvalidExit), "CalleeFault with fuel 0");

	// A Replay exit whose static map carries `iter_done` (a forged region) is refused.
	let mut forged = calls_region();
	forged.exits[2].iter_done = true;
	let r = run(&forged, &w, &s, 1);
	let plan = validate(&forged, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned);
	assert_eq!(plan.err(), Some(Refused::InvalidExit), "Replay with iter_done");
	let mut forged = calls_region();
	forged.exits[2].restore = true;
	let r = run(&forged, &w, &s, 1);
	let plan = validate(&forged, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned);
	assert_eq!(plan.err(), Some(Refused::InvalidExit), "Replay with restore");

	// The honest checkpoint commits exactly the pre-Call slots and `next`: i = 0 written,
	// total and the temporary untouched (not written on this path), next = 1.
	let r = run(&region, &w, &s, 1);
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned).unwrap();
	let exited = apply(plan, &mut w, &mut s).unwrap().unwrap();
	assert_eq!((exited.status, exited.pc, exited.fuel), (Status::Replay, 5, 0));
	assert_eq!(w[4], Value::I64(0));
	assert_eq!((w[1], w[5]), (Value::I64(0), Value::Unit));
	assert_eq!(s.range_iterator_for_native(w[3]), Some((1, 10)));

	// An honest CalleeFault: total = MAX, so add(MAX, 0)... is fine, add(MAX, 1) faults
	// after the callee's charge succeeded.
	let mut s = store();
	let mut w = calls_window(&mut s, i64::MAX, 1, 10);
	let r = run(&region, &w, &s, 50);
	assert_eq!((r.returned, r.record.exit), (Status::Replay as u32, fault_exit));
	assert_eq!(r.record.fuel, 49, "the checkpoint: the header paid, the callee's charge undone");
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned).unwrap();
	let exited = apply(plan, &mut w, &mut s).unwrap().unwrap();
	assert_eq!((exited.status, exited.pc, exited.fuel), (Status::Replay, 5, 49));
	assert_eq!(w[4], Value::I64(1));
	assert_eq!(s.range_iterator_for_native(w[3]), Some((2, 10)));
}

// ------------------------------------------------------------------- B1-17

#[test]
fn b1_payload_commit_atomic_with_scalar_writeback() {
	let region = calls_region();
	type Perturb = fn(&mut Store, &[Value]);
	let perturbations: [(&str, Perturb); 6] = [
		("a invalidate", |s, w| s.invalidate(w[3]).unwrap()),
		("b one iterator_next", |s, w| {
			s.iterator_next(w[3]).unwrap();
		}),
		("c readers", |s, w| s.set_readers_for_test(w[3], 1)),
		("d retire and reuse", |s, w| {
			// The window still names the old handle; its entry is retired and the slot
			// reused with the next generation.
			s.release(w[3]).unwrap();
			let _ = s.string("reused").unwrap();
		}),
		("e close", |s, _| s.close().unwrap()),
		("f queued work", |s, _| {
			let v = s.string("queued").unwrap();
			s.push_work_for_test(v);
		}),
	];
	for (name, perturb) in perturbations {
		let mut s = store();
		let w = calls_window(&mut s, 0, 0, 10);
		let r = run(&region, &w, &s, 9);
		perturb(&mut s, &w);
		// The property: a rejected commit changes nothing relative to the state
		// immediately BEFORE validation (the perturbed state).
		let before = snapshot(&w, &s);
		let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit), "{name}");
		assert_eq!(snapshot(&w, &s), before, "{name}");
	}

	// An injected M1 failure in `apply`: InvalidExit, and NO slot written.
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	let r = run(&region, &w, &s, 9);
	assert_ne!(r.record.written, 0);
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned).unwrap();
	let before = snapshot(&w, &s);
	inject_m1_failure_for_test();
	assert_eq!(apply(plan, &mut w, &mut s).unwrap(), Err(Refused::InvalidExit));
	assert_eq!(snapshot(&w, &s), before, "M1 refused: nothing mutated");

	// The same record without the injection commits next and every validated slot.
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned).unwrap();
	apply(plan, &mut w, &mut s).unwrap().unwrap();
	assert_ne!(snapshot(&w, &s), before);
}

/// Codex 8034: an M2 failure after M1 committed `next` is surfaced as an invariant error
/// (an `Err`, never an `Ok(Err(_))` fallback that would pretend nothing happened).
#[test]
fn b1_m2_failure_after_m1_surfaces_as_invariant() {
	let region = calls_region();
	let mut s = store();
	let mut w = calls_window(&mut s, 0, 0, 10);
	let r = run(&region, &w, &s, 9);
	let plan = validate(&region, &w, &s, r.entry_fuel, r.iter, &r.storage[..r.cells], &r.record, r.returned).unwrap();
	inject_m2_failure_for_test();
	let error = apply(plan, &mut w, &mut s).expect_err("surfaced, not recovered");
	assert_eq!(error.kind, ErrorKind::InvalidCode);
	// M1 ran: the iterator moved, which is exactly why this must not look like a fallback.
	assert_ne!(s.range_iterator_for_native(w[3]), Some((0, 10)));
}
