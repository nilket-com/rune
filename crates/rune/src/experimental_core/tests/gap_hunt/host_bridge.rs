//! H1 stock host oracle. Test-only; future identity controls are not stock probes.
use super::*;
use std::boxed::Box;
#[path = "host_bridge/fixture.rs"]
mod fixture;
struct Case {
	label: &'static str,
	source: &'static str,
	mode: &'static str,
	policy: &'static str,
	stock: bool,
}
include!("host_bridge/cases.rs");

fn typed(value: &crate::runtime::Value, depth: usize) -> String {
	use crate::runtime::Inline;
	assert!(depth <= 16, "typed observation depth");
	if let Some(inline) = value.as_inline() {
		return match inline {
			Inline::Unit => "unit".into(),
			Inline::Signed(n) => format!("i64:{n}"),
			Inline::Unsigned(n) => format!("u64:{n}"),
			Inline::Float(n) => format!("f64:{:016x}", n.to_bits()),
			Inline::Bool(n) => format!("bool:{n}"),
			Inline::Char(n) => format!("char:{}", *n as u32),
			_ => format!("inline:{inline:?}"),
		};
	}
	if let Ok(v) = value.borrow_string_ref() {
		return format!("string:{}", hex(v.as_bytes()));
	}
	if let Ok(v) = value.borrow_ref::<Option<crate::runtime::Value>>() {
		return match &*v {
			Some(v) => format!("Some({})", typed(v, depth + 1)),
			None => "None".into(),
		};
	}
	if let Ok(v) = value.borrow_ref::<Result<crate::runtime::Value, crate::runtime::Value>>() {
		return match &*v {
			Ok(v) => format!("Ok({})", typed(v, depth + 1)),
			Err(v) => format!("Err({})", typed(v, depth + 1)),
		};
	}
	if let Ok(v) = value.borrow_tuple_ref() {
		return format!(
			"tuple:[{}]",
			v.iter()
				.map(|v| typed(v, depth + 1))
				.collect::<Vec<_>>()
				.join(",")
		);
	}
	// Catalogue includes aggregate alias witnesses; Debug is separately retained.
	format!("other:{}", value.type_info())
}

pub(super) fn observe_h1(path: &Path, mode: &str, engine: &str) -> Observation {
	assert_eq!(engine, "stock", "H1 stock-only checkpoint");
	let mode = mode.strip_prefix("h1-").unwrap();
	let result_path = PathBuf::from(std::env::var_os("GAP_RESULT").unwrap());
	let tape = fixture::Recorder::new();
	let context = fixture::stock_context(&tape);
	let mut src = sources(path, "main");
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut src)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	fs::write(
		result_path.with_extension("diagnostics"),
		format!("{:?}", diagnostics.diagnostics()),
	)
	.unwrap();
	let observation = match built {
		Err(e) => Observation::new(
			"compile-error",
			"FrontendDiagnostic",
			"stock compiler",
			format!("{e:?}: {:?}", diagnostics.diagnostics()),
			String::new(),
		),
		Ok(mut vm) => {
			let fuel = mode
				.strip_prefix("stock-fuel-")
				.map(|s| s.parse().unwrap())
				.unwrap_or(FUEL);
			let previous = std::panic::take_hook();
			std::panic::set_hook(Box::new(|_| eprintln!("H1 HOST UNWIND")));
			let invoked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				crate::runtime::budget::with(fuel, || vm.call(["main"], ())).call()
			}));
			std::panic::set_hook(previous);
			match invoked {
				Err(payload) => {
					let payload = payload
						.downcast_ref::<&str>()
						.copied()
						.or_else(|| payload.downcast_ref::<String>().map(String::as_str))
						.unwrap_or("non-string payload");
					Observation::new(
						"unwind",
						"RustPanic",
						"outer stock containment",
						payload.into(),
						String::new(),
					)
				}
				Ok(Err(e)) => {
					let mut metadata =
						format!("primary:{:?}\nchain:{:?}\n", e.error().kind(), e.chain());
					for at in e.stacktrace() {
						if let Some(inst) =
							at.unit.debug_info().and_then(|d| d.instruction_at(at.ip))
						{
							let range = inst.span.range();
							metadata.push_str(&format!(
								"location:{}:{}:{}:{}\nframes:{:?}\n",
								at.ip,
								inst.source_id.into_index(),
								range.start,
								range.end,
								at.frames
							));
						} else {
							metadata.push_str(&format!(
								"location:{}:none\nframes:{:?}\n",
								at.ip, at.frames
							));
						}
					}
					// Raw Debug is provenance only; select stable typed fields separately.
					let primary = match e.error().kind() {
						crate::runtime::VmErrorKind::ExpectedNumber { .. } => {
							"ExpectedNumber".into()
						}
						crate::runtime::VmErrorKind::ExpectedType { .. } => "ExpectedType".into(),
						kind => format!("{kind:?}"),
					};
					assert!(
						!primary.contains("full_name: 0x"),
						"unreviewed typed primary schema"
					);
					let mut auxiliary = Vec::new();
					for at in e.chain() {
						assert!(
							matches!(
								at.kind(),
								crate::runtime::VmErrorKind::BadArgument { arg: 0 | 1 }
							),
							"unreviewed auxiliary schema"
						);
						auxiliary.push(hex(at.to_string().as_bytes()));
					}
					let (_, tail) = metadata.split_once('\n').unwrap();
					let (chain, locations) = tail.split_once('\n').unwrap();
					let stable = format!(
						"primary:{primary}\n{chain}\nauxiliary:{}\n{locations}",
						auxiliary.join(",")
					);
					fs::write(result_path.with_extension("stable-error"), stable).unwrap();
					fs::write(result_path.with_extension("error"), metadata).unwrap();
					Observation::new(
						"error",
						&stock_class(e.error().kind()),
						"stock runtime",
						e.error().kind().to_string(),
						String::new(),
					)
				}
				Ok(Ok(value)) => {
					fs::write(result_path.with_extension("typed"), typed(&value, 0)).unwrap();
					let mut text = crate::alloc::String::new();
					match vm.with(|| {
						crate::runtime::Formatter::format_with(&mut text, |f| value.debug_fmt(f))
					}) {
						Ok(()) => Observation::new(
							"value",
							"",
							"",
							String::new(),
							format!("{}{text}", stock_scalar(&value)),
						),
						Err(e) => Observation::new(
							"observation-error",
							"Debug",
							"stock Debug",
							e.to_string(),
							String::new(),
						),
					}
				}
			}
		}
	};
	let (calls, events) = tape.snapshot();
	let mut effects = format!("calls:{calls}\n");
	for event in events {
		effects.push_str(&format!("event:{}\n", hex(event.as_bytes())));
	}
	fs::write(result_path.with_extension("effects"), effects).unwrap();
	observation
}

#[test]
fn catalogue_invariants() {
	assert_eq!(CASES.len(), 150);
	assert_eq!(CASES.iter().filter(|c| c.stock).count(), 140);
	let mut labels = std::collections::BTreeSet::new();
	for case in CASES {
		assert!(labels.insert(case.label));
		assert!(case.source.starts_with("pub fn main") || case.source.contains("pub fn main"));
		assert!(!case.policy.is_empty());
	}
}

#[test]
#[ignore = "ONE bounded H1 stock probe; new GAP_OUT required"]
fn stock_probe() {
	catalogue_invariants();
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	fs::write(
		out.join("catalogue.json"),
		include_str!("host_bridge/cases.json"),
	)
	.unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-h1-oracle-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let started = Instant::now();
	let mut table = String::from("case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\ttyped_hex\teffects_hex\terror_hex\tdiagnostics_hex\tstable_error_hex\tpolicy\n");
	for (n, case) in CASES.iter().filter(|c| c.stock).enumerate() {
		assert!(
			started.elapsed() < Duration::from_secs(600),
			"aggregate cap"
		);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, case.source).unwrap();
		for ext in ["typed", "effects", "error", "diagnostics", "stable-error"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{ext}")));
		}
		let mode = format!("h1-{}", case.mode);
		fs::write(out.join(format!("{id}.argv")), format!("{:?} --exact {CHILD} --ignored --nocapture --quiet\nGAP_CASE={path:?}\nGAP_ORIGIN={path:?}\nGAP_MODE={mode}\nGAP_ENGINE=stock\nGAP_RESULT={:?}\nstdin=/dev/null\n", std::env::current_exe().unwrap(), scratch.0.join("result"))).unwrap();
		let run = launch(&path, &path, &mode, "stock", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {}: {}", case.label, run.record);
		fs::write(out.join(format!("{id}.rc")), "0\n").unwrap();
		let mut fields = Vec::new();
		for ext in ["typed", "effects", "error", "diagnostics", "stable-error"] {
			let bytes = fs::read(scratch.0.join(format!("result.{ext}"))).unwrap_or_default();
			assert!(bytes.len() <= CAP, "metadata cap");
			fs::write(out.join(format!("{id}.{ext}")), &bytes).unwrap();
			fields.push(hex(&bytes));
		}
		table.push_str(&format!(
			"{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
			case.label,
			case.mode,
			hex(case.source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr),
			fields.join("\t"),
			case.policy
		));
		fs::write(out.join("stock-host.tsv"), &table).unwrap();
	}
	fs::write(
		out.join("deferred.txt"),
		CASES
			.iter()
			.filter(|c| !c.stock)
			.map(|c| format!("{}\t{}\n", c.label, c.mode))
			.collect::<String>(),
	)
	.unwrap();
	contract_completed("stock", 140);
}

#[test]
#[ignore = "ONE bounded byte-exact stock replay; explicit projected table and baseline origin"]
fn stock_replay() {
	catalogue_invariants();
	let expected_path = PathBuf::from(std::env::var_os("H1_EXPECTED").expect("projected table"));
	let expected = fs::read_to_string(expected_path).unwrap();
	let mut lines = expected.lines();
	assert_eq!(lines.next().unwrap(), "case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\ttyped_hex\teffects_hex\tstable_error_hex\tdiagnostics_hex\tpolicy");
	let rows: Vec<_> = lines.collect();
	assert_eq!(rows.len(), 140);
	let baseline =
		PathBuf::from(std::env::var_os("H1_BASELINE").expect("original stock source origins"));
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-h1-replay-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let started = Instant::now();
	for (n, case) in CASES.iter().filter(|c| c.stock).enumerate() {
		assert!(
			started.elapsed() < Duration::from_secs(600),
			"aggregate cap"
		);
		let frozen: Vec<_> = rows[n].split('\t').collect();
		assert_eq!(frozen.len(), 11);
		assert_eq!(frozen[0], case.label);
		assert_eq!(frozen[1], case.mode);
		assert_eq!(frozen[2], hex(case.source.as_bytes()));
		assert_eq!(frozen[10], case.policy);
		let id = format!("{n:05}");
		let origin = baseline.join(format!("{id}.source.rn"));
		assert_eq!(fs::read_to_string(&origin).unwrap(), case.source);
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, case.source).unwrap();
		for ext in ["typed", "effects", "error", "diagnostics", "stable-error"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{ext}")));
		}
		let mode = format!("h1-{}", case.mode);
		fs::write(out.join(format!("{id}.argv")), format!("{:?} --exact {CHILD} --ignored --nocapture --quiet\nGAP_CASE={path:?}\nGAP_ORIGIN={origin:?}\nGAP_MODE={mode}\nGAP_ENGINE=stock\nGAP_RESULT={:?}\nstdin=/dev/null\n", std::env::current_exe().unwrap(), scratch.0.join("result"))).unwrap();
		let run = launch(&path, &origin, &mode, "stock", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {}: {}", case.label, run.record);
		fs::write(out.join(format!("{id}.rc")), "0\n").unwrap();
		let mut actual = vec![
			case.label.to_owned(),
			case.mode.to_owned(),
			hex(case.source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr),
		];
		for ext in ["typed", "effects", "stable-error", "diagnostics"] {
			let bytes = fs::read(scratch.0.join(format!("result.{ext}"))).unwrap_or_default();
			assert!(bytes.len() <= CAP);
			fs::write(out.join(format!("{id}.{ext}")), &bytes).unwrap();
			actual.push(hex(&bytes));
		}
		// Preserve raw metadata without putting it in the compared vector.
		fs::write(
			out.join(format!("{id}.error")),
			fs::read(scratch.0.join("result.error")).unwrap_or_default(),
		)
		.unwrap();
		actual.push(case.policy.to_owned());
		fs::write(out.join(format!("{id}.compared")), actual.join("\t")).unwrap();
		assert_eq!(actual.join("\t"), rows[n], "replay {}", case.label);
	}
	contract_completed("stock", 140);
}
