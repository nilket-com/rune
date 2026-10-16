//! Region entry and exit commit: the safe glue between the executor and a region runner
//! (the simulator in tests; the native boundary in production).
//!
//! Order: (1) entry checks, no mutation; (2) unbox into a zeroed stack buffer (B1: plus the
//! iterator's `next` and `end` cells); (3) run; (4) [`validate`] the whole record against
//! the static exit table and re-check the old values, the iterator payload and the store,
//! no mutation (shared borrows only), producing a private [`Plan`]; (5) [`apply`] the plan:
//! M1, the iterator's `next` (the only step that can still refuse, before any mutation),
//! then M2, exactly the validated written slots. Steps 1 to 4 leave every slot, the store
//! and the fuel untouched, so any refusal there is an exact interpreter fallback. Steps 4
//! and 5 are adjacent in [`enter`], under its exclusive borrows of the window and the
//! store, with no callback between them.
use super::abi::ExitRecord;
use super::{EntryClass, Region, ReplayReason, Scalar, Status, MAX_REGION_CELLS, MAX_REGION_SLOTS};
use crate::experimental_core::error::{Error, ErrorKind, Result};
use crate::experimental_core::heap::native::{range_next_reachable, RangeCommitRefused};
use crate::experimental_core::heap::Store;
use crate::experimental_core::value::Value;

/// Why an entry fell back to the interpreter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
	/// A cancellation is pending: the interpreter consumes it at this charged pc.
	Cancel,
	/// The store is closed or has queued release work, so a scalar `put` would not be a
	/// guaranteed no-op.
	Store,
	/// A guarded live-in has another tag, or a written slot holds a value whose release is
	/// not a no-op (a heap handle).
	Tag { slot: u32 },
	/// The window is shorter than a region slot, or the region exceeds the cell bound
	/// (invalid code; unreachable when validated).
	Window,
	/// The runner returned a record that does not match the static exit table.
	InvalidExit,
	/// B1: the header's iterator slot does not hold a live, unleased RangeIterator handle
	/// of this open store.
	Iter { slot: u32 },
	/// B1: not enough frame or slot room for an inlined callee's frame.
	Room,
}

/// B1: the executor's capacity for one more frame of `slots` slots, computed from its frame
/// and slot stacks with the region's window as the top frame (no physical frame growth on
/// the inline path). Zero frame room or insufficient slot room refuses the region before any
/// execution. The interpreter can then grow its buffers through transactional setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Room {
	pub(crate) frames: usize,
	pub(crate) slots: usize,
}

/// Tests: room for any callee (the executor's caps).
#[cfg(test)]
pub(crate) const ROOM_FOR_TEST: Room = Room {
	frames: 1024,
	slots: 65_536,
};

/// The committed outcome: resume the interpreter at `pc` with `fuel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exited {
	pub(crate) status: Status,
	pub(crate) pc: usize,
	pub(crate) fuel: usize,
}

/// B1: the iterator as entry read it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct IterEntry {
	value: Value,
	next: i64,
	end: i64,
}

/// Enters `region` over the current frame's slot `window`. `Ok(Err(_))` is a fallback with
/// nothing changed; `Ok(Ok(_))` has written back the region's slots (and B1's `next`).
/// `Err`: an invariant failure after a partial commit (unreachable by the entry proof),
/// never a silent fallback.
#[inline(never)]
pub(crate) fn enter(
	region: &Region,
	window: &mut [Value],
	store: &mut Store,
	cancel_pending: bool,
	fuel: usize,
	room: Room,
	run: impl FnOnce(&Region, &mut [i64], &mut ExitRecord) -> u32,
) -> Result<core::result::Result<Exited, Refused>> {
	let mut storage = [0i64; MAX_REGION_CELLS];
	let (entry_fuel, iter) = match unbox(
		region,
		window,
		store,
		cancel_pending,
		fuel,
		room,
		&mut storage,
	) {
		Ok(entered) => entered,
		Err(refused) => return Ok(Err(refused)),
	};
	let buf = &mut storage[..region.cells()];
	let mut record = ExitRecord::new(entry_fuel);
	let returned = run(region, buf, &mut record);

	// Validation (no mutation), then the write-back of exactly what was validated.
	let plan = match validate(
		region, window, store, entry_fuel, iter, buf, &record, returned,
	) {
		Ok(plan) => plan,
		Err(refused) => return Ok(Err(refused)),
	};
	apply(plan, window, store)
}

/// Steps 1 and 2: every entry check, then the unboxed buffer. Shared borrows only: a
/// refusal changes nothing. The cell bound is checked first, before any cell is filled.
fn unbox(
	region: &Region,
	window: &[Value],
	store: &Store,
	cancel_pending: bool,
	fuel: usize,
	room: Room,
	storage: &mut [i64; MAX_REGION_CELLS],
) -> core::result::Result<(u64, Option<IterEntry>), Refused> {
	if cancel_pending {
		return Err(Refused::Cancel);
	}
	if !store.scalar_release_is_noop() {
		return Err(Refused::Store);
	}
	let n = region.slots.len();
	let cells = region.cells();
	if n > MAX_REGION_SLOTS || cells > MAX_REGION_CELLS || region.entry.len() != n {
		return Err(Refused::Window);
	}
	// B1: the full callee window, on top of the caller frame (the room the executor
	// computed with the region's window as its top frame).
	if region.callee_slots > 0 && (room.frames < 1 || room.slots < region.callee_slots as usize) {
		return Err(Refused::Room);
	}
	let Ok(entry_fuel) = u64::try_from(fuel) else {
		return Err(Refused::Window);
	};
	let buf = &mut storage[..cells];
	for (l, (&slot, class)) in region.slots.iter().zip(region.entry.iter()).enumerate() {
		let Some(&value) = window.get(slot as usize) else {
			return Err(Refused::Window);
		};
		buf[l] = match (*class, value) {
			(EntryClass::Guard(Scalar::I64), Value::I64(v)) => v,
			(EntryClass::Guard(Scalar::Bool), Value::Bool(b)) => i64::from(b),
			(EntryClass::Scalar, v) if Store::is_slot_scalar(&v) => 0,
			_ => return Err(Refused::Tag { slot }),
		};
	}
	let iter = match region.iter {
		None => None,
		Some(spec) => {
			let Some(&value) = window.get(spec.slot as usize) else {
				return Err(Refused::Window);
			};
			let Some((next, end)) = range_iterator(store, value) else {
				return Err(Refused::Iter { slot: spec.slot });
			};
			buf[n] = next;
			buf[n + 1] = end;
			Some(IterEntry { value, next, end })
		}
	};
	Ok((entry_fuel, iter))
}

fn range_iterator(store: &Store, value: Value) -> Option<(i64, i64)> {
	store.range_iterator_for_native(value)
}

/// V3.6: each increment of `next` follows a header charge that no exit undoes, so the
/// increments never exceed the fuel this entry consumed. Computed in i128/u128: an i64
/// difference such as `i64::MAX - i64::MIN` (2^64 - 1) overflows i64 and must not wrap.
/// `record_fuel <= entry_fuel` is checked before the subtraction.
pub(crate) fn iter_progress_ok(
	entry_next: i64,
	next: i64,
	entry_fuel: u64,
	record_fuel: u64,
) -> bool {
	let Some(progress) = i128::from(next).checked_sub(i128::from(entry_next)) else {
		return false;
	};

	// Non-negative: `next` never moves backwards.
	let Ok(progress) = u128::try_from(progress) else {
		return false;
	};

	// Subtract the record's fuel only after the `<=` check.
	let Some(consumed) = entry_fuel.checked_sub(record_fuel) else {
		return false;
	};

	progress <= u128::from(consumed)
}

/// B1: the validated iterator commit. Built only by [`validate`]; private fields.
#[derive(Clone, Copy, Debug)]
struct IterCommit {
	value: Value,
	expect: (i64, i64),
	new: i64,
}

/// What [`apply`] writes, as [`validate`] established it. Private, built only by
/// `validate`, consumed by `apply`. It borrows the region and the run's buffer for `'r`,
/// so it cannot outlive or be reused across them, and it records the identity of the
/// window and the store it validated, which `apply` checks before any write. It keeps no
/// reference to the record: the masks below are the validated values, copied.
struct Plan<'r> {
	region: &'r Region,
	buf: &'r [i64],
	/// Identity of the validated window (address and length) and store (address): never
	/// dereferenced, only compared.
	window: (*const Value, usize),
	store: *const Store,
	/// The validated ACTUAL written mask of the path taken (`record.written`).
	written: u64,
	/// The effective tag of each written local, from the exit's static masks or, for its
	/// `dynamic` slots, the record's dynamic masks: Bool in `bools`, Unit in `units`, I64 in
	/// neither. Disjoint, and both within `written`.
	bools: u64,
	units: u64,
	/// B1: `Some` iff the region has an iterator: the payload commit M1 performs.
	iter: Option<IterCommit>,
	exited: Exited,
}

#[cfg(test)]
impl Plan<'_> {
	fn written(&self) -> u64 {
		self.written
	}

	fn bools(&self) -> u64 {
		self.bools
	}

	fn units(&self) -> u64 {
		self.units
	}
}

/// Pure: reads the record, `buf`, the window and the store through shared borrows and
/// mutates nothing. Every failure is [`Refused::InvalidExit`].
///
/// - V1, the record against the static exit table: known status equal to the returned
///   value; exit id in range with matching status and pc; `reserved == 0`; `written`
///   within `definite | maybe` and covering `definite`; the dynamic masks (`bools`,
///   `units`) disjoint, within `tracked` and within `written`; for a written slot that is
///   static at this exit but tracked, the dynamic masks agree with its static tag; payloads
///   of Bool-tagged written slots 0 or 1, of Unit-tagged ones 0; fuel at most the entry
///   fuel (checked before any expression subtracts it), 0 for `Fuel`, at least 1 for a
///   restored `Retry`; B1: a Replay exit carries its reason and no `restore` or
///   `iter_done`, and its fuel is 0 for CalleeFuel, at least 1 for CalleeFault; a
///   non-Replay exit carries no reason.
/// - V2, the old values: the store is open with no queued work, and every written slot's
///   current value is one whose release is a no-op (`Store::is_slot_scalar`). So each
///   `release_slot` in [`apply`] is `Ok(())` with no store access.
/// - V3 (B1, with an iterator): the window still holds the entry's handle; the store's
///   payload is still exactly the entry's `(next, end)` with no reader; `end` unchanged;
///   the final `next` reachable from the entry's; an `iter_done` exit exhausted it; and
///   the progress does not exceed the fuel consumed (widened, [`iter_progress_ok`]).
#[allow(clippy::too_many_arguments)]
fn validate<'r>(
	region: &'r Region,
	window: &[Value],
	store: &Store,
	entry_fuel: u64,
	iter: Option<IterEntry>,
	buf: &'r [i64],
	record: &ExitRecord,
	returned: u32,
) -> core::result::Result<Plan<'r>, Refused> {
	let n = region.slots.len();
	let Some(exit) = region.exits.get(record.exit as usize) else {
		return Err(Refused::InvalidExit);
	};
	if buf.len() != region.cells() || n > MAX_REGION_SLOTS || buf.len() > MAX_REGION_CELLS {
		return Err(Refused::InvalidExit);
	}
	if region.iter.is_some() != iter.is_some() {
		return Err(Refused::InvalidExit);
	}
	let all = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
	let written = record.written;
	let written_ok = written & !(exit.definite | exit.maybe) == 0 && exit.definite & !written == 0;
	let statics_ok = exit.units & exit.bools == 0
		&& (exit.units | exit.bools) & exit.dynamic == 0
		&& (exit.units | exit.bools | exit.dynamic) & !all == 0;
	let masks_ok = record.bools & record.units == 0
		&& (record.bools | record.units) & !region.tracked == 0
		&& (record.bools | record.units) & !written == 0;

	// The effective tag of each written local: the record's dynamic masks for this exit's
	// dynamic slots, the static masks for the others.
	let dynamic = written & exit.dynamic;
	let bools = (record.bools & dynamic) | (exit.bools & written & !exit.dynamic);
	let units = (record.units & dynamic) | (exit.units & written & !exit.dynamic);
	// A tracked slot that is static at this exit: the path-exact dynamic masks must agree
	// with its static tag.
	let agreed = written & region.tracked & !exit.dynamic;
	let agree_ok = record.bools & agreed == exit.bools & agreed
		&& record.units & agreed == exit.units & agreed;
	let payloads_ok = (0..n).all(|l| {
		let bit = 1u64 << l;
		if bools & bit != 0 {
			matches!(buf[l], 0 | 1)
		} else if units & bit != 0 {
			buf[l] == 0
		} else {
			true
		}
	});
	// `record.fuel <= entry_fuel` first; nothing here subtracts.
	let fuel_ok = record.fuel <= entry_fuel
		&& match (exit.status, exit.replay) {
			(Status::Fuel, None) => record.fuel == 0,
			(Status::Retry, None) if exit.restore => record.fuel >= 1,
			(Status::Retry | Status::Continue, None) => true,
			(Status::Replay, Some(ReplayReason::CalleeFuel)) => record.fuel == 0,
			(Status::Replay, Some(ReplayReason::CalleeFault)) => record.fuel >= 1,
			_ => false,
		};
	// B1: a Replay checkpoint is never restored and never an iterator end.
	let replay_ok = exit.status != Status::Replay || (!exit.restore && !exit.iter_done);
	let iter_done_ok = !exit.iter_done || (exit.status == Status::Continue && iter.is_some());
	if returned != record.status
		|| Status::from_u32(record.status) != Some(exit.status)
		|| record.pc != exit.pc
		|| record.reserved != 0
		|| !written_ok
		|| !statics_ok
		|| !masks_ok
		|| !agree_ok
		|| !payloads_ok
		|| !fuel_ok
		|| !replay_ok
		|| !iter_done_ok
	{
		return Err(Refused::InvalidExit);
	}
	let Ok(fuel) = usize::try_from(record.fuel) else {
		return Err(Refused::InvalidExit);
	};

	// V2: re-checked under the same borrows, so apply's infallibility is a local argument.
	if !store.scalar_release_is_noop() {
		return Err(Refused::InvalidExit);
	}
	for (l, &slot) in region.slots.iter().enumerate() {
		if written & (1 << l) == 0 {
			continue;
		}
		match window.get(slot as usize) {
			Some(old) if Store::is_slot_scalar(old) => (),
			_ => return Err(Refused::InvalidExit),
		}
	}

	// V3: the iterator, before any mutation.
	let iter = match (region.iter, iter) {
		(Some(spec), Some(entry)) => {
			let next = buf[n];
			let unchanged_handle = window.get(spec.slot as usize) == Some(&entry.value);
			let unchanged_payload =
				range_iterator(store, entry.value) == Some((entry.next, entry.end));
			let end_ok = buf[n + 1] == entry.end;
			let reachable = range_next_reachable((entry.next, entry.end), next);
			let done_ok = !exit.iter_done || next == entry.next.max(entry.end);
			let progress_ok = iter_progress_ok(entry.next, next, entry_fuel, record.fuel);
			if !(unchanged_handle
				&& unchanged_payload
				&& end_ok && reachable
				&& done_ok && progress_ok)
			{
				return Err(Refused::InvalidExit);
			}
			Some(IterCommit {
				value: entry.value,
				expect: (entry.next, entry.end),
				new: next,
			})
		}
		(None, None) => None,
		_ => return Err(Refused::InvalidExit),
	};

	Ok(Plan {
		region,
		buf,
		window: (window.as_ptr(), window.len()),
		store: store as *const Store,
		written,
		bools,
		units,
		iter,
		exited: Exited {
			status: exit.status,
			pc: exit.pc as usize,
			fuel,
		},
	})
}

#[cfg(test)]
std::thread_local! {
	static INJECT_M1_FAILURE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
	static INJECT_M2_FAILURE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Makes the next M1 refuse, before any mutation (tests only).
#[cfg(test)]
pub(crate) fn inject_m1_failure_for_test() {
	INJECT_M1_FAILURE.with(|c| c.set(true));
}

/// Makes the next M2 fail after M1 committed (tests only): the invariant path.
#[cfg(test)]
pub(crate) fn inject_m2_failure_for_test() {
	INJECT_M2_FAILURE.with(|c| c.set(true));
}

#[cfg(test)]
fn take_injected(flag: &'static std::thread::LocalKey<core::cell::Cell<bool>>) -> bool {
	flag.with(|c| c.replace(false))
}

/// The error of an M2 failure after M1 committed `next`: an invariant failure (the entry
/// proof makes it unreachable), surfaced as an error, never recovered as a fallback.
fn write_back_invariant() -> Error {
	Error::new(ErrorKind::InvalidCode, "native region write-back invariant")
}

/// Applies the plan. M1 (B1): the iterator's `next`, through the setter that re-validates
/// and mutates nothing on refusal: the only step that can still refuse, so it runs before
/// any mutation (`InvalidExit`, nothing written). M2: exactly the plan's written slots,
/// each with its validated tag: `Unit`, `Bool(buf[l] != 0)` or `I64(buf[l])`. Each
/// `release_slot` replaces a value `validate` proved a scalar in an open, idle store, and
/// M1 changes neither `closed`, `work` nor any window slot, so it is `Ok(())` without
/// mutation: exactly the interpreter's last `put` per slot. An M2 failure anyway is an
/// invariant failure, surfaced as an `Err`. Refuses (`InvalidExit`, nothing written) a
/// window or store other than the ones `validate` checked.
fn apply(
	plan: Plan<'_>,
	window: &mut [Value],
	store: &mut Store,
) -> Result<core::result::Result<Exited, Refused>> {
	if plan.window != (window.as_ptr(), window.len())
		|| !core::ptr::eq(plan.store, store as *const Store)
	{
		return Ok(Err(Refused::InvalidExit));
	}

	// M1.
	if let Some(c) = plan.iter {
		#[cfg(test)]
		if take_injected(&INJECT_M1_FAILURE) {
			return Ok(Err(Refused::InvalidExit));
		}
		if commit_next(store, c).is_err() {
			return Ok(Err(Refused::InvalidExit));
		}
	}

	#[cfg(test)]
	if take_injected(&INJECT_M2_FAILURE) {
		return Err(write_back_invariant());
	}

	// M2.
	for (l, &slot) in plan.region.slots.iter().enumerate() {
		let bit = 1u64 << l;
		if plan.written & bit == 0 {
			continue;
		}
		let value = if plan.units & bit != 0 {
			Value::Unit
		} else if plan.bools & bit != 0 {
			Value::Bool(plan.buf[l] != 0)
		} else {
			Value::I64(plan.buf[l])
		};
		let old = core::mem::replace(&mut window[slot as usize], value);
		debug_assert!(Store::is_slot_scalar(&old));
		if store.release_slot(old).is_err() {
			debug_assert!(false, "native region write-back invariant");
			return Err(write_back_invariant());
		}
	}
	Ok(Ok(plan.exited))
}

fn commit_next(store: &mut Store, c: IterCommit) -> core::result::Result<(), RangeCommitRefused> {
	store.commit_range_next_for_native(c.value, c.expect, c.new)
}

/// P2b: `validate` and `apply` directly (private to this module). Safe glue; Miri-able.
#[cfg(test)]
mod tests {
	use super::super::simulate::simulate;
	use super::super::tests::{store, while_region, window};
	use super::{apply, validate, ExitRecord, Refused, Value};
	use crate::experimental_core::region::{EntryClass, Region, Scalar};
	use std::vec::Vec;

	/// The unboxed entry buffer of `region` over `window` (the entry checks passed).
	fn unbox(region: &Region, window: &[Value]) -> Vec<i64> {
		region
			.slots
			.iter()
			.zip(region.entry.iter())
			.map(|(&slot, class)| match (*class, window[slot as usize]) {
				(EntryClass::Guard(Scalar::I64), Value::I64(v)) => v,
				(EntryClass::Guard(Scalar::Bool), Value::Bool(b)) => i64::from(b),
				_ => 0,
			})
			.collect()
	}

	#[test]
	fn p2b_validate_is_pure_and_rechecks_old_values() {
		let (_code, region) = while_region();
		let original = window();
		let mut buf = unbox(&region, &original);
		let mut record = ExitRecord::new(50);
		let returned = simulate(&region, &mut buf, &mut record);
		assert_ne!(record.written, 0);
		let written: Vec<usize> = (0..region.slots.len())
			.filter(|&l| record.written & (1 << l) != 0)
			.collect();

		// (a) A written slot swapped to a heap handle after entry: InvalidExit.
		let mut s = store();
		let mut w = original.clone();
		let slot = region.slots[written[0]] as usize;
		w[slot] = s.string("swapped").unwrap();
		let before = (w.clone(), s.state_for_test());
		let plan = validate(&region, &w, &s, 50, None, &buf, &record, returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit));
		assert_eq!((w.clone(), s.state_for_test()), before);
		s.release(w[slot]).unwrap();

		// (b) One queued release: the store is no longer idle, so a scalar release is not a
		// guaranteed no-op: InvalidExit.
		let mut s = store();
		let queued = s.string("queued").unwrap();
		s.push_work_for_test(queued);
		let w = original.clone();
		let before = (w.clone(), s.state_for_test());
		let plan = validate(&region, &w, &s, 50, None, &buf, &record, returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit));
		assert_eq!((w.clone(), s.state_for_test()), before);

		// (c) A closed store: InvalidExit.
		let mut s = store();
		s.close().unwrap();
		let plan = validate(&region, &original, &s, 50, None, &buf, &record, returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit));

		// (d) Honest: the plan keeps the validated ACTUAL written mask, and `apply` writes
		// exactly those slots, nothing else, with no store mutation.
		let mut s = store();
		let mut w = original.clone();
		let plan = validate(&region, &w, &s, 50, None, &buf, &record, returned).unwrap();
		assert_eq!(plan.written(), record.written);
		assert_eq!(plan.units() & plan.bools(), 0);
		assert_eq!((plan.units() | plan.bools()) & !plan.written(), 0);
		let state = s.state_for_test();
		let exited = apply(plan, &mut w, &mut s).unwrap().unwrap();
		assert_eq!(
			(exited.pc, exited.fuel),
			(record.pc as usize, record.fuel as usize)
		);
		assert_eq!(s.state_for_test(), state);
		for (l, &slot) in region.slots.iter().enumerate() {
			if record.written & (1 << l) == 0 {
				assert_eq!(
					w[slot as usize], original[slot as usize],
					"untouched local {l}"
				);
			} else {
				assert!(
					matches!(
						w[slot as usize],
						Value::I64(_) | Value::Bool(_) | Value::Unit
					),
					"local {l}: {:?}",
					w[slot as usize]
				);
			}
		}

		// (e) A plan is bound to the window it validated: another window of the same length
		// is refused before any write.
		let mut s2 = store();
		let w = original.clone();
		let plan = validate(&region, &w, &s2, 50, None, &buf, &record, returned).unwrap();
		let mut other = original.clone();
		let outcome = apply(plan, &mut other, &mut s2).unwrap();
		assert_eq!(outcome.err(), Some(Refused::InvalidExit));
		assert_eq!(other, original);
		drop(w);

		// (e2) ... and to the store it validated.
		let s3 = store();
		let mut w = original.clone();
		let plan = validate(&region, &w, &s3, 50, None, &buf, &record, returned).unwrap();
		let mut s4 = store();
		let outcome = apply(plan, &mut w, &mut s4).unwrap();
		assert_eq!(outcome.err(), Some(Refused::InvalidExit));
		assert_eq!(w, original);
		drop(s3);

		// (f) Validation reads `buf`: a written Bool payload changed after the run is caught.
		let exit = region.exits[record.exit as usize];
		let bool_local = (0..region.slots.len())
			.find(|&l| exit.bools & (1 << l) != 0 && record.written & (1 << l) != 0)
			.expect("a written Bool slot");
		let mut forged = buf.clone();
		forged[bool_local] = 2;
		let s = store();
		let plan = validate(&region, &original, &s, 50, None, &forged, &record, returned);
		assert_eq!(plan.err(), Some(Refused::InvalidExit));
	}
}

#[cfg(test)]
mod b1_tests;

#[cfg(test)]
mod r1_tests;
