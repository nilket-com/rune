fn observe_fresh(source: &str, mode: &str) -> (Json, Json) {
	use crate::experimental_core::{
		error::{Detail, ErrorKind},
		value::{Scalar, Type},
	};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c8.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let compiled = frontend::compile(&mut sources, &context, &mut diagnostics, 9);
	let limits = match mode {
		"c8-main" => Limits {
			entries: 4096,
			bytes: 4 << 20,
		},
		"c8-small-store" => Limits {
			entries: 3,
			bytes: 4096,
		},
		_ => panic!("unknown C8 mode"),
	};
	let mut store = Store::new(limits).unwrap();
	let mut executor = Executor::new().unwrap();
	let mut no_cleanup_error = true;
	let mut call_chain = vec![];
	let mut invoked = false;
	let mut returned_root = false;
	let mut typed_failure_kind = None;
	let mut failure_site_ops = vec![];
	let observation = match compiled {
		Err(e) => {
			json!({"status":if e.refusal.is_some() && e.refusal != Some(crate::experimental_core::error::Refusal::FrontendDiagnostic) {"refused"} else {"compile-error"},"kind":format!("{:?}",e.kind),"error":format!("{e:?}"),"refusal":e.refusal.map(|r|r.name()),"diagnostics":format!("{:?}",diagnostics.diagnostics()),"location":e.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"operation":e.operation})
		}
		Ok(program) => {
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			invoked = true;
			match executor.invoke(&program, &mut store, main, &[], FUEL) {
				Ok(root) => {
					returned_root = true;
					let observation = match store.root_tag(&root).unwrap() {
						Type::Unit => json!({"status":"value","type":"Unit"}),
						_ => match store.root_scalar(&root).unwrap() {
							Some(Scalar::F64(v)) => {
								json!({"status":"value","type":"F64","bits":format!("{:016x}",v.to_bits())})
							}
							Some(Scalar::I64(v)) => {
								json!({"status":"value","type":"I64","value":v.to_string()})
							}
							Some(Scalar::U64(v)) => {
								json!({"status":"value","type":"U64","value":v.to_string()})
							}
							Some(Scalar::Bool(v)) => {
								json!({"status":"value","type":"Bool","value":v})
							}
							other => json!({"status":"unmapped-value","raw":format!("{other:?}")}),
						},
					};
					store.root_release(root).unwrap();
					observation
				}
				Err(failure) => {
					super::d1e_diagnostics::fresh(&failure, &program, &sources);
					no_cleanup_error = failure.cleanup_error.is_none();
					typed_failure_kind = Some(format!("{:?}", failure.error.kind));
					call_chain = failure
						.call_chain
						.iter()
						.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
						.collect();
					// Preserve every actual operation sharing the source location.
					// No fabricated instruction PC or inferred panic reason.
					if let Some(at) = failure.error.location {
						for id in 0..program.code.functions() {
							let function = program.code.function(id as u32).unwrap();
							for (pc, location) in function.locations.iter().enumerate() {
								if *location == at {
									failure_site_ops.push(json!({"function":id,"pc":pc,"operation":format!("{:?}",function.operation(pc).unwrap())}));
								}
							}
						}
					}
					let class = match failure.error.kind {
						ErrorKind::Type => "Type",
						ErrorKind::IntegerCoercion => "IntegerCoercion",
						ErrorKind::DivisionByZero => "DivisionByZero",
						ErrorKind::AccessConflict => "AccessConflict",
						ErrorKind::Panic => "Panic",
						ErrorKind::IndexOutOfBounds => "IndexOutOfBounds",
						ErrorKind::StoreCapacity => "StoreCapacity",
						_ => "UNMAPPED",
					};
					// Only actual typed argument metadata is exposed. Indexed errors
					// without such metadata have an empty chain and STOP on mismatch.
					let argument = match failure.error.detail {
						Detail::HostMessage {
							cause,
							arg: Some(arg),
						} => Some((cause, arg)),
						other => other.numeric_argument(),
					};
					let chain = argument
						.map(|(cause, arg)| {
							vec![
								json!({"kind":format!("{cause:?}"),"display":format!("Bad argument #{arg}")}),
							]
						})
						.unwrap_or_default();
					// Fresh retains source callsites, not stock VM stack windows. Keep
					// the actual representation; no retained-frame adapter is applied.
					let frames = if call_chain.is_empty() {
						"[]".into()
					} else {
						serde_json::to_string(&call_chain).unwrap()
					};
					let sites = failure
						.error
						.location
						.iter()
						.map(
							|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}),
						)
						.collect::<Vec<_>>();
					json!({"status":"error","class":class,"kind":format!("{:?}",failure.error.kind),"display":failure.message(&program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
				}
			}
		}
	};
	let row_live = store.live_entries();
	let row_payload = store.payload_bytes();
	let row_idle = executor.is_idle();
	let clean = row_live == 0 && row_payload == 0 && row_idle;
	// Keep observations even on imbalance; the parent enforces the audit after
	// the child has preserved its envelope. Reuse uses this same executor/store.
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main() { 7 }").unwrap())
		.unwrap();
	let reuse_program =
		frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 10).unwrap();
	let reuse_result = executor.invoke(
		&reuse_program,
		&mut store,
		reuse_program.function(Hash::type_hash(["main"])).unwrap(),
		&[],
		FUEL,
	);
	let (reuse, reuse_raw) = match reuse_result {
		Ok(root) => {
			let scalar = store.root_scalar(&root).unwrap();
			store.root_release(root).unwrap();
			(scalar == Some(Scalar::I64(7)), format!("{scalar:?}"))
		}
		Err(e) => (false, format!("{e:?}")),
	};
	(
		observation,
		json!({"no_cleanup_error":no_cleanup_error,"clean_after_row":clean,"row_live_entries":row_live,"row_payload_bytes":row_payload,"row_idle":row_idle,"reuse":reuse,"reuse_raw":reuse_raw,"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"idle":executor.is_idle(),"call_chain":call_chain,"invoked":invoked,"returned_root":returned_root,"typed_failure_kind":typed_failure_kind,"failure_site_ops":failure_site_ops,"limits_entries":limits.entries,"limits_bytes":limits.bytes}),
	)
}
