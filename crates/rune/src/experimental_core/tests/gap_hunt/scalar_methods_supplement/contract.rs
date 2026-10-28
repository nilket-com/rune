// C6 projection of both engines' observations; semantic missing-method hashes verified.
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
fn check_display(source: &str, text: &str) {
	if let Some(rest) = text.strip_prefix("Missing instance function `") {
		use crate as rune;
		let (hash, rest) = rest.split_once("` for `").unwrap();
		let name = rest.strip_suffix('`').unwrap();
		let ty = match name {
			"std::bool" => Hash::type_hash(crate::item!(::std::bool)),
			"std::string::String" => Hash::type_hash(crate::item!(::std::string::String)),
			"std::vec::Vec" => Hash::type_hash(crate::item!(::std::vec::Vec)),
			"std::option::Option" => Hash::type_hash(crate::item!(::std::option::Option)),
			"std::tuple::Tuple" => Hash::type_hash(crate::item!(::std::tuple::Tuple)),
			"std::u64" => Hash::type_hash(crate::item!(::std::u64)),
			_ => panic!("unverified receiver"),
		};
		let methods = ["abs", "round", "powf"]
			.into_iter()
			.filter(|n| source.contains(&format!(".{n}(")))
			.collect::<Vec<_>>();
		assert_eq!(methods.len(), 1);
		assert_eq!(
			hash,
			Hash::associated_function(ty, Hash::ident(methods[0])).to_string()
		);
	} else {
		assert!(!has_address(text));
	}
}
fn project(source: &str, o: &Json) -> Json {
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
			check_display(source, o["display"].as_str().unwrap());
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
#[ignore = "C6 bounded execution contract against live stock; explicit unique GAP_OUT"]
fn contract() {
	assert_eq!(CASES.len(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, &(label, source, _, policy)) in CASES.iter().enumerate() {
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, &folder.join("stock"));
		let expected = project(source, &stock.observation["outcome"]);
		fs::write(
			folder.join("expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		let fresh = launch("fresh", i, &folder.join("fresh"));
		let (e, stdout, stderr) = (&fresh.observation, &fresh.stdout, &fresh.stderr);
		let audit = &e["fresh_audit"];
		exact_keys(
			audit.as_object().unwrap(),
			&[
				"no_cleanup_error",
				"clean_after_row",
				"reuse",
				"live_entries",
				"payload_bytes",
				"idle",
				"input_f64_bits",
				"call_chain",
				"invoked",
			],
		);
		for k in ["no_cleanup_error", "clean_after_row", "reuse", "idle"] {
			assert_eq!(audit[k], true, "audit {label} {k}");
		}
		assert_eq!(audit["live_entries"], 0);
		assert_eq!(audit["payload_bytes"], 0);
		assert_ne!(e["outcome"]["class"], "UNMAPPED");
		let category = match policy {
			"Retained-arity" => {
				assert_eq!(expected["class"], "ArgumentCount");
				assert_eq!(e["outcome"]["status"], "refused");
				assert_eq!(e["outcome"]["refusal"], "Some(Arity)");
				assert_eq!(audit["invoked"], false);
				assert!(audit["call_chain"].as_array().unwrap().is_empty());
				assert_eq!(stdout, b"\nrunning 1 test\n");
				assert!(stderr.is_empty());
				"Retained-arity"
			}
			"Refused-sibling" => {
				assert_eq!(e["outcome"]["status"], "refused");
				assert_eq!(e["outcome"]["refusal"], "Some(Method)");
				assert_eq!(audit["invoked"], false);
				assert!(audit["call_chain"].as_array().unwrap().is_empty());
				assert_eq!(stdout, b"\nrunning 1 test\n");
				assert!(stderr.is_empty());
				"Refused-sibling"
			}
			"Exact" => {
				assert_eq!(audit["invoked"], true);
				let got = project(source, &e["outcome"]);
				fs::write(
					folder.join("compared.json"),
					serde_json::to_vec_pretty(&got).unwrap(),
				)
				.unwrap();
				let category = if label == "nested/call-error" {
					assert!(retained_frame_matches(
						source,
						&expected,
						&got,
						&audit["call_chain"]
					));
					"Retained-frame"
				} else {
					assert_eq!(got, expected, "execution {label}");
					assert!(audit["call_chain"].as_array().unwrap().is_empty());
					"Exact"
				};
				assert_eq!(hex(stdout), hex(&stock.stdout), "stdout {label}");
				assert_eq!(hex(stderr), hex(&stock.stderr), "stderr {label}");
				category
			}
			_ => panic!("unknown C6 policy"),
		};
		for run in [&stock, &fresh] {
			assert!(
				!has_address(&String::from_utf8_lossy(&run.stdout))
					&& !has_address(&String::from_utf8_lossy(&run.stderr))
			);
		}
		*policies.entry(category).or_insert(0usize) += 1;
	}
	assert_eq!(policies, std::collections::BTreeMap::from([("Exact", 7)]));
	contract_completed("fresh", COUNT);
	fs::write(
		out.join("COMPLETE"),
		format!("C6 fresh {COUNT} rows complete\n"),
	)
	.unwrap();
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
/// The returned outcome has the intentional method-diagnostic mapping applied.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, folder: &Path) -> super::live_oracle::Run {
	let (label, source, mode, policy) = CASES[index];
	let index_text = index.to_string();
	let mut run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::scalar_methods_supplement::child",
		label,
		folder,
		"C6S_RESULT",
		&[
			("C6_ENGINE", engine.as_ref()),
			("C6S_CASE", index_text.as_ref()),
		],
	);
	let e = &mut run.observation;
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
	e["outcome"] = super::method_diagnostics::intentional_json_before(
		"scalar_methods_supplement",
		label,
		source,
		engine,
		&e["outcome"],
	);
	run
}

include!("../scalar_methods/frame_policy.rs");
