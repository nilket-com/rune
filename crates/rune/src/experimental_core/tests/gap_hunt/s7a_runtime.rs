//! S7a stock observations, frozen before changing runtime error handling.
use super::*;
use serde_json::{json, Value as Json};

fn rows() -> Vec<Json> {
	let document: Json = serde_json::from_str(include_str!("s7a_runtime.json")).unwrap();
	document["rows"].as_array().unwrap().clone()
}

fn stock_typed(kind: &crate::runtime::VmErrorKind) -> Json {
	use crate::runtime::VmErrorKind as K;
	match kind {
		K::MissingProtocolFunction { protocol, instance } => {
			json!({"tag":"MissingProtocolFunction","protocol":protocol.hash.to_string(),"instance":instance.type_hash().to_string()})
		}
		K::MissingInstanceFunction { hash, instance } => {
			json!({"tag":"MissingInstanceFunction","hash":hash.to_string(),"instance":instance.type_hash().to_string()})
		}
		K::ExpectedType { expected, actual } => {
			json!({"tag":"ExpectedType","expected":expected.type_hash().to_string(),"actual":actual.type_hash().to_string()})
		}
		K::AccessError { error } => json!({"tag":"AccessError","access":format!("{error:?}")}),
		_ => json!({"raw_kind":format!("{kind:?}")}),
	}
}

fn fresh_typed(error: &crate::experimental_core::error::Error) -> Json {
	use crate::experimental_core::error::{Detail, ErrorKind, MovedAccess};
	match error.detail {
		Detail::NoDisplay { found } if error.kind == ErrorKind::Type => {
			json!({"tag":"MissingProtocolFunction","protocol":crate::runtime::Protocol::DISPLAY_FMT.hash.to_string(),"instance":found.builtin_hash().unwrap().to_string()})
		}
		Detail::MissingInstanceFunction {
			target,
			hash,
			instance,
		} if error.kind == ErrorKind::Type => {
			assert_eq!(target.builtin_hash(), Some(instance));
			json!({"tag":"MissingInstanceFunction","hash":hash.to_string(),"instance":instance.to_string()})
		}
		Detail::Moved {
			access: MovedAccess::Read,
		}
		| Detail::FormatArgumentMoved {
			access: MovedAccess::Read,
			..
		} => {
			json!({"tag":"AccessError","access":format!("{:?}",crate::runtime::AccessError::fresh_moved_read())})
		}
		_ => {
			json!({"unmapped_kind":format!("{:?}",error.kind),"detail":format!("{:?}",error.detail)})
		}
	}
}

fn fresh_compile_status(refusal: Option<crate::experimental_core::error::Refusal>) -> &'static str {
	if refusal.is_some() {
		"refused"
	} else {
		"compile-error"
	}
}

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
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":e.to_string(),"typed":stock_typed(kind),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

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
					json!({"status":"error","class":class,"kind":format!("{:?}",failure.error.kind),"display":failure.message(&program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}"),"typed":fresh_typed(&failure.error)})
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
#[ignore = "S7a bounded stock collector; S7A_RESULT must be a new path"]
fn s7a_collect_stock() {
	use std::io::Write;
	let observed: Vec<_> = rows().into_iter().map(|row|json!({"name":row["name"],"source_sha256":row["source_sha256"],"stock":observe_stock(row["source"].as_str().unwrap(),"c5-main"),"before_fresh":observe_fresh(row["source"].as_str().unwrap(),"c5-main")})).collect();
	let mut file = fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("S7A_RESULT").unwrap())
		.unwrap();
	file.write_all(&serde_json::to_vec_pretty(&observed).unwrap())
		.unwrap();
}

#[test]
fn s7a_sources_are_pinned() {
	use sha2::{Digest, Sha256};
	for row in rows() {
		let hash = Sha256::digest(row["source"].as_str().unwrap().as_bytes());
		assert_eq!(hex(&hash), row["source_sha256"].as_str().unwrap());
	}
}

// Only the named opaque Debug divergence permits a variable address. Every byte
// outside those address fields, including nested generic type spelling, is exact.
// Frozen stock text and raw observations remain intact; this is not an Exact row.
fn opaque_text_parts(text: &str) -> Vec<&str> {
	let mut rest = text;
	let mut parts = Vec::new();
	while let Some(at) = rest.find(" object at 0x") {
		let end = at + " object at 0x".len();
		parts.push(&rest[..end]);
		let address = &rest[end..];
		let digits = address.bytes().take_while(u8::is_ascii_hexdigit).count();
		assert!(digits > 0);
		assert_eq!(address.as_bytes()[digits], b'>');
		assert!(address[..digits].bytes().all(|b| !b.is_ascii_uppercase()));
		rest = &address[digits..];
	}
	assert!(!parts.is_empty());
	parts.push(rest);
	parts
}

fn decode_hex(text: &str) -> Vec<u8> {
	assert_eq!(text.len() % 2, 0);
	text.as_bytes()
		.chunks_exact(2)
		.map(|b| u8::from_str_radix(core::str::from_utf8(b).unwrap(), 16).unwrap())
		.collect()
}

fn assert_stock_observation(row: &Json, actual: &Json) {
	let expected = &row["stock"];
	if expected["status"] == "value" {
		assert_eq!(actual["status"], "value", "{}", row["name"]);
		assert_eq!(actual["type"], expected["type"]);
		let before =
			String::from_utf8(decode_hex(expected["bytes_hex"].as_str().unwrap())).unwrap();
		let after = String::from_utf8(decode_hex(actual["bytes_hex"].as_str().unwrap())).unwrap();
		if before.contains(" object at 0x") {
			assert!(
				row["policy"].as_str().unwrap().contains("opaque Debug")
					|| row["policy"] == "Known-gap moved iterator"
			);
			assert_eq!(
				opaque_text_parts(&after),
				opaque_text_parts(&before),
				"{}",
				row["name"]
			);
		} else {
			assert_eq!(actual, expected, "{}", row["name"]);
		}
	} else {
		for key in ["status", "class", "display", "typed", "chain", "sites"] {
			assert_eq!(actual[key], expected[key], "{} {key}", row["name"]);
		}
	}
}

#[test]
fn s7a_stock_contract() {
	for row in rows() {
		assert_stock_observation(
			&row,
			&observe_stock(row["source"].as_str().unwrap(), "c5-main"),
		);
	}
}
