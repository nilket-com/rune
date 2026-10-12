//! Compile-only tests of the P0 backend: lowering, the verifier, and the lowered-code audit.
//! Nothing here loads code into executable memory or calls it.
//!
//! With `RUNE_P0_AUDIT_DIR` set, each compiled region's IR, optimized IR, VCode listing and
//! raw machine code are written there for an external disassembly of the audit receipts.
use super::audit::{self, Finding};
use super::lower::{self, compile, diagnose_machine_only};
use super::{NativeRefusal, MAX_PROGRAM_CODE_BYTES};
use crate::experimental_core::region::{analyze, Region};
use crate::experimental_core::{frontend, region::abi::ExitRecord};
use crate::{Context, Diagnostics, Source, Sources};
use cranelift_codegen::ir::{self, types, InstBuilder, MemFlagsData, UserFuncName};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use std::format;
use std::panic;
use std::prelude::rust_2021::*;

fn regions(text: &str) -> Vec<Region> {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let program = frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap();
	let mut out = Vec::new();
	for id in 0..program.code.functions() as u32 {
		for (_, region) in analyze(&program.code, id).unwrap() {
			if let Ok(region) = region {
				out.push(region);
			}
		}
	}
	out
}

fn dump(name: &str, compiled: &lower::Compiled) {
	let Some(dir) = std::env::var_os("RUNE_P0_AUDIT_DIR") else {
		return;
	};
	let dir = std::path::PathBuf::from(dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join(format!("{name}.clif")), &compiled.listing().clif).unwrap();
	std::fs::write(dir.join(format!("{name}.opt.clif")), &compiled.listing().optimized).unwrap();
	std::fs::write(dir.join(format!("{name}.vcode")), &compiled.listing().vcode).unwrap();
	std::fs::write(dir.join(format!("{name}.bin")), compiled.bytes()).unwrap();
}

const WHILE: &str = "pub fn main(_a) { let n = 0; while n < 10000 { n += 1; } n }";
const NUMERIC: &str =
    "pub fn main(_a) { let n = 0; let total = 0; while n < 10000 { total += n % 17; n += 1; } total }";

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn kernel_regions_compile_and_pass_the_audit() {
	for (name, src) in [("while", WHILE), ("numeric", NUMERIC)] {
		let found = regions(src);
		assert_eq!(found.len(), 1, "{name}");
		let compiled = compile(&found[0], 0).unwrap();
		dump(name, &compiled);
		assert_eq!(compiled.listing().report.code_bytes, compiled.bytes().len());
		assert_eq!(compiled.cells(), found[0].cells());
		assert!(compiled.alignment().is_power_of_two());
		assert!(
			compiled.bytes().len() < 4096,
			"{name}: {} bytes",
			compiled.bytes().len()
		);
		// Deterministic: a fixed baseline ISA, no host feature detection.
		assert_eq!(compile(&found[0], 0).unwrap().bytes(), compiled.bytes());
		if name == "numeric" {
			// `% 17` became a multiply/shift sequence: no divide survived the mid-end. Its
			// magic multiplier is read from the constant pool (one RIP-relative operand).
			assert!(
				!compiled.listing().optimized.contains("srem"),
				"{}",
				compiled.listing().optimized
			);
			assert!(
				compiled.listing().optimized.contains("smulhi"),
				"{}",
				compiled.listing().optimized
			);
			assert_eq!(compiled.listing().report.rip_relative, 1, "{}", compiled.listing().vcode);
		} else {
			assert_eq!(compiled.listing().report.rip_relative, 0, "{}", compiled.listing().vcode);
		}
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn runtime_divisors_are_refused_by_the_ir_gate_and_lower_to_trapping_divides() {
	for (name, src) in [
		(
			"div_runtime",
			"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a / b; i += 1; } t }",
		),
		(
			"rem_runtime",
			"pub fn main(a, b) { let t = 0; let i = 0; while i < 3 { t += a % b; i += 1; } t }",
		),
	] {
		let found = regions(src);
		assert_eq!(found.len(), 1);
		assert!(
			matches!(
				compile(&found[0], 0),
				Err(NativeRefusal::Audit(Finding::Division(_)))
			),
			"{name}"
		);
		// Without the IR gate the machine audit still refuses: the backend emits an idiv
		// with trap metadata even behind our explicit zero and MIN/-1 branches.
		match diagnose_machine_only(&found[0]) {
			Err(NativeRefusal::Audit(Finding::Traps(sites))) => {
				assert!(!sites.is_empty());
				if let Some(dir) = std::env::var_os("RUNE_P0_AUDIT_DIR") {
					std::fs::create_dir_all(&dir).unwrap();
					std::fs::write(
						std::path::Path::new(&dir).join(format!("{name}.traps.txt")),
						sites.join("\n"),
					)
					.unwrap();
				}
			}
			other => panic!("{name}: expected trap sites, got {:?}", other.err()),
		}
	}
	// Constant divisors 0 and -1 are never folded into the infallible path: they keep their
	// Retry exits and so are refused like runtime divisors.
	for src in [
		"pub fn main(a) { let t = 0; let i = 0; while i < 3 { t += a % 0; i += 1; } t }",
		"pub fn main(a) { let t = 0; let i = 0; while i < 3 { t += a / -1; i += 1; } t }",
	] {
		for region in regions(src) {
			assert!(matches!(
				compile(&region, 0),
				Err(NativeRefusal::Audit(Finding::Division(_)))
			));
		}
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn every_region_of_the_test_programs_compiles_or_is_refused_by_the_audit() {
	let programs = [
        WHILE,
        NUMERIC,
        "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x *= b; i += 1; } x }",
        "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x -= b; i += 1; } x }",
        "pub fn main(a, b) { let x = a; let i = 0; while i < 4 { x += x; i += b; } x }",
        "pub fn main(a) { let i = 0; let s = 0; while i < a { if i % 3 == 0 { s += i; } else { s -= 1; } i += 1; } s }",
        "pub fn main(a, flag) { let i = 0; let s = 0; while i < a { if flag { s += 2; } else { s += 1; } i += 1; } s }",
        "pub fn main(a) { let i = 0; let s = 0; let j = 0; while i < a { j -= j; while j < i { s += j; j += 1; } i += 1; } s }",
        "pub fn main(a) { let i = 0; let s = 0; while i < a { if i > 3 { if i < 9 { s += 1; } } i += 1; } s }",
        "pub fn main(a) { let n = 0; let q = 0; while n < a { q += n / 7; n += 1; } q }",
    ];
	let mut compiled = 0;
	for (p, src) in programs.iter().enumerate() {
		for (r, region) in regions(src).iter().enumerate() {
			let result = compile(region, 0);
			match result {
				Ok(code) => {
					dump(&format!("program{p}_region{r}"), &code);
					compiled += 1;
				}
				Err(e) => panic!("{src}: {e:?}"),
			}
		}
	}
	assert_eq!(compiled, 11);
}

#[test]
#[cfg_attr(miri, ignore = "Miri: compiles Rune source and runs Cranelift")]
fn code_cache_bound_refuses_without_publishing() {
	let found = regions(WHILE);
	assert_eq!(
		compile(&found[0], MAX_PROGRAM_CODE_BYTES).err(),
		Some(NativeRefusal::Size)
	);
}

/// A hand-built function with one extra instruction, for the IR audit's negative cases.
pub(super) fn with_extra(extra: impl FnOnce(&mut FunctionBuilder<'_>, ir::Value, ir::Value)) -> ir::Function {
	let mut func = ir::Function::with_name_signature(UserFuncName::user(0, 0), lower::signature());
	let mut fctx = FunctionBuilderContext::new();
	let mut b = FunctionBuilder::new(&mut func, &mut fctx);
	let entry = b.create_block();
	b.append_block_params_for_function_params(entry);
	b.switch_to_block(entry);
	let (buf, rec) = (b.block_params(entry)[0], b.block_params(entry)[1]);
	extra(&mut b, buf, rec);
	let zero = b.ins().iconst(types::I32, 0);
	b.ins().return_(&[zero]);
	b.seal_all_blocks();
	b.finalize(lower::isa().unwrap().frontend_config());
	func
}

#[test]
fn ir_audit_negative_cases() {
	let trusted = |_: &mut FunctionBuilder<'_>| MemFlagsData::trusted();
	// In bounds: buf[1] of 2 slots, and the record's last u32 field.
	let ok = with_extra(|b, buf, rec| {
		let mem = trusted(b);
		let v = b.ins().load(types::I64, mem, buf, 8);
		b.ins().store(mem, v, buf, 8);
		let s = b.ins().iconst(types::I32, 1);
		b.ins().store(mem, s, rec, ExitRecord::PC);
	});
	assert_eq!(audit::ir(&ok, &audit::AccessMap::uniform_for_test(2)), Ok(()));
	// buf[2] of 2 slots, a negative offset, a record overrun, and a derived address.
	for case in [
		with_extra(|b, buf, _| {
			let mem = trusted(b);
			b.ins().load(types::I64, mem, buf, 16);
		}),
		with_extra(|b, buf, _| {
			let mem = trusted(b);
			b.ins().load(types::I64, mem, buf, -8);
		}),
		with_extra(|b, _, rec| {
			let mem = trusted(b);
			let v = b.ins().iconst(types::I64, 0);
			// 44 + 8 overruns the record under both the 40-byte (P0) and 48-byte (P2b) layouts.
			b.ins().store(mem, v, rec, 44);
		}),
		with_extra(|b, buf, _| {
			let mem = trusted(b);
			let p = b.ins().iadd_imm_s(buf, 8);
			b.ins().load(types::I64, mem, p, 0);
		}),
		// An in-bounds access that may trap (default flags carry a heap-out-of-bounds code).
		with_extra(|b, buf, _| {
			let mem = MemFlagsData::new();
			b.ins().load(types::I64, mem, buf, 0);
		}),
	] {
		assert!(
			matches!(audit::ir(&case, &audit::AccessMap::uniform_for_test(2)), Err(Finding::Memory(_))),
			"{}",
			case.display()
		);
	}
	// A trap and a stack-slot address are outside the allowlist.
	let trap = with_extra(|b, _, _| {
		b.ins().trap(ir::TrapCode::INTEGER_OVERFLOW);
		let next = b.create_block();
		b.switch_to_block(next);
	});
	assert!(matches!(audit::ir(&trap, &audit::AccessMap::uniform_for_test(2)), Err(Finding::Opcode(_))));
	let global = with_extra(|b, _, _| {
		let fp = b.ins().get_frame_pointer(types::I64);
		let _ = fp;
	});
	assert!(matches!(audit::ir(&global, &audit::AccessMap::uniform_for_test(2)), Err(Finding::Opcode(_))));
}
