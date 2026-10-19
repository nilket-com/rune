// C7 strict projection: no address exemptions or retained-frame adapters.
const REPLAY_CAP: usize = 2 << 20;
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
fn replay_drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = REPLAY_CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	bytes
}
#[test]
#[cfg(target_os = "linux")]
#[ignore = "C7 frozen stock replay; explicit unique GAP_OUT, bounded children"]
fn stock_contract() {
	run_contract("stock");
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C7 strict fresh replay; explicit unique GAP_OUT, bounded children"]
fn fresh_contract() {
	run_contract("fresh");
}

fn run_contract(engine: &str) {
	let frozen = include_str!("../../index_targets_stock.tsv");
	let lines = frozen.lines().skip(1).collect::<Vec<_>>();
	assert_eq!(lines.len(), COUNT);
	assert_eq!(CASES.len(), COUNT);
	assert_eq!(
		frozen.lines().next().unwrap(),
		"case\tmode\tsource_hex\tcompared_observation_hex\tstdout_hex\tstderr_hex\tpolicy"
	);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, ((label, source, mode, policy), line)) in CASES.iter().zip(lines).enumerate() {
		let c = line.split('\t').collect::<Vec<_>>();
		assert_eq!(c.len(), 7);
		assert_eq!(c[0], *label);
		assert!(labels.insert(label));
		assert_eq!(c[1], *mode);
		assert_eq!(c[2], hex(source.as_bytes()));
		assert_eq!(c[6], *policy);
		let folder = out.join(format!("{i:03}"));
		fs::create_dir(&folder).unwrap();
		let result = folder.join("observation.json");
		use std::os::unix::process::CommandExt;
		let mut command = Command::new("/usr/bin/prlimit");
		command
			.args([
				"--cpu=15",
				"--as=2147483648",
				"--fsize=2097152",
				"--core=0",
				"--",
			])
			.arg(std::env::current_exe().unwrap());
		command.process_group(0);
		let mut child = command
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::deferred_index::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("C7_ENGINE", engine)
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C7_CASE", i.to_string())
			.env("C7_RESULT", &result)
			.current_dir(&folder)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let exceeded = Arc::new(AtomicBool::new(false));
		let flag = exceeded.clone();
		let pipe = child.stdout.take().unwrap();
		let stdout = std::thread::spawn(move || replay_drain(pipe, flag));
		let flag = exceeded.clone();
		let pipe = child.stderr.take().unwrap();
		let stderr = std::thread::spawn(move || replay_drain(pipe, flag));
		let start = Instant::now();
		let mut stop = false;
		let mut exited = false;
		loop {
			if child.try_wait().unwrap().is_some() {
				exited = true;
			}
			if !exceeded.load(Ordering::Relaxed)
				&& exited && stdout.is_finished()
				&& stderr.is_finished()
			{
				break;
			}
			if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(20) {
				stop = true;
				let _ = Command::new("/usr/bin/kill")
					.args(["-KILL", "--", &format!("-{}", child.id())])
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.status();
				break;
			}
			std::thread::sleep(Duration::from_millis(10));
		}
		// wait() also returns the cached status when try_wait() already reaped
		// the child. Every exit path explicitly collects it before joining.
		let status = child.wait().unwrap();
		let stdout = stdout.join().unwrap();
		let stderr = stderr.join().unwrap();
		fs::write(folder.join("stdout"), &stdout).unwrap();
		fs::write(folder.join("stderr"), &stderr).unwrap();
		fs::write(
			folder.join("receipt.txt"),
			format!(
				"case:{label}\nrc:{status}\nstop:{stop}\nelapsed:{:?}\n",
				start.elapsed()
			),
		)
		.unwrap();
		assert!(!stop && status.success() && !exceeded.load(Ordering::Relaxed));
		assert!(fs::metadata(&result).unwrap().len() <= REPLAY_CAP as u64);
		let mut e: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		e["outcome"] =
			super::d1c_diagnostics::before("deferred_index", label, source, engine, &e["outcome"]);
		if engine == "stock" {
			exact_keys(
				e.as_object().unwrap(),
				&[
					"schema",
					"engine",
					"index",
					"label",
					"mode",
					"policy",
					"source_hex",
					"fuel",
					"outcome",
				],
			);
		} else {
			exact_keys(
				e.as_object().unwrap(),
				&[
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
				],
			);
		}
		assert_eq!(e["engine"], engine);
		assert_eq!(e["schema"], 1);
		assert_eq!(e["index"], i);
		assert_eq!(e["label"], *label);
		assert_eq!(e["mode"], *mode);
		assert_eq!(e["policy"], *policy);
		assert_eq!(e["source_hex"], c[2]);
		assert_eq!(e["fuel"], FUEL);
		let expected: Json = serde_json::from_str(&unhex(c[3])).unwrap();
		fs::write(
			folder.join("expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		let category = if engine == "stock" {
			let got = project(&e["outcome"]);
			fs::write(
				folder.join("compared.json"),
				serde_json::to_vec_pretty(&got).unwrap(),
			)
			.unwrap();
			assert_eq!(got, expected, "stock {label}");
			assert_eq!(hex(&stdout), c[4], "stdout {label}");
			assert_eq!(hex(&stderr), c[5], "stderr {label}");
			*policy
		} else {
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
			let (entries, bytes) = if *mode == "c7-small-store" {
				(2, 4096)
			} else {
				(4096, 4 << 20)
			};
			assert_eq!(audit["limits_entries"], entries);
			assert_eq!(audit["limits_bytes"], bytes);
			assert_eq!(audit["reuse_raw"], "Some(I64(7))");
			audit["call_chain"].as_array().unwrap();
			assert_ne!(e["outcome"]["class"], "UNMAPPED");
			let mut category = *policy;
			match *policy {
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
					let refusal = if *policy == "Refused-Range" {
						"Range"
					} else {
						"ByteStr"
					};
					assert_eq!(o["refusal"], refusal);
					let (_, start, end) = REFUSAL_SITES
						.iter()
						.find(|(name, _, _)| name == label)
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
					assert_eq!(&stdout, b"\nrunning 1 test\n");
					assert!(stderr.is_empty());
				}
				"Retained-resource" => {
					assert_eq!(*label, "limits/result-entry");
					assert_eq!(*mode, "c7-small-store");
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
					assert_eq!(hex(&stdout), c[4], "stdout {label}");
					assert_eq!(hex(&stderr), c[5], "stderr {label}");
				}
				"Exact" | "Refused-Range" => {
					if *policy == "Refused-Range" {
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
										"retained pin drift {label}: stock={expected} fresh={got}"
									)
								});
					} else {
						assert_eq!(got, expected, "fresh {label}");
					}
					assert_eq!(hex(&stdout), c[4], "stdout {label}");
					assert_eq!(hex(&stderr), c[5], "stderr {label}");
				}
				_ => panic!("unknown C7 policy"),
			}
			category
		};
		assert!(
			!has_address(&String::from_utf8_lossy(&stdout))
				&& !has_address(&String::from_utf8_lossy(&stderr))
		);
		*policies.entry(category).or_insert(0usize) += 1;
	}
	let expected = if engine == "stock" {
		std::collections::BTreeMap::from([
			("Exact", 306),
			("Refused-Range", 10),
			("Refused-ByteStr", 1),
			("Retained-resource", 1),
		])
	} else {
		std::collections::BTreeMap::from([
			("Exact", 286),
			("Retained-chain", 17),
			("Retained-frame", 2),
			("Retained-display", 1),
			("C12-admitted-range", 10),
			("Refused-ByteStr", 1),
			("Retained-resource", 1),
		])
	};
	assert_eq!(policies, expected);
	contract_completed(engine, COUNT);
	fs::write(
		out.join("COMPLETE"),
		format!("C7 {engine} {COUNT} rows complete\n"),
	)
	.unwrap();
}

include!("retained.rs");
