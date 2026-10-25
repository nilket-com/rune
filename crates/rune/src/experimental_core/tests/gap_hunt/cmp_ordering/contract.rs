fn d1_cmp_expectation(label: &str, source: &str, expected: &Json, engine: &str) -> Json {
	let rows: Vec<Json> = serde_json::from_str(include_str!("d1_diagnostics.json")).unwrap();
	let mut expected = expected.clone();
	if let Some(row) = rows.iter().find(|row| row["label"] == label) {
		assert_eq!(row["source_hex"], hex(source.as_bytes()));
		assert_eq!(expected["display"], row["changes"][engine]["before"]);
		expected["display"] = row["changes"][engine]["after"].clone();
	}
	if label == "access/leased-parent-mutate" {
		assert_eq!(
			hex(source.as_bytes()),
			"70756220666e206d61696e28297b6c657420763d5b2261225d3b666f72207820696e20767b782e636d70287b762e7075736828226322293b2262227d293b7d377d0a70756220666e2063395f726575736528297b377d"
		);
		assert_eq!(
			expected["display"],
			"Cannot write, the value is being read from"
		);
		expected["display"] = json!(D1C_ACCESS_DISPLAY);
	}
	let moved_value = match label {
		"access/moved-receiver" => Some(
			"70756220666e206d61696e28297b6c657420733d2261223b6c657420616c6961733d733b64726f702873293b616c6961732e636d7028226222297d0a70756220666e2063395f726575736528297b377d",
		),
		"access/moved-rhs" => Some(
			"70756220666e206d61696e28297b6c657420733d2261223b6c657420616c6961733d733b64726f702873293b2262222e636d7028616c696173297d0a70756220666e2063395f726575736528297b377d",
		),
		_ => Option::None,
	};
	if let Some(source_hex) = moved_value {
		assert_eq!(hex(source.as_bytes()), source_hex);
		assert_eq!(
			expected["display"],
			"Cannot read, the value has been moved"
		);
		expected["display"] = json!("Cannot read the value: it has been moved");
	}
	expected
}

// Stock replay is a normal green test. No strict new test is registered here.
const REPLAY_CAP: usize = 2 << 20;
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
fn legacy_contract() {
	run_contract("stock");
}

#[test]
#[cfg(target_os = "linux")]
fn contract() {
	run_contract("fresh");
}

fn run_contract(engine: &str) {
	let frozen = include_str!("../../cmp_ordering_stock.tsv");
	// Pin the entire approved TSV, including the archival Debug column.
	let mut hash = Command::new("/usr/bin/sha256sum")
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	hash.stdin
		.take()
		.unwrap()
		.write_all(frozen.as_bytes())
		.unwrap();
	let hash = hash.wait_with_output().unwrap();
	assert!(hash.status.success());
	assert!(hash.stderr.is_empty());
	assert!(String::from_utf8(hash.stdout)
		.unwrap()
		.starts_with("57491aa1e653620249949b9991399f3d412d61b0fb48c3de2a37c3c8383e936e  -"));
	let mut lines = frozen.lines();
	assert_eq!(
		lines.next().unwrap(),
		"case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tpolicy\tfresh_expectation_hex\truling_hex"
	);
	let lines = lines.collect::<Vec<_>>();
	assert_eq!(lines.len(), COUNT);
	assert_eq!(COUNT, 210);
	let counts: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_p1_counts.json")).unwrap();
	let mut stale_admissions: Json = serde_json::from_str(C9_STALE_REFUSAL_ADMISSIONS).unwrap();
	assert_eq!(stale_admissions["schema"], 1);
	assert_eq!(stale_admissions["rows"].as_array().unwrap().len(), 2);
	let c13: Json = serde_json::from_str(include_str!("c13_admissions.json")).unwrap();
	assert_eq!(c13["schema"], 1);
	assert_eq!(c13["rows"].as_array().unwrap().len(), 3);
	stale_admissions["rows"]
		.as_array_mut()
		.unwrap()
		.extend(c13["rows"].as_array().unwrap().iter().cloned());
	let c17c: Json = serde_json::from_str(include_str!("c17c_admissions.json")).unwrap();
	assert_eq!(c17c["schema"], 1);
	assert_eq!(c17c["rows"].as_array().unwrap().len(), 2);
	stale_admissions["rows"]
		.as_array_mut()
		.unwrap()
		.extend(c17c["rows"].as_array().unwrap().iter().cloned());
	let c18a: Json = serde_json::from_str(include_str!("c18a_admissions.json")).unwrap();
	assert_eq!(c18a["schema"], 1);
	assert_eq!(c18a["rows"].as_array().unwrap().len(), 1);
	stale_admissions["rows"]
		.as_array_mut()
		.unwrap()
		.extend(c18a["rows"].as_array().unwrap().iter().cloned());
	let archived_dispositions: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_dispositions.json")).unwrap();
	assert!(matches!(engine, "stock" | "fresh"));
	let owned = std::env::var_os("GAP_OUT").is_none();
	let out = std::env::var_os("GAP_OUT")
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			std::env::temp_dir().join(format!(
				"rune-c9-stock-replay-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			))
		});
	fs::create_dir(&out).unwrap();
	let start_all = Instant::now();
	for (i, ((label, source, mode, policy), line)) in CASES.iter().zip(lines).enumerate() {
		assert!(
			start_all.elapsed() < Duration::from_secs(1800),
			"C9 replay whole-run bound"
		);
		let c = line.split('\t').collect::<Vec<_>>();
		assert_eq!(c.len(), 9);
		assert_eq!(c[0], *label);
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
				"--fsize=8388608",
				"--core=0",
				"--",
			])
			.arg(std::env::current_exe().unwrap());
		command.process_group(0);
		let mut child = command
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::cmp_ordering::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("C9_ENGINE", engine)
			.env("PATH", "/usr/bin:/bin")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C9_CASE", i.to_string())
			.env("C9_RESULT", &result)
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
		e["observation"]["outcome"] = super::d1c_diagnostics::d1d_before("cmp_ordering", label, source, engine, &e["observation"]["outcome"]);
		assert_eq!(e["schema"], 1);
		assert_eq!(e["engine"], engine);
		assert_eq!(e["index"], i);
		assert_eq!(e["label"], *label);
		assert_eq!(e["mode"], *mode);
		assert_eq!(e["policy"], *policy);
		assert_eq!(e["source_hex"], c[2]);
		assert_eq!(e["fuel"], FUEL);
		if engine == "stock" {
			let observed = &e["observation"];
			assert_eq!(observed["invoked"], true);
			assert_eq!(observed["host_calls"], 0);
			let stock: Json = serde_json::from_str(&pins_decode(c[3])).unwrap();
			// P1 validates both archived and common Debug counts. Archive contents
			// remain raw metadata; only the common semantic object is compared.
			project_p1(&observed["outcome"], &counts[*label]).unwrap();
			let observed_projection = semantic_projection(&observed["outcome"]).unwrap();
			let observed_projection = super::d1b_diagnostics::intentional_json_before(
				"cmp_ordering",
				label,
				source,
				"stock",
				&observed_projection,
			);
			compare_p1(
				&observed_projection,
				&d1_cmp_expectation(
					label,
					source,
					&semantic_projection(&stock).unwrap(),
					"stock",
				),
				&counts[*label],
			)
			.unwrap();
			assert_eq!(observed["reuse"]["status"], "value");
			assert_eq!(
				observed["reuse"]["value"]["payload"],
				json!({"type":"I64","value":"7"})
			);
		} else if let Some(admission) = stale_admissions["rows"]
			.as_array()
			.unwrap()
			.iter()
			.find(|row| row["label"] == *label)
		{
			// C11/C12 admissions preserve the C9 source, stock observation and streams.
			let observed = &e["observation"];
			let audit = &observed["fresh_audit"];
			let stock: Json = serde_json::from_str(&pins_decode(c[3])).unwrap();
			assert_eq!(admission["index"], i);
			assert_eq!(admission["source_hex"], c[2]);
			assert_eq!(
				admission["source_sha256"],
				archived_dispositions[*label]["source_sha256"]
			);
			assert_eq!(admission["legacy_policy"], c[6]);
			assert_eq!(admission["legacy_policy"], "Refused-surface");
			assert_eq!(
				admission["legacy_fresh_refusal"],
				archived_dispositions[*label]["fresh_expectation"]
			);
			assert_eq!(admission["disposition"], "Exact");
			assert_eq!(admission["expected"], semantic_projection(&stock).unwrap());
			assert_eq!(admission["stdout_hex"], c[4]);
			assert_eq!(admission["stderr_hex"], c[5]);
			compare_p1(
				&observed["outcome"],
				&d1_cmp_expectation(label, source, &admission["expected"], "fresh"),
				&counts[*label],
			)
			.unwrap();
			assert_eq!(
				audit["returned_root"],
				observed["outcome"]["status"] == "value"
			);
			assert_eq!(observed["invoked"], true);
			assert_eq!(observed["host_calls"], 0);
			for name in ["no_cleanup_error", "row_idle", "reuse", "idle"] {
				assert_eq!(audit[name], true);
			}
			for name in [
				"row_live_entries",
				"row_payload_bytes",
				"live_entries",
				"payload_bytes",
			] {
				assert_eq!(audit[name], 0);
			}
			assert_eq!(audit["call_chain"], json!([]));
			// The new observer pins reuse as audit["reuse"] == true above.
		} else {
			compare_envelope(source, label, &e, &stdout, &stderr).unwrap();
		}

		assert_eq!(hex(&stdout), c[4]);
		assert_eq!(hex(&stderr), c[5]);
	}
	fs::write(
		out.join("COMPLETE"),
		"210 frozen stock replay rows PASS; all actual raw retained\n",
	)
	.unwrap();
	if owned {
		fs::remove_dir_all(out).unwrap();
	}
}

// Called only by the implementation's future registered new contract.
#[allow(dead_code)]
fn compare_envelope(
	source: &str,
	label: &str,
	envelope: &Json,
	stdout: &[u8],
	stderr: &[u8],
) -> std::result::Result<(), String> {
	let dispositions: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_dispositions.json")).unwrap();
	let counts: Json =
		serde_json::from_str(include_str!("../../cmp_ordering_p1_counts.json")).unwrap();
	if dispositions != serde_json::from_str::<Json>(C9_PINNED_DISPOSITIONS).unwrap()
		|| counts != serde_json::from_str::<Json>(C9_PINNED_COUNTS).unwrap()
	{
		return Err("manifest drift".into());
	}
	let d1_expected = d1_cmp_expectation(
		label,
		source,
		&dispositions.get(label).ok_or("unknown label")?["fresh_expectation"],
		"fresh",
	);
	let expected = &d1_expected;
	if envelope["source_hex"] != hex(source.as_bytes())
		|| envelope["label"] != label
		|| envelope["engine"] != "fresh"
	{
		return Err("source or identity drift".into());
	}
	let frozen = include_str!("../../cmp_ordering_stock.tsv");
	let (index, cells) = frozen
		.lines()
		.skip(1)
		.enumerate()
		.find_map(|(i, line)| {
			let cells = line.split('\t').collect::<Vec<_>>();
			(cells[0] == label).then_some((i, cells))
		})
		.ok_or("unknown frozen label")?;
	if envelope["schema"] != 1
		|| envelope["index"] != index
		|| envelope["fuel"] != FUEL
		|| envelope["mode"] != cells[1]
		|| envelope["policy"] != cells[6]
		|| cells[2] != hex(source.as_bytes())
		|| cells[4] != hex(stdout)
		|| cells[5] != hex(stderr)
	{
		return Err("frozen identity/source/stream drift".into());
	}
	let obs = &envelope["observation"];
	let audit = &obs["fresh_audit"];
	let outcome = &obs["outcome"];
	if audit["no_cleanup_error"] != true
		|| audit["row_live_entries"] != 0
		|| audit["row_payload_bytes"] != 0
		|| audit["row_idle"] != true
		|| audit["reuse"] != true
		|| audit["live_entries"] != 0
		|| audit["payload_bytes"] != 0
		|| audit["idle"] != true
		|| obs["host_calls"] != 0
	{
		return Err("cleanup, balance, host or reuse drift".into());
	}
	match dispositions[label]["policy"].as_str().unwrap() {
		"Exact" => {
			if obs["invoked"] != true || audit["returned_root"] != (outcome["status"] == "value") {
				return Err("invocation or root drift".into());
			}
			if label == "access/leased-parent-mutate" {
				compare_c9_retained(label, source, outcome, expected, &counts[label])
			} else if compare_p1(
				&super::d1b_diagnostics::intentional_json_before(
					"cmp_ordering",
					label,
					source,
					"fresh",
					&semantic_projection(outcome).map_err(|_| "execution projection failed")?,
				),
				expected,
				&counts[label],
			)
			.is_ok()
			{
				Ok(())
			} else {
				compare_c9_diagnostic_retained(
					label,
					source,
					outcome,
					expected,
					&counts[label],
					&audit["call_chain"],
				)
			}
		}
		"Refused-surface" | "Retained-arity" | "Refused-runtime" => {
			let at = &outcome["location"];
			let start = at["start"].as_u64().ok_or("missing call start")? as usize;
			let end = at["end"].as_u64().ok_or("missing call end")? as usize;
			let bytes = source
				.as_bytes()
				.get(start..end)
				.ok_or("invalid call span")?;
			let mut actual = json!({"status":outcome["status"],"kind":outcome["kind"],"operation":outcome["operation"],"refusal":outcome["refusal"],"invoked":obs["invoked"],"source":at["source"],"start":start,"end":end,"span_hex":hex(bytes)});
			if dispositions[label]["policy"] == "Refused-runtime" {
				actual["call_chain"] = audit["call_chain"].clone();
			} else if !audit["call_chain"]
				.as_array()
				.ok_or("missing chain")?
				.is_empty()
			{
				return Err("compile refusal invoked caller chain".into());
			}
			if actual != *expected {
				return Err("typed refusal drift".into());
			}
			Ok(())
		}
		_ => Err("unlisted policy".into()),
	}
}
