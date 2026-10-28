fn live_stock_case(index: usize, dir: &Path, counts: &Json) -> (super::live_oracle::Run, Json) {
	let (label, source, fixture, policy, mode, fuel, _) = CASES[index];
	let index_text = index.to_string();
	let run = super::live_oracle::run_child_with_limits(
		"experimental_core::tests::gap_hunt::callback_sort::child", label, dir,
		"C11_RESULT", &[("C11_CASE", index_text.as_ref()), ("C11_ENGINE", "stock".as_ref())],
		45, Duration::from_secs(45),
	);
	let mut raw = run.observation.clone();
	assert_eq!(raw["schema"], 1);
	assert_eq!(raw["engine"], "stock");
	assert_eq!(raw["index"], index);
	assert_eq!(raw["label"], label);
	assert_eq!(raw["source_hex"], hex(source.as_bytes()));
	assert_eq!(raw["fixture"], fixture);
	assert_eq!(raw["policy"], policy);
	assert_eq!(raw["mode"], mode);
	assert_eq!(raw["fuel"], fuel);
	raw["observation"]["outcome"] = super::d1c_diagnostics::before(
		"callback_sort", label, source, "stock", &raw["observation"]["outcome"],
	);
	let mut stock = common(&project_p1(&raw["observation"], &counts[label]).unwrap());
	// This progression compares the stock kind display directly; its fresh
	// public display is translated separately in c11_s7a_runtime_progression.
	if label != "type/non-vector" {
		stock["outcome"] = super::d1b_diagnostics::intentional_json_before(
			"callback_sort", label, source, "stock", &stock["outcome"],
		);
		stock = super::method_diagnostics::intentional_json_before(
			"callback_sort", label, source, "stock", &stock,
		);
	}
	(run, stock)
}

#[test]
#[ignore = "C11 strict execution 131; live child observations required"]
fn contract() {
	let out =
		PathBuf::from(std::env::var_os("C11_REPLAY_OUT").expect("explicit raw replay directory"));
	fs::create_dir(&out).unwrap();
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));

	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let start = Instant::now();
	let mut completed = 0;

	for (i, line) in INPUTS.lines().skip(1).enumerate() {
		assert!(start.elapsed() < Duration::from_secs(3600));
		let cells = line.split('\t').collect::<Vec<_>>();
		assert_eq!(cells.len(), 6);
		let (label, source, fixture, proposed_policy, mode, fuel, cancel) = CASES[i];
		assert_eq!(cells[0], label);
		assert_eq!(decode(cells[2]), source.as_bytes());
		assert_eq!(cells[1], mode);
		if label == "depth/8" {
			assert_eq!(proposed_policy, "Retained-depth");
			assert_eq!(cells[5], "Exact");
		} else {
			assert_eq!(proposed_policy, cells[5]);
		}
		assert_eq!(dispositions[label]["policy"], cells[5]);
		let dir = out.join(format!("{i:03}"));
		fs::create_dir(&dir).unwrap();
		let (stock_run, live_stock) = live_stock_case(i, &dir.join("stock"), &counts);
		let completion = if cells[5] == "Retained-fuel" {
			let anchor = CASES.iter().position(|case| case.0 == "fuel/128").unwrap();
			Some(live_stock_case(anchor, &dir.join("stock-completion"), &counts).1)
		} else if cells[5] == "Retained-cancel" {
			Some(live_stock.clone())
		} else { None };
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
			.env("HOME", fs::canonicalize(&dir).unwrap())
			.env("LANG", "C.UTF-8")
			.env("TZ", "UTC")
			.env("C11_CASE", i.to_string())
			.env("C11_ENGINE", "fresh")
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
		assert_eq!(stdout, decode(cells[3]));
		assert_eq!(stderr, decode(cells[4]));
		assert_eq!(stdout, stock_run.stdout, "stock stdout {label}");
		assert_eq!(stderr, stock_run.stderr, "stock stderr {label}");
		assert!(fs::metadata(&result).unwrap().len() <= 64 * 1024 * 1024);
		// The child archive is already preserved; only a copy is projected.
		let mut raw: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		raw["observation"]["outcome"] = super::d1c_diagnostics::before(
			"callback_sort",
			label,
			source,
			"fresh",
			&raw["observation"]["outcome"],
		);
		assert_eq!(raw["schema"], 1);
		assert_eq!(raw["engine"], "fresh");
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
		assert_eq!(
			observation["fresh_audit"]["cancel_at_charge"],
			json!(cancel)
		);

		if fixture == "self-cycle" {
			assert_eq!(observation["cleanup"]["cycle_cleared"], true);
		}
		compare_row(
			label,
			source,
			fixture,
			cells[5],
			observation,
			&counts[label],
			&dispositions[label],
			&live_stock,
			completion.as_ref(),
		)
		.unwrap();
		completed += 1;
	}
	assert_eq!(completed, 131);
	contract_completed("fresh", completed);
}
fn compare_row(
	label: &str,
	source: &str,
	fixture: &str,
	policy: &str,
	observation: &Json,
	counts: &Json,
	disposition: &Json,
	live_stock: &Json,
	completion: Option<&Json>,
) -> Result<(), String> {
	let audit = &observation["fresh_audit"];
	let clean = json!({"cleanup_error":null,"executor_idle":true,"live_entries":0,"payload_bytes":0,"reuse_i64":7});
	if audit["clean"] != clean
		|| audit["row_idle"] != true
		|| audit["row_live_entries"] != 0
		|| audit["row_payload_bytes"] != 0
		|| audit["source_sha256"] != digest(source.as_bytes())
	{
		return Err(format!("audit drift {label}: {audit}"));
	}
	if c11_check_stale_refusal_admission(label, source, observation, counts, disposition, live_stock)? {
		return Ok(());
	}
	if c11_s7a_runtime_progression(label, source, fixture, policy, observation, disposition, live_stock)? {
		return Ok(());
	}
	if matches!(policy, "Refused-surface" | "Refused-runtime") {
		let spans = read_json(include_str!("../../callback_sort_refusal_spans.json"));
		let expected = &spans[label];
		let baseline = live_stock;
		let mut actual = observation["outcome"].clone();
		actual["source_sha256"] = json!(digest(source.as_bytes()));
		actual["invoked"] = observation["invoked"].clone();
		if !refusal_valid(label, source, &actual, &audit["clean"])
			|| actual["location"]
				!= json!({"source":expected["source"],"start":expected["start"],"end":expected["end"]})
			|| expected["source_sha256"] != digest(source.as_bytes())
		{
			return Err(format!("refusal identity/span drift {label}"));
		}
		let at = &actual["location"];
		let start = at["start"].as_u64().ok_or("missing start")? as usize;
		let end = at["end"].as_u64().ok_or("missing end")? as usize;
		if source.as_bytes().get(start..end).map(hex)
			!= expected["span_hex"].as_str().map(str::to_owned)
		{
			return Err("refusal byte span drift".into());
		}
		if observation["before"] != baseline["before"]
			|| observation["before"] != observation["post"]
			|| observation["reuse"] != baseline["reuse"]
			|| observation["events_after_reuse"]
				!= if label == "old-c9/refusal/iter-any" {
					json!([])
				} else {
					json!([{"event":"mark","hex":"7265757365"}])
				} || observation["events"] != json!([])
		{
			return Err("refusal effect drift".into());
		}
		return Ok(());
	}
	let actual = common(&project_p1(observation, counts)?);
	if policy == "Retained-depth" {
        let depth: usize = label.strip_prefix("depth/").ok_or("depth label")?.parse().map_err(|_| "depth number")?;
        let expected = c11_expected_depth(depth, false);
        if audit["typed_failure_kind"] != Json::Null || actual != expected {
            return Err(format!("derived depth drift {label}"));
        }
        return Ok(());
    }
	if matches!(policy, "Retained-fuel" | "Retained-cancel") {
		let (stock, _anchor) = resource_completion_live(label, source, fixture, live_stock, completion.ok_or("missing live completion")?)?;
		let kind = if policy == "Retained-fuel" {
			"FuelExhausted"
		} else {
			"Cancelled"
		};
		if !resource_valid(&stock, &actual, &audit["resource"], kind) {
			return Err(format!("resource constraint drift {label}"));
		}
		if actual["outcome"]["status"] == "error" {
            let error = &actual["outcome"];
            let sites = error["sites"].as_array().ok_or("resource error sites")?;
            if error["kind"] != kind || error["typed"] != json!({"raw_kind":kind})
                || error["display"] != format!("execute: {kind}") || error["chain"] != json!([])
                || error.as_object().map(|o| o.len()) != Some(7) || sites.len() != 1 {
                return Err(format!("resource error detail drift {label}"));
            }
            let site = &sites[0];
            let start = site["start"].as_u64().ok_or("resource start")? as usize;
            let end = site["end"].as_u64().ok_or("resource end")? as usize;
            if site["source"] != 0 || source.get(start..end).is_none()
                || serde_json::from_str::<Json>(site["frames"].as_str().ok_or("resource frames")?).is_err() {
                return Err(format!("resource site drift {label}"));
            }
        }
		return Ok(());
	}
	if !matches!(policy, "Exact" | "Intentional-divergence") {
		return Err("unknown execution policy".into());
	}
	let expected = c11_live_expectation(label, fixture, &actual, live_stock)?;

    if c11_diagnostic_span(label).is_some() {
		return c11_enforce_diagnostic(
			label,
			source,
			&actual,
			&expected,
			&audit["raw_failure"]["call_chain"],
		);
	}
	if actual != expected {
		return Err(format!("strict portable drift {label}"));
	}
	Ok(())
}

// A failure stops the new sort immediately. Stock continues its comparison
// tape; only the reviewed failure rows have that difference. Compare the
// retained prefix to the live stock tape and model the resulting graph.
fn c11_live_expectation(label: &str, input_fixture: &str, actual: &Json, stock: &Json) -> Result<Json, String> {
    let count = match label {
        "failure/first-call-failure" | "failure/all-failing" | "failure/non-ordering-return"
        | "failure/panic" | "frames/caller-chain" | "failure/child-mutation"
        | "failure/shared-child-mutation" => Some(1),
        "failure/later-failure" => Some(3),
        "nested/different-error" => Some(2),
        _ => None,
    };
    let Some(count) = count else { return Ok(stock.clone()); };
    let tape = actual["events"].as_array().ok_or("failure tape")?;
    let stock_tape = stock["events"].as_array().ok_or("stock failure tape")?;
    if tape.len() != count || stock_tape.get(..count) != Some(tape.as_slice()) {
        return Err(format!("failure callback prefix drift {label}"));
    }
    let mut after = tape.clone();
    after.push(json!({"event":"mark","hex":"7265757365"}));
    if actual["events_after_reuse"] != json!(after) || actual["before"] != stock["before"] {
        return Err(format!("failure lifecycle drift {label}"));
    }
    let expected_post = if matches!(label, "failure/child-mutation" | "failure/shared-child-mutation") {
        let mut store = Store::new(Limits::default()).map_err(|_| "model store")?;
        let (root, values, state) = fixture(&mut store, input_fixture);
        let first = store.get(values, 0).map_err(|_| "model first")?;
        let child = store.get(first, 1).map_err(|_| "model child")?;
        store.push(child, Value::I64(9)).map_err(|_| "model mutation")?;
        let result = graph(&mut store, root);
        for value in [child, first, values, state, root] { store.release(value).map_err(|_| "model release")?; }
        if store.live_entries() != 0 || store.payload_bytes() != 0 { return Err("model cleanup".into()); }
        result
    } else { actual["before"].clone() };
    if actual["post"] != expected_post { return Err(format!("failure post-state drift {label}")); }
    let mut expected = stock.clone();
    expected["events"] = actual["events"].clone();
    expected["events_after_reuse"] = actual["events_after_reuse"].clone();
    expected["post"] = expected_post;
    Ok(expected)
}

fn c11_check_stale_refusal_admission(
	label: &str,
	source: &str,
	observation: &Json,
	counts: &Json,
	disposition: &Json,
	live_stock: &Json,
) -> Result<bool, String> {
	let overlay: Json = serde_json::from_str(C11_STALE_REFUSAL_ADMISSIONS)
		.map_err(|_| "stale refusal admissions JSON")?;
	if overlay["schema"] != 1 {
		return Err("stale refusal admissions schema".into());
	}
	let Some(row) = overlay["rows"]
		.as_array()
		.ok_or("stale refusal admissions rows")?
		.iter()
		.find(|row| row["label"] == label)
	else {
		return Ok(false);
	};
	let index = usize::try_from(row["index"].as_u64().ok_or("stale admission index")?)
		.map_err(|_| "stale admission index overflow")?;
	if index >= CASES.len()
		|| CASES[index].0 != label
		|| CASES[index].1 != source
		|| row["source_sha256"] != digest(source.as_bytes())
		|| row["legacy_policy"] != "Refused-surface"
		|| disposition["policy"] != row["legacy_policy"]
	{
		return Err(format!("stale refusal identity drift {label}"));
	}
	let legacy_spans = read_json(include_str!("../../callback_sort_refusal_spans.json"));
	if row["legacy_refusal_span"] != legacy_spans[label]
		|| legacy_spans[label]["source_sha256"] != digest(source.as_bytes())
	{
		return Err(format!("stale refusal span drift {label}"));
	}
	let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
	let legacy = &refusals[label];
	let legacy_operation = row["legacy_operation"]
		.as_str()
		.ok_or("stale refusal operation")?;
	if !matches!(legacy_operation, "CallClosure" | "Method")
		|| legacy["fresh_constraint"]["operation"] != legacy_operation
		|| legacy["fresh_constraint"]["source_sha256"] != digest(source.as_bytes())
	{
		return Err(format!("stale refusal history drift {label}"));
	}
	let stock = live_stock;
	let actual = common(&project_p1(observation, counts)?);
	if label == "refused/move-capture" {
		// The root fix closes the old ownership gap; require actual stock now.
		if actual != *stock {
			return Err(format!("C11c exact move ownership drift {label}"));
		}
		return Ok(true);
	}
	match row["disposition"]
		.as_str()
		.ok_or("stale admission disposition")?
	{
		"Exact" => {
			if actual != *stock {
				return Err(format!("stale admission not stock-exact {label}"));
			}
		}
		"Retained-frame" => {
			let frame = &row["retained_frame"];
			let sites = actual["outcome"]["sites"]
				.as_array()
				.ok_or("retained frame sites")?;
			if sites != &[frame["fresh_site"].clone()] {
				return Err("retained frame literal drift".into());
			}
			let call_chain = frame["call_chain"].clone();
			let text = sites[0]["frames"].as_str().ok_or("retained frame text")?;
			let parsed: Json = serde_json::from_str(text).map_err(|_| "retained frame JSON")?;
			if parsed != call_chain {
				return Err("retained frame caller drift".into());
			}
			let retained = c11_retained_frame_shape(source, &actual, stock, &call_chain)?;
			if retained["categories"] != frame["categories"] {
				return Err("retained frame category drift".into());
			}
		}
		"Retained-ownership" => {
			let path = row["only_allowed_difference"]
				.as_str()
				.ok_or("ownership difference path")?;
			let differences = c11_json_differences(stock, &actual, "", Some(path));
			if differences != [path]
				|| stock["post"]["nodes"][2][0] != "OpaqueAny"
				|| actual["post"]["nodes"][2][0] != "Vec"
			{
				return Err("retained ownership boundary drift".into());
			}
		}
		_ => return Err("unknown stale refusal disposition".into()),
	}
	Ok(true)
}

fn c11_json_differences(
	left: &Json,
	right: &Json,
	path: &str,
	atomic_path: Option<&str>,
) -> Vec<String> {
	if Some(path) == atomic_path {
		return if left == right {
			Vec::new()
		} else {
			vec![path.into()]
		};
	}
	if let (Some(left), Some(right)) = (left.as_object(), right.as_object()) {
		let keys = left
			.keys()
			.chain(right.keys())
			.collect::<std::collections::BTreeSet<_>>();
		return keys
			.into_iter()
			.flat_map(|key| {
				let child = format!("{path}/{key}");
				match (left.get(key), right.get(key)) {
					(Some(left), Some(right)) => {
						c11_json_differences(left, right, &child, atomic_path)
					}
					_ => vec![child],
				}
			})
			.collect();
	}
	if let (Some(left), Some(right)) = (left.as_array(), right.as_array()) {
		let len = left.len().max(right.len());
		return (0..len)
			.flat_map(|index| {
				let child = format!("{path}/{index}");
				match (left.get(index), right.get(index)) {
					(Some(left), Some(right)) => {
						c11_json_differences(left, right, &child, atomic_path)
					}
					_ => vec![child],
				}
			})
			.collect();
	}
	if left == right {
		Vec::new()
	} else {
		vec![if path.is_empty() {
			"/".into()
		} else {
			path.into()
		}]
	}
}

// Validate the frozen representation shape; exact per-row pins are checked separately.
fn c11_retained_frame_shape(
	source: &str,
	actual: &Json,
	stock: &Json,
	call_chain: &Json,
) -> Result<Json, String> {
	let sites = actual["outcome"]["sites"].as_array().ok_or("execution sites")?;
	let oracle_sites = stock["outcome"]["sites"].as_array().ok_or("stock sites")?;
	if sites.len() != 1 || oracle_sites.is_empty() || oracle_sites[0]["frames"] != "[]" {
		return Err("not callback windows".into());
	}
	let execution = &sites[0];
	let mut primary = execution.clone();
	primary["frames"] = json!("[]");
	let mut categories = Vec::new();
	if primary != oracle_sites[0] {
		c11_c5_primary(source, &primary, &oracle_sites[0])?;
		categories.push("Retained-site");
	}
	if execution["frames"] != "[]" || oracle_sites.len() > 1 {
		categories.push("Retained-frame (callback)");
	}
	let frames: Json = serde_json::from_str(execution["frames"].as_str().ok_or("execution frames")?)
		.map_err(|_| "frame JSON")?;
	if frames != *call_chain {
		return Err("actual chain drift".into());
	}
	let callers = frames.as_array().ok_or("caller array")?;
	let mut mapped = Vec::new();
	let mut caller_hex = Vec::new();
	for at in callers {
		if at.as_object().ok_or("caller object")?.len() != 3 || at["source"] != 0 {
			return Err("caller schema".into());
		}
		let start = usize::try_from(at["start"].as_u64().ok_or("caller start")?)
			.map_err(|_| "start overflow")?;
		let end =
			usize::try_from(at["end"].as_u64().ok_or("caller end")?).map_err(|_| "end overflow")?;
		let bytes = source.get(start..end).ok_or("caller span")?;
		let expr =
			crate::parse::parse_all::<crate::ast::Expr>(bytes, crate::SourceId::EMPTY, false)
				.map_err(|_| "caller parse")?;
		if !matches!(expr, crate::ast::Expr::Call(_)) {
			return Err("caller is not call".into());
		}
		caller_hex.push(hex(bytes.as_bytes()));
		let mut site = at.clone();
		site["frames"] = json!("[]");
		mapped.push(site);
	}
	let mut consumed = 0;
	let mut legacy_counts = Vec::new();
	for window in &oracle_sites[1..] {
		let mut explicit = window.clone();
		explicit["frames"] = json!("[]");
		if mapped.get(consumed) != Some(&explicit) {
			return Err("explicit caller drift".into());
		}
		consumed += 1;
		let legacy = window["frames"].as_str().ok_or("legacy frames")?;
		let count = if legacy == "[]" {
			0
		} else {
			let count = legacy.matches("CallFrame {").count();
			if count == 0 || !legacy.starts_with("[CallFrame {") || !legacy.ends_with(" }]") {
				return Err("unknown legacy frame shape".into());
			}
			count
		};
		consumed = consumed.checked_add(count).ok_or("caller count overflow")?;
		if consumed > mapped.len() {
			return Err("legacy caller shortfall".into());
		}
		legacy_counts.push(count);
	}
	if consumed != mapped.len() {
		return Err("leftover callers".into());
	}
	let mut expected = stock.clone();
	expected["outcome"]["sites"] = actual["outcome"]["sites"].clone();
	if *actual != expected {
		return Err("non-site callback drift".into());
	}
	Ok(
		json!({"stock_sites":oracle_sites,"fresh_sites":sites,"call_chain":call_chain,"caller_hex":caller_hex,"mapped_callers":mapped,"legacy_counts":legacy_counts,"categories":categories}),
	)
}

fn c11_control_stock(sites: Json) -> Json {
    json!({"outcome":{"status":"error","class":"test","kind":"test","typed":{},"display":"test","chain":[],"sites":sites},"before":{},"post":{},"events":[],"events_after_reuse":[],"reuse":{},"invoked":true})
}

#[test]
fn callback_frame_discovery_rejects_semantic_and_unpaired_drift() {
	c11_mixed_frame_controls();
	let source = CASES[40].1;
	let stock = &c11_control_stock(json!([{"source":0,"start":45,"end":52,"frames":"[]"},{"source":0,"start":86,"end":104,"frames":"[]"}]));
	let at = json!({"source":0,"start":86,"end":104});
	let callers = json!([at]);
	let mut actual = stock.clone();
	let mut primary = stock["outcome"]["sites"][0].clone();
	primary["frames"] = json!(serde_json::to_string(&callers).unwrap());
	actual["outcome"]["sites"] = json!([primary]);
	assert!(c11_retained_frame_shape(source, &actual, stock, &callers).is_ok());
	for chain in [
		json!([]),
		json!([at, at]),
		json!([at, at, at]),
		json!([{"source":0,"start":45,"end":52}]),
	] {
		let mut bad = actual.clone();
		bad["outcome"]["sites"][0]["frames"] = json!(serde_json::to_string(&chain).unwrap());
		assert!(c11_retained_frame_shape(source, &bad, stock, &chain).is_err());
	}
	for field in ["events", "before", "post", "reuse"] {
		let mut bad = actual.clone();
		bad[field] = Json::Null;
		assert!(c11_retained_frame_shape(source, &bad, stock, &callers).is_err());
	}
	for field in ["kind", "typed", "display", "chain", "status"] {
		let mut bad = actual.clone();
		bad["outcome"][field] = Json::Null;
		assert!(c11_retained_frame_shape(source, &bad, stock, &callers).is_err());
	}
	assert!(c11_retained_frame_shape(source, &actual, stock, &json!([])).is_err());
	let single_source = CASES[43].1;
	let single_stock = &c11_control_stock(json!([{"source":0,"start":55,"end":73,"frames":"[]"}]));
	assert!(
		c11_retained_frame_shape(single_source, single_stock, single_stock, &json!([])).is_ok()
	);
	let mut bad = single_stock.clone();
	let chain = json!([{"source":0,"start":55,"end":73}]);
	bad["outcome"]["sites"][0]["frames"] = json!(serde_json::to_string(&chain).unwrap());
	assert!(c11_retained_frame_shape(single_source, &bad, single_stock, &chain).is_err());
}

// Existing C5 condition on the primary, independent of callback caller handling.
fn c11_c5_primary(source: &str, execution: &Json, stock: &Json) -> Result<(), String> {
	use crate::ast::Spanned;
	let start = usize::try_from(execution["start"].as_u64().ok_or("C5 start")?)
		.map_err(|_| "start overflow")?;
	let end =
		usize::try_from(execution["end"].as_u64().ok_or("C5 end")?).map_err(|_| "end overflow")?;
	let oracle_end = stock["end"].as_u64().ok_or("C5 stock end")?;
	if stock["start"] != 0
		|| start == 0
		|| start >= end
		|| oracle_end <= start as u64
		|| oracle_end > end as u64
	{
		return Err("C5 prefix bounds".into());
	}
	let invocation = source.get(start..end).ok_or("C5 span")?;
	if !["format!(", "print!(", "println!("]
		.iter()
		.any(|name| invocation.starts_with(name))
		|| !invocation.ends_with(')')
	{
		return Err("C5 invocation".into());
	}
	let expr =
		crate::parse::parse_all::<crate::ast::Expr>(invocation, crate::SourceId::EMPTY, false)
			.map_err(|_| "C5 expression")?;
	if !matches!(expr, crate::ast::Expr::MacroCall(_)) {
		return Err("C5 not macro".into());
	}
	let file = crate::parse::parse_all::<crate::ast::File>(source, crate::SourceId::EMPTY, false)
		.map_err(|_| "C5 source")?;
	if !file.items.iter().any(|(item, _)| {
		if let crate::ast::Item::Fn(item) = item {
			let span = item.span().range();
			span.start <= start && end <= span.end
		} else {
			false
		}
	}) {
		return Err("C5 outside function".into());
	}
	let mut expected = stock.clone();
	expected["start"] = execution["start"].clone();
	expected["end"] = execution["end"].clone();
	if *execution != expected {
		return Err("C5 other primary drift".into());
	}
	Ok(())
}

#[test]
fn callback_and_c5_discovery_compose_only_independently_valid_halves() {
	let source = CASES[78].1;
	assert_eq!(CASES[78].0, "parent/read-format");
	let stock = &c11_control_stock(json!([{"source":0,"start":0,"end":54,"frames":"[]"},{"source":0,"start":103,"end":121,"frames":"[]"}]));
	let caller = json!({"source":0,"start":103,"end":121});
	let chain = json!([caller]);
	let mut primary = stock["outcome"]["sites"][0].clone();
	primary["start"] = json!(37);
	primary["frames"] = json!(serde_json::to_string(&chain).unwrap());
	let mut actual = stock.clone();
	actual["outcome"]["sites"] = json!([primary]);
	let both = c11_retained_frame_shape(source, &actual, stock, &chain).unwrap();
	assert_eq!(
		both["categories"],
		json!(["Retained-site", "Retained-frame (callback)"])
	);
	let mut callback_stock = stock.clone();
	callback_stock["outcome"]["sites"][0]["start"] = json!(37);
	assert_eq!(
		c11_retained_frame_shape(source, &actual, &callback_stock, &chain).unwrap()["categories"],
		json!(["Retained-frame (callback)"])
	);
	let mut site_stock = stock.clone();
	site_stock["outcome"]["sites"] = json!([stock["outcome"]["sites"][0]]);
	let mut site_actual = actual.clone();
	site_actual["outcome"]["sites"][0]["frames"] = json!("[]");
	assert_eq!(
		c11_retained_frame_shape(source, &site_actual, &site_stock, &json!([])).unwrap()
			["categories"],
		json!(["Retained-site"])
	);
	for (field, value) in [
		("start", json!(1)),
		("end", json!(55)),
		("source", json!(1)),
	] {
		let mut bad = stock.clone();
		bad["outcome"]["sites"][0][field] = value;
		assert!(c11_retained_frame_shape(source, &actual, &bad, &chain).is_err());
	}
	for start in [36, 38] {
		let mut bad = actual.clone();
		bad["outcome"]["sites"][0]["start"] = json!(start);
		assert!(c11_retained_frame_shape(source, &bad, stock, &chain).is_err());
	}
	for bad_chain in [json!([]), json!([caller,{"source":0,"start":37,"end":54}])] {
		let mut bad = actual.clone();
		bad["outcome"]["sites"][0]["frames"] = json!(serde_json::to_string(&bad_chain).unwrap());
		assert!(c11_retained_frame_shape(source, &bad, stock, &bad_chain).is_err());
	}
	let mut bad = actual.clone();
	bad["events"] = Json::Null;
	assert!(c11_retained_frame_shape(source, &bad, stock, &chain).is_err());
}

// Called by the existing discovery control: no extra ignored/control test surface.
fn c11_mixed_frame_controls() {
	let source = CASES[89].1;
	assert_eq!(CASES[89].0, "frames/caller-chain");
	let stock = &c11_control_stock(json!([{"source":0,"start":31,"end":54,"frames":"[]"},{"source":0,"start":101,"end":119,"frames":"[CallFrame { ip: 3, top: 0, isolated: None, out: discard }, CallFrame { ip: 12, top: 3, isolated: None, out: discard }]"}]));
	let chain = json!([{"source":0,"start":101,"end":119},{"source":0,"start":134,"end":142},{"source":0,"start":77,"end":85}]);
	let mut actual = stock.clone();
	let mut primary = stock["outcome"]["sites"][0].clone();
	primary["frames"] = json!(serde_json::to_string(&chain).unwrap());
	actual["outcome"]["sites"] = json!([primary]);
	assert_eq!(
		c11_retained_frame_shape(source, &actual, stock, &chain).unwrap()["legacy_counts"],
		json!([2])
	);
	// Pure explicit windows are covered by row040 in the containing control.
	let mut short = chain.clone();
	short.as_array_mut().unwrap().pop();
	let mut extra = chain.clone();
	extra.as_array_mut().unwrap().push(chain[2].clone());
	let mut swapped = chain.clone();
	swapped.as_array_mut().unwrap().swap(0, 1);
	let mut noncall = chain.clone();
	noncall[1] = json!({"source":0,"start":31,"end":54});
	for bad_chain in [short, extra, swapped, noncall] {
		let mut bad = actual.clone();
		bad["outcome"]["sites"][0]["frames"] = json!(serde_json::to_string(&bad_chain).unwrap());
		assert!(c11_retained_frame_shape(source, &bad, stock, &bad_chain).is_err());
	}
	let mut shifted = stock.clone();
	shifted["outcome"]["sites"][1]["start"] = json!(102);
	assert!(c11_retained_frame_shape(source, &actual, &shifted, &chain).is_err());
	let mut short_stock = stock.clone();
	short_stock["outcome"]["sites"][1]["frames"] =
		json!("[CallFrame { ip: 3, top: 0, isolated: None, out: discard }]");
	assert!(c11_retained_frame_shape(source, &actual, &short_stock, &chain).is_err());
}

// Model the depth probe independently from saved observations. The historical
// 1024-frame limit admits depths through 510; default limits admit these cases.
fn c11_expected_depth(depth: usize, historical: bool) -> Json {
    let fail = historical && depth >= 511;
    let iterations = if fail { 511 } else { depth + 1 };
    let events: Vec<_> = (0..iterations).map(|n| json!({"event":"compare","n":n+1,"a":(depth-n).to_string(),"b":(depth-n).to_string()})).collect();
    let mut after = events.clone();
    after.push(json!({"event":"mark","hex":"7265757365"}));
    let before = json!({"root":0,"nodes":[["Tuple","0xfeb65b3f4755aef0",[1,2]],["Vec","0xc11b1e769aea94f2",[3,4]],["Vec","0xc11b1e769aea94f2",[]],["I64","0x1cad9186c9641c4f",depth.to_string()],["I64","0x1cad9186c9641c4f",depth.to_string()]]});
    let outcome = if fail {
        let mut frames = vec![json!({"source":0,"start":54,"end":72}); 510];
        frames.push(json!({"source":0,"start":123,"end":141}));
        json!({"status":"error","class":"FrameCapacity","kind":"FrameCapacity","typed":{"raw_kind":"FrameCapacity"},"display":"execute: FrameCapacity","chain":[],"sites":[{"source":0,"start":54,"end":72,"frames":serde_json::to_string(&frames).unwrap()}]})
    } else {
        json!({"status":"value","graph":{"root":0,"nodes":[["Vec","0xc11b1e769aea94f2",[1,2]],["I64","0x1cad9186c9641c4f",depth.to_string()],["I64","0x1cad9186c9641c4f",depth.to_string()]]}})
    };
    json!({"outcome":outcome,"before":before,"post":before,"events":events,"events_after_reuse":after,"reuse":{"status":"value","graph":{"root":0,"nodes":[["I64","0x1cad9186c9641c4f","7"]]}},"invoked":true})
}

#[test]
fn s6_callback_depth_checks_historical_limits_and_default_progressions() {
    let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));
    for &(label, source, fixture, _, mode, fuel, cancel) in CASES.iter().filter(|row| row.0.starts_with("depth/")) {
        let depth: usize = label.strip_prefix("depth/").unwrap().parse().unwrap();
        let historical = observe_with_limits(source, fixture, mode, fuel, cancel, crate::experimental_core::execute::ExecutorLimits { frames: 1024, slots: 65_536 });
        assert_eq!(common(&project_p1(&historical, &counts[label]).unwrap()), c11_expected_depth(depth, true), "{label}");
        if depth >= 511 {
            let current = observe(source, fixture, mode, fuel, cancel);
            assert_eq!(common(&project_p1(&current, &counts[label]).unwrap()), c11_expected_depth(depth, false), "{label}");
            assert_eq!(current["fresh_audit"]["typed_failure_kind"], Json::Null);
        }
    }
}
