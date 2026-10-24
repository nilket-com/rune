//! Modified-provider contexts refuse formatting before execution, including nested values.
use super::*;
use crate::experimental_core::error::{ErrorKind, Refusal};
use crate::runtime::{Formatter, Protocol, Range};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
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
	assert!(context.fresh_format_defaults());
	providers(&mut context, &calls);
	assert!(!context.fresh_format_defaults());
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
