//! Region entry and exit commit: the safe glue between the executor and a region runner
//! (the simulator in tests; the native boundary later).
//!
//! Order: (1) entry checks, no mutation; (2) unbox into a zeroed stack buffer; (3) run;
//! (4) validate the whole record against the static exit table, no mutation; (5) write
//! back the written slots. Steps 1 to 4 leave every slot, the store and the fuel untouched,
//! so any refusal there is an exact interpreter fallback.
use super::abi::ExitRecord;
use super::{EntryClass, Region, Scalar, Status, MAX_REGION_SLOTS};
use crate::experimental_core::error::Result;
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
	/// The window is shorter than a region slot (invalid code; unreachable when validated).
	Window,
	/// The runner returned a record that does not match the static exit table.
	InvalidExit,
}

/// The committed outcome: resume the interpreter at `pc` with `fuel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exited {
	pub(crate) status: Status,
	pub(crate) pc: usize,
	pub(crate) fuel: usize,
}

/// Enters `region` over the current frame's slot `window`. `Ok(Err(_))` is a fallback with
/// nothing changed; `Ok(Ok(_))` has written back the region's slots.
pub(crate) fn enter(
	region: &Region,
	window: &mut [Value],
	store: &mut Store,
	cancel_pending: bool,
	fuel: usize,
	run: impl FnOnce(&Region, &mut [i64], &mut ExitRecord) -> u32,
) -> Result<core::result::Result<Exited, Refused>> {
	if cancel_pending {
		return Ok(Err(Refused::Cancel));
	}
	if !store.scalar_release_is_noop() {
		return Ok(Err(Refused::Store));
	}
	let n = region.slots.len();
	if n > MAX_REGION_SLOTS || region.entry.len() != n {
		return Ok(Err(Refused::Window));
	}
	let Ok(entry_fuel) = u64::try_from(fuel) else {
		return Ok(Err(Refused::Window));
	};
	let mut storage = [0i64; MAX_REGION_SLOTS];
	let buf = &mut storage[..n];
	for (l, (&slot, class)) in region.slots.iter().zip(region.entry.iter()).enumerate() {
		let Some(&value) = window.get(slot as usize) else {
			return Ok(Err(Refused::Window));
		};
		buf[l] = match (*class, value) {
			(EntryClass::Guard(Scalar::I64), Value::I64(v)) => v,
			(EntryClass::Guard(Scalar::Bool), Value::Bool(b)) => i64::from(b),
			(EntryClass::Scalar, v) if Store::is_slot_scalar(&v) => 0,
			_ => return Ok(Err(Refused::Tag { slot })),
		};
	}
	let mut record = ExitRecord::new(entry_fuel);
	let returned = run(region, buf, &mut record);
	// Validation: nothing has been mutated yet.
	let Some(exit) = region.exits.get(record.exit as usize) else {
		return Ok(Err(Refused::InvalidExit));
	};
	let written_ok =
		record.written & !(exit.definite | exit.maybe) == 0 && exit.definite & !record.written == 0;
	// A written slot is a Bool by its static tag, or, for a type-ambiguous slot, by the
	// dynamic mask, which may only carry tracked bits.
	let is_bool = |l: usize| {
		let bit = 1u64 << l;
		if exit.dynamic & bit != 0 {
			record.bools & bit != 0
		} else {
			exit.bools & bit != 0
		}
	};
	let bools_ok = record.bools & !region.tracked == 0
		&& (0..n)
			.filter(|&l| record.written & (1 << l) != 0 && is_bool(l))
			.all(|l| matches!(buf[l], 0 | 1));
	let fuel_ok = record.fuel <= entry_fuel
		&& match exit.status {
			Status::Fuel => record.fuel == 0,
			Status::Retry if exit.restore => record.fuel >= 1,
			_ => true,
		};
	if returned != record.status
		|| Status::from_u32(record.status) != Some(exit.status)
		|| record.pc != exit.pc
		|| record.reserved != 0
		|| !written_ok
		|| !bools_ok
		|| !fuel_ok
	{
		return Ok(Err(Refused::InvalidExit));
	}
	let Ok(fuel) = usize::try_from(record.fuel) else {
		return Ok(Err(Refused::InvalidExit));
	};
	// Write-back: each old value was checked at entry to be one whose release is a no-op
	// in this open, idle store, and nothing touched the store since, so this is exactly
	// the interpreter's `put` with its release.
	for (l, &slot) in region.slots.iter().enumerate() {
		if record.written & (1 << l) == 0 {
			continue;
		}
		let value = if is_bool(l) {
			Value::Bool(buf[l] != 0)
		} else {
			Value::I64(buf[l])
		};
		let old = core::mem::replace(&mut window[slot as usize], value);
		debug_assert!(Store::is_slot_scalar(&old));
		store.release_slot(old)?;
	}
	Ok(Ok(Exited {
		status: exit.status,
		pc: exit.pc as usize,
		fuel,
	}))
}
