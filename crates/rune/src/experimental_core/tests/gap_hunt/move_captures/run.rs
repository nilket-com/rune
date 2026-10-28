// Independent new observation. No frozen outcome or disposition is consulted.
use crate::experimental_core::error::ErrorKind;
use crate::experimental_core::{
	execute::Executor,
	frontend,
	heap::{Limits, Store},
};
include!("graph.rs");

fn context() -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c11b"]).unwrap();
	module.host_function("tap", tap).unwrap();
	module.host_function("tap_bool", tap_bool).unwrap();
	context.install(module).unwrap();
	context
}
fn child(index: usize, row: &Json) -> ! {
	let source = row["source"].as_str().unwrap();
	TAPE.with(|t| t.borrow_mut().clear());
	let context = context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c11b.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut sources, &context, &mut diagnostics, 11);
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
	let mut program = None;
	let mut cleanup_error = false;
	let mut invoked = false;
	let outcome = match built {
		Err(e) => {
			json!({"status":if e.refusal.is_some() && e.refusal!=Some(crate::experimental_core::error::Refusal::FrontendDiagnostic){"refused"}else{"compile-error"},"kind":format!("{:?}",e.kind),"refusal":e.refusal.map(|r|r.name()),"location":e.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{e:?}")})
		}
		Ok(p) => {
			invoked = true;
			let result = match executor.invoke(
				&p,
				&mut store,
				p.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				row["fuel"].as_u64().unwrap() as usize,
			) {
				Ok(root) => {
					let value = store.import(&root).unwrap();
					let runtime_graph = graph(&mut store, value);
					store.root_release(root).unwrap();
					json!({"status":"value","graph":runtime_graph})
				}
				Err(failure) => {
					super::d1e_diagnostics::execution(&failure, &p, &sources);
					cleanup_error = failure.cleanup_error.is_some();
					let class = match failure.error.kind {
						ErrorKind::Type => "Type",
						ErrorKind::ArgumentCount => "ArgumentCount",
						ErrorKind::AccessConflict => "AccessConflict",
						ErrorKind::Panic => "Panic",
						ErrorKind::FuelExhausted => "FuelExhausted",
						ErrorKind::DivisionByZero => "DivisionByZero",
						ErrorKind::IndexOutOfBounds => "IndexOutOfBounds",
						_ => "UNMAPPED",
					};
					let chain = failure
						.invocation_context()
						.map(|at| match at {
							crate::runtime::InvocationContext::BadArgument { arg } => {
								json!({"kind":"BadArgument","display":format!("Bad argument #{arg}")})
							}
						})
						.collect::<Vec<_>>();
					let frames = serde_json::to_string(
						&failure
							.call_chain
							.iter()
							.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
							.collect::<Vec<_>>(),
					)
					.unwrap();
					let sites = failure
						.error
						.location
						.iter()
						.map(
							|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}),
						)
						.collect::<Vec<_>>();
					json!({"status":"error","class":class,"kind":format!("{:?}",failure.error.kind),"display":failure.message(&p).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
				}
			};
			program = Some(p);
			result
		}
	};
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let row_idle = executor.is_idle();
	let logical = std::env::var_os("D1E_TRACE_OUT")
		.and_then(|p| fs::read(p).ok())
		.map(|b| serde_json::from_slice::<Json>(&b).unwrap())
		.unwrap_or(Json::Null);
	// Cycles remain explicitly charged until close, as in the existing C11b policy.
	let reopened = row_live != 0 || row_bytes != 0;
	if reopened {
		store.close().unwrap();
		store = Store::new(Limits {
			entries: 4096,
			bytes: 4 << 20,
		})
		.unwrap();
	}
	let mut reuse_store = store;
	let reuse = if let Some(p) = program.as_ref().filter(|_| row["reuse_enabled"] == true) {
		let root = executor
			.invoke(
				p,
				&mut reuse_store,
				p.function(Hash::type_hash(["c11b_reuse"])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		let value = reuse_store.import(&root).unwrap();
		let runtime_graph = graph(&mut reuse_store, value);
		reuse_store.root_release(root).unwrap();
		json!({"status":"value","graph":runtime_graph})
	} else {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory("pub fn main(){7}").unwrap())
			.unwrap();
		let p = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 12).unwrap();
		let root = executor
			.invoke(
				&p,
				&mut reuse_store,
				p.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				200000,
			)
			.unwrap();
		reuse_store.root_release(root).unwrap();
		Json::Null
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let mut result = json!({"schema":1,"engine":"fresh","index":index,"label":row["label"],"source":source,"origin":row["origin"],"logical":logical,"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"observation":{"outcome":outcome,"events":events,"reuse":reuse},"fresh_audit":{"invoked":invoked,"cleanup_error":cleanup_error,"row_idle":row_idle,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"reuse_same_store":!reopened,"reuse_idle":executor.is_idle(),"reuse_live_entries":reuse_store.live_entries(),"reuse_payload_bytes":reuse_store.payload_bytes()}});
	result["projected"] = projection(&result);
	fs::write(
		std::env::var_os("C11C_RESULT").unwrap(),
		serde_json::to_vec_pretty(&result).unwrap(),
	)
	.unwrap();
	use std::io::Write;
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0)
}
