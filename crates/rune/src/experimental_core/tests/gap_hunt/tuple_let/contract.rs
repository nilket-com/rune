// C8 strict projection: no address exemptions or retained-frame adapters.
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
		"compile-error" => {
			exact_keys(obj, &["status", "error", "diagnostics"]);
			o.clone()
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
#[ignore = "C8 frozen stock replay; bounded children and exclusive GAP_OUT"]
fn stock_contract() {
	run_contract("stock");
}
#[test]
#[cfg(target_os = "linux")]
#[ignore = "C8 strict fresh replay; bounded children and exclusive GAP_OUT"]
fn fresh_contract() {
	run_contract("fresh");
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

fn run_contract(engine: &str) {
	assert!(matches!(engine, "stock" | "fresh"));
	let frozen = include_str!("../../tuple_let_stock.tsv");
	let mut lines = frozen.lines();
	assert_eq!(lines.next().unwrap(), "case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tpolicy\tfresh_expectation_hex\truling_hex");
	let lines = lines.collect::<Vec<_>>();
	assert_eq!(lines.len(), COUNT);
	assert_eq!(CASES.len(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	fs::create_dir(&out).unwrap();
	let c16e: Json = if engine == "fresh" {
		let rows: Json = serde_json::from_str(include_str!("c16e_admissions.json")).unwrap();
		assert_eq!(rows.as_object().unwrap().len(), 9);
		rows
	} else {
		json!({})
	};
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	for (i, ((label, source, mode, policy), line)) in CASES.iter().zip(lines).enumerate() {
		let c = line.split('\t').collect::<Vec<_>>();
		assert_eq!(c.len(), 9);
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
				"experimental_core::tests::gap_hunt::tuple_let::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("C8_ENGINE", engine)
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C8_CASE", i.to_string())
			.env("C8_RESULT", &result)
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
			super::d1c_diagnostics::before("tuple_let", label, source, engine, &e["outcome"]);
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
		let stock_expected: Json = serde_json::from_str(&unhex(c[3])).unwrap();
		let fresh_expected: Json = serde_json::from_str(&unhex(c[7])).unwrap();
		let ruling: Json = serde_json::from_str(&unhex(c[8])).unwrap();
		fs::write(
			folder.join("stock-expected.json"),
			serde_json::to_vec_pretty(&stock_expected).unwrap(),
		)
		.unwrap();
		fs::write(
			folder.join("fresh-expected.json"),
			serde_json::to_vec_pretty(&fresh_expected).unwrap(),
		)
		.unwrap();
		fs::write(
			folder.join("ruling.json"),
			serde_json::to_vec_pretty(&ruling).unwrap(),
		)
		.unwrap();
		match *policy {
			"Intentional-divergence" => {
				check_intentional(label, source, &stock_expected, &fresh_expected, &ruling)
			}
			"Exact" | "Depth-audit" => {
				assert_eq!(fresh_expected, stock_expected);
				assert!(ruling.is_null());
			}
			"Refused-Pattern" | "Refused-Range" => {
				assert!(ruling.is_null());
				assert_eq!(fresh_expected["status"], "refused");
				assert_eq!(fresh_expected["kind"], "Unsupported");
				assert_eq!(fresh_expected["invoked"], false);
				assert_eq!(
					fresh_expected["refusal"],
					if *policy == "Refused-Pattern" {
						"Pattern"
					} else {
						"Range"
					}
				);
				let start = fresh_expected["start"].as_u64().unwrap() as usize;
				let end = fresh_expected["end"].as_u64().unwrap() as usize;
				assert_eq!(
					hex(source.as_bytes().get(start..end).unwrap()),
					fresh_expected["span_hex"]
				);
			}
			_ => panic!("unreviewed C8 policy"),
		}
		let c16e_admission = c16e.get(*label);
		if let Some(rule) = c16e_admission {
			use sha2::{Digest, Sha256};
			assert_eq!(rule["index"], i);
			assert_eq!(rule["previous_policy"], *policy);
			assert_eq!(rule["policy"], "Exact");
			assert_eq!(rule["source_sha256"], hex(&Sha256::digest(source.as_bytes())));
			assert_eq!(rule["stock_row_sha256"], hex(&Sha256::digest(line.as_bytes())));
		}
		// C12, C16a, C16b and C16e admissions retain the historical freeze above and now
		// owe their exact independently frozen stock values.
		let fresh_expected = if c16e_admission.is_some() || *policy == "Refused-Range"
			|| matches!(
				*label,
				"refusal/vec-pattern" | "refusal/for-pair" | "refusal/for-nested"
			) {
			stock_expected.clone()
		} else {
			fresh_expected
		};
		let got = if engine == "stock" {
			project(&e["outcome"])
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
			let (entries, bytes) = if *mode == "c8-small-store" {
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
			if fresh_expected["status"] == "refused" || matches!(*label, "depth/128" | "depth/129")
			{
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
				assert_eq!(audit["returned_root"], fresh_expected["status"] == "value");
				project(&e["outcome"])
			}
		};
		fs::write(
			folder.join("compared.json"),
			serde_json::to_vec_pretty(&got).unwrap(),
		)
		.unwrap();
		let expected = if engine == "stock" {
			stock_expected
		} else {
			let manifest: Json =
				serde_json::from_str(include_str!("../../tuple_let_retained.json")).unwrap();
			retained_expected(
				&manifest,
				label,
				source,
				&stock_expected,
				&fresh_expected,
				&e["fresh_audit"],
				&e["outcome"],
			)
		};
		fs::write(
			folder.join("effective-expected.json"),
			serde_json::to_vec_pretty(&expected).unwrap(),
		)
		.unwrap();
		assert_eq!(got, expected, "C8 strict {engine} {label}");
		assert_eq!(hex(&stdout), c[4], "stdout {label}");
		assert_eq!(hex(&stderr), c[5], "stderr {label}");
		assert!(
			!has_address(&String::from_utf8_lossy(&stdout))
				&& !has_address(&String::from_utf8_lossy(&stderr))
		);
		*policies.entry(*policy).or_insert(0usize) += 1;
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
	contract_completed(engine, COUNT);
	fs::write(
		out.join("COMPLETE"),
		format!("C8 {engine} {COUNT} rows complete\n"),
	)
	.unwrap();
}
