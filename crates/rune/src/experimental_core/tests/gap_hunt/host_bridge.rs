//! H1 stock host oracle. Test-only; future identity controls are not stock probes.
use super::*;
use std::boxed::Box;
#[path = "host_bridge/contract.rs"]
mod contract;
#[path = "host_bridge/fixture.rs"]
mod fixture;
#[path = "host_bridge/fresh.rs"]
mod fresh;
#[path = "host_bridge/identity.rs"]
mod identity;
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
	if engine == "fresh" {
		return fresh::observe(path, mode);
	}
	assert_eq!(engine, "stock", "H1 observer engine");
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

#[test]
#[ignore = "H1 source-first precedence supplement; exactly four new stock children"]
fn stock_precedence_supplement() {
	precedence_run(false);
}

#[test]
#[ignore = "ONE bounded byte-exact replay of four frozen host precedence rows"]
fn stock_precedence_replay() {
	precedence_run(true);
}

fn precedence_run(replay: bool) {
	let cases = [
		Case {
			label: r####"h1/supplement/moved-string-to-i64"####,
			source: r####"pub fn main(){let s="x";drop(s);h1::id_i64(s)}"####,
			mode: "normal",
			policy: "stock-first-pending",
			stock: true,
		},
		Case {
			label: r####"h1/supplement/moved-vec-to-str"####,
			source: r####"pub fn main(){let v=[1];drop(v);h1::greet(v)}"####,
			mode: "normal",
			policy: "stock-first-pending",
			stock: true,
		},
		Case {
			label: r####"h1/supplement/early-bool-before-moved-string"####,
			source: r####"pub fn main(){let s="x";drop(s);h1::mix3(7,8,s)}"####,
			mode: "normal",
			policy: "stock-first-pending",
			stock: true,
		},
		Case {
			label: "h1/supplement/coercion-before-moved-string",
			source: r####"pub fn main(){let s="x";drop(s);h1::coercion_pair(9223372036854775808u64,s)}"####,
			mode: "normal",
			policy: "stock-first-pending",
			stock: true,
		},
	];
	let expected = include_str!("host_bridge/precedence.tsv");
	let rows: Vec<_> = expected.lines().skip(1).collect();
	assert_eq!(rows.len(), 4);
	let baseline = if replay {
		Some(PathBuf::from(
			std::env::var_os("H1_BASELINE").expect("original supplement origins"),
		))
	} else {
		None
	};
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-h1-supplement-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let started = Instant::now();
	let mut table = String::from("case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\ttyped_hex\teffects_hex\tstable_error_hex\tdiagnostics_hex\tpolicy\n");
	for (n, case) in cases.iter().enumerate() {
		assert!(started.elapsed() < Duration::from_secs(60));
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, case.source).unwrap();
		let origin = baseline
			.as_ref()
			.map(|p| p.join(format!("{id}.source.rn")))
			.unwrap_or_else(|| path.clone());
		assert_eq!(fs::read_to_string(&origin).unwrap(), case.source);
		for ext in ["typed", "effects", "error", "diagnostics", "stable-error"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{ext}")));
		}
		fs::write(out.join(format!("{id}.argv")), format!("{:?} --exact {CHILD} --ignored --nocapture --quiet\nGAP_CASE={path:?}\nGAP_ORIGIN={origin:?}\nGAP_MODE=h1-normal\nGAP_ENGINE=stock\nGAP_RESULT={:?}\nstdin=/dev/null\n", std::env::current_exe().unwrap(), scratch.0.join("result"))).unwrap();
		let run = launch(&path, &origin, "h1-normal", "stock", &scratch.0, &out, &id);
		assert!(run.complete, "{}: {}", case.label, run.record);
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
		fs::write(
			out.join(format!("{id}.error")),
			fs::read(scratch.0.join("result.error")).unwrap_or_default(),
		)
		.unwrap();
		actual.push(case.policy.to_owned());
		let compared = actual.join("\t");
		fs::write(out.join(format!("{id}.compared")), &compared).unwrap();
		if replay {
			assert_eq!(compared, rows[n], "precedence replay {}", case.label);
		}
		table.push_str(&compared);
		table.push('\n');
		fs::write(out.join("stock-precedence.tsv"), &table).unwrap();
	}
	contract_completed("stock", 4);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
)]
fn fresh_fixture_certifies_only_closed_supported_handlers() {
	let tape = fixture::Recorder::new();
	let context = fixture::fresh_context(&tape);
	for name in [
		"id_unit",
		"id_bool",
		"id_i64",
		"id_u64",
		"id_f64",
		"id_char",
		"greet",
		"unit",
		"add",
		"coercion_pair",
		"mix3",
		"mix4",
		"mark",
		"parse",
		"maybe",
		"f64_bits",
		"sized_string",
		"some_unit",
		"none_unit",
		"ok_unit",
		"err_unit",
		"some_bool",
		"none_bool",
		"ok_bool",
		"err_bool",
		"some_i64",
		"none_i64",
		"ok_i64",
		"err_i64",
		"some_u64",
		"none_u64",
		"ok_u64",
		"err_u64",
		"some_f64",
		"none_f64",
		"ok_f64",
		"err_f64",
		"some_char",
		"none_char",
		"ok_char",
		"err_char",
		"some_string",
		"none_string",
		"ok_string",
		"err_string",
		"mixed_error",
		"rust_panic",
	] {
		let hash = Hash::type_hash(["h1", name]);
		assert!(
			context.fresh_host(hash).is_some(),
			"missing enrollment {name}"
		);
	}
	for name in [
		"plain_unenrolled",
		"owned_string",
		"mut_str",
		"vec_input",
		"needs_environment",
		"async_creation",
		"raw_unknown",
	] {
		assert!(
			context.fresh_host(Hash::type_hash(["h1", name])).is_none(),
			"untrusted enrollment {name}"
		);
	}
	assert!(context
		.fresh_host(Hash::type_hash(["h1", "vm_error_control"]))
		.is_some());
	assert_eq!(tape.snapshot(), (0, Vec::new()));
}
