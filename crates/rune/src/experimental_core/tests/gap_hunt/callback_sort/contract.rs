fn drain_replay(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut result = Vec::new();
	let mut buffer = [0u8; 4096];
	loop {
		let count = pipe.read(&mut buffer).unwrap();
		if count == 0 {
			break;
		}
		if result.len() + count > CAP {
			exceeded.store(true, Ordering::SeqCst);
		} else {
			result.extend_from_slice(&buffer[..count]);
		}
	}
	result
}
#[test]
#[ignore = "C11 frozen stock replay; bounded children and external raw receipts"]
fn stock_contract() {
	let out =
		PathBuf::from(std::env::var_os("C11_REPLAY_OUT").expect("explicit raw replay directory"));
	fs::create_dir(&out).unwrap();
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
	let cross = read_json(include_str!("../../callback_sort_canonical.json"));
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let start = Instant::now();
	let mut completed = 0;
	let mut digested = 0;
	for (i, line) in FROZEN.lines().skip(1).enumerate() {
		assert!(start.elapsed() < Duration::from_secs(3600));
		let cells = line.split('\t').collect::<Vec<_>>();
		assert_eq!(cells.len(), 9);
		let (label, source, fixture, proposed_policy, mode, fuel, cancel) = CASES[i];
		assert_eq!(cells[0], label);
		assert_eq!(decode(cells[2]), source.as_bytes());
		assert_eq!(cells[1], mode);
		if label == "depth/8" {
			assert_eq!(proposed_policy, "Retained-depth");
			assert_eq!(cells[6], "Exact");
		} else {
			assert_eq!(proposed_policy, cells[6]);
		}
		assert_eq!(dispositions[label]["policy"], cells[6]);
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let result = dir.join("observation.json");
		fs::write(dir.join("source.rn"), source).unwrap();
		let mut child = Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"experimental_core::tests::gap_hunt::callback_sort::child",
				"--ignored",
				"--nocapture",
				"--quiet",
			])
			.env_clear()
			.env("PATH", "/usr/bin:/bin")
			.env("HOME", "/home/me")
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C11_CASE", i.to_string())
			.env("C11_ENGINE", "stock")
			.env("C11_RESULT", &result)
			.current_dir("/tmp")
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let exceeded = Arc::new(AtomicBool::new(false));
		let so = child.stdout.take().unwrap();
		let se = child.stderr.take().unwrap();
		let stdout_limit = exceeded.clone();
		let stderr_limit = exceeded.clone();
		let stdout = std::thread::spawn(move || drain_replay(so, stdout_limit));
		let stderr = std::thread::spawn(move || drain_replay(se, stderr_limit));
		let launched = Instant::now();
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			if launched.elapsed() > Duration::from_secs(45)
				|| start.elapsed() > Duration::from_secs(3600)
				|| exceeded.load(Ordering::SeqCst)
			{
				child.kill().unwrap();
				let status = child.wait().unwrap();
				break status;
			}
			std::thread::sleep(Duration::from_millis(5));
		};
		let stdout = stdout.join().unwrap();
		let stderr = stderr.join().unwrap();
		fs::write(dir.join("stdout"), &stdout).unwrap();
		fs::write(dir.join("stderr"), &stderr).unwrap();
		fs::write(dir.join("receipt.json"),serde_json::to_vec_pretty(&json!({"index":i,"label":label,"status":format!("{status}"),"elapsed_ms":launched.elapsed().as_millis(),"stream_bound_exceeded":exceeded.load(Ordering::SeqCst)})).unwrap()).unwrap();
		assert!(
			status.success() && !exceeded.load(Ordering::SeqCst),
			"child STOP {i}"
		);
		assert_eq!(stdout, decode(cells[4]));
		assert_eq!(stderr, decode(cells[5]));
		assert!(fs::metadata(&result).unwrap().len() <= 64 * 1024 * 1024);
		// The child archive is already preserved; only a copy is projected.
		let mut raw: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		raw["observation"]["outcome"] = super::d1c_diagnostics::before("callback_sort", label, source, "stock", &raw["observation"]["outcome"]);
		assert_eq!(raw["schema"], 1);
		assert_eq!(raw["engine"], "stock");
		assert_eq!(raw["index"], i);
		assert_eq!(raw["label"], label);
		assert_eq!(raw["source_hex"], hex(source.as_bytes()));
		assert_eq!(raw["fixture"], fixture);
		assert_eq!(raw["policy"], proposed_policy);
		assert_eq!(raw["mode"], mode);
		assert_eq!(raw["fuel"], fuel);
		assert_eq!(raw["stock_cancel_armed"], false);
		assert_eq!(raw["fresh_cancel_at_charge_proposal"], json!(cancel));
		let observation = &raw["observation"];
		assert_eq!(observation["invoked"], true);
		if fixture == "self-cycle" {
			assert_eq!(observation["cleanup"]["cycle_cleared"], true);
		}
		let actual = common(&project_p1(observation, &counts[label]).unwrap());
		let mut actual = actual;
		let d1b_outcome = super::d1b_diagnostics::intentional_json_before(
			"callback_sort",
			label,
			source,
			"stock",
			&actual["outcome"],
		);
		actual["outcome"] = d1b_outcome;
		let actual = super::method_diagnostics::intentional_json_before(
			"callback_sort",
			label,
			source,
			"stock",
			&actual,
		);
		let stored: Json = serde_json::from_slice(&decode(cells[3])).unwrap();
		check_cell(&stored, &actual).unwrap();
		digested += usize::from(stored.get("storage").is_some());
		let bytes = canonical(&actual);
		assert_eq!(cross[label]["stock"]["bytes"], bytes.len());
		assert_eq!(cross[label]["stock"]["sha256"], digest(&bytes));
		completed += 1;
	}
	assert_eq!((completed, digested), (131, 9));
	contract_completed("stock", completed);
}
