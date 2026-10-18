// No expected row, frozen result, projection count or sidecar is an input here.
fn fresh_context() -> Context {
	let mut cx = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c11"]).unwrap();
	m.fresh_host_function("tick", tick).unwrap();
	m.fresh_host_function("mark", mark).unwrap();
	cx.install(m).unwrap();
	cx
}
fn fresh_fixture(store: &mut Store, name: &str) -> (FreshValue, FreshValue, FreshValue) {
	// Reuse ONLY the input factory. Import its actual graph, never a result.
	let (v, state) = fixture(name);
	let input = roots_graph(&v, &state);
	cleanup_fixture(name, &v);
	let nodes = input["nodes"].as_array().unwrap();
	let mut values = Vec::new();
	for node in nodes {
		let value = match node[0].as_str().unwrap() {
			"I64" => FreshValue::I64(node[2].as_str().unwrap().parse().unwrap()),
			"Vec" | "Tuple" => {
				let len = node[2].as_array().unwrap().len();
				let placeholders = vec![FreshValue::Unit; len];
				store
					.list(
						if node[0] == "Vec" {
							Type::Vec
						} else {
							Type::Tuple
						},
						&placeholders,
					)
					.unwrap()
			}
			other => panic!("unlisted input graph tag {other}"),
		};
		values.push(value);
	}
	for (index, node) in nodes.iter().enumerate() {
		if matches!(node[0].as_str(), Some("Vec" | "Tuple")) {
			for (at, child) in node[2].as_array().unwrap().iter().enumerate() {
				store
					.set(values[index], at, values[child.as_u64().unwrap() as usize])
					.unwrap();
			}
		}
	}
	let root = values[input["root"].as_u64().unwrap() as usize];
	store.retain(root).unwrap();
	for value in values {
		store.release(value).unwrap();
	}
	let v = store.get(root, 0).unwrap();
	let state = store.get(root, 1).unwrap();
	(root, v, state)
}
fn actual_sites(failure: &crate::experimental_core::execute::Failure) -> Json {
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
fn fresh_error(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
) -> Json {
	let error = &failure.error;
	let location = error
		.location
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}));
	if error.kind == ErrorKind::Unsupported
		&& error.operation == "Method"
		&& error.refusal.is_none()
	{
		return json!({"status":"runtime-unsupported","kind":"Unsupported","operation":error.operation,"refusal":null,"location":location,"raw":format!("{failure:?}")});
	}
	if matches!(
		error.kind,
		ErrorKind::FuelExhausted
			| ErrorKind::Cancelled
			| ErrorKind::FrameCapacity
			| ErrorKind::StoreCapacity
			| ErrorKind::Allocation
	) {
		let class = format!("{:?}", error.kind);
		return json!({"status":"error","class":class,"kind":class,"typed":{"raw_kind":class},"display":failure.message(program).to_string(),"chain":[],"sites":actual_sites(failure),"raw":format!("{failure:?}")});
	}
	let kind = if error.kind == ErrorKind::Panic && error.detail == Detail::PanicMessage {
		Some(stock::VmErrorKind::Panic {
			reason: stock::Panic::custom(
				failure
					.panic_message
					.as_deref()
					.expect("actual panic text")
					.to_owned(),
			),
		})
	} else {
		fresh_primary_kind(error, program)
	};
	let Some(kind) = kind else {
		return json!({"status":"unmapped-error","actual_kind":format!("{:?}",error.kind),"actual_detail":format!("{:?}",error.detail),"location":location,"raw":format!("{failure:?}")});
	};
	let class = if error.kind == ErrorKind::Panic {
		"Panic".into()
	} else {
		stock_class(&kind)
	};
	let chain = error
		.detail
		.numeric_argument()
		.map(|(_, arg)| {
			let k = stock::VmErrorKind::BadArgument { arg: arg.into() };
			vec![json!({"kind":format!("{k:?}"),"typed":typed_kind(&k),"display":k.to_string()})]
		})
		.unwrap_or_default();
	// This raw primary Kind is rendered from actual typed data. The complete
	// actual Failure (including source frames and cleanup) is retained in audit.
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"typed":typed_kind(&kind),"display":failure.message(program).to_string(),"chain":chain,"sites":actual_sites(failure),"raw":format!("{kind:?}")})
}
fn slot_ledger(program: &frontend::Program) -> Json {
	json!((0..program.code.functions())
		.map(|id| {
			let f = program.code.function(id as u32).unwrap();
			json!({"id":id,"slots":f.slots,"parameters":f.parameters})
		})
		.collect::<Vec<_>>())
}
fn observe_fresh(
	source: &str,
	fixture_name: &str,
	mode: &str,
	fuel: usize,
	cancel: Option<usize>,
) -> Json {
	reset_tape();
	let context = fresh_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c11.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let compiled = frontend::compile(&mut sources, &context, &mut diagnostics, 11);
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
	let (held, v, state) = fresh_fixture(&mut store, fixture_name);
	let before = fresh_graph(&mut store, held);
	let mut invoked = false;
	let mut cleanup_error = None::<String>;
	let mut actual_failure = Json::Null;
	let mut typed_failure = None::<String>;
	let mut ledger = Json::Null;
	let mut depth_metadata = Json::Null;
	let outcome = match compiled.as_ref() {
		Err(error) => {
			json!({"status":if error.refusal.is_some(){"refused"}else{"unmapped-compile-error"},"kind":format!("{:?}",error.kind),"operation":error.operation,"refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
		}
		Ok(program) => {
			ledger = slot_ledger(program);
			if fixture_name.starts_with("depth/") {
				depth_metadata = crate::experimental_core::code::c11_depth_metadata(program);
			}
			let a = store.export(v);
			let b = store.export(state); // borrowed Root stamps, no new owners
			let args = if mode == "args" {
				vec![&a, &b]
			} else {
				assert_eq!(mode, "c9-noargs");
				vec![]
			};
			invoked = true;
			crate::experimental_core::execute::cancel_at_charge_for_test(cancel);
			let result = executor.invoke(
				program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&args,
				fuel,
			);
			crate::experimental_core::execute::cancel_at_charge_for_test(None);
			match result {
				Ok(root) => {
					let value = store.import(&root).unwrap();
					let g = fresh_graph(&mut store, value);
					store.root_release(root).unwrap();
					json!({"status":"value","graph":g})
				}
				Err(f) => {
					super::d1e_diagnostics::fresh(&f, program, &sources);
					typed_failure = Some(format!("{:?}", f.error.kind));
					cleanup_error = f.cleanup_error.as_ref().map(|e| format!("{e:?}"));
					actual_failure = json!({"failure":format!("{f:?}"),"typed_kind":format!("{:?}",f.error.kind),"typed_detail":format!("{:?}",f.error.detail),"call_chain":f.call_chain.iter().map(|at|json!({"source":at.source,"start":at.start,"end":at.end})).collect::<Vec<_>>()});
					fresh_error(&f, program)
				}
			}
		}
	};
	let post = fresh_graph(&mut store, held);
	let events = tape();
	let cleanup = if fixture_name == "self-cycle" {
		store.set(v, 0, FreshValue::Unit).unwrap();
		store.set(v, 1, FreshValue::Unit).unwrap();
		json!({"cycle_cleared":true})
	} else {
		json!({"cycle_cleared":null})
	};
	store.release(v).unwrap();
	store.release(state).unwrap();
	store.release(held).unwrap();
	let row_idle = executor.is_idle();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	// Same Executor and Store, including after a compile refusal. A separate
	// tiny program supplies reuse only when there is no compiled original.
	let fallback = if compiled.is_err() {
		let mut s = Sources::new();
		s.insert(
			Source::memory(if mode == "c9-noargs" {
				"pub fn c9_reuse(){7}"
			} else {
				"pub fn c11_reuse(){c11::mark(\"reuse\");7}"
			})
			.unwrap(),
		)
		.unwrap();
		Some(frontend::compile(&mut s, &context, &mut Diagnostics::new(), 12).unwrap())
	} else {
		None
	};
	let program = compiled.as_ref().ok().or(fallback.as_ref()).unwrap();
	let reuse_name = if mode == "c9-noargs" {
		"c9_reuse"
	} else {
		"c11_reuse"
	};
	let reuse = match executor.invoke(
		program,
		&mut store,
		program.function(Hash::type_hash([reuse_name])).unwrap(),
		&[],
		FUEL,
	) {
		Ok(root) => {
			let value = store_value(&store, &root);
			let g = fresh_graph(&mut store, value);
			store.root_release(root).unwrap();
			json!({"status":"value","graph":g})
		}
		Err(f) => {
			if cleanup_error.is_none() {
				cleanup_error = f.cleanup_error.as_ref().map(|e| format!("{e:?}"));
			}
			fresh_error(&f, program)
		}
	};
	let reuse_i64 = if reuse["status"] == "value"
		&& reuse["graph"]["nodes"] == json!([["I64", "0x1cad9186c9641c4f", "7"]])
	{
		Some(7)
	} else {
		None
	};
	let clean = json!({"cleanup_error":cleanup_error,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"reuse_i64":reuse_i64});
	let mut resource = clean.clone();
	resource["typed_failure_kind"] = json!(typed_failure);
	let audit = json!({"clean":clean,"resource":resource,"typed_failure_kind":typed_failure,"raw_failure":actual_failure,"slot_ledger":ledger,"depth_metadata":depth_metadata,"source_sha256":digest(source.as_bytes()),"cancel_at_charge":cancel,"row_idle":row_idle,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"reuse_used_fallback":fallback.is_some()});
	json!({"outcome":outcome,"invoked":invoked,"diagnostics":format!("{:?}",diagnostics.diagnostics()),"before":before,"post":post,"events":events,"events_after_reuse":tape(),"cleanup":cleanup,"reuse":reuse,"fresh_audit":audit})
}
fn store_value(store: &Store, root: &crate::experimental_core::value::Root) -> FreshValue {
	store.import(root).unwrap()
}
#[test]
fn fresh_fixture_graph_is_independent_and_alias_exact() {
	let mut seen = std::collections::HashSet::new();
	for (_, _, fixture_name, _, _, _, _) in CASES {
		if !seen.insert(fixture_name) {
			continue;
		}
		let (v, state) = fixture(fixture_name);
		let expected = roots_graph(&v, &state);
		cleanup_fixture(fixture_name, &v);
		let mut store = Store::new(Limits {
			entries: 4096,
			bytes: 4 << 20,
		})
		.unwrap();
		let (held, v, state) = fresh_fixture(&mut store, fixture_name);
		assert_eq!(fresh_graph(&mut store, held), expected);
		if *fixture_name == "self-cycle" {
			store.set(v, 0, FreshValue::Unit).unwrap();
			store.set(v, 1, FreshValue::Unit).unwrap();
		}
		store.release(v).unwrap();
		store.release(state).unwrap();
		store.release(held).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}
}
#[test]
fn fresh_cancel_hook_stops_before_effect_and_reuse_is_real() {
	let source="fn compare(a,b){c11::tick(a,b);a.cmp(b)} pub fn main(v,state){v.sort_by(compare);v} pub fn c11_reuse(){c11::mark(\"reuse\");7}";
	let result = observe_fresh(source, "ints", "args", FUEL, Some(0));
	assert_eq!(result["outcome"]["class"], "Cancelled");
	assert_eq!(result["events"], json!([]));
	assert_eq!(result["post"], result["before"]);
	assert_eq!(
		result["fresh_audit"]["clean"],
		json!({"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7})
	);
	assert_eq!(
		result["events_after_reuse"],
		json!([{"event":"mark","hex":"7265757365"}])
	);
}

#[test]
fn transparent_helper_failure_uses_nested_callsite() {
	let (_, source, fixture, _, mode, _, _) = CASES[83];
	let actual = observe_fresh(source, fixture, mode, FUEL, None);
	let sites = &actual["outcome"]["sites"];
	assert_eq!(sites[0]["start"], 31);
	assert_eq!(sites[0]["end"], 49);
	assert_eq!(
		serde_json::from_str::<Json>(sites[0]["frames"].as_str().unwrap()).unwrap(),
		json!([{"source":0,"start":98,"end":116}])
	);
	assert_eq!(
		actual["fresh_audit"]["raw_failure"]["call_chain"],
		json!([{"source":0,"start":98,"end":116}])
	);
	assert_eq!(actual["fresh_audit"]["clean"]["live_entries"], 0);
}
#[test]
fn transparent_helpers_do_not_duplicate_user_callers() {
	for (index, start, end, chain) in [
		(40, 45, 52, json!([{"source":0,"start":86,"end":104}])),
		(43, 55, 73, json!([])),
	] {
		let (_, source, fixture, _, mode, _, _) = CASES[index];
		let actual = observe_fresh(source, fixture, mode, FUEL, None);
		assert_eq!(actual["outcome"]["sites"][0]["start"], start);
		assert_eq!(actual["outcome"]["sites"][0]["end"], end);
		assert_eq!(
			serde_json::from_str::<Json>(actual["outcome"]["sites"][0]["frames"].as_str().unwrap())
				.unwrap(),
			chain
		);
		assert_eq!(actual["fresh_audit"]["raw_failure"]["call_chain"], chain);
		assert_eq!(actual["fresh_audit"]["clean"]["live_entries"], 0);
	}
}
