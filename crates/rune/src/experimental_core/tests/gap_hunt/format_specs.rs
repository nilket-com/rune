//! C5 test-only stock oracle. No production admission changes.
use super::*;
use serde_json::{json, Value as Json};
include!("format_specs/cases.rs");

fn observe_stock(source: &str, mode: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c5.rn", source).unwrap())
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
	let result = crate::runtime::budget::with(FUEL, || {
		if let Some(bits) = mode.strip_prefix("c5-f64-") {
			let input = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
			vm.call(["main"], (input,))
		} else {
			assert_eq!(mode, "c5-main");
			vm.call(["main"], ())
		}
	})
	.call();
	match result {
		Ok(value) => {
			if value.into_unit().is_ok() {
				return json!({"status":"value","type":"Unit"});
			}
			// Observe only explicit Unit or String. No Debug observer can hide output.
			let string = value
				.borrow_string_ref()
				.expect("C5 subject did not return String");
			json!({"status":"value","type":"String","bytes_hex":hex(string.as_bytes()),"bytes":string.len()})
		}
		Err(e) => {
			// Capture metadata directly inside the crate, no parsing Display/Debug.
			let kind = e.error().kind();
			let class = stock_class(kind);
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
#[ignore = "C5 stock oracle child; external bounded controller only"]
fn child() {
	let index: usize = std::env::var("C5_CASE").unwrap().parse().unwrap();
	assert_eq!(CASES.len(), COUNT);
	let (label, source, mode, policy) = CASES[index];
	let observation = observe_stock(source, mode);
	let output = std::env::var_os("C5_RESULT").unwrap();
	fs::write(output, serde_json::to_vec_pretty(&json!({"schema":1,"index":index,"label":label,"mode":mode,"policy":policy,"source_hex":hex(source.as_bytes()),"fuel":FUEL,"outcome":observation})).unwrap()).unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("format_specs/contract.rs");
