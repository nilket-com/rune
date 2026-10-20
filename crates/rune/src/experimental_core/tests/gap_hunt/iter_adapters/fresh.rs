// Independent fresh observations. This module reads no frozen outcome or policy.
include!("fresh_graph.rs");

/// Typed stock kinds for fresh details outside `fresh_primary_kind`. Never consults frozen observations.
fn typed_kind(
	error: &crate::experimental_core::error::Error,
	program: &frontend::Program,
) -> Option<stock::VmErrorKind> {
	use crate::experimental_core::detail::Detail;
	use stock::VmErrorKind as K;
	let info = |name| super::small_forms::fresh_type_info(name, program);
	Some(match error.detail {
		Detail::NotCallable { found } => K::UnsupportedCallFn {
			actual: info(found)?,
		},
		Detail::FieldOnNonRecord { target, field } => K::UnsupportedObjectSlotIndexGet {
			target: info(target)?,
			field: crate::sync::Arc::try_new(
				stock::StaticString::new(field.resolve(program)?).ok()?,
			)
			.ok()?,
		},
		Detail::IndexTarget { target, index } => K::UnsupportedIndexGet {
			target: info(target)?,
			index: info(index)?,
		},
		Detail::TryOperand { found } => K::UnsupportedTryOperand {
			actual: info(found)?,
		},
		Detail::Cast {
			found,
			to_numeric: true,
		} => K::UnsupportedAs {
			value: info(found)?,
		},
		Detail::MissingField { target, field } => K::MissingField {
			target: info(target)?,
			field: crate::alloc::String::try_from(field.resolve(program)?).ok()?,
		},
		Detail::MovedArgument { access, .. } => K::AccessError {
			error: stock::AccessError::fresh_moved_for_test(match access {
				crate::experimental_core::error::MovedAccess::Read => 0,
				crate::experimental_core::error::MovedAccess::Write => 1,
				crate::experimental_core::error::MovedAccess::Take => 2,
			})?,
		},
		Detail::MissingIntoIter { found } => K::MissingProtocolFunction {
			protocol: &crate::runtime::Protocol::INTO_ITER,
			instance: info(found)?,
		},
		Detail::Moved { access } => K::AccessError {
			error: stock::AccessError::fresh_moved_for_test(match access {
				crate::experimental_core::error::MovedAccess::Read => 0,
				crate::experimental_core::error::MovedAccess::Write => 1,
				crate::experimental_core::error::MovedAccess::Take => 2,
			})?,
		},
		Detail::TakeSharedArgument { readers, .. } => K::AccessError {
			error: stock::AccessError::fresh_shared_take_for_test(readers),
		},
		Detail::TupleLength { actual, expected } => K::ExpectedTupleLength { actual, expected },
		_ => return None,
	})
}

fn fresh_failure(
	failure: &crate::experimental_core::execute::Failure,
	program: &frontend::Program,
	sources: &Sources,
) -> Json {
	super::d1e_diagnostics::fresh(failure, program, sources);
	use crate::experimental_core::{detail::Detail, error::ErrorKind};
	let error = &failure.error;
	let kind = if error.kind == ErrorKind::FuelExhausted {
		Some(stock::VmErrorKind::Halted {
			halt: stock::VmHaltInfo::Limited,
		})
	} else if error.kind == ErrorKind::Panic && error.detail == Detail::PanicMessage {
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
		typed_kind(error, program)
			.or_else(|| super::small_forms::fresh_primary_kind(error, program))
	}
	.unwrap_or_else(|| panic!("unmapped C17e failure: {failure:?}"));
	let class = match kind {
		stock::VmErrorKind::Panic { .. } => "Panic".to_owned(),
		stock::VmErrorKind::UnsupportedCallFn { .. } => "Type".to_owned(),
		_ => stock_class(&kind),
	};
	let chain = failure.invocation_context().map(|at| match at {
		stock::InvocationContext::BadArgument { arg } => json!({"kind":format!("BadArgument {{ arg: {arg} }}"),"display":format!("Bad argument #{arg}")}),
	}).collect::<Vec<_>>();
	let frames = serde_json::to_string(&failure.call_chain.iter().map(|at| json!({"source":at.source,"start":at.start,"end":at.end})).collect::<Vec<_>>()).unwrap();
	let sites = failure.error.location.iter().map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames})).collect::<Vec<_>>();
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":failure.message(program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
}

#[test]
#[ignore = "C17e fresh observation; bounded external operator only"]
fn fresh_child() {
	let index: usize = std::env::var("C17E_CASE").unwrap().parse().unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let row = &cases[index];
	TAPE.with(|t| t.borrow_mut().clear());
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c17e"]).unwrap();
	module.fresh_host_function("tap", tap).unwrap();
	context.install(module).unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::new("c17e.rn", row["source"].as_str().unwrap()).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut sources, &context, &mut diagnostics, 16);
	let mut store = Store::new(Limits { entries: 4096, bytes: 4 << 20 }).unwrap();
	let mut executor = Executor::new().unwrap();
	executor.set_host_runtime(Some(crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap())).unwrap();
	let mut cleanup_error = false;
	let outcome = match &built {
		Err(error) => json!({"status":"refused","refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")}),
		Ok(program) => match executor.invoke(program, &mut store, program.function(Hash::type_hash(["main"])).unwrap(), &[], row["fuel"].as_u64().unwrap() as usize) {
			Ok(root) => {
				let value = store.import(&root).unwrap();
				let graph = fresh_graph(&mut store, value);
				store.root_release(root).unwrap();
				json!({"status":"value","graph":graph})
			}
			Err(failure) => {
				cleanup_error = failure.cleanup_error.is_some();
				fresh_failure(&failure, program, &sources)
			}
		},
	};
	let row_idle = executor.is_idle();
	let row_live_entries = store.live_entries();
	let row_payload_bytes = store.payload_bytes();
	let fallback = if built.is_err() {
		let mut sources = Sources::new();
		sources.insert(Source::memory("pub fn c17e_reuse(){7}").unwrap()).unwrap();
		Some(frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 17).unwrap())
	} else { None };
	let program = built.as_ref().ok().or(fallback.as_ref()).unwrap();
	let root = executor.invoke(program, &mut store, program.function(Hash::type_hash(["c17e_reuse"])).unwrap(), &[], 200000).unwrap();
	let value = store.import(&root).unwrap();
	let reuse_graph = fresh_graph(&mut store, value);
	store.root_release(root).unwrap();
	let reuse = if row["reuse_enabled"] == true { json!({"status":"value","graph":reuse_graph}) } else { Json::Null };
	let events = TAPE.with(|t| t.borrow().clone());
	let logical = std::env::var_os("D1E_TRACE_OUT").and_then(|p| fs::read(p).ok()).map(|b| serde_json::from_slice::<Json>(&b).unwrap()).unwrap_or(Json::Null);
	let result = json!({"schema":1,"engine":"fresh","index":index,"label":row["label"],"source":row["source"],"origin":row["origin"],"source_sha256":row["source_sha256"],"fuel":row["fuel"],"diagnostics":format!("{:?}",diagnostics.diagnostics()),"logical":logical,"observation":{"outcome":outcome,"events":events,"reuse":reuse},"fresh_audit":{"invoked":built.is_ok(),"cleanup_error":cleanup_error,"row_idle":row_idle,"row_live_entries":row_live_entries,"row_payload_bytes":row_payload_bytes,"executor_idle":executor.is_idle(),"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"reuse_used_fallback":fallback.is_some()}});
	fs::write(std::env::var_os("C17E_RESULT").unwrap(), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
