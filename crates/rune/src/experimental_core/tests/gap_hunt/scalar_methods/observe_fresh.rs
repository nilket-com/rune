fn observe_fresh(source: &str, mode: &str) -> (Json, Json) {
	use crate::experimental_core::{
		error::ErrorKind,
		value::{Scalar, Type},
	};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c6.rn", source).unwrap())
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
	let mut call_chain = vec![];
	let mut invoked = false;
	let observation = match compiled {
		Err(e) => {
			json!({"status":fresh_compile_status(e.refusal),"error":format!("{e:?}"),"refusal":format!("{:?}",e.refusal),"diagnostics":format!("{:?}",diagnostics.diagnostics()),"location":e.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"operation":e.operation})
		}
		Ok(program) => {
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			let mut inputs = Vec::new();
			if let Some(pair) = mode.strip_prefix("c6-f64x2-") {
				let (x, y) = pair.split_once('-').unwrap();
				for bits in [x, y] {
					inputs.push(
						store.root_f64(f64::from_bits(u64::from_str_radix(bits, 16).unwrap())),
					);
				}
				input_bits = Some(pair.to_owned());
			} else if let Some(bits) = mode.strip_prefix("c6-f64-") {
				inputs.push(store.root_f64(f64::from_bits(u64::from_str_radix(bits, 16).unwrap())));
				input_bits = Some(bits.to_owned());
			} else if let Some(n) = mode.strip_prefix("c6-i64-") {
				inputs.push(store.root_i64(n.parse::<i64>().unwrap()));
			} else {
				assert_eq!(mode, "c6-main");
			}
			let args = inputs.iter().collect::<Vec<_>>();
			invoked = true;
			let result = executor.invoke(&program, &mut store, main, &args, FUEL);
			let observation = match result {
				Ok(root) => {
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
					no_cleanup_error = failure.cleanup_error.is_none();
					call_chain = failure
						.call_chain
						.iter()
						.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
						.collect();
					// Closed map: these are the only three exact frozen runtime classes,
					// plus the explicitly retained resource class. Unknown remains raw.
					let class = match failure.error.kind {
						ErrorKind::Type => "Type",
						ErrorKind::IntegerCoercion => "IntegerCoercion",
						ErrorKind::Overflow => "Overflow",
						ErrorKind::DivisionByZero => "DivisionByZero",
						ErrorKind::AccessConflict => "AccessConflict",
						ErrorKind::FormatLimit => "FormatLimit",
						_ => "UNMAPPED",
					};
					let chain = match failure.error.detail.numeric_argument() {
						Some((cause, arg)) => {
							assert!(matches!(
								cause,
								crate::experimental_core::error::HostCause::ExpectedType
									| crate::experimental_core::error::HostCause::ExpectedNumber
									| crate::experimental_core::error::HostCause::IntegerCoercion
							));
							vec![
								json!({"kind":format!("{cause:?}"),"display":format!("Bad argument #{arg}")}),
							]
						}
						None => vec![],
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
						.map(
							|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}),
						)
						.collect::<Vec<_>>();
					json!({"status":"error","class":class,"kind":format!("{:?}",failure.error.kind),"display":failure.message(&program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
				}
			};
			for root in inputs {
				store.root_release(root).unwrap();
			}
			observation
		}
	};
	let clean = store.live_entries() == 0 && store.payload_bytes() == 0 && executor.is_idle();
	assert!(clean, "C6 fresh balance");
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
		json!({"no_cleanup_error":no_cleanup_error,"clean_after_row":clean,"reuse":true,"live_entries":0,"payload_bytes":0,"idle":true,"input_f64_bits":input_bits,"call_chain":call_chain,"invoked":invoked}),
	)
}

fn fresh_compile_status(refusal: Option<crate::experimental_core::error::Refusal>) -> &'static str {
	if refusal == Some(crate::experimental_core::error::Refusal::FrontendDiagnostic) {
		"compile-error"
	} else {
		"refused"
	}
}
