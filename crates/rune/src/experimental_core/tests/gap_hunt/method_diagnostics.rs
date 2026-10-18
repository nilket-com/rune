//! D1 message observations across the public error consumers, independently of expectations.
use super::*;
use crate::runtime as stock;
use serde_json::{json, Value as Json};
use std::cell::RefCell;
const CASES: &str = include_str!("method_diagnostics/cases.json");
thread_local! {static TAPE: RefCell<Vec<Json>> = const {RefCell::new(Vec::new())};}
fn tap(tag: i64, value: i64) -> i64 {
	TAPE.with(|tape| {
		tape.borrow_mut()
			.push(json!({"tag":tag,"kind":"i64","value":value.to_string()}))
	});
	value
}
fn context(fresh: bool) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	for name in ["c13", "c15b"] {
		let mut module = crate::Module::with_item([name]).unwrap();
		if fresh {
			module.fresh_host_function("tap", tap).unwrap();
		} else {
			module.function("tap", tap).build().unwrap();
		}
		context.install(module).unwrap();
	}
	context
}
fn stock_error(error: &stock::VmError, sources: &Sources) -> Json {
	let identity = error.missing_instance_function().map(|m|json!({"variant":"MissingInstanceFunction","hash":m.hash.to_string(),"instance":m.instance.to_string(),"target":m.instance_name().to_string()}));
	let mut output = crate::termcolor::NoColor::new(std::vec::Vec::new());
	error.emit(&mut output, sources).unwrap();
	let sites = error.stacktrace().iter().map(|at| {
		let location = at.unit.debug_info().and_then(|debug|debug.instruction_at(at.ip));
		json!({"location":location.map(|v|json!({"source":v.source_id.into_index(),"start":v.span.range().start,"end":v.span.range().end})),"frames_debug":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	json!({"status":"error","identity":identity,"primary_display":error.to_string(),"kind_display":error.error().kind().to_string(),"kind_debug":format!("{:?}",error.error().kind()),"rich_emit":String::from_utf8(output.into_inner()).unwrap(),"sites":sites})
}
fn stock_observe(row: &Json) -> Json {
	let context = context(false);
	let mut sources = Sources::new();
	sources
		.insert(
			Source::new(
				row["source_name"].as_str().unwrap(),
				row["source"].as_str().unwrap(),
			)
			.unwrap(),
		)
		.unwrap();
	let mut options = crate::compile::Options::default();
	options.debug_info(row["context"].as_bool().unwrap());
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_options(&options)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	let outcome = match stock::budget::with(200_000, || vm.call(["main"], ())).call() {
		Ok(value) => json!({"status":"value","i64":crate::from_value::<i64>(value).unwrap()}),
		Err(error) => stock_error(&error, &sources),
	};
	let reuse =
		crate::from_value::<i64>(vm.call([row["reuse"].as_str().unwrap()], ()).unwrap()).unwrap();
	json!({"outcome":outcome,"reuse":reuse,"diagnostics":format!("{:?}",diagnostics.diagnostics())})
}
fn fresh_observe(row: &Json) -> Json {
	let context = context(true);
	let mut sources = Sources::new();
	sources
		.insert(
			Source::new(
				row["source_name"].as_str().unwrap(),
				row["source"].as_str().unwrap(),
			)
			.unwrap(),
		)
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let mut program = frontend::compile(&mut sources, &context, &mut diagnostics, 16).unwrap();
	if !row["context"].as_bool().unwrap() {
		program.sources.clear();
	}
	let mut store = Store::new(Limits {
		entries: 4096,
		bytes: 4 << 20,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let outcome = match executor.invoke(
		&program,
		&mut store,
		program.function(Hash::type_hash(["main"])).unwrap(),
		&[],
		200_000,
	) {
		Ok(root) => {
			let scalar = store.root_scalar(&root).unwrap();
			store.root_release(root).unwrap();
			json!({"status":"value","scalar":format!("{scalar:?}")})
		}
		Err(failure) => {
			let identity = failure.missing_instance_function().map(|m|json!({"variant":"MissingInstanceFunction","hash":m.hash.to_string(),"instance":m.instance.to_string(),"target":m.instance_name(&program).to_string()}));
			assert!(failure.cleanup_error.is_none());
			json!({"status":"error","identity":identity,"primary_display":failure.message(&program).to_string(),"error_message":failure.error.message(&program).to_string(),"kind_debug":format!("{:?}",failure.error.kind),"detail_debug":format!("{:?}",failure.error.detail),"site":failure.error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"call_chain":failure.call_chain.iter().map(|at|json!({"source":at.source,"start":at.start,"end":at.end})).collect::<Vec<_>>()})
		}
	};
	let row_idle = executor.is_idle();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program
				.function(Hash::type_hash([row["reuse"].as_str().unwrap()]))
				.unwrap(),
			&[],
			200_000,
		)
		.unwrap();
	let reuse = store.root_scalar(&root).unwrap();
	store.root_release(root).unwrap();
	json!({"outcome":outcome,"reuse":format!("{reuse:?}"),"diagnostics":format!("{:?}",diagnostics.diagnostics()),"audit":{"row_idle":row_idle,"row_live":row_live,"row_bytes":row_bytes,"idle":executor.is_idle(),"live":store.live_entries(),"bytes":store.payload_bytes()}})
}
#[test]
#[ignore = "D1 bounded source-bound external observation operator only"]
fn child() {
	let index: usize = std::env::var("D1_CASE").unwrap().parse().unwrap();
	let engine = std::env::var("D1_ENGINE").unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	TAPE.with(|tape| tape.borrow_mut().clear());
	let observed = match engine.as_str() {
		"stock" => stock_observe(row),
		"fresh" => fresh_observe(row),
		_ => panic!("D1 engine"),
	};
	let events = TAPE.with(|tape| tape.borrow().clone());
	let result = json!({"schema":1,"engine":engine,"index":index,"id":row["id"],"source_sha256":row["source_sha256"],"context":row["context"],"observed":observed,"events":events});
	use std::io::Write;
	let mut file = fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("D1_RESULT").unwrap())
		.unwrap();
	file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
		.unwrap();
}
