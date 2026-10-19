#[test]
#[ignore = "C11 strict fresh 131; external depth binding and raw receipts required"]
fn fresh_contract() {
	let out =
		PathBuf::from(std::env::var_os("C11_REPLAY_OUT").expect("explicit raw replay directory"));
	fs::create_dir(&out).unwrap();
	let counts = read_json(include_str!("../../callback_sort_p1_counts.json"));

	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let start = Instant::now();
	let binding: Json = serde_json::from_slice(
		&fs::read(std::env::var_os("C11_DEPTH_BINDING").expect("reviewed depth binding")).unwrap(),
	)
	.unwrap();
	c11_check_depth_binding(&binding).unwrap();
	let mut completed = 0;

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
		assert_eq!(stdout, decode(cells[4]));
		assert_eq!(stderr, decode(cells[5]));
		assert!(fs::metadata(&result).unwrap().len() <= 64 * 1024 * 1024);
		// The child archive is already preserved; only a copy is projected.
		let mut raw: Json = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
		raw["observation"]["outcome"] = super::d1c_diagnostics::before("callback_sort", label, source, "fresh", &raw["observation"]["outcome"]);
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
		compare_fresh_row(
			label,
			source,
			fixture,
			cells[6],
			observation,
			&counts[label],
			&dispositions[label],
			&binding,
		)
		.unwrap();
		completed += 1;
	}
	assert_eq!(completed, 131);
	contract_completed("fresh", completed);
}
fn compare_fresh_row(
	label: &str,
	source: &str,
	fixture: &str,
	policy: &str,
	observation: &Json,
	counts: &Json,
	disposition: &Json,
	binding: &Json,
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
	if fixture.starts_with("depth/")
		&& (audit["depth_metadata"] != binding["metadata"]
			|| binding["source_sha256"] != digest(source.as_bytes()))
	{
		return Err("depth emitted metadata binding drift".into());
	}
	if c11_check_stale_refusal_admission(label, source, observation, counts, disposition)? {
		return Ok(());
	}
	if matches!(policy, "Refused-surface" | "Refused-runtime") {
		let spans = read_json(include_str!("../../callback_sort_refusal_spans.json"));
		let expected = &spans[label];
		let refusals = read_json(include_str!("../../callback_sort_refusals.json"));
		let baseline = &refusals[label]["stock_common"];
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
		let expected = binding["rows"].get(label).ok_or("missing derived row")?;
		if audit["typed_failure_kind"]
			!= if expected["outcome"]["status"] == "error" {
				json!("FrameCapacity")
			} else {
				Json::Null
			} {
			return Err("derived typed failure drift".into());
		}
		if &actual != expected {
			return Err(format!("derived depth drift {label}"));
		}
		return Ok(());
	}
	if matches!(policy, "Retained-fuel" | "Retained-cancel") {
		let resources = read_json(include_str!("../../callback_sort_resources.json"));
		let (stock, anchor) = resource_completion(label, source, fixture, &resources)?;
		let kind = if policy == "Retained-fuel" {
			"FuelExhausted"
		} else {
			"Cancelled"
		};
		if !resource_valid(&stock, &actual, &audit["resource"], kind) {
			return Err(format!("resource constraint drift {label}"));
		}
		c11_enforce_resource(label, source, fixture, &actual, &audit["resource"], &anchor)?;
		return Ok(());
	}
	if !matches!(policy, "Exact" | "Intentional-divergence") {
		return Err("unknown fresh policy".into());
	}
	let manifest = c11_retained_manifest()?;
	if manifest["diagnostics"]
		.as_array()
		.ok_or("diagnostic rows")?
		.iter()
		.any(|r| r["label"] == label)
	{
		return c11_enforce_diagnostic(
			label,
			source,
			&actual,
			&disposition["fresh_expectation"],
			&audit["raw_failure"]["call_chain"],
		);
	}
	if actual != disposition["fresh_expectation"] {
		return Err(format!("strict portable drift {label}"));
	}
	Ok(())
}

fn c11_check_stale_refusal_admission(
	label: &str,
	source: &str,
	observation: &Json,
	counts: &Json,
	disposition: &Json,
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
	let stock = legacy["stock_common"].clone();
	if row["stock_common_sha256"] != digest(&canonical(&stock)) {
		return Err(format!("stale refusal stock pin drift {label}"));
	}
	let actual = common(&project_p1(observation, counts)?);
	if label == "refused/move-capture" {
		// C11c P0 stopgap reverses only this capturing-move admission.
		// Pin its complete old admission before requiring compile-time refusal.
		if row["fresh_expectation"]["outcome"]["status"] != "value"
			|| row["fresh_expectation"]["invoked"] != true
			|| digest(&canonical(&row["fresh_expectation"]))
				!= "c687396d8a21727c22946d6d9d569d57fd4dc7fbaa184b60ae5b7a34710881f3"
		{
			return Err("C11c historical move admission drift".into());
		}
		let mut expected = row["fresh_expectation"].clone();
		expected["outcome"] = json!({"status":"refused","kind":"Unsupported","operation":"CallClosure","refusal":"CallClosure","location":{"source":0,"start":72,"end":104}});
		expected["post"] = expected["before"].clone();
		expected["invoked"] = json!(false);
		if actual != expected {
			return Err(format!("C11c move refusal drift {label}"));
		}
		return Ok(true);
	}
	if actual != row["fresh_expectation"] {
		return Err(format!("stale refusal fresh pin drift {label}"));
	}
	match row["disposition"]
		.as_str()
		.ok_or("stale admission disposition")?
	{
		"Exact" => {
			if actual != stock {
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
			let retained = c11_retained_frame_shape(source, &actual, &stock, &call_chain)?;
			if retained["categories"] != frame["categories"] {
				return Err("retained frame category drift".into());
			}
		}
		"Retained-ownership" => {
			let path = row["only_allowed_difference"]
				.as_str()
				.ok_or("ownership difference path")?;
			let differences = c11_json_differences(&stock, &actual, "", Some(path));
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
	let fresh_sites = actual["outcome"]["sites"].as_array().ok_or("fresh sites")?;
	let stock_sites = stock["outcome"]["sites"].as_array().ok_or("stock sites")?;
	if fresh_sites.len() != 1 || stock_sites.is_empty() || stock_sites[0]["frames"] != "[]" {
		return Err("not callback windows".into());
	}
	let fresh = &fresh_sites[0];
	let mut primary = fresh.clone();
	primary["frames"] = json!("[]");
	let mut categories = Vec::new();
	if primary != stock_sites[0] {
		c11_c5_primary(source, &primary, &stock_sites[0])?;
		categories.push("Retained-site");
	}
	if fresh["frames"] != "[]" || stock_sites.len() > 1 {
		categories.push("Retained-frame (callback)");
	}
	let frames: Json = serde_json::from_str(fresh["frames"].as_str().ok_or("fresh frames")?)
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
	for window in &stock_sites[1..] {
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
		json!({"stock_sites":stock_sites,"fresh_sites":fresh_sites,"call_chain":call_chain,"caller_hex":caller_hex,"mapped_callers":mapped,"legacy_counts":legacy_counts,"categories":categories}),
	)
}

#[test]
fn callback_frame_discovery_rejects_semantic_and_unpaired_drift() {
	c11_mixed_frame_controls();
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let source = CASES[40].1;
	let stock = &dispositions[CASES[40].0]["fresh_expectation"];
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
	let single_stock = &dispositions[CASES[43].0]["fresh_expectation"];
	assert!(
		c11_retained_frame_shape(single_source, single_stock, single_stock, &json!([])).is_ok()
	);
	let mut bad = single_stock.clone();
	let chain = json!([{"source":0,"start":55,"end":73}]);
	bad["outcome"]["sites"][0]["frames"] = json!(serde_json::to_string(&chain).unwrap());
	assert!(c11_retained_frame_shape(single_source, &bad, single_stock, &chain).is_err());
}

// Existing C5 condition on the primary, independent of callback caller handling.
fn c11_c5_primary(source: &str, fresh: &Json, stock: &Json) -> Result<(), String> {
	use crate::ast::Spanned;
	let start = usize::try_from(fresh["start"].as_u64().ok_or("C5 start")?)
		.map_err(|_| "start overflow")?;
	let end =
		usize::try_from(fresh["end"].as_u64().ok_or("C5 end")?).map_err(|_| "end overflow")?;
	let stock_end = stock["end"].as_u64().ok_or("C5 stock end")?;
	if stock["start"] != 0
		|| start == 0
		|| start >= end
		|| stock_end <= start as u64
		|| stock_end > end as u64
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
	expected["start"] = fresh["start"].clone();
	expected["end"] = fresh["end"].clone();
	if *fresh != expected {
		return Err("C5 other primary drift".into());
	}
	Ok(())
}

#[test]
fn callback_and_c5_discovery_compose_only_independently_valid_halves() {
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let source = CASES[78].1;
	assert_eq!(CASES[78].0, "parent/read-format");
	let stock = &dispositions[CASES[78].0]["fresh_expectation"];
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
		c11_retained_frame_shape(source, &site_actual, &site_stock, &json!([])).unwrap()["categories"],
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
	let dispositions = read_json(include_str!("../../callback_sort_dispositions.json"));
	let source = CASES[89].1;
	assert_eq!(CASES[89].0, "frames/caller-chain");
	let stock = &dispositions[CASES[89].0]["fresh_expectation"];
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
