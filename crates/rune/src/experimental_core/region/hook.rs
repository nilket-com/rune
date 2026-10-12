//! Test-only executor hook: installs the regions of one `Code` for the current thread and
//! runs them at their headers, through the exit simulator (`Mode::Simulate`) or, in the P1
//! tests on x86-64 Linux, through the loaded native code (`Mode::Native`), cross-checked
//! against the simulator on every entry. The production tier (`cache`, P2a) is separate,
//! has no simulator check, and never runs while this hook is installed on the thread.
//!
//! Native mode (P1, test-only):
//!
//! - `install_native` compiles every admitted region with `lower::compile` and loads it with
//!   `NativeRegion::load` into this thread's `Installed`, which owns the loaded code (the
//!   type is `!Send`) and drops it with the install. Each compile or load refusal is counted
//!   by its kind; a refused region is never entered (the interpreter runs it).
//! - Each loaded artifact carries the identity of the region it was compiled from (code id,
//!   function id, header pc), checked at every entry.
//! - At entry, the same guard and buffer fill as the simulator path run (`commit::enter`),
//!   then the native call, then the same commit. The simulator also runs, on an independent
//!   copy of the same pre-entry buffer and record: status, record and every written buffer
//!   entry must be equal, or the first mismatch is kept for the test to report.
//! - A `None` from `NativeRegion::call` (contract mismatch) counts as `refused_call` and
//!   falls back to the interpreter with nothing changed.
use super::abi::ExitRecord;
use super::commit::{self, Refused, Room};
use super::simulate::{simulate, simulate_traced};
use super::{analyze, Node, NodeOp, Region, ReplayReason, Status, Target, MAX_REGION_CELLS};
use crate::experimental_core::code::Code;
use crate::experimental_core::heap::Store;
use crate::experimental_core::value::Value;
use core::cell::{Cell, RefCell};
use std::string::String;
use std::vec::Vec as StdVec;

#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
use crate::experimental_native::{audit::Finding, boundary::NativeRegion, lower, NativeRefusal};

/// How installed regions run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
	/// The hook never enters: the plain interpreter.
	Off,
	/// The exit simulator.
	Simulate,
	/// The loaded native code, checked against the simulator on every entry.
	Native,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
	pub(crate) regions: usize,
	pub(crate) entries: usize,
	pub(crate) continue_exits: usize,
	pub(crate) fuel_exits: usize,
	pub(crate) retry_exits: usize,
	pub(crate) refused_cancel: usize,
	pub(crate) refused_store: usize,
	pub(crate) refused_tag: usize,
	pub(crate) refused_other: usize,
	/// Native mode: committed native entries and their exits by kind.
	pub(crate) native_entries: usize,
	pub(crate) native_continue: usize,
	pub(crate) native_fuel: usize,
	pub(crate) native_retry: usize,
	/// Native mode: `NativeRegion::call` returned `None` (fallback, nothing changed).
	pub(crate) refused_call: usize,
	/// Native mode: entries whose native and simulated results differed.
	pub(crate) mismatches: usize,
	/// Native mode, informational only: unwritten buffer entries (unspecified scratch)
	/// that differ between the native and simulated runs.
	pub(crate) scratch_differences: usize,
	/// P2b, committed entries: slots written back as `Unit` by their static exit tag, and
	/// by the record's dynamic unit mask.
	pub(crate) unit_static: usize,
	pub(crate) unit_dynamic: usize,
	/// B1, committed entries: Replay exits (any mode), and by reason.
	pub(crate) replay_exits: usize,
	pub(crate) replay_fuel: usize,
	pub(crate) replay_fault: usize,
	/// B1: committed Continue exits at the IterNext none edge.
	pub(crate) iter_done_exits: usize,
	/// B1, native mode: committed native Replay exits, by reason, and none-edge exits.
	pub(crate) native_replay: usize,
	pub(crate) native_replay_fuel: usize,
	pub(crate) native_replay_fault: usize,
	pub(crate) native_iter_done: usize,
	/// B1: entry refusals by the iterator check and by the frame/slot room.
	pub(crate) refused_iter: usize,
	pub(crate) refused_room: usize,
	/// B1: committed Replays whose checkpoint was compared with the traced simulator's
	/// snapshot at the site's `Enter` (equal, or a mismatch is recorded).
	pub(crate) replay_checked: usize,
}

/// P2b: how many slots a committed (so validated) record wrote back as `Unit`, by static
/// tag and by dynamic tag.
fn unit_writebacks(region: &Region, record: &ExitRecord) -> (usize, usize) {
	let Some(e) = region.exits.get(record.exit as usize) else {
		return (0, 0);
	};
	let by_static = record.written & e.units & !e.dynamic;
	let by_dynamic = record.written & e.dynamic & record.units;
	(
		by_static.count_ones() as usize,
		by_dynamic.count_ones() as usize,
	)
}

/// Compile and load outcomes of one `install_native`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeInstall {
	/// Admitted regions (analysis).
	pub(crate) regions: usize,
	/// Regions compiled, audited and loaded.
	pub(crate) loaded: usize,
	/// Division with a runtime divisor (refused by the IR gate).
	pub(crate) division_runtime: usize,
	/// Division by the constant 0 (keeps its Retry exit; refused).
	pub(crate) division_const0: usize,
	/// Division by the constant -1 (keeps its Retry exit; refused).
	pub(crate) division_constneg1: usize,
	pub(crate) size: usize,
	pub(crate) codegen: usize,
	/// Any other audit finding.
	pub(crate) audit_other: usize,
	/// Isa, Shape or Verifier.
	pub(crate) other: usize,
	/// `NativeRegion::load` returned an error.
	pub(crate) load_refused: usize,
}

pub(crate) use super::{division_kind, DivisionKind};

/// One loaded region and the identity of the analysed region it was compiled from.
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
struct Loaded {
	code: u32,
	function: u32,
	header: u32,
	slots: usize,
	native: NativeRegion,
}

struct Installed {
	code: u32,
	mode: Mode,
	regions: StdVec<Region>,
	/// Parallel to `regions`: `Some` for each region whose native code is loaded.
	#[cfg(all(
		feature = "fresh-core-native",
		target_arch = "x86_64",
		target_os = "linux"
	))]
	native: StdVec<Option<Loaded>>,
	stats: Stats,
}

std::thread_local! {
	static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };

	/// An install set aside by `suspend`, so a plain-interpreter run sees no install at all.
	static SUSPENDED: RefCell<Option<Installed>> = const { RefCell::new(None) };

	/// B1 test protocol: a transform applied to every analysed region at install.
	static TRANSFORM: Cell<Option<fn(&mut Region)>> = const { Cell::new(None) };

	/// The first native/simulator mismatch since the last `take_mismatch`.
	static MISMATCH: RefCell<Option<String>> = const { RefCell::new(None) };

	/// The executor's fuel when its last invocation on this thread ended.
	static FINAL_FUEL: Cell<Option<usize>> = const { Cell::new(None) };
}

fn analysed(code: &Code) -> StdVec<Region> {
	let mut regions = StdVec::new();
	let transform = TRANSFORM.with(|t| t.get());
	for id in 0..code.functions() as u32 {
		for (_, region) in analyze(code, id).expect("analysis") {
			if let Ok(mut region) = region {
				if let Some(transform) = transform {
					transform(&mut region);
				}
				regions.push(region);
			}
		}
	}
	regions
}

/// B1 test protocol: installs a transform applied after analysis to every region of the
/// next installs on this thread (`None` removes it). Test-only hook; production analysis
/// never sees it.
pub(crate) fn set_region_transform_for_test(transform: Option<fn(&mut Region)>) {
	TRANSFORM.with(|t| t.set(transform));
}

/// B1 forced-Replay protocol (B1-42): after every `Enter`, a `ForceReplay` node routes a
/// zero checkpoint to the site's REAL CalleeFuel exit and a positive one to the site's
/// REAL CalleeFault exit, when the site has one (a pure, infallible callee has none, and
/// none is invented: its calls are then forced only at checkpoint 0). Both exits obey the
/// production fuel rules: no reason and no rule is added or relaxed.
pub(crate) fn force_replay(region: &mut Region) {
	for k in 0..region.sites.len() {
		let site = region.sites[k];
		let enter = site.enter as usize;
		let continue_to = region.nodes[enter].next.expect("Enter has a successor");
		let positive = site.fault_exit.map_or(continue_to, Target::Exit);
		let node = region.nodes.len() as u32;
		region
			.nodes
			.try_push(Node {
				pc: site.pc,
				charged: None,
				op: NodeOp::ForceReplay {
					zero: Target::Exit(site.fuel_exit),
					positive,
				},
				next: None,
			})
			.expect("push");
		region.nodes[enter].next = Some(Target::Node(node));
	}
}

/// Analyses every function of `code` and installs its admitted regions for this thread,
/// run by the simulator.
pub(crate) fn install(code: &Code) -> usize {
	let regions = analysed(code);
	let count = regions.len();

	#[cfg(all(
		feature = "fresh-core-native",
		target_arch = "x86_64",
		target_os = "linux"
	))]
	let native = regions.iter().map(|_| None).collect();

	INSTALLED.with(|i| {
		*i.borrow_mut() = Some(Installed {
			code: code.id(),
			mode: Mode::Simulate,
			regions,
			#[cfg(all(
				feature = "fresh-core-native",
				target_arch = "x86_64",
				target_os = "linux"
			))]
			native,
			stats: Stats {
				regions: count,
				..Stats::default()
			},
		})
	});
	count
}

/// As [`install`], and compiles and loads every region's native code; the mode is
/// `Native`. With `dump`, the exact loaded bytes and listings of each region are written
/// there as `f{function}_h{header}.{bin,clif,opt.clif,vcode}`.
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
pub(crate) fn install_native(code: &Code, dump: Option<&std::path::Path>) -> NativeInstall {
	let regions = analysed(code);
	let mut summary = NativeInstall {
		regions: regions.len(),
		..NativeInstall::default()
	};
	let mut native = StdVec::new();
	let mut program_bytes = 0usize;

	for region in &regions {
		let compiled = match lower::compile(region, program_bytes) {
			Ok(compiled) => compiled,
			Err(refusal) => {
				match refusal {
					NativeRefusal::Audit(Finding::Division(_)) => match division_kind(region) {
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
				native.push(None);
				continue;
			}
		};

		assert_eq!(
			compiled.cells(),
			region.cells(),
			"compiled for another region"
		);
		program_bytes += compiled.bytes().len();

		if let Some(dir) = dump {
			let name = std::format!("f{}_h{}", region.function, region.header);
			let listing = compiled.listing();
			std::fs::create_dir_all(dir).expect("dump dir");
			std::fs::write(dir.join(std::format!("{name}.bin")), compiled.bytes()).expect("dump");
			std::fs::write(dir.join(std::format!("{name}.clif")), &listing.clif).expect("dump");
			std::fs::write(
				dir.join(std::format!("{name}.opt.clif")),
				&listing.optimized,
			)
			.expect("dump");
			std::fs::write(dir.join(std::format!("{name}.vcode")), &listing.vcode).expect("dump");
		}

		match NativeRegion::load(&compiled) {
			Ok(loaded) => {
				summary.loaded += 1;
				native.push(Some(Loaded {
					code: code.id(),
					function: region.function,
					header: region.header,
					slots: region.cells(),
					native: loaded,
				}));
			}
			Err(_) => {
				summary.load_refused += 1;
				native.push(None);
			}
		}
	}

	let count = regions.len();
	INSTALLED.with(|i| {
		*i.borrow_mut() = Some(Installed {
			code: code.id(),
			mode: Mode::Native,
			regions,
			native,
			stats: Stats {
				regions: count,
				..Stats::default()
			},
		})
	});
	summary
}

/// Whether this thread has an install (the P2 production tier then stays out).
pub(crate) fn is_installed() -> bool {
	INSTALLED.with(|i| i.borrow().is_some())
}

/// Switches how the installed regions run, keeping the installed (and loaded) regions.
pub(crate) fn set_mode(mode: Mode) {
	INSTALLED.with(|i| {
		if let Some(installed) = i.borrow_mut().as_mut() {
			installed.mode = mode;
		}
	});
}

/// Returns the statistics since the last call (or the install) and resets them.
pub(crate) fn take_stats() -> Option<Stats> {
	INSTALLED.with(|i| {
		i.borrow_mut().as_mut().map(|installed| {
			let regions = installed.stats.regions;
			core::mem::replace(
				&mut installed.stats,
				Stats {
					regions,
					..Stats::default()
				},
			)
		})
	})
}

/// Sets this thread's install aside (the hook then sees none) until `resume`.
pub(crate) fn suspend() {
	let installed = INSTALLED.with(|i| i.borrow_mut().take());
	SUSPENDED.with(|s| *s.borrow_mut() = installed);
}

/// Restores the install set aside by `suspend`.
pub(crate) fn resume() {
	let installed = SUSPENDED.with(|s| s.borrow_mut().take());
	INSTALLED.with(|i| *i.borrow_mut() = installed);
}

/// Removes the installed regions (dropping any loaded native code) and returns their
/// statistics.
pub(crate) fn uninstall() -> Option<Stats> {
	INSTALLED.with(|i| i.borrow_mut().take().map(|i| i.stats))
}

/// The first native/simulator mismatch since the last call, if any.
pub(crate) fn take_mismatch() -> Option<String> {
	MISMATCH.with(|m| m.borrow_mut().take())
}

/// Called by the executor when an invocation ends (test builds with the feature only).
pub(crate) fn record_final_fuel(fuel: usize) {
	FINAL_FUEL.with(|f| f.set(Some(fuel)));
}

/// The fuel the last invocation on this thread ended with.
pub(crate) fn take_final_fuel() -> Option<usize> {
	FINAL_FUEL.with(|f| f.take())
}

/// Called by the executor at a charged instruction it did not just resume at, before the
/// instruction's own cancel/fuel step. `Some((pc, fuel))`: the region ran and committed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_enter(
	code: u32,
	function: u32,
	pc: usize,
	window: &mut [Value],
	store: &mut Store,
	cancel_pending: bool,
	fuel: usize,
	room: Room,
) -> Option<(usize, usize)> {
	INSTALLED.with(|i| {
		let mut installed = i.borrow_mut();
		let installed = installed.as_mut()?;
		if installed.code != code || installed.mode == Mode::Off {
			return None;
		}

		let index = installed
			.regions
			.iter()
			.position(|r| r.function == function && r.header as usize == pc)?;

		let mut seen = None;
		let mut traced = None;
		let (outcome, native) = match installed.mode {
			Mode::Off => return None,
			Mode::Simulate => (
				commit::enter(
					&installed.regions[index],
					window,
					store,
					cancel_pending,
					fuel,
					room,
					|region, buf, record| {
						let mut pre = [0i64; MAX_REGION_CELLS];
						pre[..buf.len()].copy_from_slice(buf);
						let pre_record = *record;
						let status = simulate(region, buf, record);
						seen = Some(*record);
						traced = Some(trace_check(
							region,
							&pre[..buf.len()],
							pre_record,
							buf,
							record,
						));
						status
					},
				),
				false,
			),
			Mode::Native => {
				let (outcome, record, check) =
					enter_native(installed, index, window, store, cancel_pending, fuel, room)?;
				seen = record;
				traced = check;
				(outcome, true)
			}
		};

		let outcome = outcome.expect("commit cannot fail after a successful entry check");
		let stats = &mut installed.stats;
		match outcome {
			Ok(exited) => {
				stats.entries += 1;
				let region = &installed.regions[index];
				let exit = seen.and_then(|record| region.exits.get(record.exit as usize).copied());
				if let Some(record) = seen {
					let (by_static, by_dynamic) = unit_writebacks(region, &record);
					stats.unit_static += by_static;
					stats.unit_dynamic += by_dynamic;
				}
				let iter_done = exit.is_some_and(|e| e.iter_done);
				let reason = exit.and_then(|e| e.replay);
				match exited.status {
					Status::Continue => stats.continue_exits += 1,
					Status::Fuel => stats.fuel_exits += 1,
					Status::Retry => stats.retry_exits += 1,
					Status::Replay => stats.replay_exits += 1,
				}
				stats.iter_done_exits += usize::from(iter_done);
				stats.replay_fuel += usize::from(reason == Some(ReplayReason::CalleeFuel));
				stats.replay_fault += usize::from(reason == Some(ReplayReason::CalleeFault));
				if exited.status == Status::Replay {
					match traced {
						Some(Ok(())) => stats.replay_checked += 1,
						Some(Err(text)) => {
							stats.mismatches += 1;
							MISMATCH.with(|m| {
								m.borrow_mut().get_or_insert(text);
							});
						}
						None => (),
					}
				}
				if native {
					stats.native_entries += 1;
					match exited.status {
						Status::Continue => stats.native_continue += 1,
						Status::Fuel => stats.native_fuel += 1,
						Status::Retry => stats.native_retry += 1,
						Status::Replay => stats.native_replay += 1,
					}
					stats.native_iter_done += usize::from(iter_done);
					stats.native_replay_fuel +=
						usize::from(reason == Some(ReplayReason::CalleeFuel));
					stats.native_replay_fault +=
						usize::from(reason == Some(ReplayReason::CalleeFault));
				}
				Some((exited.pc, exited.fuel))
			}
			Err(refused) => {
				match refused {
					Refused::Cancel => stats.refused_cancel += 1,
					Refused::Store => stats.refused_store += 1,
					Refused::Tag { .. } => stats.refused_tag += 1,
					Refused::Iter { .. } => stats.refused_iter += 1,
					Refused::Room => stats.refused_room += 1,
					Refused::Window | Refused::InvalidExit => {
						let region = &installed.regions[index];
						panic!(
							"region contract violation f{} h{}: {refused:?}",
							region.function, region.header
						)
					}
				}
				None
			}
		}
	})
}

type Entered =
	crate::experimental_core::error::Result<core::result::Result<commit::Exited, Refused>>;

/// B1 (B1-41): the traced simulator run from the same pre-entry buffer and record. Its
/// status and record must equal the run's; at a Replay exit the committed checkpoint must
/// equal the snapshot at the last `Enter` (written, bools, units, fuel, the slot values
/// under `written`, `next`), and every completed call wrote its result exactly once (one
/// `Ret` per `Enter`, but the last when the exit is a Replay).
fn trace_check(
	region: &Region,
	pre: &[i64],
	pre_record: ExitRecord,
	buf: &[i64],
	record: &ExitRecord,
) -> Result<(), String> {
	let mut tbuf = [0i64; MAX_REGION_CELLS];
	let cells = pre.len();
	tbuf[..cells].copy_from_slice(pre);
	let mut trecord = pre_record;
	let (status, trace) = simulate_traced(region, &mut tbuf[..cells], &mut trecord);
	let n = region.slots.len();
	let describe = |what: &str| {
		std::format!(
			"region f{} h{}: {what}\n  pre {:?} {:?}\n  run record {:?} buf {:?}\n  traced status {status} record {:?} buf {:?} trace {:?}",
			region.function,
			region.header,
			pre,
			pre_record,
			record,
			buf,
			trecord,
			&tbuf[..cells],
			trace
		)
	};
	if status != record.status || trecord != *record {
		return Err(describe("traced simulator differs from the run"));
	}
	let replay = region
		.exits
		.get(record.exit as usize)
		.is_some_and(|e| e.status == Status::Replay);
	let completed = trace.enters - usize::from(replay);
	if trace.rets != completed {
		return Err(describe(
			"a completed call did not write its result exactly once",
		));
	}
	if replay {
		let Some(snap) = trace.snapshots.last() else {
			return Err(describe("Replay without an Enter"));
		};
		let values_ok = (0..n)
			.filter(|&l| record.written & (1 << l) != 0)
			.all(|l| buf[l] == snap.cells[l]);
		let next_ok = region.iter.is_none() || buf[n] == snap.cells[n];
		if (record.written, record.bools, record.units, record.fuel)
			!= (snap.written, snap.bools, snap.units, snap.fuel)
			|| !values_ok
			|| !next_ok
		{
			return Err(describe(
				"committed checkpoint differs from the pre-Call snapshot",
			));
		}
	}
	Ok(())
}

/// Native mode entry. `None`: the region has no loaded code (its compile or load was
/// refused) or the call refused its arguments; the interpreter runs the instruction. With
/// the outcome, the native record the commit validated.
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
#[allow(clippy::too_many_arguments)]
fn enter_native(
	installed: &mut Installed,
	index: usize,
	window: &mut [Value],
	store: &mut Store,
	cancel_pending: bool,
	fuel: usize,
	room: Room,
) -> Option<(Entered, Option<ExitRecord>, Option<Result<(), String>>)> {
	let region = &installed.regions[index];
	let loaded = installed.native.get(index)?.as_ref()?;
	assert!(
		loaded.code == installed.code
			&& loaded.function == region.function
			&& loaded.header == region.header
			&& loaded.slots == region.cells(),
		"loaded artifact does not belong to region f{} h{}",
		region.function,
		region.header
	);

	let mut call_refused = false;
	let mut mismatch = None;
	let mut seen = None;
	let mut traced = None;
	let outcome = commit::enter(
		region,
		window,
		store,
		cancel_pending,
		fuel,
		room,
		|region, buf, record| {
			let mut pre = [0i64; MAX_REGION_CELLS];
			pre[..buf.len()].copy_from_slice(buf);
			let pre_record = *record;
			let status = native_run(
				region,
				&loaded.native,
				buf,
				record,
				&mut call_refused,
				&mut mismatch,
			);
			seen = Some(*record);
			if !call_refused {
				traced = Some(trace_check(
					region,
					&pre[..buf.len()],
					pre_record,
					buf,
					record,
				));
			}
			status
		},
	);

	let stats = &mut installed.stats;
	if let Some((text, scratch)) = mismatch {
		if text.is_empty() {
			stats.scratch_differences += scratch;
		} else {
			stats.mismatches += 1;
			MISMATCH.with(|m| {
				m.borrow_mut().get_or_insert(text);
			});
		}
	}

	if call_refused {
		stats.refused_call += 1;
		return None;
	}
	Some((outcome, seen, traced))
}

#[cfg(not(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
)))]
#[allow(clippy::too_many_arguments)]
fn enter_native(
	_: &mut Installed,
	_: usize,
	_: &mut [Value],
	_: &mut Store,
	_: bool,
	_: usize,
	_: Room,
) -> Option<(Entered, Option<ExitRecord>, Option<Result<(), String>>)> {
	None
}

/// The native run of one entry, checked against the simulator on an independent copy of
/// the same pre-entry buffer and record. `mismatch` receives `(description, 0)` on a
/// mismatch, or `("", n)` with the count of differing unwritten (unspecified) entries.
#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
fn native_run(
	region: &Region,
	native: &NativeRegion,
	buf: &mut [i64],
	record: &mut ExitRecord,
	call_refused: &mut bool,
	mismatch: &mut Option<(String, usize)>,
) -> u32 {
	// B1: `buf` is the region's cells: the slots, then `next` and `end` with an iterator.
	let n = buf.len();
	let slots = region.slots.len();
	let mut pre = [0i64; MAX_REGION_CELLS];
	pre[..n].copy_from_slice(buf);
	let pre_record = *record;

	let Some(status) = native.call(buf, record) else {
		// Nothing ran: the record still says "not written", which commit rejects before
		// any write; the hook turns that into a fallback.
		*call_refused = true;
		return 0;
	};

	let mut sim = pre;
	let mut sim_record = pre_record;
	let sim_status = simulate(region, &mut sim[..n], &mut sim_record);

	let written_differs = (0..slots)
		.filter(|&l| (record.written | sim_record.written) & (1 << l) != 0)
		.any(|l| buf[l] != sim[l]);
	// B1: `next` equal on every exit, `end` unchanged in both runs.
	let iter_differs = region.iter.is_some()
		&& (n != slots + 2
			|| buf[slots] != sim[slots]
			|| buf[slots + 1] != pre[slots + 1]
			|| sim[slots + 1] != pre[slots + 1]);
	if status != sim_status || *record != sim_record || written_differs || iter_differs {
		*mismatch = Some((
			std::format!(
				"region f{} h{}: pre-entry buf {:?} record {:?}\n  native status {} record {:?} buf {:?}\n  simulator status {} record {:?} buf {:?}",
				region.function,
				region.header,
				&pre[..n],
				pre_record,
				status,
				record,
				&buf[..n],
				sim_status,
				sim_record,
				&sim[..n]
			),
			0,
		));
	} else {
		let scratch = (0..slots)
			.filter(|&l| record.written & (1 << l) == 0)
			.filter(|&l| buf[l] != sim[l])
			.count();
		*mismatch = Some((String::new(), scratch));
	}
	status
}
