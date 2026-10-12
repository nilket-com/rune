//! FUEL (on top of R1): in a calls-shaped region (one inlined site whose callee's pc-0
//! node `c` is reached from the header `h` only along the header's some edge, through an
//! uncharged `Enter`/`Arg` chain, and no other charged node), the header's block holds a
//! guard `fuel >= 2` (unsigned). Fuel 0 and 1 continue into the existing lowering of every
//! node (the slow copy, cold); any other fuel runs a second copy of every node (the fast
//! copy) whose two charges decrement without a check. No exit, stored byte, mask, tag,
//! status, pc, fuel or checkpoint changes.
//!
//! `fuel_selected_regions_guard_both_charges` and `fuel_selection_matches_the_lowering`
//! fail without the guard (the header block then ends in the existing `fuel == 0` check,
//! with no guard and no cold node block). The compile-only
//! controls (`fuel_controls_keep_the_existing_checks`), the executing whole-buffer
//! differential (`fuel_full_buffer_byte_equal_every_fuel`, child process), the machine-byte
//! census against the R1 lowering (`fuel_unselected_regions_keep_r1_machine_bytes`) and the
//! golden comparator controls (`fuel_selected_only_controls`) pass before and after.
//!
//! The selection golden (`p3a_tests::p3a_selection_golden`) is pinned to `FUEL_GOLDEN`,
//! captured once from this implementation after the semantic tests passed; its delta against
//! R1's golden is size-only and selected-only (`fuel_selection_golden_delta`, with the
//! comparator controls `fuel_selected_only_controls`).
//!
//! Whole-buffer equality here is scratch equality of native code against the simulator
//! (every cell, written or not, plus the whole record and the status); the simulator is the
//! R1 semantics (R1's own every-fuel test holds native R1 to it on the same points).
//! Production state equivalence (the interpreter's state, the store and the continuation
//! after a Replay) is the existing three-mode differential (`b1_tests`), which runs
//! unchanged; its forced-Replay protocol (`hook::force_replay`) disqualifies a region here,
//! so forced Replays exercise the existing lowering and natural Replays the fast copy.
use super::audit::Finding;
use super::boundary::NativeRegion;
use super::p2_tests::{V2_CALLS, V2_NUMERIC, V2_WHILE};
use super::r1_tests::{canonical_entry, PAYLOADS, RANGES};
use super::w13_tests::{corpus_regions, kernel};
use super::{lower, NativeRefusal};
use crate::experimental_core::region::{
	abi::ExitRecord, analyze, hook, simulate::simulate, EntryClass, NodeOp, Region, Status, Target,
	VIRTUAL_BASE,
};
use cranelift_codegen::flowgraph::ControlFlowGraph;
use cranelift_codegen::ir::{self, condcodes::IntCC, InstructionData, Opcode};
use std::collections::{BTreeMap, BTreeSet};
use std::format;
use std::os::unix::process::ExitStatusExt;
use std::prelude::rust_2021::*;
use std::println;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "RUNE_FUEL_NATIVE_CHILD";
const DEADLINE: Duration = Duration::from_secs(1200);

fn run_child(inner: &str) {
	let exe = std::env::current_exe().expect("current test binary");
	let name = format!("experimental_native::fuel_tests::{inner}");
	let started = Instant::now();
	let mut child = Command::new(exe)
		.args([
			"--exact",
			name.as_str(),
			"--ignored",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD_ENV, "1")
		.env_remove("PERF_BUILDID_DIR")
		.stdin(Stdio::null())
		.spawn()
		.expect("spawn child");

	let status = loop {
		if let Some(status) = child.try_wait().expect("wait child") {
			break status;
		}
		if started.elapsed() > DEADLINE {
			let _ = child.kill();
			let status = child.wait().expect("reap child");
			std::panic!(
				"{inner}: TIMEOUT after {:?}, killed (status {status:?}, signal {:?})",
				DEADLINE,
				status.signal()
			);
		}
		std::thread::sleep(Duration::from_millis(20));
	};

	println!(
		"CHILD\t{inner}\tstatus={status:?}\tsignal={:?}\twall_ms={}",
		status.signal(),
		started.elapsed().as_millis()
	);
	assert_eq!(status.signal(), None, "{inner}: child killed by a signal");
	assert!(status.success(), "{inner}: child failed ({status:?})");
}

fn require_child() {
	assert!(
		std::env::var_os(CHILD_ENV).is_some(),
		"FUEL native worker tests run only in the child process started by their parent test"
	);
}

// ------------------------------------------------------------- the test's own selection

/// How many references target node `j` (the test's own restatement; exhaustive over
/// `NodeOp`, no wildcard): the entry's jump to the header, every fall-through, every
/// Branch edge, a Jump, an IterNext's none edge and the B1 test protocol's two targets.
fn refs_to(region: &Region, j: u32) -> usize {
	let hit = |t: Target| usize::from(t == Target::Node(j));
	let mut refs = usize::from(region.header_node == j);
	for node in &region.nodes {
		refs += node.next.map_or(0, hit);
		refs += match node.op {
			NodeOp::Branch { yes, no, .. } => hit(yes) + hit(no),
			NodeOp::Jump { target } => hit(target),
			NodeOp::IterNext { none, .. } => hit(none),
			NodeOp::ForceReplay { zero, positive } => hit(zero) + hit(positive),
			NodeOp::Enter { .. } => 0,
			NodeOp::Arg { .. } => 0,
			NodeOp::Ret { .. } => 0,
			NodeOp::Literal { .. } => 0,
			NodeOp::Copy { .. } => 0,
			NodeOp::Arith { .. } => 0,
			NodeOp::Compare { .. } => 0,
		};
	}
	refs
}

/// Whether a node rules the region out, by the test's own classification (exhaustive, no
/// wildcard): any IterNext but the header's, and the B1 test protocol.
fn rules_out(op: &NodeOp, is_header: bool) -> bool {
	match *op {
		NodeOp::IterNext { .. } => !is_header,
		NodeOp::ForceReplay { .. } => true,
		NodeOp::Enter { .. } => false,
		NodeOp::Arg { .. } => false,
		NodeOp::Ret { .. } => false,
		NodeOp::Literal { .. } => false,
		NodeOp::Copy { .. } => false,
		NodeOp::Arith { .. } => false,
		NodeOp::Compare { .. } => false,
		NodeOp::Branch { .. } => false,
		NodeOp::Jump { .. } => false,
	}
}

/// The test's own statement of FUEL's selection, as `(h, c)`: an iterator; exactly one
/// site; the header a charged IterNext whose none edge is an exit; the walk from its
/// fall-through visits the site's `Enter` (site 0, the site's slots) and then `Arg`
/// 0..parameters at `first..`, each uncharged and referenced once, and arrives at
/// `c = first + parameters`, referenced once and charged with the site's CalleeFuel exit;
/// exactly two charged nodes; nothing ruled out.
pub(super) fn selected(region: &Region) -> Option<(u32, u32)> {
	region.iter?;
	if region.sites.len() != 1 {
		return None;
	}
	let site = region.sites[0];
	let h = region.header_node;
	let header = region.nodes.get(h as usize)?;
	header.charged?;
	if !matches!(
		header.op,
		NodeOp::IterNext {
			none: Target::Exit(_),
			..
		}
	) {
		return None;
	}
	let mut want = Vec::new();
	want.push((
		site.enter,
		NodeOp::Enter {
			site: 0,
			slots: site.slots,
		},
	));
	for p in 0..site.parameters {
		let at = site.first + p;
		let NodeOp::Arg { src, .. } = region.nodes.get(at as usize)?.op else {
			return None;
		};
		want.push((at, NodeOp::Arg { param: p, src }));
	}
	let mut at = header.next?;
	for (j, op) in want {
		if at != Target::Node(j) {
			return None;
		}
		let node = region.nodes.get(j as usize)?;
		if node.op != op || node.charged.is_some() || refs_to(region, j) != 1 {
			return None;
		}
		at = node.next?;
	}
	let c = site.first + site.parameters;
	if at != Target::Node(c) || c == h || refs_to(region, c) != 1 {
		return None;
	}
	if region.nodes.get(c as usize)?.charged != Some(site.fuel_exit) {
		return None;
	}
	if region.nodes.iter().filter(|n| n.charged.is_some()).count() != 2 {
		return None;
	}
	let ruled_out = region
		.nodes
		.iter()
		.enumerate()
		.any(|(i, n)| rules_out(&n.op, i as u32 == h));
	(!ruled_out).then_some((h, c))
}

// ------------------------------------------------------------------- IR inspection

/// Node `i`'s block and exit `e`'s block: `lower::build` creates the entry block, then one
/// block per node, then one per exit, before anything else.
fn node_block(i: u32) -> ir::Block {
	ir::Block::from_u32(1 + i)
}

fn exit_block(region: &Region, e: u32) -> ir::Block {
	ir::Block::from_u32(1 + region.nodes.len() as u32 + e)
}

/// FUEL's copies of node `i` and exit `e`: created right after the exits, every node's
/// copy, then every exit's.
fn fast_node_block(region: &Region, i: u32) -> ir::Block {
	ir::Block::from_u32(1 + (region.nodes.len() + region.exits.len()) as u32 + i)
}

fn fast_exit_block(region: &Region, e: u32) -> ir::Block {
	ir::Block::from_u32(1 + (2 * region.nodes.len() + region.exits.len()) as u32 + e)
}

fn def(func: &ir::Function, v: ir::Value) -> Option<InstructionData> {
	func.dfg
		.value_def(func.dfg.resolve_aliases(v))
		.inst()
		.map(|i| func.dfg.insts[i])
}

fn iconst(func: &ir::Function, v: ir::Value) -> Option<i64> {
	match def(func, v)? {
		InstructionData::UnaryImm {
			opcode: Opcode::Iconst,
			imm,
		} => Some(imm.bits()),
		_ => None,
	}
}

/// `block`'s terminator when it is a `brif` on an `icmp`: (condition code, the icmp's
/// constant right operand if any, then-block, else-block).
fn brif_icmp(
	func: &ir::Function,
	block: ir::Block,
) -> Option<(IntCC, Option<i64>, ir::Block, ir::Block)> {
	let last = func.layout.last_inst(block)?;
	let InstructionData::Brif { arg, blocks, .. } = func.dfg.insts[last] else {
		return None;
	};
	let InstructionData::IntCompare {
		opcode: Opcode::Icmp,
		args,
		cond,
	} = def(func, arg)?
	else {
		return None;
	};
	let pool = &func.dfg.value_lists;
	Some((
		cond,
		iconst(func, args[1]),
		blocks[0].block(pool),
		blocks[1].block(pool),
	))
}

/// The blocks of `func` whose terminator is the existing charge check `brif(icmp eq f, 0)`
/// with `target` as its then-block.
fn checks_into(func: &ir::Function, target: ir::Block) -> Vec<ir::Block> {
	func.layout
		.blocks()
		.filter(
			|&b| matches!(brif_icmp(func, b), Some((IntCC::Equal, Some(0), t, _)) if t == target),
		)
		.collect()
}

/// The `icmp uge _, 2` guards of `func`, by block.
pub(super) fn guards(func: &ir::Function) -> Vec<ir::Block> {
	func.layout
		.blocks()
		.filter(|&b| {
			matches!(
				brif_icmp(func, b),
				Some((IntCC::UnsignedGreaterThanOrEqual, Some(2), _, _))
			)
		})
		.collect()
}

/// FUEL's slow copy, from the IR alone: every block reachable from the else-block of the
/// guard that ends the header's block (`node_block(h)`), without passing through the
/// header's block again. Empty when the header's block does not end in a guard.
pub(super) fn slow_copy(func: &ir::Function, h: u32) -> BTreeSet<ir::Block> {
	let mut seen = BTreeSet::new();
	let Some((IntCC::UnsignedGreaterThanOrEqual, Some(2), _, slow)) =
		brif_icmp(func, node_block(h))
	else {
		return seen;
	};
	let mut work = vec![slow];
	while let Some(block) = work.pop() {
		if block == node_block(h) || !seen.insert(block) {
			continue;
		}
		work.extend(func.block_successors(block));
	}
	seen
}

/// The calls kernel plus every corpus region.
fn all_regions() -> Vec<(String, Region)> {
	let mut regions = vec![(String::from("calls kernel"), kernel(V2_CALLS))];
	regions.extend(corpus_regions());
	regions
}

// ------------------------------------------------------------------- compile-only

/// FUEL applies to the calls kernel and to every selected corpus region: the header's
/// block ends in `brif(icmp uge fuel, 2)` whose then-block is the header's fast copy (not
/// cold) and whose else-block (the slow copy) is cold; every node block but the header's
/// is cold and no fast node block is; every exit block and every fast exit block is cold,
/// and every predecessor of an exit block is cold (only the slow copy reaches them); the
/// header's Fuel exit and the site's CalleeFuel exit are each reached by exactly one
/// existing check `brif(icmp eq f, 0)`, and their fast copies by nothing (the fast copy
/// has no check). Without the guard this fails: the header's block ends in the existing
/// check.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn fuel_selected_regions_guard_both_charges() {
	let isa = lower::shared_isa().unwrap();
	let regions = all_regions();
	assert!(
		selected(&regions[0].1).is_some(),
		"the calls kernel is selected"
	);
	let mut checked = 0;
	for (name, region) in &regions {
		let Some((h, c)) = selected(region) else {
			continue;
		};
		let func = lower::build(region, &isa).unwrap();
		let shown = || format!("{name}\n{}", func.display());
		let Some((cond, k, fast, slow)) = brif_icmp(&func, node_block(h)) else {
			std::panic!(
				"{name}: the header block does not end in a brif on an icmp\n{}",
				shown()
			);
		};
		assert_eq!(
			(cond, k),
			(IntCC::UnsignedGreaterThanOrEqual, Some(2)),
			"{}",
			shown()
		);
		assert_eq!(fast, fast_node_block(region, h), "{}", shown());
		assert!(
			!func.layout.is_cold(fast),
			"the fast copy is cold: {}",
			shown()
		);
		assert!(
			func.layout.is_cold(slow),
			"the slow copy is not cold: {}",
			shown()
		);
		assert_eq!(guards(&func).len(), 1, "{}", shown());
		let site = region.sites[0];
		let header_exit = region.nodes[h as usize].charged.unwrap();
		let cfg = ControlFlowGraph::with_function(&func);
		for e in [header_exit, site.fuel_exit] {
			assert_eq!(
				checks_into(&func, exit_block(region, e)).len(),
				1,
				"exit {e}: {}",
				shown()
			);
			assert_eq!(
				cfg.pred_iter(fast_exit_block(region, e)).count(),
				0,
				"fast exit {e}: {}",
				shown()
			);
		}
		for e in 0..region.exits.len() as u32 {
			let exit = exit_block(region, e);
			assert!(
				func.layout.is_cold(exit) && func.layout.is_cold(fast_exit_block(region, e)),
				"exit {e}: {}",
				shown()
			);
			assert!(
				cfg.pred_iter(exit).all(|p| func.layout.is_cold(p.block)),
				"exit {e}: {}",
				shown()
			);
		}
		for i in 0..region.nodes.len() as u32 {
			assert_eq!(
				func.layout.is_cold(node_block(i)),
				i != h,
				"node {i} (c = {c}): {}",
				shown()
			);
			assert!(
				!func.layout.is_cold(fast_node_block(region, i)),
				"fast node {i}: {}",
				shown()
			);
		}
		checked += 1;
	}
	println!("FUEL\tguard\tregions={checked}");
	assert!(checked > 0);
}

/// The census regions, the bench kernels and the controls: the lowering holds the guard
/// exactly when the test's own selection holds. Without the guard this fails for every
/// selected region.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn fuel_selection_matches_the_lowering() {
	let isa = lower::shared_isa().unwrap();
	let mut regions = census_regions();
	regions.extend(
		controls()
			.into_iter()
			.map(|(what, r)| (format!("control {what}"), r)),
	);
	let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
	for (name, region) in &regions {
		// A region the builder refuses (none expected) is reported, not hidden.
		let func = match lower::build(region, &isa) {
			Ok(func) => func,
			Err(e) => {
				println!("FUEL\tselection_build_refused\t{name}\t{e:?}");
				*counts.entry("build_refused").or_default() += 1;
				continue;
			}
		};
		let guarded = !guards(&func).is_empty();
		assert_eq!(
			guarded,
			selected(region).is_some(),
			"{name}\n{}",
			func.display()
		);
		*counts
			.entry(if guarded { "selected" } else { "unselected" })
			.or_default() += 1;
	}
	println!("FUEL\tselection\tregions={}\t{counts:?}", regions.len());
	assert!(counts.contains_key("selected") && counts.contains_key("unselected"));
}

/// Mutations of the calls kernel. Compile-only: fed to `lower::build`, never loaded or
/// executed (analysis would not produce most of them). Each breaks exactly one selection
/// condition.
fn controls() -> Vec<(&'static str, Region)> {
	let base = kernel(V2_CALLS);
	let (h, c) = selected(&base).expect("the calls kernel is selected");
	let site = base.sites[0];
	let header_exit = base.nodes[h as usize].charged.unwrap();
	let with = |f: &dyn Fn(&mut Region)| {
		let mut r = kernel(V2_CALLS);
		f(&mut r);
		r
	};
	// A third charged node: the caller's Copy of the call's result (not a callee node).
	let third = base
		.nodes
		.iter()
		.position(|n| matches!(n.op, NodeOp::Copy { .. }) && n.pc != site.pc && n.charged.is_none())
		.expect("a caller Copy to charge");
	// The back edge (the Jump to the header).
	let back = base
		.nodes
		.iter()
		.position(|n| {
			n.op == NodeOp::Jump {
				target: Target::Node(h),
			}
		})
		.expect("the back edge");
	vec![
		(
			"forced_replay",
			with(&|r: &mut Region| hook::force_replay(r)),
		),
		(
			"third_charge",
			with(&|r: &mut Region| r.nodes[third].charged = Some(header_exit)),
		),
		(
			"uncharged_callee",
			with(&|r: &mut Region| r.nodes[c as usize].charged = None),
		),
		(
			"side_entry",
			with(&|r: &mut Region| {
				r.nodes[back].op = NodeOp::Jump {
					target: Target::Node(site.enter),
				}
			}),
		),
		(
			"second_site",
			with(&|r: &mut Region| {
				let s = r.sites[0];
				r.sites.try_push(s).expect("push");
			}),
		),
		(
			"none_to_node",
			with(&|r: &mut Region| {
				if let NodeOp::IterNext { none, .. } = &mut r.nodes[h as usize].op {
					*none = Target::Node(back as u32);
				}
			}),
		),
	]
}

/// Negative controls (compile-only), each against the test's own selection and the
/// lowering: the existing header check stays the header block's terminator, the function
/// holds no guard and no node block is cold. The unmutated calls kernel is the positive
/// control (`fuel_selected_regions_guard_both_charges`). Passes before and after FUEL.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn fuel_controls_keep_the_existing_checks() {
	let isa = lower::shared_isa().unwrap();
	let mut regions: Vec<(String, Region)> = controls()
		.into_iter()
		.map(|(w, r)| (String::from(w), r))
		.collect();
	for (name, text) in [("while kernel", V2_WHILE), ("numeric kernel", V2_NUMERIC)] {
		regions.push((String::from(name), kernel(text)));
	}
	for (what, region) in &regions {
		assert_eq!(selected(region), None, "{what}");
		let func = lower::build(region, &isa).unwrap();
		let h = region.header_node;
		let header_exit = region.nodes[h as usize].charged.unwrap();
		let Some((cond, k, then, _)) = brif_icmp(&func, node_block(h)) else {
			std::panic!(
				"{what}: the header block does not end in a brif on an icmp\n{}",
				func.display()
			);
		};
		assert_eq!(
			(cond, k, then),
			(IntCC::Equal, Some(0), exit_block(region, header_exit)),
			"{what}\n{}",
			func.display()
		);
		assert!(guards(&func).is_empty(), "{what}\n{}", func.display());
		for i in 0..region.nodes.len() as u32 {
			assert!(
				!func.layout.is_cold(node_block(i)),
				"{what}: node {i} is cold\n{}",
				func.display()
			);
		}
		println!("FUEL\tcontrol\t{what}\texisting_checks_kept");
	}
}

/// The B1 three-mode differential's cases (`b1_tests::corpus_programs`) and T5's
/// (`b1_tests::w13_t5_programs`), each with its admitted regions and how many of them the
/// test's own selection selects. Compile-only (analysis, no lowering), so identical before
/// and after FUEL. A case whose every admitted region is selected (and which runs with no
/// forced-Replay transform: none of these does) has all its natural Replays (CalleeFuel and
/// CalleeFault) in selected regions; its `B1COV` rows count the interpreter's continuation
/// after them against the plain interpreter at every fuel.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn fuel_natural_replay_cases() {
	let mut programs = super::b1_tests::corpus_programs();
	programs.extend(super::b1_tests::w13_t5_programs());
	let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
	for (name, src) in &programs {
		let code = src.code();
		let (mut admitted, mut chosen) = (0, 0);
		for id in 0..code.functions() as u32 {
			for (_, region) in analyze(code, id).unwrap() {
				if let Ok(region) = region {
					admitted += 1;
					chosen += usize::from(selected(&region).is_some());
				}
			}
		}
		let class = match (admitted, chosen) {
			(0, _) => "no_region",
			(a, c) if a == c => "all_selected",
			(_, 0) => "none_selected",
			_ => "some_selected",
		};
		*counts.entry(class).or_default() += 1;
		println!("FUEL\tb1_case\t{name}\tadmitted={admitted}\tselected={chosen}\t{class}");
	}
	println!("FUEL\tb1_cases\tcases={}\t{counts:?}", programs.len());
	assert!(counts.contains_key("all_selected"));
}

// --------------------------------------------------------------- whole buffer, every fuel

/// Coverage of the whole-buffer runs: by status, and, on the calls kernel, by path.
#[derive(Default)]
struct Cov {
	points: usize,
	by_status: BTreeMap<u32, usize>,
	/// Fuel 0: the header's Fuel exit at the header pc.
	fuel0_header: usize,
	/// Fuel 1: the exhausted iterator's exit after the header's charge only (fuel 0 left).
	fuel1_exhausted: usize,
	/// Fuel 1: the CalleeFuel Replay at the Call pc with checkpoint 0.
	fuel1_callee_fuel: usize,
	/// Fuel >= 2: the exhausted iterator's exit (the second charge never taken).
	fast_exhausted: usize,
	/// Fuel >= 2 at entry, a Fuel or CalleeFuel exit: fast iterations, then the slow copy.
	fast_then_slow: usize,
	/// The callee fault Replay, by the sign of the stored pre-add total (both overflow
	/// directions).
	fault_positive: usize,
	fault_negative: usize,
	/// Entry ranges: reversed (next > end), empty (next == end), touching MIN or MAX.
	reversed: usize,
	empty: usize,
	min_or_max: usize,
	/// Fuels at or above 2^63 - 1 (the unsigned guard on a negative i64).
	huge_fuel: usize,
	/// Re-entries from a previous exit's state, by that exit's status.
	chained_after: BTreeMap<u32, usize>,
}

/// The fuels of every first entry: 0..=64, 4096, 2^20, and the unsigned-guard boundaries.
fn fuels() -> Vec<u64> {
	(0..=64)
		.chain([
			4096,
			1 << 20,
			(1 << 63) - 1,
			1 << 63,
			(1 << 63) + 1,
			u64::MAX - 1,
			u64::MAX,
		])
		.collect()
}

/// The calls kernel's cell of the caller local copied into the callee Add's lhs.
fn total_cell(region: &Region, c: u32) -> Option<usize> {
	let NodeOp::Arith { lhs, .. } = region.nodes[c as usize].op else {
		return None;
	};
	region.nodes.iter().find_map(|n| match n.op {
		NodeOp::Arg { param, src } if VIRTUAL_BASE + param == lhs => Some(src as usize),
		_ => None,
	})
}

/// One native and one simulated run from the same entry; the WHOLE buffer, the whole
/// record and the status must be equal. On a selected region the exit pc is also checked
/// against the path the entry fuel implies.
#[allow(clippy::too_many_arguments)]
fn compare(
	native: &NativeRegion,
	region: &Region,
	entry: &[i64],
	fuel: u64,
	fold: Option<(u32, u32)>,
	total: Option<usize>,
	at: &str,
	cov: &mut Cov,
) -> (u32, Vec<i64>, ExitRecord) {
	let (mut buf, mut sim) = (entry.to_vec(), entry.to_vec());
	let (mut record, mut sim_record) = (ExitRecord::new(fuel), ExitRecord::new(fuel));
	let status = native.call(&mut buf, &mut record).expect("runs");
	let sim_status = simulate(region, &mut sim, &mut sim_record);
	assert_eq!(status, sim_status, "{at}");
	assert_eq!(record, sim_record, "{at}");
	assert_eq!(buf, sim, "{at}");
	cov.points += 1;
	*cov.by_status.entry(status).or_default() += 1;
	let n = region.slots.len();
	if region.iter.is_some() {
		let (next, end) = (entry[n], entry[n + 1]);
		cov.reversed += usize::from(next > end);
		cov.empty += usize::from(next == end);
		cov.min_or_max += usize::from([next, end].iter().any(|&x| x == i64::MIN || x == i64::MAX));
	}
	cov.huge_fuel += usize::from(fuel >= (1 << 63) - 1);
	let Some((h, _)) = fold else {
		return (status, buf, record);
	};
	let site = region.sites[0];
	let header = region.nodes[h as usize];
	let header_exit = header.charged.unwrap();
	let NodeOp::IterNext {
		none: Target::Exit(done),
		..
	} = header.op
	else {
		unreachable!()
	};
	let exit = record.exit;
	// Completed `next` advances in this run (non-negative and small: bounded ranges).
	let progress =
		u64::try_from(i128::from(buf[n]) - i128::from(entry[n])).expect("next never moves back");
	match fuel {
		0 => {
			assert_eq!(
				(exit, record.pc, record.fuel),
				(header_exit, header.pc, 0),
				"{at}: fuel 0"
			);
			cov.fuel0_header += 1;
		}
		1 if exit == done => {
			assert_eq!(
				(record.pc, record.fuel),
				(region.exits[done as usize].pc, 0),
				"{at}: fuel 1 exhausted"
			);
			cov.fuel1_exhausted += 1;
		}
		1 => {
			assert_eq!(
				(exit, record.pc, record.fuel),
				(site.fuel_exit, site.pc, 0),
				"{at}: fuel 1 callee charge"
			);
			cov.fuel1_callee_fuel += 1;
		}
		_ if exit == done => {
			// Each completed iteration advanced `next` once and took both charges; the
			// exhausted one took the header's only.
			let taken = progress.checked_mul(2).and_then(|t| t.checked_add(1));
			assert_eq!(
				Some(record.fuel),
				taken.and_then(|t| fuel.checked_sub(t)),
				"{at}: exhausted"
			);
			cov.fast_exhausted += 1;
		}
		_ if exit == header_exit || exit == site.fuel_exit => {
			assert_eq!(
				record.fuel, 0,
				"{at}: Fuel or CalleeFuel after fast iterations"
			);
			cov.fast_then_slow += 1;
		}
		_ => {}
	}
	if Some(exit) == site.fault_exit {
		// The checkpoint is the fuel before the Call: the faulting iteration (the last
		// `next` advance) has taken the header's charge only.
		let taken = progress
			.checked_sub(1)
			.and_then(|k| k.checked_mul(2))
			.and_then(|t| t.checked_add(1));
		assert_eq!(
			Some(record.fuel),
			taken.and_then(|t| fuel.checked_sub(t)),
			"{at}: fault checkpoint"
		);
		assert_eq!(
			record.pc, site.pc,
			"{at}: the fault Replay resumes at the Call"
		);
		match total.map(|l| buf[l].signum()) {
			Some(1) => cov.fault_positive += 1,
			Some(-1) => cov.fault_negative += 1,
			_ => {}
		}
	}
	(status, buf, record)
}

/// Native against the simulator on the WHOLE `buf`, the whole record and the status, for
/// the calls kernel and every selected corpus region, from canonical entry buffers
/// (`r1_tests::canonical_entry`), every bounded range (empty, reversed, MIN, MAX), payload
/// (two one small add from overflow, one each way) and fuel (0..=64, 4096, 2^20 and the
/// unsigned-guard boundaries 2^63 - 1, 2^63, 2^63 + 1, u64::MAX - 1, u64::MAX); then chains
/// of re-entries from every resumable exit (Fuel, Replay, Retry), canonically refilled, at
/// fuels 1..=6. Passes before and after FUEL.
#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: executes generated machine code in a child process"
)]
fn fuel_full_buffer_byte_equal_every_fuel() {
	run_child("child_full_buffer");
}

#[test]
#[ignore = "FUEL native worker: run by its parent test in a child process"]
fn child_full_buffer() {
	require_child();
	let fuels = fuels();
	let mut calls = Cov::default();
	let mut corpus = Cov::default();
	let (mut regions, mut division) = (0, 0);
	for (name, region) in all_regions() {
		let fold = selected(&region);
		if fold.is_none() {
			continue;
		}
		let compiled = match lower::compile(&region, 0) {
			Ok(c) => c,
			Err(NativeRefusal::Audit(Finding::Division(_))) => {
				division += 1;
				continue;
			}
			Err(other) => std::panic!("{name}: {other:?}"),
		};
		let native = NativeRegion::load(&compiled).unwrap();
		regions += 1;
		let total = fold.and_then(|(_, c)| total_cell(&region, c));
		let cov = if name == "calls kernel" {
			&mut calls
		} else {
			&mut corpus
		};
		for &range in &RANGES {
			for &payload in &PAYLOADS {
				let entry = canonical_entry(&region, payload, Some(range));
				for &fuel in &fuels {
					let at = format!("{name} range={range:?} payload={payload} fuel={fuel}");
					compare(&native, &region, &entry, fuel, fold, total, &at, cov);
				}

				// Re-entries from each resumable exit's state, canonically refilled (a guarded
				// cell keeps its written value or its entry value, `Scalar` cells 0, `next` and
				// `end` as the exit left them). Not the interpreter's own continuation after a
				// Replay or Retry (the three-mode differential covers that); an input the
				// native code must still agree on.
				for fuel in 1..=6u64 {
					let mut entry = entry.clone();
					let mut after = None;
					for step in 0..64 {
						let at = format!(
							"{name} range={range:?} payload={payload} fuel={fuel} chain={step}"
						);
						let (status, buf, record) =
							compare(&native, &region, &entry, fuel, fold, total, &at, cov);
						if let Some(prev) = after {
							*cov.chained_after.entry(prev).or_default() += 1;
						}
						let resumable = status == Status::Fuel as u32
							|| status == Status::Replay as u32
							|| status == Status::Retry as u32;
						if !resumable {
							break;
						}
						after = Some(status);
						for (l, class) in region.entry.iter().enumerate() {
							if let EntryClass::Guard(_) = class {
								if record.written & (1u64 << l) != 0 {
									entry[l] = buf[l];
								}
							} else {
								entry[l] = 0;
							}
						}
						let n = region.slots.len();
						entry[n] = buf[n];
						entry[n + 1] = buf[n + 1];
					}
				}
			}
		}
	}
	for (what, c) in [("calls", &calls), ("corpus", &corpus)] {
		println!(
			"FUEL\tfull_buffer\t{what}\tpoints={}\tby_status={:?}\tfuel0_header={}\tfuel1_exhausted={}\tfuel1_callee_fuel={}\tfast_exhausted={}\tfast_then_slow={}\tfault_positive={}\tfault_negative={}\treversed={}\tempty={}\tmin_or_max={}\thuge_fuel={}\tchained_after={:?}",
			c.points,
			c.by_status,
			c.fuel0_header,
			c.fuel1_exhausted,
			c.fuel1_callee_fuel,
			c.fast_exhausted,
			c.fast_then_slow,
			c.fault_positive,
			c.fault_negative,
			c.reversed,
			c.empty,
			c.min_or_max,
			c.huge_fuel,
			c.chained_after
		);
	}
	println!(
		"FUEL\tfull_buffer\tTOTAL\tregions={regions}\tpoints={}\tdivision_refused={division}",
		calls.points + corpus.points
	);
	let c = &calls;
	assert!(regions > 0);
	assert!(
		c.fuel0_header > 0 && c.fuel1_exhausted > 0 && c.fuel1_callee_fuel > 0,
		"fuel 0 and both fuel-1 paths"
	);
	assert!(
		c.fast_exhausted > 0 && c.fast_then_slow > 0,
		"fast exits and fast-then-slow exits"
	);
	assert!(
		c.fault_positive > 0 && c.fault_negative > 0,
		"both overflow directions reach the fault Replay"
	);
	assert!(c.reversed > 0 && c.empty > 0 && c.min_or_max > 0 && c.huge_fuel > 0);
	for s in [Status::Fuel, Status::Replay] {
		assert!(
			c.chained_after.contains_key(&(s as u32)),
			"re-entry after {s:?}"
		);
	}
}

// ------------------------------------------------------- census and R1 machine bytes

/// The machine-code listing of every census region (each admitted region of every
/// selection-golden program, every corpus region, the three bench kernels), captured from
/// the R1 lowering: 336 lines (sha256 6765203e…1648). Data only.
const R1_BYTES: &str = include_str!("fixtures/r1_region_bytes.txt");

fn fnv1a(bytes: &[u8]) -> u64 {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for &byte in bytes {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	hash
}

fn census_regions() -> Vec<(String, Region)> {
	let mut out = Vec::new();
	for (name, src) in super::p3a_tests::golden_programs() {
		let code = src.code();
		for id in 0..code.functions() as u32 {
			let Ok(found) = analyze(code, id) else {
				continue;
			};
			for (header, region) in found {
				if let Ok(region) = region {
					out.push((format!("golden {name} f{id} h{header}"), region));
				}
			}
		}
	}
	for (name, region) in corpus_regions() {
		out.push((format!("corpus {name}"), region));
	}
	for (name, text) in [
		("while", V2_WHILE),
		("numeric", V2_NUMERIC),
		("calls", V2_CALLS),
	] {
		out.push((format!("kernel {name}"), kernel(text)));
	}
	out
}

/// A region's compile outcome: its machine code's size and FNV-1a 64, or its refusal.
fn compile_line(region: &Region) -> (String, Option<Vec<u8>>) {
	match lower::compile(region, 0) {
		Ok(c) => (
			format!("ok len={} fnv={:016x}", c.bytes().len(), fnv1a(c.bytes())),
			Some(c.bytes().to_vec()),
		),
		Err(NativeRefusal::Audit(Finding::Division(_))) => (String::from("refused division"), None),
		Err(NativeRefusal::Size) => (String::from("refused size"), None),
		Err(NativeRefusal::Codegen(_)) => (String::from("refused codegen"), None),
		Err(NativeRefusal::Audit(_)) => (String::from("refused audit_other"), None),
		Err(other) => (format!("refused other {other:?}"), None),
	}
}

/// The census and the machine-byte comparison against R1: an unselected region must have
/// exactly R1's outcome and bytes (size and FNV-1a 64); a selected region keeps its outcome
/// kind and may change. With `RUNE_FUEL_BYTES_DIR` every compiled region's bytes are also
/// written (`NNNN.bin` in census order, plus `INDEX.txt`) for an external full-byte
/// comparison. Passes before and after FUEL.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn fuel_unselected_regions_keep_r1_machine_bytes() {
	let regions = census_regions();
	let dir = std::env::var_os("RUNE_FUEL_BYTES_DIR").map(std::path::PathBuf::from);
	let mut text = String::new();
	let mut now = Vec::new();
	for (i, (name, region)) in regions.iter().enumerate() {
		let (line, bytes) = compile_line(region);
		if let (Some(dir), Some(bytes)) = (&dir, &bytes) {
			std::fs::create_dir_all(dir).unwrap();
			std::fs::write(dir.join(format!("{i:04}.bin")), bytes).unwrap();
		}
		text.push_str(&format!("{name}\t{line}\n"));
		now.push((name.clone(), line, selected(region).is_some()));
	}
	if let Some(dir) = &dir {
		let index: String = now
			.iter()
			.enumerate()
			.map(|(i, (name, line, s))| {
				format!("{i:04}\t{name}\tselected={}\t{line}\n", u8::from(*s))
			})
			.collect();
		std::fs::write(dir.join("INDEX.txt"), index).unwrap();
	}
	let r1: Vec<&str> = R1_BYTES.lines().collect();
	assert_eq!(
		r1.len(),
		now.len(),
		"census region count against the R1 listing"
	);
	let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
	for ((name, line, sel), old) in now.iter().zip(&r1) {
		let (old_name, old_line) = old.split_once('\t').unwrap();
		assert_eq!(name, old_name, "census order");
		let kind = |l: &str| l.split(' ').next().unwrap_or("").to_owned();
		if !sel {
			assert_eq!(
				line, old_line,
				"{name}: an UNSELECTED region's machine code differs from R1's"
			);
			*counts
				.entry(if line.starts_with("ok") {
					"unselected_identical"
				} else {
					"unselected_refused_same"
				})
				.or_default() += 1;
		} else {
			assert_eq!(
				kind(line),
				kind(old_line),
				"{name}: a selected region's outcome kind changed"
			);
			let key = match (line.starts_with("ok"), line == old_line) {
				(true, true) => "selected_unchanged",
				(true, false) => "selected_changed",
				(false, _) => "selected_refused_same",
			};
			*counts.entry(key).or_default() += 1;
			println!("FUEL\tcensus_selected\t{name}\tr1={old_line}\tnow={line}");
		}
	}
	println!(
		"FUEL\tcensus\tregions={}\tlisting_fnv={:016x}\tr1_listing_fnv={:016x}\t{counts:?}",
		now.len(),
		fnv1a(text.as_bytes()),
		fnv1a(R1_BYTES.as_bytes())
	);
	assert!(counts.get("unselected_identical").copied().unwrap_or(0) > 0);
	assert!(counts.contains_key("selected_changed") || counts.contains_key("selected_unchanged"));
}

// ------------------------------------------------------------------- the golden

/// The selection-golden regions FUEL selects, by golden key `program<TAB>fN<TAB>hH`, and the
/// programs that hold one, computed from each golden program's analysed regions (an actual
/// census, not the golden text).
fn golden_selected() -> (BTreeSet<String>, BTreeSet<String>) {
	let (mut regions, mut programs) = (BTreeSet::new(), BTreeSet::new());
	for (name, src) in super::p3a_tests::golden_programs() {
		let code = src.code();
		for id in 0..code.functions() as u32 {
			let Ok(found) = analyze(code, id) else {
				continue;
			};
			for (header, region) in found {
				if let Ok(region) = region {
					if selected(&region).is_some() {
						regions.insert(format!("{name}\tf{id}\th{header}"));
						programs.insert(name.clone());
					}
				}
			}
		}
	}
	(regions, programs)
}

/// FUEL's selected-only rule over two selection texts with the same lines (after
/// `w13_tests::check_w13_delta` has accepted their delta: size-only, no page crossing):
/// every changed COMPILE or LOADED line is a selected region, and every changed SUMMARY
/// line is a program holding one. Any other changed line is refused. Returns the changed
/// selected regions.
pub(super) fn check_fuel_selected_only(old: &str, new: &str) -> Result<BTreeSet<String>, String> {
	let (regions, programs) = golden_selected();
	let mut changed = BTreeSet::new();
	for (a, b) in old.lines().zip(new.lines()) {
		if a == b {
			continue;
		}
		let fields: Vec<&str> = a.split('\t').collect();
		match fields[0] {
			"COMPILE" | "LOADED" => {
				let key = fields[1..4].join("\t");
				if !regions.contains(&key) {
					return Err(format!("an UNSELECTED region's line changed: {a} -> {b}"));
				}
				changed.insert(key);
			}
			"SUMMARY" => {
				if !programs.contains(fields[1]) {
					return Err(format!(
						"a SUMMARY of a program without a selected region changed: {a} -> {b}"
					));
				}
			}
			kind => return Err(format!("a {kind} line changed: {a} -> {b}")),
		}
	}
	println!(
		"FUEL\tgolden_selected\tselected_regions={}\tselected_programs={}\tchanged_regions={}\tunchanged_selected_regions={}",
		regions.len(),
		programs.len(),
		changed.len(),
		regions.len() - changed.len()
	);
	Ok(changed)
}

/// The selection golden captured once from the FUEL implementation (`RUNE_P3A_GOLDEN_OUT`,
/// after the semantic tests passed): `p3a_tests::child_selection_golden` pins it line for line.
pub(super) const FUEL_GOLDEN: &str = include_str!("fixtures/fuel_selection_golden.txt");

/// FUEL's golden against R1's, fixture to fixture: the restricted size-only delta
/// (`w13_tests::compare_w13_delta`, page crossings refused) in which every changed line is a
/// FUEL-selected region's or a SUMMARY of a program holding one (`check_fuel_selected_only`,
/// whose selected set is a census of the golden programs, not the golden text). Pinned to
/// the observed 6 changed lines in 2 regions: COMPILE, LOADED and SUMMARY of v2_calls f0 h4
/// and listed_9 f0 h5.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn fuel_selection_golden_delta() {
	let r1 = super::r1_tests::R1_GOLDEN;
	let (identical, changed) = super::w13_tests::compare_w13_delta(r1, FUEL_GOLDEN);
	let regions = check_fuel_selected_only(r1, FUEL_GOLDEN)
		.unwrap_or_else(|e| std::panic!("FUEL golden delta: {e}"));
	println!("FUEL\tgolden_fixtures\tidentical={identical}\tchanged={changed}\tchanged_regions={regions:?}");
	assert_eq!(identical + changed, FUEL_GOLDEN.lines().count());
	assert_eq!(
		(changed, regions.len()),
		(6, 2),
		"COMPILE, LOADED and SUMMARY of v2_calls f0 h4 and listed_9 f0 h5"
	);
}

/// Controls of the restricted golden rule on the R1 golden text: a byte change of an
/// unselected region's COMPILE or LOADED line, or of the SUMMARY of a program without a
/// selected region, is refused by the selected-only rule; the same change on a selected
/// region is accepted by it; and a selected region whose new size crosses a page (with
/// `mapped_bytes` following, or stale) is refused by the size-only rule
/// (`w13_tests::check_w13_delta`) although the selected-only rule alone accepts it. Passes
/// before and after FUEL.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source")]
fn fuel_selected_only_controls() {
	let old = super::r1_tests::R1_GOLDEN;
	let (regions, programs) = golden_selected();
	assert!(!regions.is_empty(), "the golden has FUEL-selected regions");
	let number = |l: &str, marker: &str| -> (usize, usize, u64) {
		let at = l.find(marker).unwrap() + marker.len();
		let end = l[at..]
			.find(|c: char| !c.is_ascii_digit())
			.map_or(l.len(), |e| at + e);
		(at, end, l[at..end].parse().unwrap())
	};
	let set = |l: &str, marker: &str, v: u64| -> String {
		let (at, end, _) = number(l, marker);
		format!("{}{}{}", &l[..at], v, &l[end..])
	};
	let edit = |pick: &dyn Fn(&str) -> bool, marker: &str| -> Option<String> {
		let line = old.lines().find(|l| pick(l) && l.contains(marker))?;
		Some(old.replacen(line, &set(line, marker, number(line, marker).2 + 1), 1))
	};
	let key = |l: &str| l.split('\t').skip(1).take(3).collect::<Vec<_>>().join("\t");
	let program = |l: &str| l.split('\t').nth(1).unwrap_or("").to_owned();
	let cases: [(&str, Option<String>, bool); 4] = [
		(
			"unselected_compile",
			edit(
				&|l| l.starts_with("COMPILE\t") && !regions.contains(&key(l)),
				"\tok bytes=",
			),
			false,
		),
		(
			"unselected_loaded",
			edit(
				&|l| l.starts_with("LOADED\t") && !regions.contains(&key(l)),
				"\tcode_bytes=",
			),
			false,
		),
		(
			"unselected_summary",
			edit(
				&|l| l.starts_with("SUMMARY\t") && !programs.contains(&program(l)),
				" code_bytes=",
			),
			false,
		),
		(
			"selected_compile",
			edit(
				&|l| l.starts_with("COMPILE\t") && regions.contains(&key(l)),
				"\tok bytes=",
			),
			true,
		),
	];
	for (what, new, accepted) in cases {
		let new = new.unwrap_or_else(|| std::panic!("{what}: no such line"));
		let result = check_fuel_selected_only(old, &new);
		println!(
			"FUEL\tselected_only_control\t{what}\t{}",
			if result.is_ok() {
				"accepted"
			} else {
				"refused"
			}
		);
		assert_eq!(result.is_ok(), accepted, "{what}: {result:?}");
	}

	let compile = old
		.lines()
		.find(|l| {
			l.starts_with("COMPILE\t") && l.contains("\tok bytes=") && regions.contains(&key(l))
		})
		.unwrap();
	let k = key(compile);
	let was = number(compile, "\tok bytes=").2;
	assert!(was < 4096, "{compile}");
	let crossing = |mapped_follows: bool| -> String {
		let mut out = String::new();
		for l in old.lines() {
			let line = if l.starts_with("COMPILE\t") && key(l) == k {
				set(l, "\tok bytes=", 4100)
			} else if l.starts_with("LOADED\t") && key(l) == k {
				set(l, "\tcode_bytes=", 4100)
			} else if l.starts_with("SUMMARY\t") && program(l) == program(compile) {
				let moved = set(l, " code_bytes=", number(l, " code_bytes=").2 + 4100 - was);
				if mapped_follows {
					set(
						&moved,
						" mapped_bytes=",
						number(l, " mapped_bytes=").2 + 4096,
					)
				} else {
					moved
				}
			} else {
				l.to_owned()
			};
			out.push_str(&line);
			out.push('\n');
		}
		out
	};
	for (what, mapped_follows) in [
		("selected_page_crossing", true),
		("selected_page_crossing_mapped_stale", false),
	] {
		let new = crossing(mapped_follows);
		assert!(
			check_fuel_selected_only(old, &new).is_ok(),
			"{what}: the selected-only rule alone accepts it"
		);
		let e = super::w13_tests::check_w13_delta(old, &new).expect_err(what);
		println!("FUEL\tselected_only_control\t{what}\tsize_only_refused\t{e}");
	}
}
