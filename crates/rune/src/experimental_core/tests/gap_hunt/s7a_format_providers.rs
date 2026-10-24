//! Ordinary protocol-kind registrations refuse formatting before execution.
//! Actual-key aliases and foreign roots remain named S7b gaps; see s7a_format_key_gaps.
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
	assert!(context.fresh_formatting());
	providers(&mut context, &calls);
	assert!(!context.fresh_formatting());
	let mut capture = String::new();
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
		let actual = format!(
			"PROVIDER {} stock {observation} calls={}",
			row["name"].as_str().unwrap(),
			calls.load(Ordering::SeqCst)
		);
		assert_eq!(actual, row["stock"]);
		capture.push_str(&actual);
		capture.push('\n');
		capture.push_str(row["before_fresh"].as_str().unwrap());
		capture.push('\n');
		calls.store(0, Ordering::SeqCst);
		refused(&context, source);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
	}
	assert_eq!(
		hex(&Sha256::digest(capture.as_bytes())),
		document["capture_sha256"]
	);
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
		|a| matches!(&a.kind, crate::module::ModuleAssociatedKind::Function(f) if f.fresh_format_default)
	));
	module
		.raw_function(&Protocol::DISPLAY_FMT, |_, _, _, _| Ok(()))
		.build_associated::<crate::runtime::Function>()
		.unwrap();
	assert!(
		matches!(&module.associated.last().unwrap().kind, crate::module::ModuleAssociatedKind::Function(f) if !f.fresh_format_default)
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

fn stock_ingress(context: &Context, source: &str) -> String {
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

fn fresh_neutral(context: &Context, source: &str) -> String {
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
	assert_eq!(
		hex(&Sha256::digest(
			document["raw_capture"].as_str().unwrap().as_bytes()
		)),
		document["raw_capture_sha256"]
	);
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
		assert_eq!(
			stock_ingress(&context, source),
			row["stock"]["observation"],
			"{}",
			row["name"]
		);
		assert_eq!(
			calls.load(Ordering::SeqCst),
			row["stock"]["provider_calls"].as_u64().unwrap() as usize
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
				assert_eq!(fresh_neutral(&context, source), row["stock"]["observation"])
			}
			_ => panic!("unknown ingress policy"),
		}
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		if !row["before_fresh"]["observation"]
			.as_str()
			.unwrap()
			.starts_with("compile-refused:")
		{
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
	assert_eq!(fresh_neutral(&context, source), "value:\"safesafe\"");
	assert_eq!(calls.load(Ordering::SeqCst), 0);
	contract_completed("s7a-format-ingress", rows.len());
}

fn alias_context(setup: &str, calls: &Arc<AtomicUsize>) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	let receiver = <i64 as crate::TypeHash>::HASH;
	let (target, formatter) = match setup {
		"field" | "raw-range" => (<Range as crate::TypeHash>::HASH, Protocol::DEBUG_FMT.hash),
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
		assert_ne!(protocol.hash, Protocol::DEBUG_FMT.hash);
		assert_ne!(protocol.hash, Protocol::DISPLAY_FMT.hash);
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
	// This is deliberately the blind spot, not proof of an Exact Context.
	assert!(context.fresh_formatting());
	context
}

fn alias_fresh(context: &Context, source: &str, producer: Option<&str>) -> String {
	let render =
		frontend::compile(&mut input(source), context, &mut Diagnostics::new(), 77).unwrap();
	// Keep the renderer-first order: a foreign nominal can arrive later.
	let make = producer
		.map(|s| frontend::compile(&mut input(s), context, &mut Diagnostics::new(), 77).unwrap());
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
		Err(error) => format!("error:{}", error.message(&render)),
	};
	if let Some(root) = argument {
		store.root_release(root).unwrap();
	}
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	result
}

fn alias_stock(context: &Context, source: &str, producer: Option<&str>) -> String {
	let Some(producer) = producer else {
		return stock_ingress(context, source);
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
fn formatter_key_alias_and_foreign_root_cases_stay_named_known_gaps() {
	let document: Json = serde_json::from_str(include_str!("s7a_format_key_gaps.json")).unwrap();
	assert_eq!(document["schema"], 1);
	assert_eq!(document["policy"], "Known-gap");
	assert_eq!(document["closure_record"], "S7b host-boundary certificate");
	assert_eq!(document["default_switch_prerequisite_complete"], false);
	assert!(Context::new().fresh_formatting());
	assert!(Context::with_default_modules().unwrap().fresh_formatting());
	for capture in document["captures"].as_array().unwrap() {
		assert_eq!(
			hex(&Sha256::digest(capture["raw"].as_str().unwrap().as_bytes())),
			capture["raw_sha256"]
		);
	}
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
		// Run fresh first so the late producer is not compiled by stock beforehand.
		let fresh = alias_fresh(&context, source, producer);
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		let stock = alias_stock(&context, source, producer);
		assert_eq!(stock, row["stock"]["observation"]);
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert_eq!(row["stock"]["provider_calls"], 1);
		assert_eq!(row["before_fresh"]["provider_calls"], 0);
		assert_ne!(fresh, stock, "must not label this Context Exact");
		if row["setup"] == "raw-nominal" || row["setup"] == "raw-late-nominal" {
			assert_eq!(fresh, row["before_fresh"]["observation"]);
		} else {
			// Shape check ONLY for this Known-gap witness, never an equality mask.
			let text = fresh
				.strip_prefix("value:\"")
				.unwrap()
				.strip_suffix('"')
				.unwrap();
			let text = if row["name"] == "field-key-nested" {
				text.strip_prefix('[').unwrap().strip_suffix(']').unwrap()
			} else {
				text
			};
			let pointer = text
				.strip_prefix("<::std::ops::Range object at 0x")
				.unwrap()
				.strip_suffix('>')
				.unwrap();
			assert!(
				!pointer.is_empty()
					&& pointer.len() <= 16
					&& pointer.bytes().all(|b| b.is_ascii_hexdigit())
			);
		}
	}
	contract_completed("s7a-format-key-Known-gap", rows.len());
}
