//! Entry policy keeps its historical precedence when formatter admission is cold.
use super::*;
use crate::experimental_core::{frontend, heap::Limits};
use crate::{Context, Diagnostics, Hash, Source, Sources};

// This exact-self method is compiled as an internal entry. Its public caller is
// admitted, but an embedder cannot provide the proof-dependent receiver directly.
const INTERNAL_ENTRY: &str =
	"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S.run()}";

fn internal_entry() -> (Program, u32) {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory(INTERNAL_ENTRY).unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 77).unwrap();
	let id = program.function(Hash::type_hash(["S", "run"])).unwrap();
	assert!(!program.entry_eligible(id));
	assert_eq!(program.code.function(id).unwrap().parameters, 1);
	(program, id)
}

fn store() -> Store {
	Store::new(Limits {
		entries: 8,
		bytes: 4096,
	})
	.unwrap()
}

fn executor() -> Executor {
	Executor::with_limits(ExecutorLimits {
		frames: 4,
		slots: 16,
	})
	.unwrap()
}

fn assert_entry(failure: &Failure) {
	assert_eq!(
		failure.error.refusal,
		Some(super::super::error::Refusal::Entry)
	);
	assert_eq!(failure.error.kind, ErrorKind::Unsupported);
	assert!(failure.call_chain.is_empty());
	assert!(failure.cleanup_error.is_none());
}

#[test]
fn internal_entry_policy_precedes_closed_busy_and_wrong_arity_without_effects() {
	let (program, id) = internal_entry();

	// Keep the expected arity here so the closed-store error is the competing
	// ordinary preflight error. The scalar argument owns no heap allocation.
	let mut closed = store();
	let argument = closed.root_unit();
	let _ = closed.close();
	let mut e = executor();
	let failure = e
		.invoke(&program, &mut closed, id, &[&argument], 0)
		.unwrap_err();
	assert_entry(&failure);
	assert!(e.is_idle());
	assert_eq!((closed.live_entries(), closed.payload_bytes()), (0, 0));

	let mut s = store();
	let argument = s.root_unit();
	let mut busy = executor();
	busy.slots.try_push(Value::Unit).unwrap();
	let failure = busy
		.invoke(&program, &mut s, id, &[&argument], 0)
		.unwrap_err();
	assert_entry(&failure);
	assert_eq!(busy.slots.as_slice(), &[Value::Unit]);
	assert!(busy.frames.is_empty());
	assert!(busy.active_host_runtime.is_none());
	busy.slots.clear();
	assert!(busy.is_idle());
	s.root_release(argument).unwrap();
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));

	let mut s = store();
	let mut e = executor();
	let failure = e.invoke(&program, &mut s, id, &[], 0).unwrap_err();
	assert_entry(&failure);
	assert!(e.is_idle());
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn cold_and_ordinary_preflight_keep_the_same_validation_order() {
	let source = include_str!("../execute.rs");
	let ordinary = source.split("fn invoke_checked(").nth(1).unwrap();
	let ordinary = ordinary.split("\n\t#[cold]").next().unwrap();
	let cold = source.split("fn validate_format_entry(").nth(1).unwrap();
	let cold = cold.split("\n\t#[inline(never)]").next().unwrap();
	assert!(
		cold.find("program.entry_refused(entry.1)").unwrap()
			< cold.find("if store.is_closed()").unwrap()
	);
	let guards = [
		"if store.is_closed()",
		"if !self.is_idle()",
		".function(function)",
		"args.len() != entry.parameters",
	];
	for body in [ordinary, cold] {
		let mut previous = 0;
		for guard in guards {
			let index = body.find(guard).unwrap();
			assert!(index >= previous, "out-of-order admission check: {guard}");
			previous = index;
		}
	}
	let ordinary_hosts = "code.validate_hosts(self.configured_host_runtime.as_deref())?;";
	assert!(ordinary.find(guards[3]).unwrap() < ordinary.find(ordinary_hosts).unwrap());
	assert!(cold.find(guards[3]).unwrap() < cold.find("code.host_validation_parts()").unwrap());

	// The cold caller duplicates the immutable host checks to preserve the
	// ordinary validator's hot call graph. Pin equivalence to the original loop.
	let code = include_str!("../code.rs");
	let validator = code.split("fn validate_hosts(").nth(1).unwrap();
	let original_loop = validator
		.split("for binding in &self.hosts {")
		.nth(1)
		.unwrap();
	let original_loop = original_loop
		.split("if !self.absent.is_empty()")
		.next()
		.unwrap();
	let cold_loop = cold.split("for binding in bindings {").nth(1).unwrap();
	let cold_loop = cold_loop.split("if has_absent").next().unwrap();
	let compact = |text: &str| {
		text.chars()
			.filter(|ch| !ch.is_whitespace())
			.collect::<std::string::String>()
	};
	assert_eq!(compact(original_loop), compact(cold_loop));
	assert!(code.contains("(&self.hosts, !self.absent.is_empty())"));
	assert!(compact(cold).contains("ifhas_absent{code.validate_absence(runtime)?;}"));
	assert!(cold.contains("Error::new(ErrorKind::InvalidCode, \"validate code\")"));
	let hosts_done = cold.find("code.validate_absence(runtime)?;").unwrap();
	let authority = cold
		.find("match (expected, runtime.formatting.as_ref())")
		.unwrap();
	assert!(hosts_done < authority);
	assert!(authority < cold.find("store.validate_format_roots(").unwrap());
}

#[test]
fn ordinary_and_cold_host_validation_agree_before_formatter_mismatches() {
	use crate::experimental_core::code::{CompileTransaction, Function, Op};
	use crate::function::host::HostBinding;
	use crate::sync::Arc;
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc as Shared,
	};

	fn context(calls: &Shared<AtomicUsize>, module: &str, name: &str) -> Context {
		let mut m = crate::Module::with_item([module]).unwrap();
		let calls = calls.clone();
		m.host_function(name, move || {
			calls.fetch_add(1, Ordering::SeqCst);
			7i64
		})
		.unwrap();
		let mut cx = Context::new();
		cx.install(m).unwrap();
		cx
	}
	fn bound_code(binding: &HostBinding, absent: Option<Hash>) -> Code {
		let mut tx = CompileTransaction::default();
		let host = tx.host(binding).unwrap();
		if let Some(hash) = absent {
			tx.static_method(&Context::new(), hash).unwrap();
		}
		let mut function = Function::new(1, 0).unwrap();
		function
			.push(
				Op::Call {
					out: 0,
					function: host,
					args_start: 0,
					args_len: 0,
				},
				Location::UNKNOWN,
			)
			.unwrap();
		function
			.push(Op::Return { value: 0 }, Location::UNKNOWN)
			.unwrap();
		tx.push(function).unwrap();
		tx.finish().unwrap()
	}

	let calls = Shared::new(AtomicUsize::new(0));
	let mut cx = context(&calls, "entry_host", "tap");
	let binding = cx
		.host(Hash::type_hash(["entry_host", "tap"]))
		.unwrap()
		.clone();
	let valid = Arc::try_new(cx.runtime().unwrap()).unwrap();
	let compile_authority = cx.format_snapshot().unwrap();
	let absent = Hash::type_hash(["entry_absence", "probe"]);
	let mut extra = crate::Module::with_item(["entry_absence"]).unwrap();
	let counter = calls.clone();
	extra
		.host_function("probe", move || {
			counter.fetch_add(1, Ordering::SeqCst);
			8i64
		})
		.unwrap();
	cx.install(extra).unwrap();
	let present = Arc::try_new(cx.runtime().unwrap()).unwrap();
	let other = context(&calls, "entry_marker", "marker");
	let other_authority = other.format_snapshot().unwrap();
	let missing = Arc::try_new(other.runtime().unwrap()).unwrap();
	let different = context(&calls, "entry_host", "tap");
	let different = Arc::try_new(different.runtime().unwrap()).unwrap();

	for (label, runtime, absent_proof, invalid_signature) in [
		("valid", Some(valid.clone()), false, false),
		("missing-function", Some(missing), false, false),
		("invalid-signature", Some(valid.clone()), false, true),
		(
			"same-signature-different-handler",
			Some(different),
			false,
			false,
		),
		("absent-provider-present", Some(present), true, false),
		("absent-provider-absent", Some(valid), true, false),
		("no-runtime", None, false, false),
	] {
		let mut code = bound_code(&binding, absent_proof.then_some(absent));
		if invalid_signature {
			code.invalidate_host_signature_for_test();
		}
		for mismatch in [false, true] {
			let matching = runtime
				.as_ref()
				.and_then(|r| r.formatting.clone())
				.unwrap_or_else(|| compile_authority.clone());
			let expected = if mismatch {
				if matching.equivalent(&compile_authority) {
					other_authority.clone()
				} else {
					compile_authority.clone()
				}
			} else {
				matching
			};
			if let Some(runtime) = &runtime {
				assert_eq!(
					!expected.equivalent(runtime.formatting.as_ref().unwrap()),
					mismatch
				);
			}
			let ordinary = code.validate_hosts(runtime.as_deref());
			let mut e = executor();
			e.set_host_runtime(runtime.clone()).unwrap();
			e.canceled = true;
			let s = store();
			// Some compile authority forces the actual cold host-copy path even
			// without a runtime. No runtime means compile authority, not mismatch.
			let cold = e.validate_format_entry(None, (&code, 0), Some(&expected), &s, &[]);
			match ordinary {
				Err(original) => {
					let actual = cold.unwrap_err();
					assert_eq!(
						(actual.kind, actual.operation),
						(original.kind, original.operation),
						"{label}, mismatch={mismatch}"
					);
					assert_eq!(
						(actual.kind, actual.operation),
						(ErrorKind::InvalidCode, "validate code")
					);
				}
				Ok(()) if mismatch && runtime.is_some() => {
					let actual = cold.unwrap_err();
					let entry = Error::refused(super::super::error::Refusal::Entry);
					assert_eq!(
						(actual.kind, actual.operation, actual.refusal),
						(entry.kind, entry.operation, entry.refusal)
					);
				}
				Ok(()) => assert!(cold.is_ok(), "{label}, mismatch={mismatch}"),
			}
			assert!(e.is_idle());
			assert!(e.active_host_runtime.is_none());
			assert!(e.canceled);
			assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
			assert_eq!(calls.load(Ordering::SeqCst), 0);
		}
	}
}
