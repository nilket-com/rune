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
			super::d1e_diagnostics::stock(&e, &sources);
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
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::stock_display(&e),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

// C5 fresh observer reads source/input only; policy is consulted after observation.
fn observe_fresh(source: &str, mode: &str) -> (Json, Json) {
	use crate::experimental_core::{
		error::{Detail, ErrorKind},
		value::{Scalar, Type},
	};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c5.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let compiled = frontend::compile(&mut sources, &context, &mut diagnostics, 9);
	let mut store = Store::new(Limits {
		entries: 4096,
		bytes: 4 << 20,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	let mut input_bits = None;
	let mut no_cleanup_error = true;
	let observation = match compiled {
		Err(e) => {
			json!({"status":fresh_compile_status(e.refusal),"error":format!("{e:?}"),"refusal":format!("{:?}",e.refusal),"diagnostics":format!("{:?}",diagnostics.diagnostics()),"location":e.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"operation":e.operation})
		}
		Ok(program) => {
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			let input = mode.strip_prefix("c5-f64-").map(|bits| {
				let bits = u64::from_str_radix(bits, 16).unwrap();
				let root = store.root_f64(f64::from_bits(bits));
				let Some(Scalar::F64(value)) = store.root_scalar(&root).unwrap() else {
					panic!("F64 root");
				};
				assert_eq!(value.to_bits(), bits);
				input_bits = Some(format!("{bits:016x}"));
				root
			});
			let args = input.iter().collect::<Vec<_>>();
			if input.is_none() {
				assert_eq!(mode, "c5-main");
			}
			let result = executor.invoke(&program, &mut store, main, &args, FUEL);
			let observation = match result {
				Ok(root) => {
					let observation = match store.root_tag(&root).unwrap() {
						Type::Unit => json!({"status":"value","type":"Unit"}),
						Type::String => {
							let text = store.root_text(&root).unwrap();
							json!({"status":"value","type":"String","bytes_hex":hex(text.as_bytes()),"bytes":text.len()})
						}
						other => json!({"status":"unmapped-value","raw":format!("{other:?}")}),
					};
					store.root_release(root).unwrap();
					observation
				}
				Err(failure) => {
					super::d1e_diagnostics::fresh(&failure, &program, &sources);
					no_cleanup_error = failure.cleanup_error.is_none();
					// Closed map: these are the only three exact frozen runtime classes,
					// plus the explicitly retained resource class. Unknown remains raw.
					let class = match failure.error.kind {
						ErrorKind::Type => "Type",
						ErrorKind::DivisionByZero => "DivisionByZero",
						ErrorKind::AccessConflict => "AccessConflict",
						ErrorKind::FormatLimit => "FormatLimit",
						_ => "UNMAPPED",
					};
					let chain = match failure.error.detail {
						Detail::FormatArgumentMoved { arg, .. } => vec![
							json!({"kind":"fresh formatter argument","display":format!("Bad argument #{arg}")}),
						],
						Detail::HostMessage { arg: Some(arg), .. }
						| Detail::HostExpectedType { arg: Some(arg), .. }
						| Detail::HostExpectedNumber { arg: Some(arg), .. } => vec![
							json!({"kind":"fresh HostMessage argument","display":format!("Bad argument #{arg}")}),
						],
						_ => vec![],
					};
					// Fresh has source callsite locations, not stock VM stack windows.
					// Empty chain is represented as []; a nonempty chain is serialized
					// explicitly and will STOP against the frozen empty frames.
					let frames = if failure.call_chain.is_empty() {
						"[]".into()
					} else {
						serde_json::to_string(
							&failure
								.call_chain
								.iter()
								.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
								.collect::<Vec<_>>(),
						)
						.unwrap()
					};
					let sites = failure
						.error
						.location
						.iter()
						.chain(failure.call_chain.iter())
						.map(
							|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}),
						)
						.collect::<Vec<_>>();
					json!({"status":"error","class":class,"kind":format!("{:?}",failure.error.kind),"display":failure.message(&program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
				}
			};
			if let Some(root) = input {
				store.root_release(root).unwrap();
			}
			observation
		}
	};
	let clean = store.live_entries() == 0 && store.payload_bytes() == 0 && executor.is_idle();
	assert!(clean, "C5 fresh balance");
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main() { 7 }").unwrap())
		.unwrap();
	let reuse = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 10).unwrap();
	let result = executor
		.invoke(
			&reuse,
			&mut store,
			reuse.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			FUEL,
		)
		.unwrap();
	assert_eq!(store.root_scalar(&result).unwrap(), Some(Scalar::I64(7)));
	store.root_release(result).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
	assert!(executor.is_idle());
	(
		observation,
		json!({"no_cleanup_error":no_cleanup_error,"clean_after_row":clean,"reuse":true,"live_entries":0,"payload_bytes":0,"idle":true,"input_f64_bits":input_bits}),
	)
}

#[test]
#[ignore = "C5 stock oracle child; external bounded controller only"]
fn child() {
	let index: usize = std::env::var("C5_CASE").unwrap().parse().unwrap();
	assert_eq!(CASES.len(), COUNT);
	let (label, source, mode, policy) = CASES[index];
	let engine = std::env::var("C5_ENGINE").unwrap_or_else(|_| "stock".into());
	let (observation, audit) = match engine.as_str() {
		"stock" => (observe_stock(source, mode), None),
		"fresh" => {
			let (observation, audit) = observe_fresh(source, mode);
			(observation, Some(audit))
		}
		_ => panic!("unknown C5 engine"),
	};
	let output = std::env::var_os("C5_RESULT").unwrap();
	let mut envelope = json!({"schema":1,"index":index,"label":label,"mode":mode,"policy":policy,"source_hex":hex(source.as_bytes()),"fuel":FUEL,"outcome":observation});
	if let Some(audit) = audit {
		envelope["fresh_audit"] = audit;
	}
	// Persist the complete raw record before the parent classifies anything.
	fs::write(output, serde_json::to_vec_pretty(&envelope).unwrap()).unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("format_specs/contract.rs");

fn fresh_compile_status(refusal: Option<crate::experimental_core::error::Refusal>) -> &'static str {
	if refusal == Some(crate::experimental_core::error::Refusal::FrontendDiagnostic) {
		"compile-error"
	} else {
		"refused"
	}
}

#[test]
fn compile_status_uses_typed_refusal_even_with_a_diagnostic_wrapper() {
	use crate::experimental_core::error::Refusal;
	let wrapped_format_spec =
		json!({"refusal":"Some(FormatSpec)","diagnostics":"Fatal(FormatSpec)"});
	assert!(!wrapped_format_spec["diagnostics"]
		.as_str()
		.unwrap()
		.is_empty());
	assert_eq!(fresh_compile_status(Some(Refusal::FormatSpec)), "refused");
	assert_eq!(
		fresh_compile_status(Some(Refusal::FrontendDiagnostic)),
		"compile-error"
	);
}
