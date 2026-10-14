// C5 stable stock replay projection. Raw Debug metadata remains provenance only.
const REPLAY_CAP: usize = 2 << 20;

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
#[ignore = "C5 frozen stock replay on Linux; explicit fresh GAP_OUT; bounded children"]
#[allow(clippy::assertions_on_constants)]
fn stock_contract() {
	run_contract("stock");
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "C5 bounded fresh contract; explicit new GAP_OUT"]
fn fresh_contract() {
	run_contract("fresh");
}

#[allow(clippy::assertions_on_constants)]
fn run_contract(engine: &str) {
	assert!(
		cfg!(feature = "sha2"),
		"C5 frozen digest replay requires --features sha2"
	);
	let frozen = include_str!("../../format_specs_stock.tsv");
	let rows = frozen.lines().skip(1).collect::<Vec<_>>();
	assert_eq!(rows.len(), COUNT);
	assert_eq!(CASES.len(), COUNT);
	assert_eq!(
		frozen.lines().next().unwrap(),
		"case\tmode\tsource_hex\tcompared_observation_hex\tstdout_projection_hex\tstderr_projection_hex\tpolicy"
	);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let mut labels = std::collections::BTreeSet::new();
	let mut policies = std::collections::BTreeMap::new();
	let mut retained_sites = Vec::new();
	let mut retained_chains = Vec::new();
	let retained: Json = serde_json::from_str(include_str!("retained_errors.json")).unwrap();
	assert_eq!(retained["schema"], 1);
	assert_eq!(retained["rows"].as_array().unwrap().len(), 16);
	for (i, ((label, source, mode, policy), line)) in CASES.iter().zip(rows).enumerate() {
		let columns = line.split('\t').collect::<Vec<_>>();
		assert_eq!(columns.len(), 7);
		assert_eq!(columns[0], *label);
		assert!(labels.insert(label));
		assert_eq!(columns[1], *mode);
		assert_eq!(columns[2], hex(source.as_bytes()));
		assert_eq!(columns[6], *policy);
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
				"experimental_core::tests::gap_hunt::format_specs::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C5_ENGINE", engine)
			.env("C5_CASE", i.to_string())
			.env("C5_RESULT", &result)
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
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(20) {
				stop = true;
				let _ = Command::new("/usr/bin/kill")
					.args(["-KILL", "--", &format!("-{}", child.id())])
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.status();
				break child.wait().unwrap();
			}
			std::thread::sleep(Duration::from_millis(10));
		};
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
		assert!(
			!stop && status.success() && !exceeded.load(Ordering::Relaxed),
			"stock replay child {label}"
		);
		assert!(result.metadata().unwrap().len() <= REPLAY_CAP as u64);
		let actual: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		assert_eq!(actual["schema"], 1);
		assert_eq!(actual["index"], i);
		assert_eq!(actual["label"], *label);
		assert_eq!(actual["mode"], *mode);
		assert_eq!(actual["policy"], *policy);
		assert_eq!(actual["source_hex"], columns[2]);
		assert_eq!(actual["fuel"], FUEL);
		let expected: Json = serde_json::from_str(&unhex(columns[3])).unwrap();
		if engine == "stock" {
			assert_eq!(
				project(&actual["outcome"]),
				expected,
				"stock fields including exact sites: {label}"
			);
		} else {
			let audit = &actual["fresh_audit"];
			assert_eq!(audit["clean_after_row"], true);
			assert_eq!(audit["reuse"], true);
			assert_eq!(audit["no_cleanup_error"], true);
			assert_eq!(audit["live_entries"], 0);
			assert_eq!(audit["payload_bytes"], 0);
			assert_eq!(audit["idle"], true);
			if let Some(bits) = mode.strip_prefix("c5-f64-") {
				assert_eq!(audit["input_f64_bits"], bits);
			}
			*policies.entry(*policy).or_insert(0usize) += 1;
			let outcome = &actual["outcome"];
			assert_ne!(outcome["status"], "unmapped-value", "raw saved: {label}");
			assert_ne!(outcome["class"], "UNMAPPED", "raw saved: {label}");
			match *policy {
				"Exact" => {
					let projected = project(outcome);
					let rule = retained["rows"]
						.as_array()
						.unwrap()
						.iter()
						.find(|row| row["case"] == *label);
					if let Some(rule) = rule {
						assert!(
							retained_error_matches(rule, label, source, &expected, &projected),
							"pinned retained error drift: {label}"
						);
						assert!(
							retained_site_shape(source, &expected["sites"], &projected["sites"]),
							"retained AST site rule: {label}"
						);
						let mut other = projected.clone();
						let mut frozen_other = expected.clone();
						other.as_object_mut().unwrap().remove("sites");
						frozen_other.as_object_mut().unwrap().remove("sites");
						if rule["stock_chains"] != rule["fresh_chains"] {
							assert!(retained_chain_shape(
								&expected["chain_displays"],
								&projected["chain_displays"]
							));
							other.as_object_mut().unwrap().remove("chain_displays");
							frozen_other
								.as_object_mut()
								.unwrap()
								.remove("chain_displays");
							retained_chains.push((*label).to_owned());
						}
						assert_eq!(
							other, frozen_other,
							"retained primary fields remain exact: {label}"
						);
						retained_sites.push((*label).to_owned());
					} else {
						assert_eq!(projected, expected, "fresh exact fields/sites: {label}");
					}
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
					assert_eq!(outcome["diagnostics"],"[Fatal(FatalDiagnostic { source_id: 0, kind: CompileError(Error { span: 22:29, kind: Custom { error: \"fresh core: dynamic precision is not admitted\" } }) })]");
				}
				"Retained-resource" => {
					assert_eq!(outcome["status"], "error");
					assert_eq!(outcome["class"], "FormatLimit");
					assert_eq!(expected["status"], "value");
				}
				_ => panic!("unknown C5 policy {policy}"),
			}
		}

		assert!(!has_address(&String::from_utf8(stdout.clone()).unwrap()));
		assert!(!has_address(&String::from_utf8(stderr.clone()).unwrap()));
		let expected_stdout: Json = serde_json::from_str(&unhex(columns[4])).unwrap();
		assert_eq!(byte_projection(&stdout), expected_stdout, "stdout: {label}");
		let expected_stderr: Json = serde_json::from_str(&unhex(columns[5])).unwrap();
		assert_eq!(byte_projection(&stderr), expected_stderr, "stderr: {label}");
	}
	assert_eq!(labels.len(), COUNT);
	if engine == "fresh" {
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
	}
	if engine == "fresh" {
		assert_eq!(retained_sites.len(), 16);
		assert_eq!(retained_chains.len(), 1);
		fs::write(out.join("RETAINED-ENFORCED.json"),serde_json::to_vec_pretty(&json!({"sites":retained_sites,"chains":retained_chains,"partition":{"Exact":228,"Retained-site":15,"Retained-site-and-chain":1,"Compile-error":14,"Refused-format-type":5,"Retained-dynamic-precision":1,"Retained-resource":2}})).unwrap()).unwrap();
	}
	contract_completed(engine, COUNT);
}

// Temporary discover rule: explicit operator opt-in; the frozen per-row allowlist
// replaces this before a production commit. Never changes the frozen TSV.
fn retained_site_shape(source: &str, stock: &Json, fresh: &Json) -> bool {
	use crate::ast::Spanned;
	let (Some(stock), Some(fresh)) = (stock.as_array(), fresh.as_array()) else {
		return false;
	};
	if stock.len() != 1 || fresh.len() != 1 {
		return false;
	}
	let (stock, fresh) = (&stock[0], &fresh[0]);
	if stock["source"] != fresh["source"]
		|| stock["frames"] != "[]"
		|| fresh["frames"] != "[]"
		|| stock["start"] != 0
	{
		return false;
	}
	let (Some(start), Some(end)) = (fresh["start"].as_u64(), fresh["end"].as_u64()) else {
		return false;
	};
	let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
		return false;
	};
	if start == 0
		|| start >= end
		|| stock["end"]
			.as_u64()
			.is_none_or(|stock_end| stock_end <= start as u64 || stock_end > end as u64)
	{
		return false;
	}
	let Some(invocation) = source.get(start..end) else {
		return false;
	};
	if !["format!(", "print!(", "println!("]
		.iter()
		.any(|name| invocation.starts_with(name))
		|| !invocation.ends_with(')')
	{
		return false;
	}
	let Ok(expr) =
		crate::parse::parse_all::<crate::ast::Expr>(invocation, crate::SourceId::EMPTY, false)
	else {
		return false;
	};
	if !matches!(expr, crate::ast::Expr::MacroCall(_)) {
		return false;
	}
	let Ok(file) =
		crate::parse::parse_all::<crate::ast::File>(source, crate::SourceId::EMPTY, false)
	else {
		return false;
	};
	file.items.iter().any(|(item, _)| {
		if let crate::ast::Item::Fn(item) = item {
			let span = item.span().range();
			span.start <= start && end <= span.end
		} else {
			false
		}
	})
}

#[test]
fn retained_site_rule_rejects_frames_ids_bounds_and_non_invocations() {
	let source = "struct P{x} pub fn main(){format!(\"{:*>9.1}\",())}";
	let stock = json!([{"source":0,"start":0,"end":48,"frames":"[]"}]);
	let fresh = json!([{"source":0,"start":26,"end":48,"frames":"[]"}]);
	assert!(retained_site_shape(source, &stock, &fresh));
	let source2 =
		"fn mark(x){print!(\"m\");x} pub fn main(){format!(\"{:02}{}\",mark(()),mark(2))}";
	assert!(retained_site_shape(
		source2,
		&json!([{"source":0,"start":0,"end":74,"frames":"[]"}]),
		&json!([{"source":0,"start":40,"end":75,"frames":"[]"}])
	));
	for (key, value) in [
		("source", json!(1)),
		("start", json!(27)),
		("end", json!(47)),
		("frames", json!("[{...}]")),
	] {
		let mut bad = fresh.clone();
		bad[0][key] = value;
		assert!(!retained_site_shape(source, &stock, &bad));
	}
	assert!(!retained_site_shape(
		source,
		&json!([{"source":0,"start":12,"end":48,"frames":"[]"}]),
		&fresh
	));
	assert!(!retained_site_shape(
		"struct P{x} pub fn main(){foobar!(\"{:*>9.1}\",())}",
		&stock,
		&fresh
	));
}

fn retained_chain_shape(stock: &Json, fresh: &Json) -> bool {
	let (Some(stock), Some(fresh)) = (stock.as_array(), fresh.as_array()) else {
		return false;
	};
	if stock.len() != 1 || !fresh.is_empty() {
		return false;
	}
	let Some(text) = stock[0].as_str() else {
		return false;
	};
	text.strip_prefix("Bad argument #")
		.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn retained_chain_rule_rejects_other_context_and_nonempty_fresh_chain() {
	assert!(retained_chain_shape(
		&json!(["Bad argument #0"]),
		&json!([])
	));
	for stock in [
		json!([]),
		json!(["Bad argument #"]),
		json!(["Bad argument #-1"]),
		json!(["Bad argument #x"]),
		json!(["different"]),
		json!(["Bad argument #0", "Bad argument #1"]),
	] {
		assert!(!retained_chain_shape(&stock, &json!([])));
	}
	assert!(!retained_chain_shape(
		&json!(["Bad argument #0"]),
		&json!(["Bad argument #0"])
	));
}

fn source_sha256(source: &str) -> String {
	#[cfg(feature = "sha2")]
	{
		use sha2::{Digest, Sha256};
		hex(&Sha256::digest(source.as_bytes()))
	}
	#[cfg(not(feature = "sha2"))]
	{
		let _ = source;
		panic!("C5 frozen error pins require existing sha2 feature");
	}
}

fn retained_error_matches(
	rule: &Json,
	label: &str,
	source: &str,
	stock: &Json,
	fresh: &Json,
) -> bool {
	rule["case"] == label
		&& rule["source_sha256"] == source_sha256(source)
		&& rule["stock_sites"] == stock["sites"]
		&& rule["fresh_sites"] == fresh["sites"]
		&& rule["stock_chains"] == stock["chain_displays"]
		&& rule["fresh_chains"] == fresh["chain_displays"]
}

#[test]
#[cfg(feature = "sha2")]
fn retained_error_pins_reject_site_chain_source_and_label_drift() {
	let rows: Json = serde_json::from_str(include_str!("retained_errors.json")).unwrap();
	let rule = rows["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|row| row["case"] == "access/moved-parent/?")
		.unwrap();
	let source = CASES.iter().find(|row| row.0 == rule["case"]).unwrap().1;
	let stock = json!({"sites":rule["stock_sites"],"chain_displays":rule["stock_chains"]});
	let fresh = json!({"sites":rule["fresh_sites"],"chain_displays":rule["fresh_chains"]});
	let label = rule["case"].as_str().unwrap();
	assert!(retained_error_matches(rule, label, source, &stock, &fresh));
	let mut shifted = stock.clone();
	shifted["sites"][0]["end"] = json!(65);
	assert!(!retained_error_matches(
		rule, label, source, &shifted, &fresh
	));
	let mut shifted = fresh.clone();
	shifted["sites"][0]["start"] = json!(48);
	assert!(!retained_error_matches(
		rule, label, source, &stock, &shifted
	));
	let mut removed = stock.clone();
	removed["chain_displays"] = json!([]);
	assert!(!retained_error_matches(
		rule, label, source, &removed, &fresh
	));
	let mut added = fresh.clone();
	added["chain_displays"] = json!(["Bad argument #0"]);
	assert!(!retained_error_matches(rule, label, source, &stock, &added));
	let changed = format!("x{}", &source[1..]);
	assert_eq!(changed.len(), source.len());
	assert!(!retained_error_matches(
		rule, label, &changed, &stock, &fresh
	));
	assert!(!retained_error_matches(
		rule, "unknown", source, &stock, &fresh
	));
}
