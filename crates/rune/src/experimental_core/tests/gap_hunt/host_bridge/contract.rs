//! Closed comparison schemas. No expected bytes are used by the new observer.
use super::*;

fn selected_primary(primary: &str) -> Option<&'static str> {
	match primary {
		"ExpectedNumber" => Some("ExpectedNumber"),
		"DivideByZero" => Some("DivideByZero"),
		"ExpectedType" => Some("ExpectedType"),
		"AccessError { error: AccessError { kind: NotAccessibleRef(Snapshot(M-000000)) } }" => {
			Some("AccessErrorMoved")
		}
		"Panic { reason: Panic { inner: \"h1 vm fault\" } }" => Some("Panic"),
		_ => {
			let payload = primary
				.strip_prefix("ValueToIntegerCoercionError { from: ")?
				.strip_suffix(" }")?;
			let (from, to) = payload.split_once(", to: ")?;
			if !matches!(to, "\"i64\"" | "\"u64\"") {
				return None;
			}
			if let Some(v) = from
				.strip_prefix("Signed(")
				.and_then(|v| v.strip_suffix(')'))
			{
				let _: i64 = v.parse().ok()?;
			} else {
				let v = from
					.strip_prefix("Unsigned(")
					.and_then(|v| v.strip_suffix(')'))?;
				let _: u64 = v.parse().ok()?;
			}
			Some("ValueToIntegerCoercionError")
		}
	}
}

fn runtime_error_semantic(record: &str) -> Option<String> {
	let mut lines = record.lines();
	let primary = selected_primary(lines.next()?.strip_prefix("primary:")?)?;
	let chain = lines.next()?.strip_prefix("chain:")?;
	let (arg, aux) = match chain {
		"[]" => ("None", String::new()),
		"[VmErrorAt { index: 0, kind: BadArgument { arg: 0 } }]" => {
			("Some(0)", hex(b"Bad argument #0"))
		}
		"[VmErrorAt { index: 0, kind: BadArgument { arg: 1 } }]" => {
			("Some(1)", hex(b"Bad argument #1"))
		}
		_ => return None,
	};
	if lines.next()? != format!("auxiliary:{aux}") {
		return None;
	}
	let site: Vec<_> = lines
		.next()?
		.strip_prefix("location:")?
		.split(':')
		.collect();
	if site.len() != 4 {
		return None;
	}
	for part in &site {
		let _: usize = part.parse().ok()?;
	}
	if lines.next()? != "frames:[]" || lines.next().is_some() {
		return None;
	}
	Some(format!(
		"primary:{primary}\narg:{arg}\nsite:{}:{}:{}\nframes:[]\n",
		site[1], site[2], site[3]
	))
}

// Exact source SHA256: 46f7b713d7e412b9c75c2259406cee29c5609289fa9e381327f0665d35be4c7a.
// Literal byte equality below binds that one approved source, never an arity class.
fn retained_unit_arity(label: &str, source_hex: &str, observation: &str, stable: &str) -> bool {
	label == "h1/wrong/unit/7"
		&& source_hex == hex(b"pub fn main(){h1::unit(7)}")
		&& observation == format!("error\tArgumentCount\t{}\t{}\t", hex(b"stock runtime"),hex(b"Wrong number of arguments 1, expected 0"))
		&& stable == "primary:BadArgumentCount { actual: 1, expected: 0 }\nchain:[]\nauxiliary:\nlocation:2:0:14:25\nframes:[]\n"
}

#[test]
#[cfg_attr(
	miri,
	ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
)]
fn native_argument_failure_matches_stock_kind_message_site_and_effects() {
	use crate::experimental_core::error::ErrorKind;
	let case = CASES
		.iter()
		.find(|c| c.label == "h1/flow/argument-error-before-host")
		.unwrap();
	let source = case.source;
	assert_eq!(source, "pub fn main(){h1::add(h1::mark(\"left\",7),1/0)}");
	// The legacy engine's observation of the same source, in process.
	let legacy_tape = fixture::Recorder::new();
	let legacy = fixture::legacy_context(&legacy_tape);
	let mut legacy_sources = Sources::new();
	legacy_sources
		.insert(Source::memory(source).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut legacy_sources)
		.with_context(&legacy)
		.build_vm()
		.unwrap();
	let legacy_error = crate::runtime::budget::with(FUEL, || vm.call(["main"], ()))
		.call()
		.unwrap_err();
	let (_, stable) = super::legacy_error_metadata(&legacy_error);
	let legacy_effects = {
		let (calls, events) = legacy_tape.snapshot();
		let mut effects = format!("calls:{calls}\n");
		for event in events {
			effects.push_str(&format!("event:{}\n", hex(event.as_bytes())));
		}
		effects
	};
	let tape = fixture::Recorder::new();
	let context = fixture::context(&tape);
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 1).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let failure = executor
		.invoke(&program, &mut store, main, &[], FUEL)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
	assert_eq!(
		failure.message(&program).to_string(),
		"Division by zero in `i64` division (`/`)"
	);
	assert_eq!(legacy_error.error().kind().to_string(), "Division by zero");
	let site = failure.error.location.unwrap();
	let projected = runtime_error_semantic(&stable).unwrap();
	assert!(projected.contains(&format!(
		"site:{}:{}:{}\n",
		site.source, site.start, site.end
	)));
	assert!(failure.call_chain.is_empty());
	let (calls, events) = tape.snapshot();
	assert_eq!(calls, 1);
	assert_eq!(events, ["mark", "left"]);
	let mut effects = format!("calls:{calls}\n");
	for event in events {
		effects.push_str(&format!("event:{}\n", hex(event.as_bytes())));
	}
	assert_eq!(effects, legacy_effects);
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
	assert!(executor.is_idle());
}

#[test]
fn retained_unit_arity_is_one_label_source_and_stock_schema() {
	let source = hex(b"pub fn main(){h1::unit(7)}");
	let observation = format!(
		"error\tArgumentCount\t{}\t{}\t",
		hex(b"stock runtime"),
		hex(b"Wrong number of arguments 1, expected 0")
	);
	let stable = "primary:BadArgumentCount { actual: 1, expected: 0 }\nchain:[]\nauxiliary:\nlocation:2:0:14:25\nframes:[]\n";
	assert!(retained_unit_arity(
		"h1/wrong/unit/7",
		&source,
		&observation,
		stable
	));
	assert!(!retained_unit_arity("other", &source, &observation, stable));
	assert!(!retained_unit_arity(
		"h1/wrong/unit/7",
		&hex(b"pub fn main(){h1::unit(8)}"),
		&observation,
		stable
	));
	assert!(!retained_unit_arity(
		"h1/wrong/unit/7",
		&source,
		&observation.replace("ArgumentCount", "Type"),
		stable
	));
	assert!(!retained_unit_arity(
		"h1/wrong/unit/7",
		&source,
		&observation,
		&stable.replace("actual: 1", "actual: 2")
	));
}

#[test]
fn semantic_schema_refuses_unreviewed_payloads_and_auxiliary_chains() {
	assert!(selected_primary("ExpectedNumber").is_some());
	assert!(selected_primary(
		"ValueToIntegerCoercionError { from: Unsigned(9223372036854775808), to: \"i64\" }"
	)
	.is_some());
	for bad in [
		"Other",
		"AccessError { unknown }",
		"ValueToIntegerCoercionError { from: Float(1), to: \"i64\" }",
		"ValueToIntegerCoercionError { from: Signed(1), to: \"i8\" }",
		"ValueToIntegerCoercionError { from: Unsigned(-1), to: \"i64\" }",
	] {
		assert!(selected_primary(bad).is_none());
	}
	let good = "primary:ExpectedType\nchain:[VmErrorAt { index: 0, kind: BadArgument { arg: 0 } }]\nauxiliary:42616420617267756d656e74202330\nlocation:9:0:1:2\nframes:[]\n";
	assert!(runtime_error_semantic(good).is_some());
	for bad in [
		good.replace("index: 0", "index: 1"),
		good.replace("frames:[]", "frames:[9]"),
		good.replace("location:9:0:1:2", "location:x:0:1:2"),
		good.replace("ExpectedType", "Unknown"),
		good.replace("auxiliary:4261", "auxiliary:00"),
	] {
		assert!(runtime_error_semantic(&bad).is_none());
	}
}

#[test]
#[ignore = "ONE bounded execution H1 144-row contract against live stock"]
fn contract() {
	let precedence = precedence_cases();
	let cases: Vec<&Case> = CASES
		.iter()
		.filter(|c| c.stock)
		.chain(&precedence)
		.collect();
	assert_eq!(cases.len(), 144);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-h1-execution-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let nested = cases.iter().find(|c| c.label == "h1/flow/nested").unwrap();
	let path = out.join("full.source.rn");
	fs::write(&path, nested.source).unwrap();
	let full = stock_row(nested, &path, &scratch.0, &out, "full");
	let full: Vec<&str> = full.iter().map(String::as_str).collect();
	let started = Instant::now();
	let mut counts = std::collections::BTreeMap::new();
	for (n, case) in cases.iter().enumerate() {
		assert!(
			started.elapsed() < Duration::from_secs(600),
			"aggregate cap"
		);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		let source = case.source;
		fs::write(&path, source).unwrap();
		// Stock's numeric fuel is not compared: fuel rows owe the full-fuel result.
		let fuel = case.policy == "source-only-fuel-no-numeric-parity";
		let row = if fuel {
			assert_eq!(source, nested.source, "fuel source identity");
			full.iter().map(|s| (*s).to_owned()).collect()
		} else {
			stock_row(case, &path, &scratch.0, &out, &id)
		};
		let expected: Vec<&str> = row.iter().map(String::as_str).collect();
		for ext in ["typed", "effects", "error", "diagnostics", "semantic-error"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{ext}")));
		}
		// Every stock-compared row runs at full new fuel. The separate sweep is new-only.
		fs::write(out.join(format!("{id}.argv")),format!("{:?} --exact {CHILD} --ignored --nocapture --quiet\nGAP_CASE={path:?}\nGAP_ORIGIN={path:?}\nGAP_MODE=h1-normal\nGAP_ENGINE=execution\nGAP_RESULT={:?}\nstdin=/dev/null\n",std::env::current_exe().unwrap(),scratch.0.join("result"))).unwrap();
		let run = launch(&path, &path, "h1-normal", "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{}: {}", case.label, run.record);
		fs::write(out.join(format!("{id}.rc")), "0\n").unwrap();
		let mut sides = std::collections::BTreeMap::new();
		for ext in ["typed", "effects", "error", "diagnostics", "semantic-error"] {
			let value =
				fs::read_to_string(scratch.0.join(format!("result.{ext}"))).unwrap_or_default();
			assert!(value.len() <= CAP);
			fs::write(out.join(format!("{id}.{ext}")), &value).unwrap();
			sides.insert(ext, value);
		}
		let actual: Vec<_> = run.record.split('\t').collect();
		let oracle_observation = unhex(expected[3]);
		let stock: Vec<_> = oracle_observation.split('\t').collect();
		assert_eq!(actual.len(), 5);
		assert_eq!(stock.len(), 5);
		let policy = if case.label == "h1/wrong/unit/7" {
			assert_eq!(case.policy, "stock-kind-map-pending");
			assert!(retained_unit_arity(
				case.label,
				expected[2],
				&oracle_observation,
				&unhex(expected[8])
			));
			"compile-refusal-Arity-vs-stock-error"
		} else {
			case.policy
		};
		*counts.entry(policy).or_insert(0usize) += 1;
		match policy {
			"compile-refusal-Arity-vs-stock-error" | "fresh-Native-refusal-no-call" => {
				assert_eq!(actual[0], "refused", "{}", case.label);
				assert_eq!(
					actual[1],
					if policy == "compile-refusal-Arity-vs-stock-error" {
						"Some(Arity)"
					} else {
						"Some(Native)"
					},
					"{}",
					case.label
				);
				assert_eq!(sides["effects"], "calls:0\n");
			}
			"candidate-exact"
			| "exact-float-bits"
			| "stock-kind-map-pending"
			| "stock-first-pending"
			| "source-only-fuel-no-numeric-parity"
			| "test-only-VM-error-class" => {
				if stock[0] == "compile-error" {
					assert_eq!(actual[0], "refused");
					assert_eq!(
						sides["diagnostics"],
						unhex(expected[9]),
						"shared frontend diagnostics"
					);
				} else {
					assert_eq!(actual[0], stock[0], "{}", case.label);
					if policy != "test-only-VM-error-class" {
						assert_eq!(actual[1], stock[1], "{}", case.label);
					} else {
						assert_eq!(actual[1], "Panic");
					}
					assert_eq!(actual[3..], stock[3..], "message/value {}", case.label);
					assert_eq!(sides["typed"], unhex(expected[6]), "typed {}", case.label);
					assert_eq!(
						sides["diagnostics"],
						unhex(expected[9]),
						"diagnostics {}",
						case.label
					);
					if stock[0] == "error" {
						let projected = runtime_error_semantic(&unhex(expected[8]))
							.expect("closed frozen error schema");
						assert_eq!(
							sides["semantic-error"], projected,
							"semantic error {}",
							case.label
						);
					}
				}
				assert_eq!(
					sides["effects"],
					unhex(expected[7]),
					"effects {}",
					case.label
				);
				assert_eq!(
					run.stdout,
					unhex(expected[4]).as_bytes(),
					"stdout {}",
					case.label
				);
				assert_eq!(
					run.stderr,
					unhex(expected[5]).as_bytes(),
					"stderr {}",
					case.label
				);
			}
			"unwind-boundary-disposition-pending" => {
				assert_eq!(actual[0], "error");
				assert_eq!(actual[1], "HostPanicked");
				assert_eq!(unhex(actual[3]), "host panicked: h1 rust panic");
				assert_eq!(sides["effects"], unhex(expected[7]));
				assert_eq!(run.stdout, unhex(expected[4]).as_bytes());
				assert_eq!(run.stderr, unhex(expected[5]).as_bytes());
			}
			_ => panic!("unreviewed policy {policy}"),
		}
		fs::write(out.join(format!("{id}.PASS")), case.label).unwrap();
	}
	for (name, count) in [
		("candidate-exact", 71),
		("exact-float-bits", 7),
		("stock-kind-map-pending", 29),
		("stock-first-pending", 4),
		("source-only-fuel-no-numeric-parity", 17),
		("compile-refusal-Arity-vs-stock-error", 7),
		("fresh-Native-refusal-no-call", 7),
		("test-only-VM-error-class", 1),
		("unwind-boundary-disposition-pending", 1),
	] {
		assert_eq!(counts.get(name), Some(&count));
	}
	contract_completed("fresh", 144);
}
