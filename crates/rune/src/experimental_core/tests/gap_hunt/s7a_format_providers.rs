//! Ordinary protocol-kind registrations refuse formatting before execution.
//! S7b2 closes actual-key aliases and foreign roots.
use super::*;
use crate as rune;
use crate::experimental_core::error::{ErrorKind, Refusal};
use crate::runtime::{Formatter, Protocol, Range};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::boxed::Box;
use std::sync::atomic::AtomicUsize;

fn input(source: &str) -> Sources {
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	sources
}

fn providers(context: &mut Context, calls: &Arc<AtomicUsize>) {
	let mut module = crate::Module::new();
	let counter = calls.clone();
	module
		.associated_function(&Protocol::DEBUG_FMT, move |_: &Range, _: &mut Formatter| {
			counter.fetch_add(1, Ordering::SeqCst);
		})
		.unwrap();
	let counter = calls.clone();
	module
		.associated_function(
			&Protocol::DISPLAY_FMT,
			move |_: &Range, _: &mut Formatter| {
				counter.fetch_add(1, Ordering::SeqCst);
			},
		)
		.unwrap();
	context.install(module).unwrap();
}

fn refused(context: &Context, source: &str) {
	let mut sources = input(source);
	let mut diagnostics = Diagnostics::new();
	let Err(error) = frontend::compile(&mut sources, context, &mut diagnostics, 77) else {
		panic!("modified provider formatting admitted: {source}");
	};
	assert_eq!(error.kind, ErrorKind::Unsupported);
	assert_eq!(error.refusal, Some(Refusal::Native));
	let location = error.location.unwrap();
	assert_eq!((location.source, location.revision), (0, 77));
	assert!(location.start < location.end && location.end as usize <= source.len());
	assert!(diagnostics.has_error());
}

#[test]
fn frozen_modified_provider_contract() {
	let document: Json = serde_json::from_str(include_str!("s7a_format_providers.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 8);
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	assert!(context.formatting());
	providers(&mut context, &calls);
	assert!(!context.formatting());
	for row in rows {
		let source = row["source"].as_str().unwrap();
		assert_eq!(
			hex(&Sha256::digest(source.as_bytes())),
			row["source_sha256"]
		);
		calls.store(0, Ordering::SeqCst);
		let mut sources = input(source);
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let observation = match vm.call(["main"], ()) {
			Ok(value) => format!("value:{:?}", &*value.borrow_string_ref().unwrap()),
			Err(error) => format!("error:{error}"),
		};
		// The modified Range formatter writes no text; unrelated formatters keep
		// their ordinary output. These are small public behavior invariants.
		let (expected, expected_calls) = match row["name"].as_str().unwrap() {
			"debug" | "display" => ("value:\"\"", 1),
			"string-debug" | "string-local" => ("value:\"\\\"x\\\"\"", 0),
			"vec-nested" => ("value:\"[]\"", 1),
			"struct-nested" => ("value:\"S {r: }\"", 1),
			"struct-safe" => ("value:\"S {r: 1}\"", 0),
			"struct-display" => ("error:`S` cannot be formatted with `{}`", 0),
			name => panic!("unlisted provider case {name}"),
		};
		assert_eq!(observation, expected);
		assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
		calls.store(0, Ordering::SeqCst);
		refused(&context, source);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
	contract_completed("s7a-modified-provider", rows.len());
}

#[test]
fn every_formatting_ingress_closes_but_unrelated_programs_remain_admitted() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	providers(&mut context, &calls);
	for source in [
		"pub fn main(){format!(\"{:?}\",{panic(\"must not run\");1..3})}",
		"pub fn main(){format!(\"{}\",[1..3])}",
		"pub fn main(){let x=1..3; `value=${x}`}",
		"pub fn main(){println!(\"{:?}\",1..3)}",
		"pub fn main(){print!(\"{:?}\",1..3)}",
		"pub fn main(){std::io::println(\"plain\"); 1}",
		"pub fn main(){std::io::println(\"plain\")}",
		"pub fn main(){std::io::print(\"plain\"); 1}",
		"pub fn main(){std::io::print(\"plain\")}",
	] {
		refused(&context, source);
	}
	let mut sources = input("pub fn main(){let xs=[1,2]; xs.len()}");
	assert!(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 77).is_ok());
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn std_factory_marks_only_existing_handlers_and_raw_providers_close() {
	let mut module = crate::modules::ops::module().unwrap();
	assert!(module.associated.iter().any(
		|a| matches!(&a.kind, crate::module::ModuleAssociatedKind::Function(f) if f.format_default.is_some())
	));
	module
		.raw_function(&Protocol::DISPLAY_FMT, |_, _, _, _| Ok(()))
		.build_associated::<crate::runtime::Function>()
		.unwrap();
	assert!(
		matches!(&module.associated.last().unwrap().kind, crate::module::ModuleAssociatedKind::Function(f) if f.format_default.is_none())
	);
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.raw_function(&Protocol::DEBUG_FMT, |_, _, _, _| Ok(()))
		.build_associated::<Range>()
		.unwrap();
	context.install(module).unwrap();
	refused(&context, "pub fn main(){format!(\"{:?}\",[1..3])}");
}

#[test]
fn trait_and_failed_provider_installs_fail_closed() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.define_trait(["FormatProvider"])
		.unwrap()
		.handler(|cx| {
			cx.raw_function(&Protocol::DEBUG_FMT, |_, _, _, _| Ok(()))?;
			Ok(())
		})
		.unwrap();
	module
		.implement_trait::<Range>(crate::item!(FormatProvider))
		.unwrap();
	context.install(module).unwrap();
	refused(&context, "pub fn main(){format!(\"{:?}\",[1..3])}");
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.raw_function(&Protocol::DEBUG_FMT, |_, _, _, _| Ok(()))
		.build_associated::<crate::alloc::String>()
		.unwrap();
	assert!(context.install(module).is_err());
	refused(&context, "pub fn main(){format!(\"{:?}\",\"x\")}");
}

fn legacy_ingress(context: &Context, source: &str) -> String {
	let mut sources = input(source);
	let mut vm = match crate::prepare(&mut sources)
		.with_context(context)
		.build_vm()
	{
		Ok(vm) => vm,
		Err(error) => return format!("compile-error:{error:?}"),
	};
	match vm.call(["main"], ()) {
		Ok(value) => match value.borrow_string_ref() {
			Ok(text) => format!("value:{:?}", &*text),
			Err(_) => format!("value:{value:?}"),
		},
		Err(error) => format!("error:{error}"),
	}
}

fn neutral(context: &Context, source: &str) -> String {
	let mut sources = input(source);
	let program = frontend::compile(&mut sources, context, &mut Diagnostics::new(), 77).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let observation = match executor.invoke(
		&program,
		&mut store,
		program.function(Hash::type_hash(["main"])).unwrap(),
		&[],
		2_000_000,
	) {
		Ok(root) => {
			let text = format!("value:{:?}", store.root_text(&root).unwrap());
			store.root_release(root).unwrap();
			text
		}
		Err(error) => format!("error:{}", error.message(&program)),
	};
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	observation
}

#[test]
fn frozen_diagnostic_and_dynamic_formatting_ingresses_close() {
	let document: Json = serde_json::from_str(include_str!("s7a_format_ingresses.json")).unwrap();
	assert_eq!(document["schema"], 1);
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 26);
	let calls = Arc::new(AtomicUsize::new(0));
	let default = Context::with_default_modules().unwrap();
	let mut context = Context::with_default_modules().unwrap();
	providers(&mut context, &calls);
	for row in rows {
		let source = row["source"].as_str().unwrap();
		assert_eq!(
			hex(&Sha256::digest(source.as_bytes())),
			row["source_sha256"]
		);
		calls.store(0, Ordering::SeqCst);
		let stock_observation = legacy_ingress(&context, source);
		assert_eq!(
			calls.load(Ordering::SeqCst),
			row["expected_stock_calls"].as_u64().unwrap() as usize
		);
		calls.store(0, Ordering::SeqCst);
		match row["policy"].as_str().unwrap() {
			"Native" => refused(&context, source),
			"FrontendDiagnostic" => {
				let Err(error) =
					frontend::compile(&mut input(source), &context, &mut Diagnostics::new(), 77)
				else {
					panic!("previously refused ingress admitted");
				};
				assert_eq!(error.refusal, Some(Refusal::FrontendDiagnostic));
			}
			"Retained-neutral" => {
				assert_eq!(neutral(&context, source), stock_observation)
			}
			_ => panic!("unknown ingress policy"),
		}
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		if row["default_admitted"] == true {
			assert!(
				frontend::compile(&mut input(source), &default, &mut Diagnostics::new(), 77)
					.is_ok(),
				"default admission lost: {}",
				row["name"]
			);
		}
	}
	// An exact script method with the same name does not enter native dispatch.
	let source = "struct S;impl S{fn unwrap(self){\"safe\"}fn to_string(self){\"safe\"}}pub fn main(){S.unwrap()+S.to_string()}";
	assert_eq!(neutral(&context, source), "value:\"safesafe\"");
	assert_eq!(calls.load(Ordering::SeqCst), 0);
	contract_completed("s7a-format-ingress", rows.len());
}

fn alias_context(setup: &str, calls: &Arc<AtomicUsize>) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	let receiver = <i64 as crate::TypeHash>::HASH;
	let (target, formatter) = match setup {
		"field" | "raw-range" => (<Range as crate::TypeHash>::HASH, Protocol::DEBUG_FMT.hash),
		"raw-variant-display" => (Hash::type_hash(["E", "A"]), Protocol::DISPLAY_FMT.hash),
		"raw-debug-nominal" => (Hash::type_hash(["S"]), Protocol::DEBUG_FMT.hash),
		"raw-debug-late" => (Hash::type_hash(["LateOnly"]), Protocol::DEBUG_FMT.hash),
		"inline-debug" => (receiver, Protocol::DEBUG_FMT.hash),
		"inline-display" => (receiver, Protocol::DISPLAY_FMT.hash),
		"raw-nominal" => (Hash::type_hash(["S"]), Protocol::DISPLAY_FMT.hash),
		"raw-late-nominal" => (Hash::type_hash(["LateOnly"]), Protocol::DISPLAY_FMT.hash),
		_ => panic!("unknown alias setup"),
	};
	let mut protocol = Protocol::GET;
	let counter = calls.clone();
	if setup == "field" {
		protocol.hash = Hash::new(formatter.into_inner() ^ Hash::ident("field").into_inner());
		assert_eq!(
			Hash::field_function(protocol.hash, target, "field"),
			Hash::associated_function(target, formatter)
		);
		let protocol = Box::leak(Box::new(protocol));
		module
			.field_function(protocol, "field", move |_: &Range, _: &mut Formatter| {
				counter.fetch_add(1, Ordering::SeqCst);
			})
			.unwrap();
	} else {
		protocol.hash =
			Hash::new(receiver.into_inner() ^ target.into_inner() ^ formatter.into_inner());
		if !setup.starts_with("inline-") {
			assert_ne!(protocol.hash, Protocol::DEBUG_FMT.hash);
			assert_ne!(protocol.hash, Protocol::DISPLAY_FMT.hash);
		}
		assert_eq!(
			Hash::associated_function(receiver, protocol.hash),
			Hash::associated_function(target, formatter)
		);
		let protocol = Box::leak(Box::new(protocol));
		module
			.raw_function(&*protocol, move |memory, _, len, out| {
				assert_eq!(len, 2);
				counter.fetch_add(1, Ordering::SeqCst);
				memory.store(out, ())?;
				Ok(())
			})
			.build_associated::<i64>()
			.unwrap();
	}
	context.install(module).unwrap();
	// Actual keys close builtin aliases, independently of metadata kinds.
	assert_eq!(
		context.formatting(),
		setup != "field" && setup != "raw-range"
	);
	context
}

fn alias(context: &Context, source: &str, producer: Option<&str>) -> String {
	let render = match frontend::compile(&mut input(source), context, &mut Diagnostics::new(), 77) {
		Ok(program) => program,
		Err(error) => return format!("compile-refused:{:?}", error.refusal),
	};
	let producer_context = Context::with_default_modules().unwrap();
	// Keep the renderer-first order: a foreign nominal can arrive later.
	let make = producer.map(|s| {
		frontend::compile(
			&mut input(s),
			&producer_context,
			&mut Diagnostics::new(),
			77,
		)
		.unwrap()
	});
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let argument = make.as_ref().map(|p| {
		executor
			.invoke(
				p,
				&mut store,
				p.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				2_000_000,
			)
			.unwrap()
	});
	let args: Vec<_> = argument.iter().collect();
	let result = match executor.invoke(
		&render,
		&mut store,
		render.function(Hash::type_hash(["main"])).unwrap(),
		&args,
		2_000_000,
	) {
		Ok(root) => {
			let text = format!("value:{:?}", store.root_text(&root).unwrap());
			store.root_release(root).unwrap();
			text
		}
		Err(error)
			if error.error.refusal == Some(crate::experimental_core::error::Refusal::Entry) =>
		{
			"entry-refused:Some(Entry)".into()
		}
		Err(error) => format!("error:{}", error.message(&render)),
	};
	if let Some(root) = argument {
		store.root_release(root).unwrap();
	}
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	result
}

fn legacy_alias(context: &Context, source: &str, producer: Option<&str>) -> String {
	let Some(producer) = producer else {
		return legacy_ingress(context, source);
	};
	let mut render_input = input(source);
	let mut render = crate::prepare(&mut render_input)
		.with_context(context)
		.build_vm()
		.unwrap();
	let mut make_input = input(producer);
	let mut make = crate::prepare(&mut make_input)
		.with_context(context)
		.build_vm()
		.unwrap();
	let value = make.call(["main"], ()).unwrap();
	match render.call(["main"], (value,)) {
		Ok(value) => format!("value:{:?}", &*value.borrow_string_ref().unwrap()),
		Err(error) => format!("error:{error}"),
	}
}

#[test]
fn formatter_key_alias_and_foreign_root_gaps_close_before_execution() {
	let document: Json = serde_json::from_str(include_str!("s7a_format_key_gaps.json")).unwrap();
	assert_eq!(document["schema"], 1);
	assert_eq!(document["policy"], "Known-gap");
	assert_eq!(document["closure_record"], "S7b host-boundary certificate");
	assert_eq!(document["default_switch_prerequisite_complete"], false);
	assert!(Context::new().formatting());
	assert!(Context::with_default_modules().unwrap().formatting());
	let rows = document["rows"].as_array().unwrap();
	assert_eq!(rows.len(), 5);
	for row in rows {
		assert_eq!(row["policy"], "Known-gap");
		let source = row["source"].as_str().unwrap();
		assert_eq!(
			hex(&Sha256::digest(source.as_bytes())),
			row["source_sha256"]
		);
		let producer = row["producer_source"].as_str();
		if let Some(p) = producer {
			assert_eq!(hex(&Sha256::digest(p.as_bytes())), row["producer_sha256"]);
			assert_eq!(
				row["compile_order"],
				serde_json::json!(["renderer", "producer"])
			);
		}
		let calls = Arc::new(AtomicUsize::new(0));
		let context = alias_context(row["setup"].as_str().unwrap(), &calls);
		// Run new first so the late producer is not compiled by stock beforehand.
		let execution = alias(&context, source, producer);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		let stock = legacy_alias(&context, source, producer);
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert_ne!(execution, stock, "must not label this Context Exact");
		let expected = if row["name"] == "late-foreign-nominal-display" {
			"entry-refused:Some(Entry)"
		} else {
			"compile-refused:Some(Native)"
		};
		assert_eq!(execution, expected, "{}", row["name"]);
		// S7b2 (approval seq 23030): only the late producer refuses at entry.
		assert_eq!(
			producer.is_some(),
			row["name"] == "late-foreign-nominal-display"
		);
	}
	contract_completed("s7a-format-key-Known-gap", rows.len());
}

#[derive(crate::Any)]
struct ForeignFormat;

#[test]
fn unrelated_native_formatters_preserve_builtin_formatting() {
	let calls = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module.ty::<ForeignFormat>().unwrap();
	let counter = calls.clone();
	module
		.associated_function(
			&Protocol::DISPLAY_FMT,
			move |_: &ForeignFormat, _: &mut Formatter| {
				counter.fetch_add(1, Ordering::SeqCst);
			},
		)
		.unwrap();
	context.install(module).unwrap();
	assert!(context.formatting());
	let source = "pub fn main(){format!(\"{}\", 42)}";
	assert_eq!(alias(&context, source, None), "value:\"42\"");
	assert_eq!(calls.load(Ordering::SeqCst), 0);
	providers(&mut context, &calls);
	assert!(!context.formatting());
	refused(&context, source);
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg(test)]
mod s7b2_entry_controls {
	use super::*;
	use crate::experimental_core::{
		frontend::Program,
		value::{Root, Type},
	};
	fn compile(cx: &Context, source: &str) -> Program {
		frontend::compile(&mut input(source), cx, &mut Diagnostics::new(), 77).unwrap()
	}
	fn main(p: &Program) -> u32 {
		p.function(Hash::type_hash(["main"])).unwrap()
	}
	fn default() -> Context {
		Context::with_default_modules().unwrap()
	}
	fn run(
		e: &mut Executor,
		p: &Program,
		s: &mut Store,
		a: &[&Root],
		fuel: usize,
	) -> core::result::Result<Root, Box<crate::experimental_core::execute::Failure>> {
		e.invoke(p, s, main(p), a, fuel).map_err(Box::new)
	}
	#[test]
	fn named_static_insertions_record_both_effective_native_keys() {
		let calls = Arc::new(AtomicUsize::new(0));
		let mut cx = default();
		let mut module = crate::Module::new();
		let counter = calls.clone();
		module
			.raw_function("s7b2_named", move |_, _, _, _| {
				counter.fetch_add(1, Ordering::SeqCst);
				Ok(())
			})
			.build_associated::<i64>()
			.unwrap();
		cx.install(module).unwrap();
		let authority = cx.format_snapshot().unwrap();
		for key in [
			Hash::associated_function(<i64 as crate::TypeHash>::HASH, Hash::ident("s7b2_named")),
			Hash::type_hash(crate::item!(::std::i64::s7b2_named)),
		] {
			assert!(cx.lookup_function(key).is_some());
			// Invert the effective key to name a synthetic nominal receiver.
			// Neither the associated nor its free static alias may escape capture.
			let receiver = Hash::associated_function(key, &Protocol::DISPLAY_FMT);
			assert!(!authority.permits(receiver));
		}
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn function_parameters_cannot_hide_an_actual_formatter_key() {
		let calls = Arc::new(AtomicUsize::new(0));
		let mut cx = default();
		let mut module = crate::Module::new();
		let receiver = <i64 as crate::TypeHash>::HASH;
		let target = <Range as crate::TypeHash>::HASH;
		let parameters = [<u64 as crate::TypeHash>::HASH];
		let salt = Hash::EMPTY.with_function_parameters(Hash::parameters(parameters));
		let mut protocol = Protocol::GET;
		protocol.hash = Hash::new(
			receiver.into_inner()
				^ target.into_inner()
				^ Protocol::DEBUG_FMT.hash.into_inner()
				^ salt.into_inner(),
		);
		assert_eq!(
			Hash::associated_function(receiver, protocol.hash)
				.with_function_parameters(Hash::parameters(parameters)),
			Hash::associated_function(target, &Protocol::DEBUG_FMT)
		);
		let protocol = Box::leak(Box::new(protocol));
		let counter = calls.clone();
		module
			.raw_function(
				crate::Params::new(&*protocol, parameters),
				move |_, _, _, _| {
					counter.fetch_add(1, Ordering::SeqCst);
					Ok(())
				},
			)
			.build_associated::<i64>()
			.unwrap();
		cx.install(module).unwrap();
		refused(&cx, "pub fn main(){format!(\"{:?}\",[1..3])}");
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}

	static DERIVED_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
	#[derive(crate::Any)]
	#[rune(item = ::s7b2_derived, install_with = install_derived_formatter)]
	struct DerivedFormatterProvider;
	fn install_derived_formatter(
		module: &mut crate::Module,
	) -> core::result::Result<(), crate::ContextError> {
		module
			.raw_function(&Protocol::DEBUG_FMT, |_, _, _, _| {
				DERIVED_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
				Ok(())
			})
			.build_associated::<Range>()?;
		Ok(())
	}
	#[test]
	fn a_derived_install_with_formatter_enters_the_effective_key_certificate() {
		DERIVED_PROVIDER_CALLS.store(0, Ordering::SeqCst);
		let mut cx = default();
		let mut module = crate::Module::with_crate("s7b2_derived").unwrap();
		module.ty::<DerivedFormatterProvider>().unwrap();
		cx.install(module).unwrap();
		refused(&cx, "pub fn main(){format!(\"{:?}\",[1..3])}");
		assert_eq!(DERIVED_PROVIDER_CALLS.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn a_variant_key_alias_cannot_poison_its_safe_parent_enum() {
		let calls = Arc::new(AtomicUsize::new(0));
		let cx = alias_context("raw-variant-display", &calls);
		let source = "enum E{A{x}}pub fn main(){format!(\"{:?}\",E::A{x:7})}";
		let expected = legacy_alias(&default(), source, None);
		assert_eq!(legacy_alias(&cx, source, None), expected);
		assert_eq!(alias(&cx, source, None), expected);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn independently_built_server_contexts_and_slots_accept_equal_authority() {
		fn server_context(extra: bool) -> Context {
			let mut cx = default();
			let mut module = crate::Module::with_crate("server_shape").unwrap();
			module.host_function("tap", || 7i64).unwrap();
			if extra {
				module.host_function("extra", || 8i64).unwrap();
			}
			cx.install(module).unwrap();
			cx
		}
		let a = server_context(false);
		let p = compile(&a, "pub fn main(){format!(\"{}\",7)}");
		for extra in [false, false, true] {
			let b = server_context(extra);
			assert!(!crate::sync::Arc::ptr_eq(
				p.formatting.as_ref().unwrap(),
				b.format_snapshot().as_ref().unwrap()
			));
			let runtime = crate::sync::Arc::try_new(b.runtime().unwrap()).unwrap();
			let mut e = Executor::new().unwrap();
			e.set_host_runtime(Some(runtime)).unwrap();
			let mut s = Store::new(Limits::default()).unwrap();
			for _ in 0..2 {
				if extra {
					assert_eq!(
						run(&mut e, &p, &mut s, &[], 100).unwrap_err().error.refusal,
						Some(Refusal::Entry)
					);
				} else {
					let out = run(&mut e, &p, &mut s, &[], 100).unwrap();
					assert_eq!(s.root_text(&out).unwrap(), "7");
					s.root_release(out).unwrap();
				}
			}
			assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
		}
	}

	#[test]
	fn ineffective_nominal_debug_and_inline_keys_preserve_exact_formatting() {
		for (setup, source, producer) in [
			(
				"raw-debug-nominal",
				"struct S;pub fn main(){format!(\"{:?}\",S)}",
				None,
			),
			(
				"raw-debug-late",
				"pub fn main(x){format!(\"{:?}\",x)}",
				Some("struct LateOnly;pub fn main(){LateOnly}"),
			),
			("inline-debug", "pub fn main(){format!(\"{:?}\",42)}", None),
			("inline-display", "pub fn main(){format!(\"{}\",42)}", None),
		] {
			let calls = Arc::new(AtomicUsize::new(0));
			let cx = alias_context(setup, &calls);
			assert_eq!(
				alias(&cx, source, producer),
				alias(&default(), source, producer)
			);
			assert_eq!(
				legacy_alias(&cx, source, producer),
				legacy_alias(&default(), source, producer)
			);
			assert_eq!(calls.load(Ordering::SeqCst), 0);
		}
	}
	#[test]
	fn structural_nominal_debug_rejects_an_unsafe_any_backed_field() {
		let calls = Arc::new(AtomicUsize::new(0));
		let cx = alias_context("raw-range", &calls);
		let source = "struct S{field}pub fn main(){format!(\"{:?}\",S{field:0..2})}";
		let failure = frontend::compile(&mut input(source), &cx, &mut Diagnostics::new(), 77)
			.err()
			.unwrap();
		assert_eq!(
			failure.refusal,
			Some(crate::experimental_core::error::Refusal::Native)
		);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
	#[test]
	fn default_authority_never_enters_the_walk_or_allocates_its_scratch() {
		for cx in [Context::new(), default()] {
			let p = compile(&cx, "pub fn main(x){7}");
			assert!(p.formatting.as_ref().is_none());
			let mut e = Executor::new().unwrap();
			let mut s = Store::new(Limits::default()).unwrap();
			let x = s.root_list(Type::Vec, &[]).unwrap();
			Store::format_admission_probe(true);
			let out = run(&mut e, &p, &mut s, &[&x], 100).unwrap();
			assert_eq!(Store::format_admission_probe(false), 0);
			s.root_release(out).unwrap();
			s.root_release(x).unwrap();
			assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
		}
	}
	#[test]
	fn pure_format_runtime_mismatch_is_entry_before_print_and_binding_error_keeps_precedence() {
		let calls = Arc::new(AtomicUsize::new(0));
		let mut a = default();
		let mut m = crate::Module::with_crate("authority").unwrap();
		let counter = calls.clone();
		m.host_function("tap", move || {
			counter.fetch_add(1, Ordering::SeqCst);
			7i64
		})
		.unwrap();
		a.install(m).unwrap();
		let mut b = default();
		let mut m = crate::Module::with_crate("authority").unwrap();
		m.host_function("tap", || 8i64).unwrap();
		m.host_function("extra", || 9i64).unwrap();
		b.install(m).unwrap();
		let p = compile(
			&a,
			"pub fn main(){println!(\"sentinel\");format!(\"{:?}\",7)}",
		);
		let used = compile(&a, "pub fn main(){authority::tap()}");
		let mut e = Executor::new().unwrap();
		e.test_output = Some(std::vec::Vec::new());
		let mut s = Store::new(Limits::default()).unwrap();
		e.set_host_runtime(Some(
			crate::sync::Arc::try_new(b.runtime().unwrap()).unwrap(),
		))
		.unwrap();
		assert_eq!(
			run(&mut e, &p, &mut s, &[], 0).unwrap_err().error.refusal,
			Some(Refusal::Entry)
		);
		let invalid_binding = run(&mut e, &used, &mut s, &[], 0).unwrap_err();
		assert_eq!(invalid_binding.error.kind, ErrorKind::InvalidCode);
		assert_eq!(invalid_binding.error.operation, "validate code");
		assert!(e.test_output.as_ref().unwrap().is_empty());
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		e.set_host_runtime(Some(
			crate::sync::Arc::try_new(a.runtime().unwrap()).unwrap(),
		))
		.unwrap();
		let out = run(&mut e, &p, &mut s, &[], 1000).unwrap();
		assert_eq!(s.root_text(&out).unwrap(), "7");
		s.root_release(out).unwrap();
		assert_eq!(e.test_output.as_ref().unwrap(), b"sentinel\n");
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	#[test]
	fn foreign_nominals_nested_shared_cyclic_and_unused_refuse_in_both_compile_orders() {
		for producer_first in [false, true] {
			let calls = Arc::new(AtomicUsize::new(0));
			let cx = alias_context("raw-late-nominal", &calls);
			let safe = default();
			let producer_source = "struct LateOnly;pub fn main(){LateOnly}";
			let renderer_source = "pub fn main(x){7}";
			let (producer, renderer) = if producer_first {
				(
					compile(&safe, producer_source),
					compile(&cx, renderer_source),
				)
			} else {
				let r = compile(&cx, renderer_source);
				(compile(&safe, producer_source), r)
			};
			let mut e = Executor::new().unwrap();
			let mut s = Store::new(Limits::default()).unwrap();
			let bad = run(&mut e, &producer, &mut s, &[], 1000).unwrap();
			let shared = s.root_list(Type::Tuple, &[&bad, &bad]).unwrap();
			let object = s.root_record(None, &[("nested", &shared)]).unwrap();
			let option = s.root_variant(Type::Option, true, Some(&object)).unwrap();
			let cycle = s.root_list(Type::Vec, &[&option]).unwrap();
			s.root_push(&cycle, &cycle).unwrap();
			let live = (s.live_entries(), s.payload_bytes());
			e.request_cancel();
			assert_eq!(
				run(&mut e, &renderer, &mut s, &[&cycle], 0)
					.unwrap_err()
					.error
					.refusal,
				Some(Refusal::Entry)
			);
			assert_eq!((s.live_entries(), s.payload_bytes()), live);
			assert!(e.is_idle());
			assert_eq!(calls.load(Ordering::SeqCst), 0);
			let unit = s.root_unit();
			assert_eq!(
				run(&mut e, &renderer, &mut s, &[&unit], 1000)
					.unwrap_err()
					.error
					.kind,
				ErrorKind::Cancelled
			);
			let out = run(&mut e, &renderer, &mut s, &[&unit], 1000).unwrap();
			s.root_release(out).unwrap();
			let _ = s.close();
			assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
		}
	}
	#[test]
	fn foreign_closure_iterator_enum_and_host_constructed_record_cannot_hide_the_receiver() {
		for source in [
			"struct LateOnly;pub fn main(){let s=LateOnly;||s}",
			"struct LateOnly;pub fn main(){[LateOnly].iter()}",
			"enum LateOnly{A}pub fn main(){LateOnly::A}",
			"struct LateOnly{a}pub fn main(){LateOnly{a:1}}",
		] {
			let calls = Arc::new(AtomicUsize::new(0));
			let cx = alias_context("raw-late-nominal", &calls);
			let safe = default();
			let renderer = compile(&cx, "pub fn main(x){7}");
			let producer = compile(&safe, source);
			let mut e = Executor::new().unwrap();
			let mut s = Store::new(Limits::default()).unwrap();
			let bad = run(&mut e, &producer, &mut s, &[], 1000).unwrap();
			assert_eq!(
				run(&mut e, &renderer, &mut s, &[&bad], 0)
					.unwrap_err()
					.error
					.refusal,
				Some(Refusal::Entry)
			);
			if source.contains("LateOnly{a}") {
				let Type::Struct(n) = s.root_tag(&bad).unwrap() else {
					panic!()
				};
				let unit = s.root_unit();
				let made = s.root_record(Some(n), &[("a", &unit)]).unwrap();
				assert_eq!(
					run(&mut e, &renderer, &mut s, &[&made], 0)
						.unwrap_err()
						.error
						.refusal,
					Some(Refusal::Entry)
				);
				s.root_release(made).unwrap();
			}
			s.root_release(bad).unwrap();
			assert_eq!(calls.load(Ordering::SeqCst), 0);
			assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
		}
	}
	#[test]
	fn argument_mutation_rechecks_and_allocation_failure_precedes_every_effect() {
		let calls = Arc::new(AtomicUsize::new(0));
		let cx = alias_context("raw-late-nominal", &calls);
		let safe = default();
		let renderer = compile(&cx, "pub fn main(x){7}");
		let producer = compile(&safe, "struct LateOnly;pub fn main(){LateOnly}");
		let mut e = Executor::new().unwrap();
		let mut s = Store::new(Limits::default()).unwrap();
		let graph = s.root_list(Type::Vec, &[]).unwrap();
		Store::format_admission_probe(true);
		let live = (s.live_entries(), s.payload_bytes());
		assert_eq!(
			run(&mut e, &renderer, &mut s, &[&graph], 0)
				.unwrap_err()
				.error
				.kind,
			ErrorKind::Allocation
		);
		assert_eq!((s.live_entries(), s.payload_bytes()), live);
		assert_eq!(Store::format_admission_probe(false), 1);
		let out = run(&mut e, &renderer, &mut s, &[&graph], 100).unwrap();
		s.root_release(out).unwrap();
		let bad = run(&mut e, &producer, &mut s, &[], 100).unwrap();
		s.root_push(&graph, &bad).unwrap();
		assert_eq!(
			run(&mut e, &renderer, &mut s, &[&graph], 0)
				.unwrap_err()
				.error
				.refusal,
			Some(Refusal::Entry)
		);
		s.root_release(graph).unwrap();
		s.root_release(bad).unwrap();
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	#[test]
	fn index_alias_closes_the_actual_range_debug_key() {
		let calls = Arc::new(AtomicUsize::new(0));
		let mut cx = default();
		let mut m = crate::Module::new();
		let mut protocol = Protocol::GET;
		protocol.hash =
			Hash::new(Protocol::DEBUG_FMT.hash.into_inner() ^ Hash::index(4).into_inner());
		let protocol = Box::leak(Box::new(protocol));
		let counter = calls.clone();
		m.index_function(protocol, 4, move |_: &Range, _: &mut Formatter| {
			counter.fetch_add(1, Ordering::SeqCst);
		})
		.unwrap();
		cx.install(m).unwrap();
		refused(&cx, "pub fn main(){format!(\"{:?}\",[1..3])}");
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
	#[test]
	fn authority_snapshots_survive_context_mutation_and_reject_stale_runtime_pairs() {
		let mut cx = default();
		let mut module = crate::Module::with_crate("epoch_a").unwrap();
		module.host_function("a", || 1i64).unwrap();
		cx.install(module).unwrap();
		let first = compile(&cx, "pub fn main(){7}");
		let before = crate::sync::Arc::try_new(cx.runtime().unwrap()).unwrap();
		let mut module = crate::Module::with_crate("epoch_b").unwrap();
		module.host_function("b", || 2i64).unwrap();
		cx.install(module).unwrap();
		let second = compile(&cx, "pub fn main(){7}");
		let after = crate::sync::Arc::try_new(cx.runtime().unwrap()).unwrap();
		let mut e = Executor::new().unwrap();
		let mut s = Store::new(Limits::default()).unwrap();
		e.set_host_runtime(Some(before)).unwrap();
		let out = run(&mut e, &first, &mut s, &[], 100).unwrap();
		s.root_release(out).unwrap();
		e.set_host_runtime(Some(after)).unwrap();
		assert_eq!(
			run(&mut e, &first, &mut s, &[], 0)
				.unwrap_err()
				.error
				.refusal,
			Some(Refusal::Entry)
		);
		let out = run(&mut e, &second, &mut s, &[], 100).unwrap();
		s.root_release(out).unwrap();
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	#[test]
	fn every_root_is_owner_and_generation_checked_before_walk_scratch() {
		let calls = Arc::new(AtomicUsize::new(0));
		let cx = alias_context("raw-late-nominal", &calls);
		let renderer = compile(&cx, "pub fn main(x){7}");
		let ordinary = compile(&default(), "pub fn main(x){7}");
		let runtime = crate::sync::Arc::try_new(cx.runtime().unwrap()).unwrap();
		assert!(crate::sync::Arc::ptr_eq(
			renderer.formatting.as_ref().unwrap(),
			runtime.formatting.as_ref().unwrap()
		));
		let mut e = Executor::new().unwrap();
		e.set_host_runtime(Some(runtime)).unwrap();
		let mut baseline = Executor::new().unwrap();
		let mut s = Store::new(Limits::default()).unwrap();
		let other = Store::new(Limits::default()).unwrap();
		let wrong = other.root_unit();
		let root = s.root_string("stale").unwrap();
		let stale = Root {
			store: root.store,
			value: root.value,
		};
		s.root_release(root).unwrap();
		for root in [&wrong, &stale] {
			let before = (s.live_entries(), s.payload_bytes());
			Store::format_admission_probe(true);
			let ordinary_kind = run(&mut baseline, &ordinary, &mut s, &[root], 0)
				.unwrap_err()
				.error
				.kind;
			let guarded_kind = run(&mut e, &renderer, &mut s, &[root], 0)
				.unwrap_err()
				.error
				.kind;
			assert_eq!(ordinary_kind, ErrorKind::InvalidHandle);
			assert_eq!(guarded_kind, ordinary_kind);
			assert_eq!(Store::format_admission_probe(false), 0);
			assert_eq!((s.live_entries(), s.payload_bytes()), before);
			assert!(baseline.is_idle() && e.is_idle());
		}
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
}
