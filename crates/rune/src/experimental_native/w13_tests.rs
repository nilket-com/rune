//! W13 (rnx 0184): W1 cold exit blocks and W3 branch-edge constants. Compile-only: nothing
//! here loads or calls generated code. The every-fuel differentials that execute it are the
//! existing P1/P2/P2b/P3a/B1 suites plus `b1_tests::w13_branch_condition_sources_every_fuel`
//! (T5), whose sources `w13_t5_branch_condition_sources` checks statically.
//!
//! Tests-first: `w13_exit_blocks_are_cold`, `w13_kernel_exits_emitted_after_the_loop`,
//! `w13_branch_edges_define_the_condition_constant` and
//! `w13_kernel_compare_feeds_the_branch_only` fail at 7b1e2b0a and pass with W1 and W3.
//! `compare_w13_delta` is the restricted golden delta `p3a_tests::child_selection_golden`
//! runs against the B1 golden; `w13_golden_comparator_controls` are its negative controls.
use super::exec_tests::{compile_source, Src};
use super::lower;
use crate::experimental_core::region::{analyze, EntryClass, NodeOp, Region, Scalar, VIRTUAL_BASE};
use cranelift_codegen::ir::{self, Block, InstructionData, Opcode};
use std::format;
use std::prelude::rust_2021::*;
use std::println;

/// Every admitted region of the native corpora (the programs of the B1 audit corpus).
pub(super) fn corpus_regions() -> Vec<(String, Region)> {
	let mut programs: Vec<(String, Src)> = super::b1_tests::corpus_programs();
	for c in super::exec_tests::corpus_cases()
		.into_iter()
		.chain(super::exec_tests::new_cases())
		.chain(super::p2_tests::probe_kernel_cases())
	{
		programs.push((c.name, c.src));
	}
	let mut out = Vec::new();
	for (name, src) in &programs {
		let code = src.code();
		for id in 0..code.functions() as u32 {
			for (header, region) in analyze(code, id).unwrap() {
				if let Ok(region) = region {
					out.push((format!("{name} f{id} h{header}"), region));
				}
			}
		}
	}
	out
}

/// The one admitted region of a probe kernel.
pub(super) fn kernel(text: &str) -> Region {
	let program = compile_source(text);
	let mut found = Vec::new();
	for id in 0..program.code.functions() as u32 {
		for (_, region) in analyze(&program.code, id).unwrap() {
			if let Ok(region) = region {
				found.push(region);
			}
		}
	}
	assert_eq!(found.len(), 1, "{text}");
	found.remove(0)
}

const KERNELS: [(&str, &str); 2] = [
	("while", super::p2_tests::V2_WHILE),
	("numeric", super::p2_tests::V2_NUMERIC),
];

fn opcodes(func: &ir::Function, block: Block) -> Vec<Opcode> {
	func.layout.block_insts(block).map(|i| func.dfg.insts[i].opcode()).collect()
}

fn returns(func: &ir::Function, block: Block) -> bool {
	opcodes(func, block).last() == Some(&Opcode::Return)
}

/// The constant of a block's first instruction, when it is an `iconst`.
fn first_iconst(func: &ir::Function, block: Block) -> Option<i64> {
	let first = func.layout.first_inst(block)?;
	match func.dfg.insts[first] {
		InstructionData::UnaryImm {
			opcode: Opcode::Iconst,
			imm,
		} => Some(imm.bits()),
		_ => None,
	}
}

/// A CLIF text's blocks: (header line, instruction lines).
fn clif_blocks(text: &str) -> Vec<(&str, Vec<&str>)> {
	let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
	for line in text.lines() {
		if line.starts_with("block") {
			out.push((line, Vec::new()));
		} else if let Some(b) = out.last_mut() {
			b.1.push(line.trim());
		}
	}
	out
}

/// The VCode listing's blocks in EMISSION order (Cranelift writes the listing while it
/// emits, cold blocks last), as trimmed instruction lines; the prologue is skipped.
fn emitted_blocks(vcode: &str) -> Vec<Vec<&str>> {
	let mut out: Vec<Vec<&str>> = Vec::new();
	for line in vcode.lines() {
		let t = line.trim();
		if t.starts_with("block") && t.ends_with(':') {
			out.push(Vec::new());
		} else if let Some(b) = out.last_mut() {
			b.push(t);
		}
	}
	out
}

/// W1: in the built IR every exit block (the only blocks that return, one per region exit)
/// is cold, the entry is not, and the only other cold blocks are W3 edges into an exit
/// (`iconst; jump`, whose single successor returns). After the mid-end every block that
/// still returns is still cold.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_exit_blocks_are_cold() {
	let isa = lower::shared_isa().unwrap();
	let (mut regions, mut exits, mut edges, mut slow) = (0, 0, 0, 0);
	for (name, region) in corpus_regions() {
		let func = lower::build(&region, &isa).unwrap();
		let entry = func.layout.entry_block().unwrap();
		assert!(!func.layout.is_cold(entry), "{name}: cold entry");
		// FUEL (Codex 8321 ruling (i)): a region the test's own selection (`fuel_tests::selected`)
		// selects lowers every node and every exit twice, any other region once; the observed
		// guard must agree, so an extra or missing guard cannot redefine the expected counts.
		let fold = super::fuel_tests::selected(&region);
		let copies = if fold.is_some() { 2 } else { 1 };
		assert_eq!(super::fuel_tests::guards(&func).len(), copies - 1, "{name}: FUEL guards against the test's selection\n{}", func.display());
		// The slow copy, from the IR: every block reachable from the guard's else-block without
		// passing the header's block (empty for an unselected region). Block numbering
		// (`lower::build`): the entry, one per node, one per exit, then (FUEL) one per node and
		// one per exit of the fast copy; every later block was created while lowering.
		let slow_copy = fold.map_or_else(Default::default, |(h, _)| super::fuel_tests::slow_copy(&func, h));
		let (n, e) = (region.nodes.len(), region.exits.len());
		if let Some((h, _)) = fold {
			for &blk in &slow_copy {
				let id = blk.as_u32() as usize;
				assert!(!(1 + n + e..1 + 2 * n + 2 * e).contains(&id), "{name}: the slow copy reaches the fast {blk}\n{}", func.display());
				assert!(id != 1 + h as usize && id != 0, "{name}: the slow copy holds {blk}");
			}
		}
		let (mut slow_edges, mut fast_edges) = (0, 0);
		let mut returning = 0;
		for block in func.layout.blocks() {
			if returns(&func, block) {
				assert!(func.layout.is_cold(block), "{name}: exit {block} is not cold\n{}", func.display());
				returning += 1;
			} else if func.layout.is_cold(block) {
				let succ: Vec<Block> = func.block_successors(block).collect();
				let edge = opcodes(&func, block) == [Opcode::Iconst, Opcode::Jump] && succ.len() == 1 && returns(&func, succ[0]);
				if copies == 1 || !slow_copy.contains(&block) {
					// One copy, or the fast copy of a selected region: the original W13 checks.
					assert_eq!(opcodes(&func, block), [Opcode::Iconst, Opcode::Jump], "{name}: cold {block}");
					assert!(succ.len() == 1 && returns(&func, succ[0]), "{name}: cold edge {block} into a non-exit");
					edges += 1;
					fast_edges += 1;
				} else if edge {
					// The slow copy's W3 edge into one of its exits: the same shape.
					edges += 1;
					slow_edges += 1;
				} else {
					// Any other cold block must be a known slow-copy node block (an original node
					// block other than the header's, which holds the guard) or an auxiliary block
					// the slow copy created.
					let id = block.as_u32() as usize;
					let slow_node = (1..=n).contains(&id);
					let auxiliary = id >= 1 + 2 * n + 2 * e;
					assert!(slow_node || auxiliary, "{name}: cold {block} in the slow copy is neither a slow node block nor a slow auxiliary block\n{}", func.display());
					slow += 1;
				}
			}
		}
		if copies == 2 {
			assert_eq!(slow_edges, fast_edges, "{name}: the two copies' cold edges into exits\n{}", func.display());
		}
		assert_eq!(returning, copies * region.exits.len(), "{name}: one returning block per exit and copy");
		exits += returning;

		// The cold flags survive the mid-end (the layout is what the backend orders by).
		match lower::compile(&region, 0) {
			Ok(compiled) => {
				for (header, body) in clif_blocks(&compiled.listing().optimized) {
					if body.iter().any(|l| l.starts_with("return")) {
						assert!(header.ends_with(" cold:"), "{name}: optimized {header} returns but is not cold");
					}
				}
			}
			Err(super::NativeRefusal::Audit(super::audit::Finding::Division(_))) => (),
			Err(other) => std::panic!("{name}: {other:?}"),
		}
		regions += 1;
	}
	println!("W13\tcold\tregions={regions}\texit_blocks={exits}\tcold_edges={edges}\tfuel_slow_blocks={slow}");
	assert!(exits > 0);
}

/// W1 on the probe kernels, in the emitted order: once the first returning block is
/// emitted, every later block returns or is a lone jump (an empty edge), so no loop block
/// follows an exit and the loop's exits are not in its fall-through path.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_kernel_exits_emitted_after_the_loop() {
	for (name, text) in KERNELS {
		let compiled = lower::compile(&kernel(text), 0).unwrap();
		let vcode = &compiled.listing().vcode;
		let blocks = emitted_blocks(vcode);
		let first = blocks
			.iter()
			.position(|b| b.iter().any(|l| l.starts_with("retq")))
			.expect("an exit");
		for b in &blocks[first..] {
			let lone_jump = b.len() == 1 && b[0].starts_with("jmp");
			assert!(
				b.iter().any(|l| l.starts_with("retq")) || lone_jump,
				"{name}: a non-exit block is emitted after an exit\n{vcode}"
			);
		}
	}
}

/// W3: every Branch node lowers to a `brif` whose two successors are its edge blocks,
/// `iconst 1; jump` (yes) and `iconst 0; jump` (no), and nothing else has that shape.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_branch_edges_define_the_condition_constant() {
	let isa = lower::shared_isa().unwrap();
	let mut branches = 0;
	for (name, region) in corpus_regions() {
		let func = lower::build(&region, &isa).unwrap();
		let edge = |b: Block, k: i64| opcodes(&func, b) == [Opcode::Iconst, Opcode::Jump] && first_iconst(&func, b) == Some(k);
		let shaped: Vec<Block> = func
			.layout
			.blocks()
			.filter(|&b| {
				opcodes(&func, b).last() == Some(&Opcode::Brif) && {
					let s: Vec<Block> = func.block_successors(b).collect();
					s.len() == 2 && edge(s[0], 1) && edge(s[1], 0)
				}
			})
			.collect();
		// FUEL (Codex 8321 ruling (i)): a region the test's own selection selects lowers every
		// Branch once per copy; the observed guard must agree with the selection.
		let fold = super::fuel_tests::selected(&region);
		let copies = if fold.is_some() { 2 } else { 1 };
		assert_eq!(super::fuel_tests::guards(&func).len(), copies - 1, "{name}: FUEL guards against the test's selection\n{}", func.display());
		let per_copy = region
			.nodes
			.iter()
			.filter(|n| matches!(n.op, NodeOp::Branch { .. }))
			.count();
		let expected = copies * per_copy;
		assert_eq!(shaped.len(), expected, "{name}\n{}", func.display());
		if let Some((h, _)) = fold {
			// The const-on-edge shape in BOTH copies: every Branch once in the slow copy, once in
			// the fast copy.
			let slow_copy = super::fuel_tests::slow_copy(&func, h);
			let in_slow = shaped.iter().filter(|b| slow_copy.contains(b)).count();
			assert_eq!((in_slow, shaped.len() - in_slow), (per_copy, per_copy), "{name}\n{}", func.display());
		}
		branches += expected;
	}
	println!("W13\tedges\tbranch_nodes={branches}");
	assert!(branches > 0);
}

/// W3 on the probe kernels: after the mid-end the compare feeds the branch only. No
/// `uextend` survives (the 0/1 value is never materialised), the listing has no `setl` and
/// one `cmpq` (the duplicate compare is gone). Static listing facts only; no timing.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_kernel_compare_feeds_the_branch_only() {
	for (name, text) in KERNELS {
		let compiled = lower::compile(&kernel(text), 0).unwrap();
		let l = compiled.listing();
		assert!(!l.optimized.contains("uextend"), "{name}\n{}", l.optimized);
		assert!(!l.vcode.contains("setl"), "{name}\n{}", l.vcode);
		let compares = l.vcode.lines().filter(|x| x.trim().starts_with("cmpq")).count();
		assert_eq!(compares, 1, "{name}\n{}", l.vcode);
		println!(
			"W13\tkernel\t{name}\tcode_bytes={}\tvcode_lines={}\tcmpq={compares}",
			compiled.bytes().len(),
			l.report.vcode_lines
		);
	}
}

/// Kernel listings for the receipts: with `RUNE_W13_LISTING_DIR` set, the while, numeric and
/// calls probe kernels' IR, optimized IR, VCode and machine code are written there as
/// `{kernel}.{clif,opt.clif,vcode,bin}` (for an external objdump). Always checks that the
/// three kernels compile. Compile-only.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_kernel_listings() {
	let dir = std::env::var_os("RUNE_W13_LISTING_DIR").map(std::path::PathBuf::from);
	for (name, text) in [
		("while", super::p2_tests::V2_WHILE),
		("numeric", super::p2_tests::V2_NUMERIC),
		("calls", super::p2_tests::V2_CALLS),
	] {
		let compiled = lower::compile(&kernel(text), 0).unwrap();
		let l = compiled.listing();
		println!("W13\tlisting\t{name}\tcode_bytes={}\tvcode_lines={}", compiled.bytes().len(), l.report.vcode_lines);
		if let Some(dir) = &dir {
			std::fs::create_dir_all(dir).unwrap();
			std::fs::write(dir.join(format!("{name}.clif")), &l.clif).unwrap();
			std::fs::write(dir.join(format!("{name}.opt.clif")), &l.optimized).unwrap();
			std::fs::write(dir.join(format!("{name}.vcode")), &l.vcode).unwrap();
			std::fs::write(dir.join(format!("{name}.bin")), compiled.bytes()).unwrap();
		}
	}
}

/// Where a Branch condition comes from, as T5 needs it covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
	/// A caller slot with a `Guard(Bool)` entry (read before the region writes it on some
	/// path) that the region also writes.
	GuardedLiveIn,
	/// A slot some `Literal` defines as a `Bool`.
	Literal,
	/// A slot some `Copy` defines as a `Bool`.
	Copy,
	/// A slot some `Compare` defines.
	Compare,
	/// A virtual local of an inlined callee.
	Virtual,
	/// A caller slot a callee's `Ret` defines as a `Bool`.
	RetFed,
}

/// Per Branch node of an admitted region: (condition local, its sources).
fn branch_sources(region: &Region) -> Vec<(u32, Vec<Source>)> {
	let mut out = Vec::new();
	for node in &region.nodes {
		let NodeOp::Branch { condition: c, .. } = node.op else {
			continue;
		};
		let mut sources = Vec::new();
		if c >= VIRTUAL_BASE {
			sources.push(Source::Virtual);
		}
		let mut written = false;
		for n in &region.nodes {
			let source = match n.op {
				NodeOp::Literal { out, ty: Scalar::Bool, .. } if out == c => Some(Source::Literal),
				NodeOp::Copy { out, ty: Scalar::Bool, .. } if out == c => Some(Source::Copy),
				NodeOp::Compare { out, .. } if out == c => Some(Source::Compare),
				NodeOp::Ret { out, ty: Scalar::Bool, .. } if out == c => Some(Source::RetFed),
				_ => None,
			};
			if let Some(source) = source {
				written = true;
				if !sources.contains(&source) {
					sources.push(source);
				}
			}
		}
		if c < VIRTUAL_BASE && written && region.entry.get(c as usize) == Some(&EntryClass::Guard(Scalar::Bool)) {
			sources.push(Source::GuardedLiveIn);
		}
		out.push((c, sources));
	}
	out
}

/// T5 coverage, statically: each T5 case (`b1_tests::w13_branch_condition_sources_every_fuel`)
/// has exactly one admitted region, and that region has a Branch whose condition comes
/// from the source the case is there for. The every-fuel run asserts the admission too.
#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn w13_t5_branch_condition_sources() {
	let expected = [
		("w13_guard_bool", Source::GuardedLiveIn),
		("w13_literal_bool", Source::Literal),
		("w13_copy_bool", Source::Copy),
		("callee_f_two_arms", Source::Virtual),
		("checkpoint_two_arms", Source::Virtual),
		("callee_lt_caller_branch", Source::RetFed),
	];
	let programs = super::b1_tests::w13_t5_programs();
	assert_eq!(programs.len(), expected.len());
	for ((name, src), (want_name, want)) in programs.iter().zip(expected) {
		assert_eq!(name, want_name);
		let code = src.code();
		let mut admitted = Vec::new();
		for id in 0..code.functions() as u32 {
			for (header, region) in analyze(code, id).unwrap() {
				if let Ok(region) = region {
					admitted.push((id, header, region));
				}
			}
		}
		assert_eq!(admitted.len(), 1, "{name}: admitted regions");
		let (id, header, region) = &admitted[0];
		let branches = branch_sources(region);
		println!("W13\tt5_source\t{name}\tf{id}h{header}\tbranches={}\t(condition, sources)={branches:?}", branches.len());
		assert!(
			branches.iter().any(|(_, s)| s.contains(&want)),
			"{name}: no {want:?} branch condition in {branches:?}"
		);
	}
}

fn field(line: &str, key: &str) -> Result<u64, String> {
	let prefix = format!("{key}=");
	line.split(|c| c == ' ' || c == '\t')
		.find_map(|f| f.strip_prefix(prefix.as_str()))
		.ok_or_else(|| format!("no {key} in {line}"))?
		.parse()
		.map_err(|e| format!("{key} in {line}: {e}"))
}

fn without(line: &str, keys: &[&str]) -> String {
	line.split(' ')
		.filter(|f| !keys.iter().any(|k| f.starts_with(&format!("{k}="))))
		.collect::<Vec<_>>()
		.join(" ")
}

/// `line` up to and including `marker`, when the rest is a decimal number.
fn sized(line: &str, marker: &str) -> Option<String> {
	let at = line.find(marker)? + marker.len();
	line[at..].parse::<u64>().ok()?;
	Some(line[..at].into())
}

/// A LOADED or COMPILE line's region key: (program, function, header).
fn region_key(line: &str) -> String {
	line.split('\t').skip(1).take(3).collect::<Vec<_>>().join("\t")
}

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
	if ok {
		Ok(())
	} else {
		Err(what())
	}
}

/// One selection text's own byte bookkeeping: per program, the SUMMARY `code_bytes` is the
/// sum of its LOADED lines and `mapped_bytes` the sum of their 4096-byte page roundings (as
/// in `compare_b1_delta`); every LOADED region's `code_bytes` equals the `ok bytes` of that
/// region's COMPILE line.
fn check_bytes(text: &str) -> Result<(), String> {
	let mut compiled = std::collections::BTreeMap::new();
	for line in text.lines().filter(|l| l.starts_with("COMPILE\t")) {
		if let Some(at) = line.find("\tok bytes=") {
			let bytes: u64 = line[at + "\tok bytes=".len()..]
				.parse()
				.map_err(|e| format!("{line}: {e}"))?;
			compiled.insert(region_key(line), bytes);
		}
	}
	let check = |summary: Option<&str>, loaded: &[u64]| -> Result<(), String> {
		if let Some(s) = summary {
			let code: u64 = loaded.iter().sum();
			let mapped: u64 = loaded.iter().map(|b| b.div_ceil(4096).max(1) * 4096).sum();
			ensure(field(s, "code_bytes")? == code, || format!("SUMMARY code_bytes != sum(LOADED) {code}: {s}"))?;
			ensure(field(s, "mapped_bytes")? == mapped, || format!("SUMMARY mapped_bytes != pages(LOADED) {mapped}: {s}"))?;
		}
		Ok(())
	};
	let mut summary: Option<&str> = None;
	let mut loaded: Vec<u64> = Vec::new();
	for line in text.lines() {
		if line.starts_with("PROGRAM\t") {
			check(summary.take(), &loaded)?;
			loaded.clear();
		} else if line.starts_with("SUMMARY\t") {
			summary = Some(line);
		} else if line.starts_with("LOADED\t") {
			let bytes = field(line, "code_bytes")?;
			ensure(compiled.get(&region_key(line)) == Some(&bytes), || {
				format!("LOADED {bytes} != COMPILE ok bytes {:?}: {line}", compiled.get(&region_key(line)))
			})?;
			loaded.push(bytes);
		}
	}
	check(summary, &loaded)
}

/// The restricted W13 delta of `new` against `old` (the B1 golden): the same lines in the
/// same order, every line identical except a region's own code size (COMPILE `ok bytes=N`,
/// LOADED `code_bytes=N`) and the SUMMARY's `code_bytes`. `mapped_bytes`, every ANALYSIS
/// line, every COMPILE class, the LOADED set and every other counter must not change (a
/// size change that crosses a page is refused, not accepted). Both texts must also pass
/// [`check_bytes`]. Returns (identical lines, the changed lines as `new<TAB>was<TAB>old`).
pub(super) fn check_w13_delta(old: &str, new: &str) -> Result<(usize, Vec<String>), String> {
	let (o, n): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
	ensure(o.len() == n.len(), || format!("line count {} -> {}", o.len(), n.len()))?;
	let (mut identical, mut changed) = (0, Vec::new());
	for (a, b) in o.iter().zip(n.iter()) {
		if a == b {
			identical += 1;
			continue;
		}
		match a.split('\t').next().unwrap_or("") {
			"COMPILE" => {
				let (x, y) = (sized(a, "\tok bytes="), sized(b, "\tok bytes="));
				ensure(x.is_some() && x == y, || format!("only an ok COMPILE line's bytes may change: {a} -> {b}"))?;
			}
			"LOADED" => {
				let (x, y) = (sized(a, "\tcode_bytes="), sized(b, "\tcode_bytes="));
				ensure(x.is_some() && x == y, || format!("only a LOADED line's code_bytes may change: {a} -> {b}"))?;
			}
			"SUMMARY" => {
				let keys = ["code_bytes"];
				ensure(without(a, &keys) == without(b, &keys), || {
					format!("only a SUMMARY's code_bytes may change (mapped_bytes too is fixed): {a} -> {b}")
				})?;
			}
			kind => return Err(format!("a {kind} line changed: {a} -> {b}")),
		}
		changed.push(format!("{b}\twas\t{a}"));
	}
	check_bytes(old).map_err(|e| format!("old: {e}"))?;
	check_bytes(new).map_err(|e| format!("new: {e}"))?;
	Ok((identical, changed))
}

/// [`check_w13_delta`] for the golden test: panics on a violation, prints every changed
/// line as `W13GOLDEN changed`, and returns (identical, changed) line counts.
pub(super) fn compare_w13_delta(old: &str, new: &str) -> (usize, usize) {
	let (identical, changed) = check_w13_delta(old, new).unwrap_or_else(|e| std::panic!("W13 golden delta: {e}"));
	for line in &changed {
		println!("W13GOLDEN\tchanged\t{line}");
	}
	(identical, changed.len())
}

/// The selection golden captured once from the verified W13 implementation
/// (`RUNE_P3A_GOLDEN_OUT`); `p3a_tests::child_selection_golden` pins this run to it.
pub(super) const W13_GOLDEN: &str = include_str!("fixtures/w13_selection_golden.txt");

/// The W13 fixture against the B1 fixture: exactly the restricted W13 delta.
#[test]
fn w13_selection_golden_delta() {
	let (identical, changed) = compare_w13_delta(super::b1_tests::B1_GOLDEN, W13_GOLDEN);
	println!("W13\tgolden_fixtures\tidentical={identical}\tchanged={changed}");
	assert_eq!(identical + changed, W13_GOLDEN.lines().count());
}

/// A two-program selection text in the golden's format, sized by its regions' bytes.
fn synthetic(p1: u64, p2a: u64, p2b: u64) -> String {
	let summary = |name: &str, regions: usize, loaded: &[u64]| {
		let code: u64 = loaded.iter().sum();
		let mapped: u64 = loaded.iter().map(|b| b.div_ceil(4096).max(1) * 4096).sum();
		format!(
			"SUMMARY\t{name}\tfunctions=1 regions={regions} loaded={} division_runtime=0 code_bytes={code} mapped_bytes={mapped} metadata_bytes=1012\n",
			loaded.len()
		)
	};
	let mut t = String::new();
	t.push_str("PROGRAM\tone\n");
	t.push_str(&summary("one", 1, &[p1]));
	t.push_str(&format!("LOADED\tone\tf0\th1\tcode_bytes={p1}\n"));
	t.push_str("ANALYSIS\tone\tf0\th1\tadmitted\tfnv=00000000000000aa\tnodes=5\texits=3\tslots=4\n");
	t.push_str(&format!("COMPILE\tone\tf0\th1\tok bytes={p1}\n"));
	t.push_str("PROGRAM\ttwo\n");
	t.push_str(&summary("two", 3, &[p2a, p2b]));
	t.push_str(&format!("LOADED\ttwo\tf0\th2\tcode_bytes={p2a}\n"));
	t.push_str(&format!("LOADED\ttwo\tf1\th3\tcode_bytes={p2b}\n"));
	t.push_str("ANALYSIS\ttwo\tf0\th2\tadmitted\tfnv=00000000000000bb\tnodes=6\texits=4\tslots=5\n");
	t.push_str("ANALYSIS\ttwo\tf1\th3\tadmitted\tfnv=00000000000000cc\tnodes=7\texits=4\tslots=5\n");
	t.push_str("ANALYSIS\ttwo\tf1\th9\tadmitted\tfnv=00000000000000dd\tnodes=7\texits=4\tslots=5\n");
	t.push_str(&format!("COMPILE\ttwo\tf0\th2\tok bytes={p2a}\n"));
	t.push_str(&format!("COMPILE\ttwo\tf1\th3\tok bytes={p2b}\n"));
	t.push_str("COMPILE\ttwo\tf1\th9\tdivision_runtime\n");
	t
}

/// Negative controls for the golden comparator (synthetic texts, no compile): a size-only
/// change within the same page passes; a changed ANALYSIS line, admission, COMPILE class,
/// LOADED set, metadata or mapped bytes, a page crossing, a COMPILE size that disagrees
/// with its LOADED region and an inconsistent SUMMARY each fail.
#[test]
fn w13_golden_comparator_controls() {
	let old = synthetic(300, 576, 1041);
	check_bytes(&old).unwrap();
	assert_eq!(check_w13_delta(&old, &old).unwrap(), (old.lines().count(), Vec::new()));

	// Valid: sizes move within their pages; SUMMARY code_bytes follows.
	let new = synthetic(290, 600, 1030);
	let (identical, changed) = check_w13_delta(&old, &new).unwrap();
	assert_eq!((identical, changed.len()), (old.lines().count() - 8, 8));

	let fails = |what: &str, new: String| {
		let e = check_w13_delta(&old, &new).expect_err(what);
		println!("W13\tcomparator_control\t{what}\trefused\t{e}");
	};
	let edit = |from: &str, to: &str| {
		assert!(old.contains(from), "{from}");
		old.replacen(from, to, 1)
	};
	fails("analysis_fnv", edit("fnv=00000000000000bb", "fnv=00000000000000be"));
	fails("analysis_shape", edit("nodes=7\texits=4\tslots=5\nANALYSIS", "nodes=8\texits=4\tslots=5\nANALYSIS"));
	fails("admission", edit("f1\th9\tadmitted", "f1\th9\trefused"));
	fails("compile_class", edit("COMPILE\ttwo\tf1\th9\tdivision_runtime", "COMPILE\ttwo\tf1\th9\tdivision_const0"));
	fails("compile_ok_to_refused", edit("COMPILE\tone\tf0\th1\tok bytes=300", "COMPILE\tone\tf0\th1\tsize"));
	fails("loaded_set", edit("LOADED\ttwo\tf1\th3", "LOADED\ttwo\tf1\th4"));
	fails("metadata_bytes", edit("metadata_bytes=1012\nLOADED\tone", "metadata_bytes=1013\nLOADED\tone"));
	fails("summary_counter", edit("regions=3", "regions=2"));
	fails("mapped_bytes", edit("mapped_bytes=4096", "mapped_bytes=8192"));
	fails("line_count", format!("{old}PROGRAM\tthree\n"));
	// COMPILE and LOADED disagree (SUMMARY consistent with LOADED).
	fails("compile_vs_loaded", edit("COMPILE\tone\tf0\th1\tok bytes=300", "COMPILE\tone\tf0\th1\tok bytes=301"));
	// LOADED and COMPILE agree, SUMMARY code_bytes does not.
	fails(
		"summary_code_bytes",
		old.replacen("LOADED\tone\tf0\th1\tcode_bytes=300", "LOADED\tone\tf0\th1\tcode_bytes=310", 1)
			.replacen("COMPILE\tone\tf0\th1\tok bytes=300", "COMPILE\tone\tf0\th1\tok bytes=310", 1),
	);
	// A legitimate-looking size change that crosses a page: mapped_bytes moves, refused.
	fails("page_crossing", synthetic(300, 576, 4100));
	// The same crossing with mapped_bytes left as before: the page sum check refuses it.
	fails(
		"page_crossing_summary_stale",
		synthetic(300, 576, 4100).replace("mapped_bytes=12288", "mapped_bytes=8192"),
	);
}
