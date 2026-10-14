// C6 stock replay: raw Debug is provenance; semantic missing-method hashes verified.
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
#[ignore = "C6 frozen stock replay; explicit unique GAP_OUT, bounded children"]
fn stock_contract() {
	let frozen = include_str!("../../scalar_methods_supplement_stock.tsv");
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
				"experimental_core::tests::gap_hunt::scalar_methods_supplement::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C6S_CASE", i.to_string())
			.env("C6S_RESULT", &result)
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
			if let Some(s) = child.try_wait().unwrap() {
				break s;
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
		assert!(!stop && status.success() && !exceeded.load(Ordering::Relaxed));
		assert!(fs::metadata(&result).unwrap().len() <= REPLAY_CAP as u64);
		let e: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		exact_keys(
			e.as_object().unwrap(),
			&[
				"schema",
				"index",
				"label",
				"mode",
				"policy",
				"source_hex",
				"fuel",
				"outcome",
			],
		);
		assert_eq!(e["schema"], 1);
		assert_eq!(e["index"], i);
		assert_eq!(e["label"], *label);
		assert_eq!(e["mode"], *mode);
		assert_eq!(e["policy"], *policy);
		assert_eq!(e["source_hex"], c[2]);
		assert_eq!(e["fuel"], FUEL);
		let got = project(source, &e["outcome"]);
		let expected: Json = serde_json::from_str(&unhex(c[3])).unwrap();
		fs::write(
			folder.join("compared.json"),
			serde_json::to_vec_pretty(&got).unwrap(),
		)
		.unwrap();
		assert_eq!(got, expected, "case {label}");
		assert!(
			!has_address(&String::from_utf8_lossy(&stdout))
				&& !has_address(&String::from_utf8_lossy(&stderr))
		);
		assert_eq!(hex(&stdout), c[4], "stdout {label}");
		assert_eq!(hex(&stderr), c[5], "stderr {label}");
		*policies.entry(*policy).or_insert(0usize) += 1;
	}
	assert_eq!(policies.get("Exact"), Some(&7));
	assert_eq!(policies.len(), 1);

	fs::write(
		out.join("COMPLETE"),
		"C6 supplement7 exact frozen replay;7/0/0\n",
	)
	.unwrap();
}
