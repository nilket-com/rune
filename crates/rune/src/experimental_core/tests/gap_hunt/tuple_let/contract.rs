// C8 strict projection of both engines: no address exemptions or retained-frame adapters.
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
#[cfg(target_os = "linux")]
#[ignore = "C8 strict execution contract against live stock; bounded children and exclusive GAP_OUT"]
fn contract() {
	assert_eq!(CASES.len(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let c16e: Json = serde_json::from_str(include_str!("c16e_admissions.json")).unwrap();
	assert_eq!(c16e.as_object().unwrap().len(), 9);
	let manifest: Json =
		serde_json::from_str(include_str!("../../tuple_let_retained.json")).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, &(label, source, mode, policy)) in CASES.iter().enumerate() {
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, &folder.join("stock"));
		let oracle_expected = project(&stock.observation["outcome"]);
		fs::write(
			folder.join("stock-expected.json"),
			serde_json::to_vec_pretty(&oracle_expected).unwrap(),
		)
		.unwrap();
		let c16e_admission = c16e.get(label);
		if let Some(rule) = c16e_admission {
			use sha2::{Digest, Sha256};
			assert_eq!(rule["index"], i);
			assert_eq!(rule["previous_policy"], policy);
			assert_eq!(rule["policy"], "Exact");
			assert_eq!(
				rule["source_sha256"],
				hex(&Sha256::digest(source.as_bytes()))
			);
		}
		let expected = match policy {
			"Intentional-divergence" => {
				let (expected, ruling) = intentional_expectation(label);
				check_intentional(label, source, &oracle_expected, &expected, &ruling);
				fs::write(
					folder.join("ruling.json"),
					serde_json::to_vec_pretty(&ruling).unwrap(),
				)
				.unwrap();
				expected
			}
			"Exact" | "Depth-audit" => oracle_expected.clone(),
			// C12, C16a, C16b and C16e admitted every former refusal; each now
			// owes the exact stock result.
			"Refused-Pattern" | "Refused-Range" => {
				assert!(
					c16e_admission.is_some()
						|| policy == "Refused-Range"
						|| matches!(
							label,
							"refusal/vec-pattern" | "refusal/for-pair" | "refusal/for-nested"
						),
					"unadmitted refusal {label}"
				);
				oracle_expected.clone()
			}
			_ => panic!("unreviewed C8 policy"),
		};
		fs::write(
			folder.join("execution-expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		let fresh = launch("fresh", i, &folder.join("fresh"));
		let e = &fresh.observation;
		let got = {
			let audit = &e["fresh_audit"];
			exact_keys(
				audit.as_object().unwrap(),
				&[
					"no_cleanup_error",
					"clean_after_row",
					"row_live_entries",
					"row_payload_bytes",
					"row_idle",
					"reuse",
					"reuse_raw",
					"live_entries",
					"payload_bytes",
					"idle",
					"call_chain",
					"invoked",
					"returned_root",
					"typed_failure_kind",
					"failure_site_ops",
					"limits_entries",
					"limits_bytes",
				],
			);
			for k in [
				"no_cleanup_error",
				"clean_after_row",
				"row_idle",
				"reuse",
				"idle",
			] {
				assert_eq!(audit[k], true, "audit {label} {k}");
			}
			for k in [
				"row_live_entries",
				"row_payload_bytes",
				"live_entries",
				"payload_bytes",
			] {
				assert_eq!(audit[k], 0, "audit {label} {k}");
			}
			let (entries, bytes) = if mode == "c8-small-store" {
				(3, 4096)
			} else {
				(4096, 4 << 20)
			};
			assert_eq!(audit["limits_entries"], entries);
			assert_eq!(audit["limits_bytes"], bytes);
			assert_eq!(audit["reuse_raw"], "Some(I64(7))");
			audit["call_chain"].as_array().unwrap();
			audit["failure_site_ops"].as_array().unwrap();
			assert_ne!(e["outcome"]["class"], "UNMAPPED");
			if expected["status"] == "refused" || matches!(label, "depth/128" | "depth/129") {
				assert_eq!(audit["invoked"], false);
				assert_eq!(audit["returned_root"], false);
				assert!(audit["typed_failure_kind"].is_null());
				assert!(audit["call_chain"].as_array().unwrap().is_empty());
				refusal_projection(source, &e["outcome"], audit)
			} else {
				assert_eq!(
					audit["invoked"], true,
					"C8 compile/depth refusal STOP {label}"
				);
				assert_eq!(audit["returned_root"], expected["status"] == "value");
				project(&e["outcome"])
			}
		};
		fs::write(
			folder.join("compared.json"),
			serde_json::to_vec_pretty(&got).unwrap(),
		)
		.unwrap();
		let expected = retained_expected(
			&manifest,
			label,
			source,
			&oracle_expected,
			&expected,
			&e["fresh_audit"],
			&e["outcome"],
		);
		fs::write(
			folder.join("effective-expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		assert_eq!(got, expected, "C8 strict fresh {label}");
		assert_eq!(hex(&fresh.stdout), hex(&stock.stdout), "stdout {label}");
		assert_eq!(hex(&fresh.stderr), hex(&stock.stderr), "stderr {label}");
		for run in [&stock, &fresh] {
			assert!(
				!has_address(&String::from_utf8_lossy(&run.stdout))
					&& !has_address(&String::from_utf8_lossy(&run.stderr))
			);
		}
		*policies.entry(policy).or_insert(0usize) += 1;
	}
	assert_eq!(
		policies,
		std::collections::BTreeMap::from([
			("Exact", 74),
			("Refused-Pattern", 12),
			("Refused-Range", 2),
			("Intentional-divergence", 2),
			("Depth-audit", 8)
		])
	);
	contract_completed("fresh", COUNT);
	fs::write(
		out.join("COMPLETE"),
		format!("C8 fresh {COUNT} rows complete\n"),
	)
	.unwrap();
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
/// The returned outcome has the D1c display mapping applied.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, folder: &Path) -> super::live_oracle::Run {
	let (label, source, mode, policy) = CASES[index];
	let index_text = index.to_string();
	let mut run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::tuple_let::child",
		label,
		folder,
		"C8_RESULT",
		&[
			("C8_ENGINE", engine.as_ref()),
			("C8_CASE", index_text.as_ref()),
		],
	);
	let e = &mut run.observation;
	e["outcome"] =
		super::d1c_diagnostics::before("tuple_let", label, source, engine, &e["outcome"]);
	let keys = [
		"schema",
		"engine",
		"index",
		"label",
		"mode",
		"policy",
		"source_hex",
		"fuel",
		"outcome",
		"fresh_audit",
	];
	// Only the execution engine reports an audit.
	let keys = &keys[..keys.len() - usize::from(engine == "stock")];
	exact_keys(e.as_object().unwrap(), keys);
	assert_eq!(e["engine"], engine);
	assert_eq!(e["schema"], 1);
	assert_eq!(e["index"], index);
	assert_eq!(e["label"], label);
	assert_eq!(e["mode"], mode);
	assert_eq!(e["policy"], policy);
	assert_eq!(e["source_hex"], hex(source.as_bytes()));
	assert_eq!(e["fuel"], FUEL);
	run
}

fn refusal_projection(source: &str, outcome: &Json, audit: &Json) -> Json {
	exact_keys(
		outcome.as_object().unwrap(),
		&[
			"status",
			"kind",
			"error",
			"refusal",
			"diagnostics",
			"location",
			"operation",
		],
	);
	assert_eq!(outcome["status"], "refused");
	let at = &outcome["location"];
	exact_keys(at.as_object().unwrap(), &["source", "start", "end"]);
	let start = at["start"].as_u64().unwrap() as usize;
	let end = at["end"].as_u64().unwrap() as usize;
	let span = source.as_bytes().get(start..end).unwrap();
	json!({"status":outcome["status"],"kind":outcome["kind"],"refusal":outcome["refusal"],"source":at["source"],"start":start,"end":end,"span_hex":hex(span),"invoked":audit["invoked"]})
}
