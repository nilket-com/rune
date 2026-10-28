//! C16b test-only loop-binder stock oracle. No production admission changes.
use super::*;
use serde_json::{json, Value as Json};
include!("tuple_let/observe.rs");
fn legacy_observe_with_warnings(source: &str, mode: &str) -> Json {
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
	let warnings = crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics);
	let mut observation = (|| {
		let mut vm = match built {
			Ok(vm) => vm,
			Err(e) => {
				return json!({"status":"compile-error","error":format!("{e:?}"),"diagnostics":crate::experimental_core::tests::gap_hunt::nonwarning_diagnostics(&diagnostics),"warnings":crate::experimental_core::tests::gap_hunt::warning_multiset(&diagnostics)})
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
					_ => panic!("C16b subject returned non-scalar"),
				}
			}
			Err(e) => {
				super::d1e_diagnostics::stock(&e, &sources);
				// Capture metadata directly inside the crate, no parsing Display/Debug.
				let kind = e.error().kind();
				let class = match kind {
					crate::runtime::VmErrorKind::Panic { .. } => "Panic".into(),
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
	})();
	observation["warnings"] = json!(warnings);
	observation
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
#[ignore = "C16b oracle child; external bounded controller only"]
fn child() {
	let rows: Json = serde_json::from_str(include_str!("loop_binders/cases.json")).unwrap();
	let index: usize = std::env::var("C16_CASE").unwrap().parse().unwrap();
	let row = &rows[index];
	let source = row["source"].as_str().unwrap();
	let engine = std::env::var("C16_ENGINE").unwrap_or_else(|_| "stock".into());
	let (mut observation, mut audit) = match engine.as_str() {
		"stock" => (legacy_observe_with_warnings(source, "c8-main"), None),
		"fresh" => {
			let (observation, audit) = observe(source, "c8-main");
			(observation, Some(audit))
		}
		_ => panic!("unknown C16 engine"),
	};

	let warnings = if let Some(audit) = &mut audit {
		audit
			.as_object_mut()
			.unwrap()
			.remove("warnings")
			.expect("execution warning multiset")
	} else {
		observation
			.as_object_mut()
			.unwrap()
			.remove("warnings")
			.expect("legacy warning multiset")
	};
	let projected = if engine == "stock" {
		project(&observation)
	} else {
		observation.clone()
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":row["label"],"policy":row["policy"],"source":source,"fuel":FUEL,"warnings":warnings,"outcome":observation,"projected":projected,"fresh_audit":audit});
	fs::write(
		std::env::var_os("C16_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}
/// Cases whose logical trace is compared between engines.
const LOGICAL: [&str; 1] = ["frames/miss"];

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C16b strict execution contract against live stock; bounded children and exclusive GAP_OUT"]
fn contract() {
	let cases: Json = serde_json::from_str(include_str!("loop_binders/cases.json")).unwrap();
	let c16e_admissions: Vec<Json> =
		serde_json::from_str(include_str!("loop_binders/c16e_admissions.json")).unwrap();
	assert_eq!(c16e_admissions.len(), 5);
	let cases = cases.as_array().unwrap();
	assert_eq!(cases.len(), 118);
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
		crate::experimental_core::tests::gap_hunt::assert_warning_multisets(
			&stock.observation,
			&fresh.observation,
		);
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
		if expected["outcome"]["status"] == "compile-error" {
			assert_eq!(actual["status"], "compile-error", "{label}");
			assert_eq!(
				actual["diagnostics"], expected["outcome"]["diagnostics"],
				"{label}"
			);
			assert_eq!(audit["invoked"], false);
		} else if let Some(rule) = c16e_admissions.iter().find(|row| row["label"] == label) {
			use sha2::{Digest, Sha256};
			assert_eq!(rule["index"], i);
			assert_eq!(rule["previous_policy"], case["policy"]);
			assert_eq!(rule["disposition"], "Exact");
			assert_eq!(
				rule["source_sha256"],
				hex(&Sha256::digest(case["source"].as_str().unwrap().as_bytes()))
			);
			assert_eq!(audit["invoked"], true, "{label}");
			assert_eq!(project(actual), expected["projected"], "{label}");
		} else if case["policy"] == "Retain-refusal" {
			assert_eq!(actual["status"], "refused", "{label}");
			assert_eq!(actual["refusal"], "Pattern", "{label}");
			assert_eq!(audit["invoked"], false);
		} else if LOGICAL.contains(&label) {
			let actual = project(actual);
			let expected = &expected["projected"];
			for key in ["status", "class", "display", "chain_displays"] {
				assert_eq!(actual[key], expected[key], "{label}: {key}");
			}
			let sites = actual["sites"].as_array().unwrap();
			let oracle_sites = expected["sites"].as_array().unwrap();
			assert_eq!(sites.len(), oracle_sites.len());
			for (a, b) in sites.iter().zip(oracle_sites) {
				for key in ["source", "start", "end"] {
					assert_eq!(a[key], b[key]);
				}
			}
		} else {
			assert_eq!(audit["invoked"], true, "{label}");
			assert_eq!(project(actual), expected["projected"], "{label}");
		}
		if LOGICAL.contains(&label) {
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
		"experimental_core::tests::gap_hunt::loop_binders::child",
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
