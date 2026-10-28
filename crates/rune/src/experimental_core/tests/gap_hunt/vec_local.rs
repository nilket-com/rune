//! C16a test-only Vec local stock oracle. No production admission changes.
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
				_ => panic!("C16a subject returned non-scalar"),
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
#[ignore = "C16a oracle child; external bounded controller only"]
fn child() {
	let rows: Json = serde_json::from_str(include_str!("vec_local/cases.json")).unwrap();
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
#[ignore = "C16a strict execution contract against live stock; bounded children and exclusive GAP_OUT"]
fn contract() {
	let cases: Json = serde_json::from_str(include_str!("vec_local/cases.json")).unwrap();
	let dispositions: Json =
		serde_json::from_str(include_str!("vec_local/dispositions.json")).unwrap();
	let c16e_admissions: Vec<Json> =
		serde_json::from_str(include_str!("vec_local/c16e_admissions.json")).unwrap();
	assert_eq!(c16e_admissions.len(), 5);
	let cases = cases.as_array().unwrap();
	assert_eq!(cases.len(), 92);
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
		let observed = &fresh.observation;
		let audit = &observed["fresh_audit"];
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
		let outcome = &observed["outcome"];
		if let Some(rule) = dispositions.get(label) {
			use sha2::{Digest, Sha256};
			assert_eq!(rule["ruling"], "01a1511a-f99b-722e-a49a-e7a74e601943");
			assert_eq!(
				rule["source_sha256"],
				hex(&Sha256::digest(case["source"].as_str().unwrap().as_bytes()))
			);
			assert_eq!(audit["invoked"], true, "{label}");
			assert_eq!(project(outcome), rule["fresh"], "{label}");
			if label == "frames/miss" {
				assert_eq!(rule["policy"], "Raw-frame-disposition");
				let trace = |folder: &Path| -> Json {
					serde_json::from_slice(&fs::read(folder.join("logical.json")).unwrap()).unwrap()
				};
				assert_eq!(
					trace(&folder.join("fresh")),
					trace(&folder.join("stock")),
					"{label}: logical frames/context/rendered"
				);
			} else {
				assert_eq!(label, "value/return-rhs");
				assert_eq!(rule["policy"], "Intentional-divergence");
				assert_eq!(expected["outcome"]["status"], "compile-error");
			}
		} else if expected["outcome"]["status"] == "compile-error" {
			assert_eq!(outcome["status"], "compile-error", "{label}");
			assert_eq!(
				outcome["diagnostics"], expected["outcome"]["diagnostics"],
				"{label}"
			);
			assert_eq!(audit["invoked"], false, "{label}");
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
			assert_eq!(project(outcome), expected["projected"], "{label}");
		} else if case["policy"] == "Retain-refusal" && label != "excluded/for" {
			assert_eq!(outcome["status"], "refused", "{label}");
			assert_eq!(outcome["refusal"], "Pattern", "{label}");
			assert_eq!(audit["invoked"], false, "{label}");
			let at = &outcome["location"];
			let source = case["source"].as_str().unwrap();
			let start = at["start"].as_u64().unwrap() as usize;
			let end = at["end"].as_u64().unwrap() as usize;
			assert!(!source.get(start..end).unwrap().is_empty(), "{label}");
		} else {
			assert_eq!(audit["invoked"], true, "{label}");
			assert_eq!(project(outcome), expected["projected"], "{label}");
		}
		assert_eq!(hex(&fresh.stdout), hex(&stock.stdout), "stdout: {label}");
		assert_eq!(hex(&fresh.stderr), hex(&stock.stderr), "stderr: {label}");
	}
	super::contract_completed("fresh", cases.len());
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, case: &Json, folder: &Path) -> super::live_oracle::Run {
	let index_text = index.to_string();
	let trace = folder.join("logical.json");
	let run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::vec_local::child",
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

#[test]
fn vec_local_patterns_preserve_aliases_failures_and_store_reuse() {
	let cases: Json = serde_json::from_str(include_str!("vec_local/cases.json")).unwrap();
	let selected = [0usize, 5, 7, 8, 9, 13, 18, 19, 26, 29, 30, 33];
	assert_eq!(selected.len(), 12);
	for index in selected {
		let source = cases[index]["source"].as_str().unwrap();
		let (actual, audit) = observe(source, "c8-main");
		assert_eq!(
			project(&actual),
			project(&legacy_observe(source, "c8-main"))
		);
		for key in [
			"no_cleanup_error",
			"clean_after_row",
			"row_idle",
			"reuse",
			"idle",
		] {
			assert_eq!(audit[key], true, "{index}: {key}");
		}
		for key in [
			"row_live_entries",
			"row_payload_bytes",
			"live_entries",
			"payload_bytes",
		] {
			assert_eq!(audit[key], 0, "{index}: {key}");
		}
	}
	println!("C16a Vec local Miri rows 12");
}
