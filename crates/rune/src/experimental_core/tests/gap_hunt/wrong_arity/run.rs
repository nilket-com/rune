// Independent new observations. This module reads no frozen outcome or policy.
use super::iter_adapters::{failure, graph};

/// A new compile outcome as the contract records it.
fn refusal(error: &crate::experimental_core::error::Error) -> Json {
	json!({"status":"refused","kind":format!("{:?}",error.kind),"refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
}

#[test]
#[ignore = "S4 execution observation; bounded external operator only"]
fn child() {
	let index: usize = std::env::var("S4_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	TAPE.with(|t| t.borrow_mut().clear());
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["s4"]).unwrap();
	module.host_function("tap", tap).unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("s4.rn", program(row)).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut sources, &context, &mut diagnostics, 16);
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
	let mut cleanup_error = false;
	let outcome = match &built {
		Err(error) => refusal(error),
		Ok(program) => {
			let args = row["args"]
				.as_array()
				.unwrap()
				.iter()
				.map(|a| store.root_i64(a.as_i64().unwrap()))
				.collect::<Vec<_>>();
			let args = args.iter().collect::<Vec<_>>();
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			let fuel = row["fuel"].as_u64().unwrap() as usize;
			match executor.invoke(program, &mut store, main, &args, fuel) {
				Ok(root) => {
					let value = store.import(&root).unwrap();
					let graph = graph(&mut store, value);
					store.root_release(root).unwrap();
					json!({"status":"value","graph":graph})
				}
				Err(failure) => {
					cleanup_error = failure.cleanup_error.is_some();
					self::failure(&failure, program, &sources)
				}
			}
		}
	};
	let row_idle = executor.is_idle();
	let row_live_entries = store.live_entries();
	let row_payload_bytes = store.payload_bytes();
	let fallback = if built.is_err() {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory("pub fn s4_reuse(){7}").unwrap())
			.unwrap();
		Some(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 17).unwrap())
	} else {
		None
	};
	let program = built.as_ref().ok().or(fallback.as_ref()).unwrap();
	let reuse = program.function(Hash::type_hash(["s4_reuse"])).unwrap();
	let root = executor
		.invoke(program, &mut store, reuse, &[], 200000)
		.unwrap();
	let value = store.import(&root).unwrap();
	let reuse_graph = graph(&mut store, value);
	store.root_release(root).unwrap();
	let reuse = if row["reuse_enabled"] == true {
		json!({"status":"value","graph":reuse_graph})
	} else {
		Json::Null
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let logical = std::env::var_os("D1E_TRACE_OUT")
		.and_then(|p| fs::read(p).ok())
		.map(|b| serde_json::from_slice::<Json>(&b).unwrap())
		.unwrap_or(Json::Null);
	let result = json!({"schema":1,"engine":"fresh","index":index,"label":row["label"],"source":row["source"],"origin":row["origin"],"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"logical":logical,"observation":{"outcome":outcome,"events":events,"reuse":reuse},"fresh_audit":{"invoked":built.is_ok(),"cleanup_error":cleanup_error,"row_idle":row_idle,"row_live_entries":row_live_entries,"row_payload_bytes":row_payload_bytes,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"reuse_used_fallback":fallback.is_some()}});
	fs::write(
		std::env::var_os("S4_RESULT").unwrap(),
		serde_json::to_vec_pretty(&result).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
