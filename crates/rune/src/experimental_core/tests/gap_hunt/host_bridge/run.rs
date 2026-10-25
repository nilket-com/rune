//! Test-only new observation: typed outcomes, real effects and semantic sites.
//! Expected records are never inputs to this observer.
use super::*;
use crate::experimental_core::{
	error::{Detail, ErrorKind, HostCause},
	value::{Type, Value},
};

fn typed(store: &mut Store, value: Value, depth: usize) -> String {
	assert!(depth <= 16, "typed observation depth");
	match value {
		Value::Unit => return "unit".into(),
		Value::Bool(v) => return format!("bool:{v}"),
		Value::I64(v) => return format!("i64:{v}"),
		Value::U64(v) => return format!("u64:{v}"),
		Value::F64(v) => return format!("f64:{:016x}", v.to_bits()),
		Value::Char(v) => return format!("char:{}", v as u32),
		_ => (),
	}
	match store.tag(value).unwrap() {
		Type::String => format!("string:{}", hex(store.text(value).unwrap().as_bytes())),
		Type::Option | Type::Result => {
			let (tag, success, child) = store.variant_parts(value).unwrap();
			match (tag, success, child) {
				(Type::Option, false, None) => "None".into(),
				(Type::Option, true, Some(child)) => {
					format!("Some({})", typed(store, child, depth + 1))
				}
				(Type::Result, success, Some(child)) => format!(
					"{}({})",
					if success { "Ok" } else { "Err" },
					typed(store, child, depth + 1)
				),
				_ => panic!("invalid variant observation"),
			}
		}
		Type::Tuple => {
			let mut parts = Vec::new();
			for index in 0..store.len(value).unwrap() {
				let child = store.get(value, index).unwrap();
				parts.push(typed(store, child, depth + 1));
				store.release(child).unwrap();
			}
			format!("tuple:[{}]", parts.join(","))
		}
		Type::Vec => "other:std::vec::Vec".into(),
		Type::Object => "other:std::object::Object".into(),
		_ => panic!("unreviewed execution result type"),
	}
}

pub(super) fn observe(path: &Path, mode: &str) -> Observation {
	let mode = mode.strip_prefix("h1-").unwrap();
	let out = PathBuf::from(std::env::var_os("GAP_RESULT").unwrap());
	let tape = fixture::Recorder::new();
	let context = fixture::context(&tape);
	let mut src = sources(path, "main");
	let mut diagnostics = Diagnostics::new();
	let built = frontend::compile(&mut src, &context, &mut diagnostics, 1);
	fs::write(
		out.with_extension("diagnostics"),
		format!("{:?}", diagnostics.diagnostics()),
	)
	.unwrap();
	let observation = match built {
		Err(e) => Observation::new(
			"refused",
			&format!("{:?}", e.refusal),
			e.operation,
			format!("{e:?}"),
			String::new(),
		),
		Ok(program) => {
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = Executor::new().unwrap();
			if mode != "missing-runtime" {
				executor
					.set_host_runtime(Some(
						crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
					))
					.unwrap();
			}
			let fuel = mode
				.strip_prefix("fresh-fuel-")
				.map(|v| v.parse().unwrap())
				.unwrap_or(FUEL);
			let previous = std::panic::take_hook();
			std::panic::set_hook(Box::new(|_| eprintln!("H1 HOST UNWIND")));
			let result = executor.invoke(&program, &mut store, main, &[], fuel);
			std::panic::set_hook(previous);
			let observation = match result {
				Ok(root) => {
					fs::write(
						out.with_extension("typed"),
						typed(&mut store, root.value, 0),
					)
					.unwrap();
					let text = store.root_format(&root, true).unwrap();
					let record = Observation::new(
						"value",
						"",
						"",
						String::new(),
						format!("{}{text}", scalar(&store, &root)),
					);
					store.root_release(root).unwrap();
					record
				}
				Err(failure) => {
					// Preserve typed provenance and effects even if a classification is unknown.
					fs::write(
						out.with_extension("error"),
						format!(
							"{:?}\nhost:{:?}\npanic:{:?}\n",
							failure.error, failure.host_message, failure.panic_message
						),
					)
					.unwrap();
					write_effects(&out, &tape);

					assert!(failure.cleanup_error.is_none(), "secondary cleanup failure");
					let (primary, class, arg) = match (failure.error.kind, failure.error.detail) {
						(
							ErrorKind::Host,
							Detail::HostMessage {
								cause: HostCause::ExpectedNumber,
								arg,
							},
						)
						| (ErrorKind::Host, Detail::HostExpectedNumber { arg, .. }) => {
							("ExpectedNumber", "Type", arg)
						}
						(
							ErrorKind::Host,
							Detail::HostMessage {
								cause: HostCause::ExpectedType,
								arg,
							},
						)
						| (ErrorKind::Host, Detail::HostExpectedType { arg, .. }) => ("ExpectedType", "Type", arg),
						(
							ErrorKind::Host,
							Detail::HostMessage {
								cause: HostCause::Access,
								arg,
							},
						) => ("AccessErrorMoved", "AccessConflict", arg),
						(
							ErrorKind::IntegerCoercion,
							Detail::HostMessage {
								cause: HostCause::IntegerCoercion,
								arg,
							},
						) => ("ValueToIntegerCoercionError", "IntegerCoercion", arg),
						(ErrorKind::Panic, Detail::PanicMessage) => ("Panic", "Panic", None),
						(ErrorKind::Host, Detail::HostPanicked) => {
							("HostPanicked", "HostPanicked", None)
						}
						(ErrorKind::DivisionByZero, _) => ("DivideByZero", "DivisionByZero", None),
						(ErrorKind::FuelExhausted, _) => ("FuelExhausted", "FuelExhausted", None),
						(ErrorKind::Cancelled, _) => ("Cancelled", "Cancelled", None),
						(ErrorKind::InvalidCode, _) if mode == "missing-runtime" => {
							("AdmissionInvalidCode", "InvalidCode", None)
						}
						_ => panic!("unreviewed execution failure: {:?}", failure.error),
					};
					let site = failure
						.error
						.location
						.map(|at| format!("{}:{}:{}", at.source, at.start, at.end))
						.unwrap_or_else(|| "none".into());
					let frames: Vec<_> = failure
						.call_chain
						.iter()
						.map(|at| format!("{}:{}:{}", at.source, at.start, at.end))
						.collect();
					fs::write(
						out.with_extension("semantic-error"),
						format!("primary:{primary}\narg:{arg:?}\nsite:{site}\nframes:{frames:?}\n"),
					)
					.unwrap();
					fs::write(
						out.with_extension("error"),
						format!(
							"{:?}\nhost:{:?}\npanic:{:?}\n",
							failure.error, failure.host_message, failure.panic_message
						),
					)
					.unwrap();
					Observation::new(
						"error",
						class,
						&site,
						failure.message(&program).to_string(),
						String::new(),
					)
				}
			};
			assert!(executor.is_idle());
			assert_eq!(store.live_entries(), 0, "host observer store balance");
			assert_eq!(store.payload_bytes(), 0, "host observer byte balance");
			observation
		}
	};
	write_effects(&out, &tape);
	observation
}

fn write_effects(out: &Path, tape: &fixture::Recorder) {
	let (calls, events) = tape.snapshot();
	let mut effects = format!("calls:{calls}\n");
	for event in events {
		effects.push_str(&format!("event:{}\n", hex(event.as_bytes())));
	}
	fs::write(out.with_extension("effects"), effects).unwrap();
}
