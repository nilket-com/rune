//! C11 live stock oracle and independent new observer.
use super::*;
use serde_json::{json, Value as Json};
include!("callback_sort/cases.rs");
include!("callback_sort/runtime_graph.rs");
include!("callback_sort/fixture.rs");
const C11_STALE_REFUSAL_ADMISSIONS: &str =
	include_str!("callback_sort/stale_refusal_admissions.json");

fn typed_kind(kind: &stock::VmErrorKind) -> Json {
	use stock::VmErrorKind as K;
	match kind {
		K::BadArgument { arg } => json!({"tag":"BadArgument","arg":arg}),
		K::BadArgumentCount { actual, expected } => {
			json!({"tag":"BadArgumentCount","actual":actual,"expected":expected})
		}
		K::MaxExecutionDepth { max } => json!({"tag":"MaxExecutionDepth","max":max}),
		K::ExpectedType { expected, actual } => {
			json!({"tag":"ExpectedType","expected":expected.to_string(),"actual":actual.to_string(),"expected_debug":format!("{expected:?}"),"actual_debug":format!("{actual:?}")})
		}
		_ => json!({"raw_kind":format!("{kind:?}")}),
	}
}
fn error_projection(error: &stock::VmError) -> Json {
	let kind = error.error().kind();
	let class = match kind {
		stock::VmErrorKind::MaxExecutionDepth { .. } => "MaxExecutionDepth".to_owned(),
		stock::VmErrorKind::Panic { .. } => "Panic".to_owned(),
		_ => runtime_error_class(kind),
	};
	let chain=error.chain().iter().map(|e|json!({"kind":format!("{:?}",e.kind()),"typed":typed_kind(e.kind()),"display":e.to_string()})).collect::<Vec<_>>();
	let sites=error.stacktrace().iter().map(|at|{
		let inst=at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();let range=inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"typed":typed_kind(kind),"display":super::d1c_diagnostics::runtime_error_display(error),"chain":chain,"sites":sites,"raw":format!("{error:?}")})
}
fn legacy_observe(source: &str, fixture_name: &str, mode: &str, fuel: usize) -> Json {
	reset_tape();
	let legacy_context = legacy_context();
	let (v, state) = input_fixture(fixture_name);
	let before = roots_graph(&v, &state);
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c11.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&legacy_context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			let post = roots_graph(&v, &state);
			let cleanup = cleanup_fixture(fixture_name, &v);
			return json!({"outcome":{"status":"compile-error","error":format!("{e:?}")},"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"invoked":false,"before":before,"post":post,"events":tape(),"cleanup":cleanup,"reuse":null});
		}
	};
	let call_sites=vm.unit().iter_instructions().filter_map(|(ip,inst)|{
		if !matches!(inst.kind,stock::inst::Kind::Call{..}|stock::inst::Kind::CallAssociated{..}|stock::inst::Kind::CallFn{..}) {return None;}
		let at=vm.unit().debug_info().unwrap().instruction_at(ip).unwrap();let range=at.span.range();
		Some(json!({"ip":ip,"instruction":format!("{inst:?}"),"source":at.source_id.into_index(),"start":range.start,"end":range.end,"span_hex":hex(source[range].as_bytes())}))
	}).collect::<Vec<_>>();
	let result = crate::runtime::budget::with(fuel, || match mode {
		"args" => vm.call(["main"], (v.clone(), state.clone())),
		"c9-noargs" => vm.call(["main"], ()),
		_ => panic!("unlisted oracle mode"),
	})
	.call();
	let outcome = match result {
		Ok(value) => json!({"status":"value","graph":runtime_graph(&value)}),
		Err(e) => {
			super::d1e_diagnostics::stock(&e, &sources);
			error_projection(&e)
		}
	};
	// Observe the held receiver and its aliases before any test-fixture cleanup.
	let post = roots_graph(&v, &state);
	let events = tape();
	let cleanup = cleanup_fixture(fixture_name, &v);
	let reuse_name = if mode == "c9-noargs" {
		"c9_reuse"
	} else {
		"c11_reuse"
	};
	let reuse = match crate::runtime::budget::with(FUEL, || vm.call([reuse_name], ())).call() {
		Ok(value) => json!({"status":"value","graph":runtime_graph(&value)}),
		Err(e) => error_projection(&e),
	};
	json!({"outcome":outcome,"invoked":true,"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"before":before,"post":post,"events":events,"events_after_reuse":tape(),"call_sites":call_sites,"cleanup":cleanup,"reuse":reuse})
}
#[test]
#[ignore = "C11 stock oracle child; bounded external controller only"]
fn child() {
	assert_eq!(CASES.len(), COUNT);
	let index: usize = std::env::var("C11_CASE").unwrap().parse().unwrap();
	let (label, source, input_fixture, policy, mode, fuel, cancel) = CASES[index];
	let engine = std::env::var("C11_ENGINE").unwrap();
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let observation = if engine == "stock" {
		legacy_observe(source, input_fixture, mode, fuel)
	} else {
		observe(source, input_fixture, mode, fuel, cancel)
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":label,"source_hex":hex(source.as_bytes()),"fixture":input_fixture,"policy":policy,"mode":mode,"fuel":fuel,"fresh_cancel_at_charge_proposal":cancel,"stock_cancel_armed":false,"observation":observation});
	fs::write(
		std::env::var_os("C11_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
include!("callback_sort/controls.rs");

include!("callback_sort/projection.rs");
include!("callback_sort/frozen_controls.rs");
include!("callback_sort/check_contract.rs");

include!("callback_sort/graph.rs");
include!("callback_sort/error.rs");
include!("callback_sort/run.rs");
include!("callback_sort/contract.rs");

include!("callback_sort/retained_enforce.rs");

include!("callback_sort/s7a_runtime_progression.rs");
