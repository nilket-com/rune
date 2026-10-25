//! Frozen stock oracles and the exact-self admission boundary.
use super::*;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
	value::Value,
};

struct Case {
	name: &'static str,
	source: &'static str,
	value: i64,
	admitted_before: bool,
	admitted_after: bool,
}

const CASES: &[Case] = &[
	Case{name:"capacity",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:true},
	Case{name:"contains",source:"struct S;impl S{fn contains(self,x){x+1}fn run(self){self.contains(8)}}pub fn main(){S.run()}",value:9,admitted_before:false,admitted_after:true},
	Case{name:"resize",source:"struct S;impl S{fn resize(self,n,x){n+x}fn run(self){self.resize(3,4)}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:true},
	Case{name:"is_empty",source:"struct S;impl S{fn is_empty(self){true}fn run(self){if self.is_empty(){1}else{0}}}pub fn main(){S.run()}",value:1,admitted_before:true,admitted_after:true},
	Case{name:"path_ingress",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S::run(S)}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"function_value_ingress",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){let f=S::run;f(S)}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"reassigned",source:"struct S;impl S{fn capacity(self){7}fn run(self){self=S;self.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"closure",source:"struct S;impl S{fn capacity(self){7}fn run(self){let f=||self.capacity();f()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"alias",source:"struct S;impl S{fn capacity(self){7}fn run(self){let s=self;s.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"enum",source:"enum E{A,B}impl E{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){E::A.run()+E::B.run()}",value:14,admitted_before:false,admitted_after:true},
	Case{name:"admitted_native_mix",source:"struct S;impl S{fn is_some(self){true}fn run(self){if self.is_some(){1}else{0}}}pub fn main(){S.run()}",value:1,admitted_before:true,admitted_after:true},
	Case{name:"admitted_literal",source:"struct S;impl S{fn capacity(self){7}}pub fn main(){S.capacity()}",value:7,admitted_before:true,admitted_after:true},
];

fn compile_source(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 21)
}

#[test]
fn self_receiver_stock_oracles() {
	let context = Context::with_default_modules().unwrap();
	for c in CASES {
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(c.source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let value: i64 = crate::from_value(vm.call(["main"], ()).unwrap()).unwrap();
		assert_eq!(value, c.value, "{}", c.name);
	}
}

#[test]
fn self_receiver_frozen_frontier() {
	let context = Context::with_default_modules().unwrap();
	for c in CASES {
		let result = compile_source(c.source, &context);
		// C18b3e's approved additive overlay leaves the frozen CASES intact.
		let progressed = matches!(c.name, "reassigned" | "closure" | "alias");
		assert_eq!(result.is_ok(), c.admitted_after || progressed, "{}", c.name);
		assert!(!c.admitted_before || c.admitted_after);
		if let Ok(program) = result {
			if progressed {
				let run = program.function(Hash::type_hash(["S", "run"])).unwrap();
				let main = program.function(Hash::type_hash(["main"])).unwrap();
				let capacity = program
					.function(Hash::type_hash(["S", "capacity"]))
					.unwrap();
				let mut expected = std::vec![run];
				if c.name == "closure" {
					let closures: std::vec::Vec<_> = (0..program.code.functions() as u32)
						.filter(|&id| {
							id != run
								&& id != main && id != capacity
								&& program.functions[id as usize] != Hash::EMPTY
								&& program.code.function(id).unwrap().parameters == 1
						})
						.collect();
					assert_eq!(closures.len(), 1);
					expected.extend(closures);
				}
				let mut actual: std::vec::Vec<_> =
					program.entry_ineligible.iter().copied().collect();
				expected.sort_unstable();
				actual.sort_unstable();
				assert_eq!(actual, expected, "{}", c.name);
				assert!(program.entry_eligible(main));
				let ids: std::vec::Vec<_> = actual
					.iter()
					.map(
						|&id| serde_json::json!({"id":id,"hash":std::format!("{:?}",program.functions[id as usize])}),
					)
					.collect();
				std::println!(
					"C18E_SELF_POLICY {}",
					serde_json::json!({"case":c.name,"denied":ids})
				);
			}
			if c.admitted_before {
				assert!(
					program.entry_ineligible.is_empty(),
					"{} must keep old lowering",
					c.name
				);
			}
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = Executor::new().unwrap();
			let root = executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					10000,
				)
				.unwrap();
			assert!(
				matches!(store.import(&root).unwrap(),Value::I64(v) if v==c.value),
				"{}",
				c.name
			);
			store.root_release(root).unwrap();
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		} else if let Err(error) = result {
			assert_eq!(error.refusal, Some(Refusal::Method), "{}", c.name);
			assert!(error.location.is_some());
			if matches!(c.name, "path_ingress" | "function_value_ingress") {
				let at = error.location.unwrap();
				assert_eq!(
					&c.source[at.start as usize..at.end as usize],
					"self.capacity()"
				);
			}
		}
	}
}

#[test]
fn self_receiver_host_entry_is_refused_without_effects_or_cancellation_consumption() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};
	let effects = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["host"]).unwrap();
	let count = effects.clone();
	module
		.host_function("mark", move || {
			count.fetch_add(1, Ordering::SeqCst);
		})
		.unwrap();
	context.install(module).unwrap();
	let source="struct S;impl S{fn capacity(self){7}fn run(self){host::mark();self.capacity()}}pub fn make(){S}pub fn main(){S.run()}";
	let program = compile_source(source, &context).unwrap();
	let run = program.function(Hash::type_hash(["S", "run"])).unwrap();
	let make = program.function(Hash::type_hash(["make"])).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	assert!(!program.entry_eligible(run));
	assert!(program.entry_eligible(main));
	assert!(!program.entry_eligible(u32::MAX));
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let right = executor
		.invoke(&program, &mut store, make, &[], 1000)
		.unwrap();
	let wrong = store
		.root_list(crate::experimental_core::value::Type::Vec, &[])
		.unwrap();
	let baseline = (store.live_entries(), store.payload_bytes());
	executor.request_cancel();
	for root in [&wrong, &right] {
		for fuel in [0, 1000] {
			let failure = executor
				.invoke(&program, &mut store, run, &[root], fuel)
				.unwrap_err();
			assert_eq!(failure.error.refusal, Some(Refusal::Entry));
			assert!(failure.call_chain.is_empty() && failure.cleanup_error.is_none());
			assert!(executor.is_idle());
			assert_eq!((store.live_entries(), store.payload_bytes()), baseline);
			assert_eq!(effects.load(Ordering::SeqCst), 0);
		}
	}
	let failure = executor
		.invoke(&program, &mut store, main, &[], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert_eq!(effects.load(Ordering::SeqCst), 0);
	let value = executor
		.invoke(&program, &mut store, main, &[], 1000)
		.unwrap();
	assert!(matches!(store.import(&value).unwrap(), Value::I64(7)));
	assert_eq!(effects.load(Ordering::SeqCst), 1);
	for root in [value, wrong, right] {
		store.root_release(root).unwrap();
	}
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn self_receiver_exact_context_override_is_not_bypassed() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["host"]).unwrap();
	module
		.function("override", |_: crate::runtime::Value| 99i64)
		.build()
		.unwrap();
	context.install(module).unwrap();
	let handler = context
		.lookup_function(Hash::type_hash(["host", "override"]))
		.unwrap()
		.clone();
	context
		.c13_insert_raw_fixture(
			Hash::associated_function(Hash::type_hash(["S"]), Hash::ident("capacity")),
			handler,
		)
		.unwrap();
	let source = CASES[0].source;
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.build_vm()
		.unwrap();
	assert_eq!(
		crate::from_value::<i64>(vm.call(["main"], ()).unwrap()).unwrap(),
		99
	);
	assert!(
		matches!(compile_source(source,&context),Err(error) if error.refusal==Some(Refusal::Method))
	);
}

#[test]
fn self_receiver_generated_method_values_cannot_escape() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["host"]).unwrap();
	module.host_function("observe", |n: i64| n + 1).unwrap();
	context.install(module).unwrap();
	let program=compile_source("struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}fn dispatch(x){x.run()}pub fn main(){host::observe(dispatch(S))}",&context).unwrap();
	let mut lookups = 0;
	for id in 0..program.code.functions() {
		let f = program.code.function(id as u32).unwrap();
		for pc in 0..f.code.len() {
			let super::super::code::Op::Call {
				out: callee,
				function: super::super::code::INSTANCE_METHOD,
				..
			} = f.cached(pc).unwrap().op
			else {
				continue;
			};
			lookups += 1;
			let super::super::code::Op::CallValue {
				value,
				args_start,
				args_len,
				out,
			} = f.cached(pc + 1).unwrap().op
			else {
				panic!("lookup must feed adjacent private call")
			};
			assert_eq!(value, callee);
			assert_ne!(out, callee);
			assert!(
				!f.args[args_start as usize..(args_start + args_len) as usize].contains(&callee)
			);
			assert!(
				matches!(f.cached(pc+2).unwrap().op,super::super::code::Op::Release{value} if value==callee)
			);
		}
	}
	assert!(lookups > 0);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			10000,
		)
		.unwrap();
	assert!(matches!(store.import(&root).unwrap(), Value::I64(8)));
	store.root_release(root).unwrap();
}

#[test]
fn self_receiver_closure_capture_proof_fulfills_the_entry_policy_pin() {
	let context = Context::with_default_modules().unwrap();
	let program = compile_source(
		CASES.iter().find(|c| c.name == "closure").unwrap().source,
		&context,
	)
	.unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let run = program.function(Hash::type_hash(["S", "run"])).unwrap();
	assert!(program.entry_eligible(main));
	assert!(!program.entry_eligible(run));
	assert_eq!(program.entry_ineligible.len(), 2);
	// C18b3d pinned the future capture entry-policy obligation. C18b3e
	// fulfills that forward obligation on the same frozen source.
	let mut program = compile_source("pub fn main(){let n=7;let f=||n;f}", &context).unwrap();
	let closure = (0..program.code.functions() as u32)
		.find(|&id| program.code.function(id).unwrap().parameters == 1)
		.unwrap();
	program.entry_ineligible.try_push(closure).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let wrong = store.root_i64(1);
	let failure = Executor::new()
		.unwrap()
		.invoke(&program, &mut store, closure, &[&wrong], 1000)
		.unwrap_err();
	assert_eq!(failure.error.refusal, Some(Refusal::Entry));
}

#[test]
fn self_receiver_late_and_unreachable_ingresses_disable_the_proof() {
	let context = Context::with_default_modules().unwrap();
	for suffix in [
		"pub fn later(){S::run(1)}",
		"pub fn later(){let f=S::run;f(1)}",
		"pub fn later(){if false {let f=S::run;}0}",
	] {
		let source = std::format!("{} {suffix}", CASES[0].source);
		assert!(
			matches!(compile_source(&source,&context),Err(error) if error.refusal==Some(Refusal::Method) && error.location.is_some()),
			"{suffix}"
		);
	}
}

fn entry_policy_fixture() -> Program {
	use crate::experimental_core::code::{Builder, Function, Op};
	let at = Location::default();
	let mut builder = Builder::default();
	let mut internal = Function::new(1, 1).unwrap();
	internal.push(Op::Return { value: 0 }, at).unwrap();
	builder.push(internal).unwrap();
	let mut entry = Function::new(1, 0).unwrap();
	entry
		.push(
			Op::Literal {
				out: 0,
				value: Value::I64(7),
			},
			at,
		)
		.unwrap();
	entry.push(Op::Return { value: 0 }, at).unwrap();
	builder.push(entry).unwrap();
	let mut program = Program::for_diagnostics_test(builder.finish().unwrap());
	program.entry_ineligible.try_push(0).unwrap();
	program
}

#[test]
fn self_entry_policy_preserves_roots_and_pending_cancel() {
	let program = entry_policy_fixture();
	let mut executor = Executor::new().unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let root = store
		.root_list(crate::experimental_core::value::Type::Vec, &[])
		.unwrap();
	executor.request_cancel();
	for fuel in [0, 1000] {
		let failure = executor
			.invoke(&program, &mut store, 0, &[&root], fuel)
			.unwrap_err();
		assert_eq!(failure.error.refusal, Some(Refusal::Entry));
		assert!(failure.call_chain.is_empty() && executor.is_idle());
		assert_eq!(store.root_len(&root).unwrap(), 0);
		assert_eq!(store.live_entries(), 1);
	}
	let failure = executor
		.invoke(&program, &mut store, 1, &[], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	let result = executor.invoke(&program, &mut store, 1, &[], 1000).unwrap();
	assert!(matches!(store.import(&result).unwrap(), Value::I64(7)));
	store.root_release(result).unwrap();
	store.root_release(root).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn self_entry_policy_also_covers_future_capture_parameters() {
	// Capture parameters enter as ordinary arguments to an independent ID.
	// A later capture proof must enroll that ID, not just its parent method.
	let program = entry_policy_fixture();
	let store = Store::new(Limits::default()).unwrap();
	let capture = store.root_i64(99);
	let mut destination = Store::new(Limits::default()).unwrap();
	let failure = Executor::new()
		.unwrap()
		.run(&program, &mut destination, 0, &[&capture], 0)
		.unwrap_err();
	assert_eq!(failure.error.refusal, Some(Refusal::Entry));
	assert_eq!(destination.live_entries(), 0);
	assert!(failure.call_chain.is_empty());
}

#[test]
fn self_entry_policy_empty_set_checks_code_only_in_invoke_code() {
	let mut program = entry_policy_fixture();
	program.entry_ineligible.clear();
	assert!(!program.entry_refused(1));
	assert!(!program.entry_refused(u32::MAX));
	assert!(program.entry_eligible(1));
	assert!(!program.entry_eligible(u32::MAX));

	let frontend = include_str!("../frontend.rs");
	let policy = frontend.split("fn entry_refused(").nth(1).unwrap();
	let policy = policy.split("\n\t}").next().unwrap();
	assert!(!policy.contains("self.code"));
	let execute = include_str!("../execute.rs");
	let invoke = execute.split("pub fn invoke(").nth(1).unwrap();
	let invoke = invoke.split("\n\t}").next().unwrap();
	assert!(invoke.contains("program.entry_refused(function)"));
	assert!(!invoke.contains("program.code.function") && !invoke.contains("entry_eligible"));

	let mut executor = Executor::new().unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let result = executor.invoke(&program, &mut store, 1, &[], 1000).unwrap();
	assert!(matches!(store.import(&result).unwrap(), Value::I64(7)));
	store.root_release(result).unwrap();
	let failure = executor
		.invoke(&program, &mut store, u32::MAX, &[], 1000)
		.unwrap_err();
	assert_eq!(failure.error.refusal, None);
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

fn normalized_family_op(
	program: &Program,
	op: crate::experimental_core::code::Op,
) -> std::string::String {
	let raw = std::format!("{op:?}").replace(
		&std::format!("code: {}", program.code.id()),
		"code: PROGRAM",
	);
	let mut out = std::string::String::new();
	let mut rest = raw.as_str();
	while let Some(at) = rest.find("Nominal(") {
		out.push_str(&rest[..at]);
		let tail = &rest[at + 8..];
		let end = tail.find(')').unwrap();
		let id = tail[..end].parse::<u32>().unwrap();
		assert_eq!(
			crate::experimental_core::nominal::hash_copy(crate::experimental_core::value::Nominal(
				id
			)),
			Some(Hash::type_hash(["S"]).into_inner())
		);
		out.push_str("Nominal(S)");
		rest = &tail[end + 1..];
	}
	out.push_str(rest);
	out
}

#[test]
fn self_receiver_native_families_preserve_m3_lowering_or_match_stock() {
	let fixtures: serde_json::Value =
		serde_json::from_str(include_str!("self_receiver_families.json")).unwrap();
	let context = Context::with_default_modules().unwrap();
	for row in fixtures["rows"].as_array().unwrap() {
		let source = row["source"].as_str().unwrap();
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let stock = crate::from_value::<i64>(vm.call(["main"], ()).unwrap()).unwrap();
		assert_eq!(stock, row["stock_expected"].as_i64().unwrap());
		let program = compile_source(source, &context).unwrap();
		let run = program.function(Hash::type_hash(["S", "run"])).unwrap();
		if row["M3"] == "Exact" {
			assert!(program.entry_ineligible.is_empty());
			let ops = (0..program.code.functions())
				.map(|id| {
					let f = program.code.function(id as u32).unwrap();
					(
						std::format!("{:?}", program.functions[id]),
						f.parameters,
						f.slots,
						(0..f.code.len())
							.map(|pc| normalized_family_op(&program, f.cached(pc).unwrap().op))
							.collect::<std::vec::Vec<_>>(),
					)
				})
				.collect::<std::vec::Vec<_>>();
			assert_eq!(
				serde_json::to_value(ops).unwrap(),
				row["normalized_ops"],
				"{}",
				row["name"]
			);
		} else {
			assert_eq!(row["M3_refusal"], "Method");
			assert_eq!(row["M3_located"], true);
			assert!(!program.entry_eligible(run));
		}
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				10000,
			)
			.unwrap();
		assert!(matches!(store.import(&root).unwrap(),Value::I64(value) if value==stock));
		store.root_release(root).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
