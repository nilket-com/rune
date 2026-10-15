//! C9 source-only stock oracle. Production admission is unchanged.
use super::*;
use serde_json::{json, Value as Json};
include!("cmp_ordering/cases.rs");
include!("cmp_ordering/fixture.rs");

fn value_projection(value: &crate::runtime::Value, depth: usize) -> Json {
	use crate::runtime::Inline;
	assert!(depth <= 16, "C9 observer depth limit");
	let type_hash = value.type_hash().to_string();
	let payload = if let Some(inline) = value.as_inline() {
		match inline {
			Inline::Unit => json!({"type":"Unit"}),
			Inline::Bool(v) => json!({"type":"Bool","value":v}),
			Inline::Signed(v) => json!({"type":"I64","value":v.to_string()}),
			Inline::Unsigned(v) => json!({"type":"U64","value":v.to_string()}),
			Inline::Float(v) => json!({"type":"F64","bits":format!("{:016x}",v.to_bits())}),
			Inline::Char(v) => json!({"type":"Char","value":*v as u32}),
			Inline::Ordering(v) => {
				let variant = match v {
					core::cmp::Ordering::Less => "Less",
					core::cmp::Ordering::Equal => "Equal",
					core::cmp::Ordering::Greater => "Greater",
				};
				json!({"type":"Ordering","variant":variant})
			}
			_ => panic!("unmapped C9 inline result: {inline:?}"),
		}
	} else if let Ok(text) = value.borrow_string_ref() {
		json!({"type":"String","hex":hex(text.as_bytes())})
	} else if let Ok(tuple) = value.borrow_tuple_ref() {
		json!({"type":"Tuple","values":tuple.iter().map(|v|value_projection(v,depth+1)).collect::<Vec<_>>()})
	} else if let Ok(values) = value.borrow_ref::<crate::runtime::Vec>() {
		json!({"type":"Vec","values":values.iter().map(|v|value_projection(v,depth+1)).collect::<Vec<_>>()})
	} else if let Ok(option) = value.borrow_ref::<Option<crate::runtime::Value>>() {
		json!({"type":"Option","value":(*option).as_ref().map(|v|value_projection(v,depth+1))})
	} else if let Ok(result) =
		value.borrow_ref::<Result<crate::runtime::Value, crate::runtime::Value>>()
	{
		match &*result {
			Ok(v) => json!({"type":"Result","variant":"Ok","value":value_projection(v,depth+1)}),
			Err(v) => json!({"type":"Result","variant":"Err","value":value_projection(v,depth+1)}),
		}
	} else {
		panic!("unmapped C9 returned type: {}", value.type_info());
	};
	json!({"type_hash":type_hash,"payload":payload})
}

fn typed_kind(kind: &crate::runtime::VmErrorKind) -> Json {
	use crate::runtime::VmErrorKind as K;
	match kind {
		K::IllegalFloatComparison { lhs, rhs } => {
			json!({"tag":"IllegalFloatComparison","lhs_bits":format!("{:016x}",lhs.to_bits()),"rhs_bits":format!("{:016x}",rhs.to_bits())})
		}
		K::BadArgument { arg } => json!({"tag":"BadArgument","arg":arg}),
		K::BadArgumentCount { actual, expected } => {
			json!({"tag":"BadArgumentCount","actual":actual,"expected":expected})
		}
		K::MissingInstanceFunction { hash, instance } => {
			json!({"tag":"MissingInstanceFunction","hash":hash.to_string(),"instance":instance.to_string()})
		}
		_ => json!({"raw_kind":format!("{kind:?}")}),
	}
}

fn error_projection(error: &crate::runtime::VmError) -> Json {
	let kind = error.error().kind();
	let class = if matches!(
		kind,
		crate::runtime::VmErrorKind::IllegalFloatComparison { .. }
	) {
		"IllegalFloatComparison".into()
	} else {
		stock_class(kind)
	};
	let chain = error.chain().iter().map(|at|json!({"kind":format!("{:?}",at.kind()),"typed":typed_kind(at.kind()),"display":at.to_string()})).collect::<Vec<_>>();
	let sites = error.stacktrace().iter().map(|at| {
		let inst = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
		let span = inst.span.range();
		json!({"source":inst.source_id.into_index(),"start":span.start,"end":span.end,"frames":format!("{:?}",at.frames)})
	}).collect::<Vec<_>>();
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"typed":typed_kind(kind),"display":kind.to_string(),"chain":chain,"sites":sites,"raw":format!("{error:?}")})
}

fn observe_stock(source: &str, mode: &str) -> Json {
	let (context, calls) = c9_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c9.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			return json!({"outcome":{"status":"compile-error","error":format!("{e:?}"),"diagnostics":format!("{:?}",diagnostics.diagnostics())},"invoked":false,"host_calls":calls.load(std::sync::atomic::Ordering::SeqCst),"reuse":null});
		}
	};
	// Bind future Refused-runtime spans to actual stock compiler debug metadata.
	let cmp_sites = vm.unit().iter_instructions().filter_map(|(ip,inst)| {
		let crate::runtime::inst::Kind::CallAssociated { hash, .. } = &inst.kind else { return None; };
		if *hash != Hash::ident("cmp") { return None; }
		let at = vm.unit().debug_info().unwrap().instruction_at(ip).unwrap();
		let range = at.span.range();
		Some(json!({"ip":ip,"instruction":format!("{inst:?}"),"source":at.source_id.into_index(),"start":range.start,"end":range.end,"span_hex":hex(source[range].as_bytes())}))
	}).collect::<Vec<_>>();
	let result = crate::runtime::budget::with(FUEL, || {
		if let Some(bits) = mode.strip_prefix("c9-f64x2-") {
			let (lhs, rhs) = bits.split_once('-').unwrap();
			vm.call(
				["main"],
				(
					f64::from_bits(u64::from_str_radix(lhs, 16).unwrap()),
					f64::from_bits(u64::from_str_radix(rhs, 16).unwrap()),
				),
			)
		} else {
			assert!(matches!(mode, "c9-main" | "c9-host"));
			vm.call(["main"], ())
		}
	})
	.call();
	let outcome = match result {
		Ok(value) => json!({"status":"value","value":value_projection(&value,0)}),
		Err(error) => error_projection(&error),
	};
	// Observe reuse as raw stock data. Do not assume an outcome before collection.
	let reuse = match crate::runtime::budget::with(FUEL, || vm.call(["c9_reuse"], ())).call() {
		Ok(value) => json!({"status":"value","value":value_projection(&value,0)}),
		Err(error) => error_projection(&error),
	};
	json!({"outcome":outcome,"invoked":true,"cmp_sites":cmp_sites,"host_calls":calls.load(std::sync::atomic::Ordering::SeqCst),"reuse":reuse,"diagnostics":format!("{:?}",diagnostics.diagnostics())})
}

#[test]
#[ignore = "C9 stock oracle child; externally bounded controller only"]
fn child() {
	assert_eq!(CASES.len(), COUNT);
	let index: usize = std::env::var("C9_CASE").unwrap().parse().unwrap();
	let (label, source, mode, policy) = CASES[index];
	let engine = std::env::var("C9_ENGINE").unwrap();
	let observation = match engine.as_str() {
		"stock" => observe_stock(source, mode),
		"fresh" => observe_fresh(source, mode),
		_ => panic!("unknown C9 engine"),
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":label,"source_hex":hex(source.as_bytes()),"mode":mode,"policy":policy,"fuel":FUEL,"observation":observation});
	fs::write(
		std::env::var_os("C9_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("cmp_ordering/p1.rs");
include!("cmp_ordering/pins.rs");
include!("cmp_ordering/p1_controls.rs");
include!("cmp_ordering/observe_fresh.rs");
include!("cmp_ordering/contract.rs");
