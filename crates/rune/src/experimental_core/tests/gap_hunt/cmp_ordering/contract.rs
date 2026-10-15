// Stock replay is a normal green test. No strict fresh test is registered here.
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
fn stock_contract() {
	run_contract("stock");
}

#[test]
#[cfg(target_os = "linux")]
fn fresh_contract() {
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
		let e: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
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
			compare_p1(
				&observed["outcome"],
				&semantic_projection(&stock).unwrap(),
				&counts[*label],
			)
			.unwrap();
			assert_eq!(observed["reuse"]["status"], "value");
			assert_eq!(
				observed["reuse"]["value"]["payload"],
				json!({"type":"I64","value":"7"})
			);
		} else {
			compare_fresh_envelope(source, label, &e, &stdout, &stderr).unwrap();
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

// Called only by the implementation's future registered fresh contract.
#[allow(dead_code)]
fn compare_fresh_envelope(
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
	let expected = &dispositions.get(label).ok_or("unknown label")?["fresh_expectation"];
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
			} else if compare_p1(outcome, expected, &counts[label]).is_ok() {
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
