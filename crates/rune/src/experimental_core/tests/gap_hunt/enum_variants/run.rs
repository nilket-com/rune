// Independent observations. This file reads neither dispositions nor stock output.
fn outcome(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
) -> Json {
	let kind = if let Some(message) = failure.panic_message.as_deref() {
		Some(stock::VmErrorKind::Panic {
			reason: stock::Panic::custom(message.to_owned()),
		})
	} else {
		constructor_error_kind(&failure.error, program)
			.or_else(|| super::small_forms::primary_kind(&failure.error, program))
	};
	let Some(kind) = kind else {
		if failure.error.kind == crate::experimental_core::error::ErrorKind::FuelExhausted {
			return json!({"status":"error","class":"FuelExhausted","raw":format!("{failure:?}")});
		}

		return json!({"status":"unmapped-error","raw":format!("{failure:?}")});
	};
	let typed = stock::VmError::new(kind);
	let mut result = error(&typed);
	if let Some((_, arg)) = failure.error.detail.numeric_argument() {
		result["chain"] = json!([{"kind":format!("BadArgument {{ arg: {arg} }}"),"display":format!("Bad argument #{arg}")}]);
	}

	result["display"] = json!(failure.message(program).to_string());
	let frames = if failure.call_chain.is_empty() {
		"[]".to_owned()
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
	result["sites"] = json!(failure
		.error
		.location
		.iter()
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}))
		.collect::<Vec<_>>());
	result["raw"] = json!(format!("{failure:?}"));
	result
}

fn scalar_graph(store: &mut Store, root: &crate::experimental_core::value::Root) -> Json {
	let value = store.import(root).unwrap();
	graph(store, value)
}

fn observe(row: &Json) -> Json {
	let source = row["source"].as_str().unwrap();
	let context = context_for(row["fixture"].as_str().unwrap(), true);
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c15b.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut sources, &context, &mut diagnostics, 14);
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
	let mut cleanup_error = Json::Null;
	let fixture_cleanup = Json::Null;
	let outcome = match &built {
		Err(error) => {
			json!({"status":"refused","refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
		}
		Ok(program) => match executor.invoke(
			program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			row["fuel"].as_u64().unwrap() as usize,
		) {
			Ok(root) => {
				let graph = scalar_graph(&mut store, &root);

				store.root_release(root).unwrap();
				json!({"status":"value","graph":graph})
			}
			Err(failure) => {
				cleanup_error = json!(failure.cleanup_error.as_ref().map(|e| format!("{e:?}")));
				self::outcome(&failure, program)
			}
		},
	};
	let events = TAPE.with(|t| t.borrow().clone());
	let row_idle = executor.is_idle();
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let fallback = if built.is_err() {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory("pub fn c15b_reuse(){7}").unwrap())
			.unwrap();
		Some(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 13).unwrap())
	} else {
		None
	};
	let program = built.as_ref().ok().or(fallback.as_ref()).unwrap();
	let reuse = match executor.invoke(
		program,
		&mut store,
		program.function(Hash::type_hash(["c15b_reuse"])).unwrap(),
		&[],
		200_000,
	) {
		Ok(root) => {
			let graph = scalar_graph(&mut store, &root);
			store.root_release(root).unwrap();
			json!({"status":"value","graph":graph})
		}
		Err(failure) => self::outcome(&failure, program),
	};
	json!({"schema":1,"engine":"fresh","index":row["id"].as_str().unwrap().parse::<usize>().unwrap(),"label":row["label"],"source_hex":hex(source.as_bytes()),"source_sha256":row["source_sha256"],"fuel":row["fuel"],"fixture":row["fixture"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"observation":{"outcome":outcome,"events":events,"reuse":reuse},"fresh_audit":{"fixture_cleanup":fixture_cleanup,"invoked":built.is_ok(),"cleanup_error":cleanup_error,"row_idle":row_idle,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"reuse_used_fallback":fallback.is_some()}})
}

// Project typed runtime diagnostics; never consult frozen observations.
fn constructor_error_kind(
	error: &crate::experimental_core::error::Error,
	program: &frontend::Program,
) -> Option<stock::VmErrorKind> {
	use crate::experimental_core::detail::Detail;
	use stock::VmErrorKind as K;
	let info = |name| super::small_forms::type_info(name, program);
	let field = |name: crate::experimental_core::detail::FieldRef| {
		crate::sync::Arc::try_new(stock::StaticString::new(name.resolve(program)?).ok()?).ok()
	};
	Some(match error.detail {
		Detail::MissingInteger { target, index } => K::MissingIndexInteger {
			target: info(target)?,
			index: index.into(),
		},
		Detail::TupleIndexSet { target } => K::UnsupportedTupleIndexSet {
			target: info(target)?,
		},
		Detail::FieldOnNonRecord {
			target,
			field: name,
		} => K::UnsupportedObjectSlotIndexGet {
			target: info(target)?,
			field: field(name)?,
		},
		Detail::MissingField {
			target,
			field: name,
		} => K::MissingField {
			target: info(target)?,
			field: crate::alloc::String::try_from(name.resolve(program)?).ok()?,
		},
		Detail::IndexSet {
			target,
			index,
			value,
		} => K::UnsupportedIndexSet {
			target: info(target)?,
			index: info(index)?,
			value: info(value)?,
		},
		Detail::IndexTarget { target, index } => K::UnsupportedIndexGet {
			target: info(target)?,
			index: info(index)?,
		},
		_ => return None,
	})
}
