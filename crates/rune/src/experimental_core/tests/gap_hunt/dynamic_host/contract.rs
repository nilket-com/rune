// Frozen stock replay plus the strict fresh contract; no discovery or waiver.
const C10_TSV: &str = include_str!("../../dynamic_host_stock.tsv");
const C10_REPLAY_CAP: usize = 64 << 20;
fn c10_rows() -> Vec<Vec<&'static str>> {
	let mut lines = C10_TSV.lines();
	assert_eq!(lines.next().unwrap(),"case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tpolicy\tfresh_expectation_hex\truling_hex");
	lines
		.map(|line| {
			let row = line.split('\t').collect::<Vec<_>>();
			assert_eq!(row.len(), 9);
			row
		})
		.collect()
}
fn c10_data_seals() {
	// Independent literals generated from accepted/proposed freeze2, never from live outputs.
	for (name, bytes, hash) in C10_DATA_SEALS {
		assert_eq!(c10_sha(bytes), *hash, "frozen data drift: {name}");
		assert!(
			bytes.len() < 1_000_000,
			"large committed oracle blob: {name}"
		);
	}
	let rows = c10_rows();
	assert_eq!(rows.len(), 126);
	assert_eq!(COUNT, 126);
	let mut counts = BTreeMap::new();
	let mut digest_rows = Vec::new();
	let mut fresh_digests = 0;
	for (i, row) in rows.iter().enumerate() {
		assert_eq!(row[0], CASES[i].0);
		assert_eq!(row[1], "c10-main");
		assert_eq!(row[2], hex(CASES[i].1.as_bytes()));
		assert_eq!(row[6], CASES[i].2);
		*counts.entry(row[6]).or_insert(0usize) += 1;
		let stored = c10_decode(row[3]);
		let expected = c10_decode(row[7]);
		if stored["encoding"] == "sha256-common-v1" {
			assert!(stored["bytes"].as_u64().unwrap() > 65536);
			assert!(C10_LARGE.contains(&row[0]));
			digest_rows.push(row[0]);
		} else {
			assert!(c10_canonical(&c10_common(&stored).unwrap()).len() <= 65536);
		}
		if expected["encoding"] == "sha256-common-v1" {
			assert_eq!(row[6], "Exact");
			assert!(expected["bytes"].as_u64().unwrap() > 65536);
			fresh_digests += 1;
		}
	}
	assert_eq!(digest_rows, C10_LARGE);
	assert_eq!(fresh_digests, 5);
	assert_eq!(
		counts,
		BTreeMap::from([
			("Exact", 93),
			("Refused-runtime", 28),
			("Retained-arity", 1),
			("Retained-isolation", 4)
		])
	);
}
fn c10_drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = C10_REPLAY_CAP - bytes.len();
		bytes.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	bytes
}
#[test]
fn stock_contract() {
	c10_contract("stock");
}
#[test]
fn fresh_contract() {
	c10_contract("fresh");
}
fn c10_contract(engine: &str) {
	c10_data_seals();
	let owned = std::env::var_os("C10_REPLAY_OUT").is_none();
	let out = std::env::var_os("C10_REPLAY_OUT")
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			std::env::temp_dir().join(format!(
				"rune-c10-stock-replay-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			))
		});
	fs::create_dir(&out).unwrap();
	let total = Instant::now();
	for (i, row) in c10_rows().iter().enumerate() {
		assert!(
			total.elapsed() < Duration::from_secs(3600),
			"C10 whole replay bound"
		);
		let folder = out.join(format!("{i:03}"));
		fs::create_dir(&folder).unwrap();
		let result = folder.join("observation.json");
		use std::os::unix::process::CommandExt;
		let mut command = Command::new("/usr/bin/prlimit");
		command
			.args([
				"--cpu=30",
				"--as=2147483648",
				"--fsize=67108864",
				"--core=0",
				"--",
			])
			.arg(std::env::current_exe().unwrap())
			.process_group(0);
		let mut child = command
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::dynamic_host::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C10_ENGINE", engine)
			.env("C10_CASE", i.to_string())
			.env("C10_RESULT", &result)
			.current_dir(&folder)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let exceeded = Arc::new(AtomicBool::new(false));
		let flag = exceeded.clone();
		let pipe = child.stdout.take().unwrap();
		let stdout = std::thread::spawn(move || c10_drain(pipe, flag));
		let flag = exceeded.clone();
		let pipe = child.stderr.take().unwrap();
		let stderr = std::thread::spawn(move || c10_drain(pipe, flag));
		let start = Instant::now();
		let mut stop = false;
		loop {
			let exited = child.try_wait().unwrap().is_some();
			if !exceeded.load(Ordering::Relaxed)
				&& exited && stdout.is_finished()
				&& stderr.is_finished()
			{
				break;
			}
			if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(45) {
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
		let status = child.wait().unwrap();
		let stdout = stdout.join().unwrap();
		let stderr = stderr.join().unwrap();
		fs::write(folder.join("stdout"), &stdout).unwrap();
		fs::write(folder.join("stderr"), &stderr).unwrap();
		fs::write(folder.join("receipt.json"),serde_json::to_vec_pretty(&json!({"index":i,"label":row[0],"success":status.success(),"stop":stop,"seconds":start.elapsed().as_secs_f64(),"stdout_sha256":c10_sha(&stdout),"stderr_sha256":c10_sha(&stderr)})).unwrap()).unwrap();
		assert!(
			!stop && status.success() && !exceeded.load(Ordering::Relaxed),
			"row {} raw at {}",
			row[0],
			folder.display()
		);
		assert!(fs::metadata(&result).unwrap().len() <= C10_REPLAY_CAP as u64);
		let envelope: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		assert_eq!(envelope["schema"], 1);
		assert_eq!(envelope["engine"], engine);
		assert_eq!(envelope["index"], i);
		assert_eq!(envelope["label"], row[0]);
		assert_eq!(envelope["source_hex"], row[2]);
		assert_eq!(envelope["policy"], row[6]);
		assert_eq!(envelope["fuel"], FUEL);
		assert_eq!(hex(&stdout), row[4]);
		assert_eq!(hex(&stderr), row[5]);
		if engine == "fresh" {
			c10_fresh_audit(&envelope["audit"], row[6], row[0]);
			let expected =
				c10_arity_expected(i, row[0], row[2], row[6], c10_decode(row[7])).unwrap();
			c10_compare(&envelope["observation"], &expected, row[0]).unwrap_or_else(|e| {
				panic!(
					"strict fresh row {i} {}: {e}; raw at {}",
					row[0],
					folder.display()
				)
			});
			continue;
		}
		let portable = c10_project(&envelope["observation"], i).unwrap();
		let actual = c10_common(&portable).unwrap();
		let stored = c10_decode(row[3]);
		let expected = if stored["encoding"] == "sha256-common-v1" {
			stored
		} else {
			c10_common(&stored).unwrap()
		};
		c10_compare(&actual, &expected, row[0])
			.unwrap_or_else(|e| panic!("row {i} {}: {e}; raw at {}", row[0], folder.display()));
	}
	contract_completed(engine, COUNT);
	if owned {
		fs::remove_dir_all(&out).unwrap();
	}
}

const C10_DATA_SEALS: &[(&str, &[u8], &str)] = &[
	(
		"dynamic_host_stock.tsv",
		include_bytes!("../../dynamic_host_stock.tsv"),
		"812385ffccc3066ecc55d321120978da6bfea0fd307f499be7d519aa626b1863",
	),
	(
		"dynamic_host_runtime.json",
		include_bytes!("../../dynamic_host_runtime.json"),
		"2e6266e0c6213a515fa3c38316f689f22f20732d4cad4b3acf0e0cd2c0eb5468",
	),
	(
		"dynamic_host_isolation.json",
		include_bytes!("../../dynamic_host_isolation.json"),
		"60af61e6fa7a425591135de25dba5b9385f89056af7712b33102fc3155003b72",
	),
	(
		"dynamic_host_arity.json",
		include_bytes!("../../dynamic_host_arity.json"),
		"2c286b514475e48feceb919ae5c7c08fbdb4eeec86eacea614958d339d958843",
	),
	(
		"dynamic_host_disclosures.json",
		include_bytes!("../../dynamic_host_disclosures.json"),
		"5d4ea1f189c8d2936aa5ea35c5ddf3c7eb8e424698957c15526675e6b5551788",
	),
	(
		"dynamic_host_p1_counts.json",
		include_bytes!("../../dynamic_host_p1_counts.json"),
		"0ebd1225cb645d6f13b8f65210c24d62925297e47b03ec75211b4783ef8c8bbc",
	),
];

// seq15065: one exact Retained-arity diagnostic pin, never a diagnostic filter.
fn c10_arity_expected(
	index: usize,
	label: &str,
	source_hex: &str,
	policy: &str,
	mut expected: Json,
) -> Result<Json, String> {
	if index != 124 && label != "argument/wrong-arity" {
		return Ok(expected);
	}
	if index != 124 || label != "argument/wrong-arity" || policy != "Retained-arity" {
		return Err("arity diagnostic pin row/policy mismatch".into());
	}
	let rules: Json = serde_json::from_str(include_str!("../../dynamic_host_arity.json")).unwrap();
	if rules.as_object().unwrap().len() != 1 {
		return Err("arity pin count".into());
	}
	let rule = &rules[label];
	let source = c10_hex_bytes(source_hex)?;
	if rule["index"] != index
		|| rule["source_sha256"] != c10_sha(&source)
		|| expected["outcome"] != rule["fresh_expectation"]
		|| expected["diagnostics"] != "[]"
	{
		return Err("arity diagnostic source/frozen expectation drift".into());
	}
	let primary = &rule["fresh_expectation"]["primary"];
	let start = primary["start"].as_u64().unwrap() as usize;
	let end = primary["end"].as_u64().unwrap() as usize;
	if source.get(start..end).map(hex).as_deref() != primary["span_hex"].as_str() {
		return Err("arity diagnostic span bytes drift".into());
	}
	expected["diagnostics"] = rule["fresh_top_level_diagnostics"].clone();
	Ok(expected)
}
