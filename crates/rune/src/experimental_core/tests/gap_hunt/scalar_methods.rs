//! C6 test-only stock oracle. No production admission changes.
use super::*;
use serde_json::{json, Value as Json};
include!("scalar_methods/cases.rs");

fn legacy_observe(source: &str, mode: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c6.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			return json!({"status":"compile-error","error":format!("{e:?}"),"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics)})
		}
	};
	let result = crate::runtime::budget::with(FUEL, || {
		if let Some(pair) = mode.strip_prefix("c6-f64x2-") {
			let (x, y) = pair.split_once('-').unwrap();
			vm.call(
				["main"],
				(
					f64::from_bits(u64::from_str_radix(x, 16).unwrap()),
					f64::from_bits(u64::from_str_radix(y, 16).unwrap()),
				),
			)
		} else if let Some(bits) = mode.strip_prefix("c6-f64-") {
			vm.call(
				["main"],
				(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()),),
			)
		} else if let Some(n) = mode.strip_prefix("c6-i64-") {
			vm.call(["main"], (n.parse::<i64>().unwrap(),))
		} else {
			assert_eq!(mode, "c6-main");
			vm.call(["main"], ())
		}
	})
	.call();
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
				_ => panic!("C6 subject returned non-scalar"),
			}
		}
		Err(e) => {
			// Capture metadata directly inside the crate, no parsing Display/Debug.
			let kind = e.error().kind();
			let class = runtime_error_class(kind);
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
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":kind.to_string(),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

#[test]
#[ignore = "C6 stock oracle child; external bounded controller only"]
fn child() {
	let index: usize = std::env::var("C6_CASE").unwrap().parse().unwrap();
	assert_eq!(CASES.len(), COUNT);
	let (label, source, mode, policy) = CASES[index];
	let engine = std::env::var("C6_ENGINE").unwrap_or_else(|_| "stock".into());
	let (observation, audit) = match engine.as_str() {
		"stock" => (legacy_observe(source, mode), None),
		"fresh" => {
			let (o, a) = observe(source, mode);
			(o, Some(a))
		}
		_ => panic!("unknown C6 engine"),
	};
	let output = std::env::var_os("C6_RESULT").unwrap();
	let mut envelope = json!({"schema":1,"engine":engine,"index":index,"label":label,"mode":mode,"policy":policy,"source_hex":hex(source.as_bytes()),"fuel":FUEL,"outcome":observation});
	if let Some(audit) = audit {
		envelope["fresh_audit"] = audit;
	}
	// Preserve complete raw result before any policy is consulted.
	fs::write(output, serde_json::to_vec_pretty(&envelope).unwrap()).unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("scalar_methods/contract.rs");

include!("scalar_methods/observe.rs");
