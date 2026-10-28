//! C16c expression-call oracle and bounded stock/new contracts.
use super::*;
use serde_json::{json, Value as Json};
include!("tuple_let/observe.rs");
fn legacy_observe(source: &str, mode: &str) -> Json {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c8.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let built = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm();
	let mut vm = match built {
		Ok(vm) => vm,
		Err(e) => {
			return json!({"status":"compile-error","error":format!("{e:?}"),"diagnostics":format!("{:?}",diagnostics.diagnostics())})
		}
	};
	assert!(matches!(mode, "c8-main" | "c8-small-store"));
	let result = crate::runtime::budget::with(FUEL, || vm.call(["main"], ())).call();
	match result {
		Ok(value) => {
			if value.into_unit().is_ok() {
				return json!({"status":"value","type":"Unit"});
			}
			use crate::runtime::Inline;
			match value.as_inline() {
				Some(Inline::Float(n)) => {
					json!({"status":"value","type":"F64","bits":format!("{:016x}",n.to_bits())})
				}
				Some(Inline::Signed(n)) => {
					json!({"status":"value","type":"I64","value":n.to_string()})
				}
				Some(Inline::Unsigned(n)) => {
					json!({"status":"value","type":"U64","value":n.to_string()})
				}
				Some(Inline::Bool(n)) => json!({"status":"value","type":"Bool","value":n}),
				_ => panic!("C16c subject returned non-scalar"),
			}
		}
		Err(e) => {
			super::d1e_diagnostics::stock(&e, &sources);
			// Capture metadata directly inside the crate, no parsing Display/Debug.
			let kind = e.error().kind();
			let class = match kind {
				crate::runtime::VmErrorKind::Panic { .. } => "Panic".into(),
				crate::runtime::VmErrorKind::UnsupportedCallFn { .. } => "Type".into(),
				_ => runtime_error_class(kind),
			};
			let chain = e
				.chain()
				.iter()
				.map(|at| json!({"kind":format!("{:?}",at.kind()),"display":at.to_string()}))
				.collect::<Vec<_>>();
			let sites = e.stacktrace().iter().map(|at| {
				let inst = at.unit.debug_info().unwrap().instruction_at(at.ip).unwrap();
				let range = inst.span.range();
				json!({"source":inst.source_id.into_index(),"start":range.start,"end":range.end,"frames":format!("{:?}",at.frames)})
			}).collect::<Vec<_>>();
			json!({"status":"error","class":class,"kind":format!("{kind:?}"),"display":super::d1c_diagnostics::runtime_error_display(&e),"chain":chain,"sites":sites,"raw":format!("{e:?}")})
		}
	}
}

fn exact_keys(o: &serde_json::Map<String, Json>, names: &[&str]) {
	assert_eq!(o.len(), names.len());
	for name in names {
		assert!(o.contains_key(*name));
	}
}
fn has_address(text: &str) -> bool {
	text.as_bytes().windows(2).enumerate().any(|(i, p)| {
		p == b"0x"
			&& text.as_bytes()[i + 2..]
				.iter()
				.take_while(|b| b.is_ascii_hexdigit())
				.count() >= 9
	})
}
fn project(o: &Json) -> Json {
	let obj = o.as_object().unwrap();
	match o["status"].as_str().unwrap() {
		"value" => {
			let t = o["type"].as_str().unwrap();
			match t {
				"Unit" => exact_keys(obj, &["status", "type"]),
				"F64" => {
					exact_keys(obj, &["status", "type", "bits"]);
					let b = o["bits"].as_str().unwrap();
					assert_eq!(b.len(), 16);
					assert!(b
						.bytes()
						.all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
				}
				"I64" | "U64" => {
					exact_keys(obj, &["status", "type", "value"]);
					let n = o["value"].as_str().unwrap();
					if t == "I64" {
						n.parse::<i64>().unwrap();
					} else {
						n.parse::<u64>().unwrap();
					}
				}
				"Bool" => {
					exact_keys(obj, &["status", "type", "value"]);
					o["value"].as_bool().unwrap();
				}
				_ => panic!("unmapped scalar"),
			}
			o.clone()
		}
		"error" => {
			exact_keys(
				obj,
				&[
					"status", "class", "kind", "display", "chain", "sites", "raw",
				],
			);
			assert!(!has_address(o["class"].as_str().unwrap()));
			assert!(!has_address(o["display"].as_str().unwrap()));
			let chain = o["chain"]
				.as_array()
				.unwrap()
				.iter()
				.map(|e| {
					exact_keys(e.as_object().unwrap(), &["kind", "display"]);
					assert!(!has_address(e["display"].as_str().unwrap()));
					e["display"].clone()
				})
				.collect::<Vec<_>>();
			for site in o["sites"].as_array().unwrap() {
				exact_keys(
					site.as_object().unwrap(),
					&["source", "start", "end", "frames"],
				);
				for k in ["source", "start", "end"] {
					site[k].as_u64().unwrap();
				}
				assert!(site["start"].as_u64().unwrap() <= site["end"].as_u64().unwrap());
				assert!(!has_address(site["frames"].as_str().unwrap()));
			}
			json!({"status":o["status"],"class":o["class"],"display":o["display"],"chain_displays":chain,"sites":o["sites"]})
		}
		"compile-error" => {
			exact_keys(obj, &["status", "error", "diagnostics"]);
			o.clone()
		}
		_ => panic!("unreviewed status"),
	}
}

#[test]
#[ignore = "C16c oracle child; external bounded controller only"]
fn child() {
	let rows: Json = serde_json::from_str(include_str!("expression_calls/cases.json")).unwrap();
	let index: usize = std::env::var("C16_CASE").unwrap().parse().unwrap();
	let row = &rows[index];
	let source = row["source"].as_str().unwrap();
	let engine = std::env::var("C16_ENGINE").unwrap_or_else(|_| "stock".into());
	let (observation, audit) = match engine.as_str() {
		"stock" => (legacy_observe(source, "c8-main"), None),
		"fresh" => {
			let (observation, audit) = observe(source, "c8-main");
			(observation, Some(audit))
		}
		_ => panic!("unknown C16 engine"),
	};
	let projected = if engine == "stock" {
		project(&observation)
	} else {
		observation.clone()
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":row["label"],"policy":row["policy"],"source":source,"fuel":FUEL,"outcome":observation,"projected":projected,"fresh_audit":audit});
	fs::write(
		std::env::var_os("C16_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
#[test]
#[cfg(target_os = "linux")]
#[ignore = "C16c strict execution contract against live stock; bounded children and exclusive GAP_OUT"]
fn contract() {
	let cases: Json = serde_json::from_str(include_str!("expression_calls/cases.json")).unwrap();
	let disposition: Json =
		serde_json::from_str(include_str!("expression_calls/disposition.json")).unwrap();
	let admissions: Json =
		serde_json::from_str(include_str!("expression_calls/c16f_admissions.json")).unwrap();
	assert_eq!(admissions.as_object().unwrap().len(), 1);
	let c17a: Json =
		serde_json::from_str(include_str!("expression_calls/c17a_admissions.json")).unwrap();
	assert_eq!(c17a.as_object().unwrap().len(), 1);
	let cases = cases.as_array().unwrap();
	assert_eq!(cases.len(), 100);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	for (i, case) in cases.iter().enumerate() {
		let label = case["label"].as_str().unwrap();
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, case, &folder.join("stock"));
		let expected = &stock.observation;
		let fresh = launch("fresh", i, case, &folder.join("fresh"));
		let audit = &fresh.observation["fresh_audit"];
		for key in [
			"no_cleanup_error",
			"clean_after_row",
			"row_idle",
			"reuse",
			"idle",
		] {
			assert_eq!(audit[key], true, "{label}: {key}");
		}
		for key in [
			"row_live_entries",
			"row_payload_bytes",
			"live_entries",
			"payload_bytes",
		] {
			assert_eq!(audit[key], 0, "{label}: {key}");
		}
		let actual = &fresh.observation["outcome"];
		let policy = &disposition[i];
		assert_eq!(policy["index"], i);
		assert_eq!(policy["label"], case["label"]);
		assert_eq!(policy["source_sha256"], case["source_sha256"]);
		if let Some(rule) = c17a.get(format!("{i:03}")) {
			// C17a admits `unwrap`; the frozen disposition still records the refusal.
			assert_eq!(i, 13);
			assert_eq!(rule["classification"], "Exact");
			assert_eq!(rule["source_sha256"], case["source_sha256"]);
			assert_eq!(policy["classification"], "Retain-refusal");
			assert_eq!(policy["refusal"], rule["previous_refusal"]);
			assert_eq!(audit["invoked"], true);
			assert_eq!(project(actual), expected["projected"], "{label}");
		} else if let Some(rule) = admissions.get(format!("{i:03}")) {
			assert_eq!(i, 98);
			assert_eq!(rule["authority"], 21347);
			assert_eq!(rule["classification"], "Exact");
			assert_eq!(rule["source_sha256"], case["source_sha256"]);
			assert_eq!(policy["classification"], "Retain-refusal");
			assert_eq!(policy["refusal"], "UnresolvedFunction");
			assert_eq!(audit["invoked"], true);
			assert_eq!(project(actual), expected["projected"], "{label}");
		} else if policy["classification"] == "Retain-refusal" {
			assert_eq!(actual["status"], "refused", "{label}");
			assert_eq!(actual["refusal"], policy["refusal"], "{label}");
			assert_eq!(actual["location"], policy["location"], "{label}");
			assert_eq!(audit["invoked"], false);
		} else if expected["outcome"]["status"] == "compile-error" {
			assert_eq!(policy["classification"], "Exact");
			assert_eq!(actual["status"], "compile-error", "{label}");
			assert_eq!(
				actual["diagnostics"], expected["outcome"]["diagnostics"],
				"{label}"
			);
			assert_eq!(audit["invoked"], false);
		} else if actual["status"] == "error" {
			assert_eq!(audit["typed_failure_kind"], actual["class"]);
			let actual = project(actual);
			for key in ["status", "class", "display"] {
				assert_eq!(actual[key], expected["projected"][key], "{label}: {key}");
			}
			assert_eq!(policy["classification"], "Exact");
			assert_eq!(
				actual["chain_displays"], expected["projected"]["chain_displays"],
				"{label}"
			);
			let sites = actual["sites"].as_array().unwrap();
			let oracle_sites = expected["projected"]["sites"].as_array().unwrap();
			assert_eq!(sites.len(), oracle_sites.len());
			for (a, b) in sites.iter().zip(oracle_sites) {
				for key in ["source", "start", "end"] {
					assert_eq!(a[key], b[key], "{label}: {key}");
				}
			}
		} else {
			assert_eq!(policy["classification"], "Exact");
			assert_eq!(audit["invoked"], true);
			assert_eq!(project(actual), expected["projected"], "{label}");
		}
		if expected["outcome"]["status"] == "error" {
			assert_eq!(
				trace(&folder.join("fresh")),
				trace(&folder.join("stock")),
				"{label}: logical frames/context/rendered"
			);
		}
		assert_eq!(hex(&fresh.stdout), hex(&stock.stdout), "stdout: {label}");
		assert_eq!(hex(&fresh.stderr), hex(&stock.stderr), "stderr: {label}");
	}
	super::contract_completed("fresh", cases.len());
}

/// Read the logical trace a child wrote to `folder`.
#[cfg(target_os = "linux")]
fn trace(folder: &Path) -> Json {
	serde_json::from_slice(&fs::read(folder.join("logical.json")).unwrap()).unwrap()
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, case: &Json, folder: &Path) -> super::live_oracle::Run {
	let index_text = index.to_string();
	let trace = folder.join("logical.json");
	let run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::expression_calls::child",
		case["label"].as_str().unwrap(),
		folder,
		"C16_RESULT",
		&[
			("C16_ENGINE", engine.as_ref()),
			("C16_CASE", index_text.as_ref()),
			("D1E_TRACE_OUT", trace.as_os_str()),
		],
	);
	let observed = &run.observation;
	assert_eq!(observed["schema"], 1);
	assert_eq!(observed["engine"], engine);
	assert_eq!(observed["index"], index);
	assert_eq!(observed["fuel"], FUEL);
	assert_eq!(observed["label"], case["label"]);
	assert_eq!(observed["source"], case["source"]);
	assert_eq!(observed["policy"], case["policy"]);
	run
}
