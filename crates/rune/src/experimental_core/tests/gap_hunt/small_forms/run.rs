// Independent observations: no frozen data or policy is read in this file.
fn c12_fixture(store: &mut Store, fixture: &str) -> (Value, Value) {
	let state = store.list(Type::Vec, &[]).unwrap();
	let value = match fixture {
		"none" => Value::Unit,
		"empty" | "cycle" => store.list(Type::Vec, &[]).unwrap(),
		"one" => store.list(Type::Vec, &[Value::I64(2)]).unwrap(),
		"ints" => store
			.list(
				Type::Vec,
				&[Value::I64(1), Value::I64(2), Value::I64(3), Value::I64(4)],
			)
			.unwrap(),
		"nested" => {
			let a = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
			let b = store.list(Type::Vec, &[Value::I64(2)]).unwrap();
			let result = store.list(Type::Vec, &[a, b]).unwrap();
			store.release(a).unwrap();
			store.release(b).unwrap();
			result
		}
		_ => panic!("unreviewed fixture"),
	};
	if fixture == "cycle" {
		store.push(value, value).unwrap();
	}
	(value, state)
}
fn c12_sites(failure: &crate::experimental_core::execute::Failure) -> Json {
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
	json!(failure
		.error
		.location
		.iter()
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}))
		.collect::<Vec<_>>())
}
fn c12_error(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
) -> Json {
	let error = &failure.error;
	if error.kind == ErrorKind::FuelExhausted {
		return json!({"status":"error","class":"FuelExhausted","kind":"FuelExhausted","display":failure.message(program).to_string(),"sites":c12_sites(failure),"chain":[],"raw":format!("{failure:?}")});
	}
	if error.kind == ErrorKind::AccessConflict
		&& matches!(
			error.detail,
			Detail::WriteBorrowed | Detail::SharedArgument { .. }
		) {
		return json!({"status":"error","class":"AccessConflict","kind":format!("{:?}",error.detail),"display":failure.message(program).to_string(),"sites":c12_sites(failure),"chain":[],"raw":format!("{failure:?}")});
	}
	let Some(kind) = primary_kind(error, program) else {
		return json!({"status":"unmapped-error","actual_kind":format!("{:?}",error.kind),"actual_detail":format!("{:?}",error.detail),"raw":format!("{failure:?}")});
	};
	json!({"status":"error","class":runtime_error_class(&kind),"kind":format!("{kind:?}"),"display":failure.message(program).to_string(),"sites":c12_sites(failure),"chain":[],"raw":format!("{failure:?}")})
}
fn c12_observe(row: &Json) -> Json {
	C12_TAPE.with(|t| t.borrow_mut().clear());
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c12"]).unwrap();
	module.host_function("tap", tap).unwrap();
	module.host_function("tap_bool", tap_bool).unwrap();
	context.install(module).unwrap();
	let fixture = row["fixture"].as_str().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c12.rn", row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut sources, &context, &mut diagnostics, 12);
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
	let (v, state) = c12_fixture(&mut store, fixture);
	let before = json!({"receiver":graph(&mut store,v),"state":graph(&mut store,state)});
	let mut invoked = false;
	let mut cleanup_error = Json::Null;
	let outcome = match &built {
		Err(error) => {
			json!({"status":"refused","refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
		}
		Ok(program) => {
			let a = store.export(v);
			let b = store.export(state);
			let args = if fixture == "none" {
				vec![]
			} else {
				vec![&a, &b]
			};
			invoked = true;
			match executor.invoke(
				program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&args,
				row["fuel"].as_u64().unwrap() as usize,
			) {
				Ok(root) => {
					let value = store.import(&root).unwrap();
					let runtime_graph = graph(&mut store, value);
					store.root_release(root).unwrap();
					json!({"status":"value","graph":runtime_graph})
				}
				Err(failure) => {
					cleanup_error = json!(failure.cleanup_error.as_ref().map(|e| format!("{e:?}")));
					c12_error(&failure, program)
				}
			}
		}
	};
	let post = json!({"receiver":graph(&mut store,v),"state":graph(&mut store,state)});
	let events = C12_TAPE.with(|t| t.borrow().clone());
	let cleanup = if fixture == "cycle" {
		store.set(v, 0, Value::Unit).unwrap();
		json!({"cycle_cleared":true})
	} else {
		Json::Null
	};
	store.release(v).unwrap();
	store.release(state).unwrap();
	let row_idle = executor.is_idle();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let fallback = if built.is_err() {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory("pub fn c12_reuse(){7}").unwrap())
			.unwrap();
		Some(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 13).unwrap())
	} else {
		None
	};
	let program = built.as_ref().ok().or(fallback.as_ref()).unwrap();
	let reuse = match executor.invoke(
		program,
		&mut store,
		program.function(Hash::type_hash(["c12_reuse"])).unwrap(),
		&[],
		200_000,
	) {
		Ok(root) => {
			let value = store.import(&root).unwrap();
			let runtime_graph = graph(&mut store, value);
			store.root_release(root).unwrap();
			json!({"status":"value","graph":runtime_graph})
		}
		Err(failure) => c12_error(&failure, program),
	};
	json!({"invoked":invoked,"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics),"observation":{"outcome":outcome,"before":before,"post":post,"events":events,"cleanup":cleanup,"reuse":reuse},"fresh_audit":{"cleanup_error":cleanup_error,"row_idle":row_idle,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"reuse_used_fallback":fallback.is_some()}})
}
