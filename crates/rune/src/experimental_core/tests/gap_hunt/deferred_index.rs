//! C7 test-only stock oracle. No production admission changes.
use super::*;
use serde_json::{json, Value as Json};
include!("deferred_index/cases.rs");

fn observe_stock(source: &str, mode: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c7.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			return json!({"status":"compile-error","error":format!("{e:?}"),"diagnostics":format!("{:?}",diagnostics.diagnostics())})
		}
	};
	assert!(matches!(mode, "c7-main" | "c7-small-store"));
	let result = crate::runtime::budget::with(FUEL, || vm.call(["main"], ())).call();
	match result {
		Ok(value) => {
			if value.into_unit().is_ok() {
				return json!({"status":"value","type":"Unit"});
			}
			use crate::runtime::Inline;
			match value.as_inline() {
				Some(Inline::Float(n)) => {
					json!({"status":"value","type":"F64","bits":format!("{:016x}",n.to_bits())})
				}
				Some(Inline::Signed(n)) => {
					json!({"status":"value","type":"I64","value":n.to_string()})
				}
				Some(Inline::Unsigned(n)) => {
					json!({"status":"value","type":"U64","value":n.to_string()})
				}
				Some(Inline::Bool(n)) => json!({"status":"value","type":"Bool","value":n}),
				_ => panic!("C7 subject returned non-scalar"),
			}
		}
		Err(e) => {
			// Capture metadata directly inside the crate, no parsing Display/Debug.
			let kind = e.error().kind();
			let class = match kind {
				crate::runtime::VmErrorKind::Panic { .. } => "Panic".into(),
				_ => stock_class(kind),
			};
			let chain = e
				.chain()
				.iter()
				.map(|at| json!({"kind":format!("{:?}",at.kind()),"display":at.to_string()}))
				.collect::<Vec<_>>();
			let sites = e.stacktrace().iter().map(|at| {
				let inst = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
				let range = inst.span.range();
				json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
			}).collect::<Vec<_>>();
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::stock_display(&e),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

#[test]
#[ignore = "C7 oracle child; external bounded controller only"]
fn child() {
	let index: usize = std::env::var("C7_CASE").unwrap().parse().unwrap();
	assert_eq!(CASES.len(), COUNT);
	let (label, source, mode, policy) = CASES[index];
	let engine = std::env::var("C7_ENGINE").unwrap();
	let (observation, audit) = match engine.as_str() {
		"stock" => (observe_stock(source, mode), None),
		"fresh" => {
			let (o, a) = observe_fresh(source, mode);
			(o, Some(a))
		}
		_ => panic!("unknown C7 engine"),
	};
	let mut envelope = json!({"schema":1,"engine":engine,"index":index,"label":label,"mode":mode,"policy":policy,"source_hex":hex(source.as_bytes()),"fuel":FUEL,"outcome":observation});
	if let Some(audit) = audit {
		envelope["fresh_audit"] = audit;
	}
	fs::write(
		std::env::var_os("C7_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("deferred_index/observe_fresh.rs");
include!("deferred_index/contract.rs");
