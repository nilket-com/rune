// C5 projection of both engines' observations. Raw Debug metadata is not compared.

fn exact_keys(o: &serde_json::Map<String, Json>, names: &[&str]) {
	assert_eq!(o.len(), names.len(), "observation schema");
	for name in names {
		assert!(o.contains_key(*name), "missing field {name}");
	}
}
fn has_address(text: &str) -> bool {
	let bytes = text.as_bytes();
	bytes.windows(2).enumerate().any(|(i, pair)| {
		pair == b"0x"
			&& bytes[i + 2..]
				.iter()
				.take_while(|b| b.is_ascii_hexdigit())
				.count() >= 9
	})
}
// Uses Rune's existing optional SHA-256 dependency only for this ignored replay.
fn byte_projection(bytes: &[u8]) -> Json {
	assert!(
		!has_address(&String::from_utf8_lossy(bytes)),
		"address in compared bytes"
	);
	if bytes.len() <= 4096 {
		return json!({"bytes_hex":hex(bytes),"bytes":bytes.len()});
	}
	#[cfg(feature = "sha2")]
	{
		use sha2::{Digest, Sha256};
		json!({"byte_len":bytes.len(),"sha256":hex(&Sha256::digest(bytes)),"head_hex":hex(&bytes[..64]),"tail_hex":hex(&bytes[bytes.len()-64..])})
	}
	#[cfg(not(feature = "sha2"))]
	panic!("C5 digest replay requires the existing sha2 feature");
}
fn project(o: &Json) -> Json {
	let obj = o.as_object().unwrap();
	let p = match o["status"].as_str().unwrap() {
		"value" if o["type"] == "Unit" => {
			exact_keys(obj, &["status", "type"]);
			o.clone()
		}
		"value" => {
			exact_keys(obj, &["status", "type", "bytes_hex", "bytes"]);
			assert_eq!(o["type"], "String");
			let text = unhex(o["bytes_hex"].as_str().unwrap());
			assert_eq!(text.len() as u64, o["bytes"].as_u64().unwrap());
			let b = byte_projection(text.as_bytes());
			let mut p = b.as_object().unwrap().clone();
			p.insert("status".into(), json!("value"));
			p.insert("type".into(), json!("String"));
			Json::Object(p)
		}
		"error" => {
			exact_keys(
				obj,
				&[
					"status", "class", "kind", "display", "chain", "sites", "raw",
				],
			);
			let chain = o["chain"]
				.as_array()
				.unwrap()
				.iter()
				.map(|a| {
					exact_keys(a.as_object().unwrap(), &["kind", "display"]);
					a["display"].clone()
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
				assert!(site["end"].as_u64().unwrap() >= site["start"].as_u64().unwrap());
				site["frames"].as_str().unwrap();
			}
			json!({"status":o["status"],"class":o["class"],"display":o["display"],"chain_displays":chain,"sites":o["sites"]})
		}
		"compile-error" => {
			exact_keys(obj, &["status", "error", "diagnostics"]);
			o.clone()
		}
		_ => panic!("unknown observation status"),
	};
	assert!(
		!has_address(&p.to_string()),
		"address in compared projection"
	);
	p
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C5 bounded execution contract against live stock; explicit new GAP_OUT"]
#[allow(clippy::assertions_on_constants)]
fn contract() {
	assert!(
		cfg!(feature = "sha2"),
		"C5 digest projection requires --features sha2"
	);
	assert_eq!(CASES.len(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, &(label, source, mode, policy)) in CASES.iter().enumerate() {
		assert!(labels.insert(label));
		let folder = out.join(format!("{i:03}"));
		let stock = launch("stock", i, &folder.join("stock"));
		let expected = super::d1b_diagnostics::intentional_json_before(
			"format_specs",
			label,
			source,
			"stock",
			&project(&stock.observation["outcome"]),
		);
		let fresh = launch("fresh", i, &folder.join("fresh"));
		crate::experimental_core::tests::gap_hunt::assert_warning_multisets(
			&stock.observation,
			&fresh.observation,
		);
		let audit = &fresh.observation["fresh_audit"];
		assert_eq!(audit["clean_after_row"], true);
		assert_eq!(audit["reuse"], true);
		assert_eq!(audit["no_cleanup_error"], true);
		assert_eq!(audit["live_entries"], 0);
		assert_eq!(audit["payload_bytes"], 0);
		assert_eq!(audit["idle"], true);
		if let Some(bits) = mode.strip_prefix("c5-f64-") {
			assert_eq!(audit["input_f64_bits"], bits);
		}
		*policies.entry(policy).or_insert(0usize) += 1;
		let outcome = &fresh.observation["outcome"];
		assert_ne!(outcome["status"], "unmapped-value", "raw saved: {label}");
		assert_ne!(outcome["class"], "UNMAPPED", "raw saved: {label}");
		match policy {
			"Exact" => {
				let projected = project(outcome);
				let projected = super::d1b_diagnostics::intentional_json_before(
					"format_specs",
					label,
					source,
					"fresh",
					&projected,
				);
				let mut expected = expected;
				if let Some(sites) = super::d1b_diagnostics::intentional_sites_before(
					"format_specs",
					label,
					source,
					"fresh",
				) {
					expected["sites"] = sites;
				}
				assert_eq!(
					projected, expected,
					"execution exact fields/sites/argument chain: {label}"
				);
			}
			"Compile-error" => {
				assert_eq!(outcome["status"], "compile-error");
				assert_eq!(
					outcome["diagnostics"], expected["diagnostics"],
					"shared frontend diagnostics: {label}"
				);
			}
			"Refused-format-type" => {
				assert_eq!(outcome["status"], "refused");
				assert_eq!(outcome["refusal"], "Some(FormatSpec)");
			}
			"Retained-dynamic-precision" => {
				assert_eq!(outcome["status"], "compile-error");
				assert_eq!(outcome["refusal"], "Some(FrontendDiagnostic)");
				assert_eq!(outcome["location"], json!({"source":0,"start":22,"end":29}));
				assert_eq!(outcome["diagnostics"],"[Fatal(FatalDiagnostic { source_id: 0, kind: CompileError(Error { span: 22:29, kind: Custom { error: \"execution core: dynamic precision is not admitted\" } }) })]");
			}
			"Retained-resource" => {
				assert_eq!(outcome["status"], "error");
				assert_eq!(outcome["class"], "FormatLimit");
				assert_eq!(expected["status"], "value");
			}
			_ => panic!("unknown C5 policy {policy}"),
		}
		for run in [&stock, &fresh] {
			assert!(!has_address(
				&String::from_utf8(run.stdout.clone()).unwrap()
			));
			assert!(!has_address(
				&String::from_utf8(run.stderr.clone()).unwrap()
			));
		}
		assert_eq!(
			byte_projection(&fresh.stdout),
			byte_projection(&stock.stdout),
			"stdout: {label}"
		);
		assert_eq!(
			byte_projection(&fresh.stderr),
			byte_projection(&stock.stderr),
			"stderr: {label}"
		);
	}
	assert_eq!(labels.len(), COUNT);
	assert_eq!(
		policies,
		std::collections::BTreeMap::from([
			("Exact", 244),
			("Compile-error", 14),
			("Refused-format-type", 5),
			("Retained-dynamic-precision", 1),
			("Retained-resource", 2)
		])
	);
	fs::write(
		out.join("RETAINED-ENFORCED.json"),
		serde_json::to_vec_pretty(&json!({
			"d1b_site_rows":16,"retained_chains":[],"d1e_intentional_chain_rows":1,
			"partition":{"Exact":228,"D1b-intentional-sites":16,"Retained-chain":0,
			"Compile-error":14,"Refused-format-type":5,"Retained-dynamic-precision":1,"Retained-resource":2}
		}))
		.unwrap(),
	)
	.unwrap();
	contract_completed("fresh", COUNT);
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
/// The returned outcome has the D1c display mapping applied.
#[cfg(target_os = "linux")]
fn launch(engine: &str, index: usize, folder: &Path) -> super::live_oracle::Run {
	let (label, source, mode, policy) = CASES[index];
	let index_text = index.to_string();
	let mut run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::format_specs::child",
		label,
		folder,
		"C5_RESULT",
		&[
			("C5_ENGINE", engine.as_ref()),
			("C5_CASE", index_text.as_ref()),
		],
	);
	let actual = &mut run.observation;
	actual["outcome"] =
		super::d1c_diagnostics::before("format_specs", label, source, engine, &actual["outcome"]);
	assert_eq!(actual["schema"], 1);
	assert_eq!(actual["index"], index);
	assert_eq!(actual["label"], label);
	assert_eq!(actual["mode"], mode);
	assert_eq!(actual["policy"], policy);
	assert_eq!(actual["source_hex"], hex(source.as_bytes()));
	assert_eq!(actual["fuel"], FUEL);
	run
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(
	miri,
	ignore = "Miri: live stock observations use bounded child processes"
)]
fn d1e_moved_parent_chain_allowance_is_retired() {
	let index = CASES
		.iter()
		.position(|case| case.0 == "access/moved-parent/?")
		.unwrap();
	let out = std::env::temp_dir().join(format!(
		"rune-c5-chain-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	));
	let stock = launch("stock", index, &out.join("stock"));
	let fresh = launch("fresh", index, &out.join("fresh"));
	let displays = |run: &super::live_oracle::Run| -> Vec<Json> {
		run.observation["outcome"]["chain"]
			.as_array()
			.unwrap()
			.iter()
			.map(|entry| entry["display"].clone())
			.collect()
	};
	assert_eq!(displays(&stock), vec![json!("Bad argument #0")]);
	assert_eq!(displays(&fresh), displays(&stock));
	assert_eq!(fresh.stdout, stock.stdout);
	assert_eq!(fresh.stderr, stock.stderr);
	fs::remove_dir_all(out).unwrap();
}
