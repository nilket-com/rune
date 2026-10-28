// C7 strict projection of both engines: no address exemptions or retained-frame adapters.
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
		_ => panic!("unreviewed status"),
	}
}
#[test]
#[cfg(target_os = "linux")]
#[ignore = "C7 strict execution contract against live stock; explicit unique GAP_OUT, bounded children"]
fn contract() {
	assert_eq!(CASES.len(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, &(label, source, mode, policy)) in CASES.iter().enumerate() {
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, &folder.join("stock"));
		let expected = project(&stock.observation["outcome"]);
		fs::write(
			folder.join("expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		let fresh = launch("fresh", i, &folder.join("fresh"));
		let (e, stdout, stderr) = (&fresh.observation, &fresh.stdout, &fresh.stderr);
		let category = {
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
					"failure_at_get",
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
			let (entries, bytes) = if mode == "c7-small-store" {
				(2, 4096)
			} else {
				(4096, 4 << 20)
			};
			assert_eq!(audit["limits_entries"], entries);
			assert_eq!(audit["limits_bytes"], bytes);
			assert_eq!(audit["reuse_raw"], "Some(I64(7))");
			audit["call_chain"].as_array().unwrap();
			assert_ne!(e["outcome"]["class"], "UNMAPPED");
			let mut category = policy;
			match policy {
				"Refused-ByteStr" => {
					let o = &e["outcome"];
					exact_keys(
						o.as_object().unwrap(),
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
					assert_eq!(o["status"], "refused");
					assert_eq!(o["kind"], "Unsupported");
					let refusal = if policy == "Refused-Range" {
						"Range"
					} else {
						"ByteStr"
					};
					assert_eq!(o["refusal"], refusal);
					let (_, start, end) = REFUSAL_SITES
						.iter()
						.find(|(name, _, _)| *name == label)
						.unwrap();
					// These explicit source-bound controls cover the entire indexed
					// expression (Range) or literal (ByteStr), with no exemption.
					if refusal == "Range" {
						assert!(
							source[*start..*end].starts_with("v[")
								&& source[*start..*end].ends_with(']')
						);
					} else {
						assert_eq!(&source[*start..*end], "b\"abc\"");
					}
					assert_eq!(o["location"], json!({"source":0,"start":start,"end":end}));
					assert_eq!(audit["invoked"], false);
					assert_eq!(audit["returned_root"], false);
					assert!(audit["typed_failure_kind"].is_null());
					assert!(audit["call_chain"].as_array().unwrap().is_empty());
					assert_eq!(stdout, b"\nrunning 1 test\n");
					assert!(stderr.is_empty());
				}
				"Retained-resource" => {
					assert_eq!(label, "limits/result-entry");
					assert_eq!(mode, "c7-small-store");
					assert_eq!(
						expected,
						json!({"status":"value","type":"Bool","value":true})
					);
					assert_eq!(audit["invoked"], true);
					assert_eq!(audit["returned_root"], false);
					assert_eq!(audit["typed_failure_kind"], "StoreCapacity");
					assert_eq!(audit["failure_at_get"], true);
					audit["failure_site_ops"].as_array().unwrap();
					assert!(audit["call_chain"].as_array().unwrap().is_empty());
					let got = project(&e["outcome"]);
					fs::write(
						folder.join("compared.json"),
						serde_json::to_vec_pretty(&got).unwrap(),
					)
					.unwrap();
					assert_eq!(got["status"], "error");
					assert_eq!(got["class"], "StoreCapacity");
					assert!(got["chain_displays"].as_array().unwrap().is_empty());
					let start = source.find("v[0..2]").unwrap();
					assert_eq!(
						got["sites"],
						json!([{"source":0,"start":start,"end":start+7,"frames":"[]"}])
					);
					assert_eq!(hex(stdout), hex(&stock.stdout), "stdout {label}");
					assert_eq!(hex(stderr), hex(&stock.stderr), "stderr {label}");
				}
				"Exact" | "Refused-Range" => {
					if policy == "Refused-Range" {
						category = "C12-admitted-range";
					}
					assert_eq!(audit["invoked"], true);
					assert_eq!(audit["returned_root"], expected["status"] == "value");
					let got = project(&e["outcome"]);
					fs::write(
						folder.join("compared.json"),
						serde_json::to_vec_pretty(&got).unwrap(),
					)
					.unwrap();
					// All sites, native chains and frame strings are compared exactly.
					// Any mismatch STOPs here, with raw observations already saved.
					if retained_case(label) {
						category =
							retained_matches(label, source, &expected, &got, &audit["call_chain"])
								.unwrap_or_else(|| {
									panic!(
									"retained pin drift {label}: stock={expected} execution={got}"
								)
								});
					} else {
						assert_eq!(got, expected, "execution {label}");
					}
					assert_eq!(hex(stdout), hex(&stock.stdout), "stdout {label}");
					assert_eq!(hex(stderr), hex(&stock.stderr), "stderr {label}");
				}
				_ => panic!("unknown C7 policy"),
			}
			category
		};
		for run in [&stock, &fresh] {
			assert!(
				!has_address(&String::from_utf8_lossy(&run.stdout))
					&& !has_address(&String::from_utf8_lossy(&run.stderr))
			);
		}
		*policies.entry(category).or_insert(0usize) += 1;
	}
	assert_eq!(
		policies,
		std::collections::BTreeMap::from([
			("Exact", 286),
			("Retained-chain", 17),
			("Retained-frame", 2),
			("Retained-display", 1),
			("C12-admitted-range", 10),
			("Refused-ByteStr", 1),
			("Retained-resource", 1),
		])
	);
	contract_completed("fresh", COUNT);
	fs::write(
		out.join("COMPLETE"),
		format!("C7 fresh {COUNT} rows complete\n"),
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
		"experimental_core::tests::gap_hunt::deferred_index::child",
		label,
		folder,
		"C7_RESULT",
		&[
			("C7_ENGINE", engine.as_ref()),
			("C7_CASE", index_text.as_ref()),
		],
	);
	let e = &mut run.observation;
	e["outcome"] =
		super::d1c_diagnostics::before("deferred_index", label, source, engine, &e["outcome"]);
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

include!("retained.rs");
