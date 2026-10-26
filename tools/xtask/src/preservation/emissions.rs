use super::*;

/// Only process-local AnyTypeInfo addresses are projected, exactly as in the
/// historical classifier. All other error fields, logical events and diagnostics
/// remain significant; serialized Units never pass through this projection.
fn stable(text: &str) -> String {
	let marker = "AnyTypeInfo { full_name: 0x";
	let mut rest = text;
	let mut out = String::new();
	while let Some(at) = rest.find(marker) {
		let end = at + marker.len();
		out.push_str(&rest[..end]);
		rest = &rest[end..];
		let n = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
		if n > 0 && rest.as_bytes().get(n) == Some(&b',') {
			out.push_str("<process-local>");
			rest = &rest[n..];
		}
	}
	out.push_str(rest);
	out
}
fn canonical(value: &Value) -> Result<Value> {
	let mut observation = value["observation"].clone();
	ensure!(observation.is_object(), "missing observation");
	for field in ["outcome", "reuse"] {
		if let Some(error) = observation[field].as_object_mut() {
			if error.get("status").and_then(Value::as_str) != Some("error") {
				continue;
			}
			error.remove("raw");
			if let Some(sites) = error.get_mut("sites").and_then(Value::as_array_mut) {
				for site in sites {
					site.as_object_mut()
						.context("invalid error site")?
						.remove("frames");
				}
			}
			for key in ["kind", "class"] {
				let s = error
					.get(key)
					.and_then(Value::as_str)
					.context("missing error kind/class")?;
				error.insert(key.to_owned(), Value::String(stable(s)));
			}
			if let Some(chain) = error.get_mut("chain").and_then(Value::as_array_mut) {
				for entry in chain {
					entry["kind"] = Value::String(stable(string(entry, "kind")?));
				}
			}
		}
	}
	// Python's historical x["logical"] / x["diagnostics"] requires the
	// keys, but preserves legitimate JSON null values (including stock row0).
	ensure!(
		value.get("logical").is_some() && value.get("diagnostics").is_some(),
		"missing logical events or diagnostics"
	);
	Ok(
		json!({"observation":observation,"logical":value["logical"],"diagnostics":value["diagnostics"]}),
	)
}
fn canon_bytes(value: &Value) -> Result<Vec<u8>> {
	// serde_json's default map representation orders keys lexically, matching
	// Python sort_keys=True,separators=(',',':'),ensure_ascii=False.
	Ok(serde_json::to_vec(value)?)
}
fn admitted(rule: &Value) -> bool {
	matches!(
		rule["classification"].as_str(),
		Some(
			"Exact"
				| "Intentional-D1"
				| "Retained-frame-limit"
				| "Retained-fuel"
				| "Retained-fuel-effects"
				| "Known-gap"
		)
	)
}
pub(super) fn rules(
	ctx: &Context,
	module: &Module,
) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>)> {
	let root = ctx.repo.join(&module.root);
	let cases = array(&read(&root.join("cases.json"))?)?.clone();
	let mut rules = array(&read(&root.join("dispositions.json"))?["rows"])?.clone();
	let mut stocks = Vec::new();
	for path in &module.stocks {
		stocks.extend(array(&read(&ctx.repo.join(path))?)?.clone());
	}
	ensure!(
		cases.len() == rules.len() && cases.len() == stocks.len(),
		"missing policy/stock row in {}",
		module.name
	);
	for path in &module.overlays {
		for item in array(&read(&ctx.repo.join(path))?["rows"])? {
			let i = number(item, "index")?;
			let rule = rules.get_mut(i).context("overlay index outside cases")?;
			ensure!(
				*rule == item["legacy"],
				"overlay predecessor mismatch: {path}, row {i}"
			);
			let old = rule.clone();
			*rule = item["admitted"].clone();
			for key in ["index", "label", "source_sha256", "stock_row_sha256"] {
				rule[key] = old[key].clone();
			}
		}
	}
	Ok((cases, rules, stocks))
}
pub(super) fn run(ctx: &Context, m: &Manifest, binary: &Path) -> Result<Value> {
	let mut count = 0;
	let mut byte_comparisons = 0;
	let mut assessments = Vec::new();
	for module in &m.modules {
		let (cases, rules, stocks) = rules(ctx, module)?;
		let baseline = read(&ctx.fixtures.join(&module.baseline_progress))?;
		let baseline = array(&baseline)?;
		ensure!(
			baseline.len() == cases.len(),
			"missing emission baseline row"
		);
		let root = ctx.out.join(&module.name);
		fs::create_dir(&root)?;
		let mut old_invoked = 0;
		let mut now_invoked = 0;
		let mut changes = Vec::new();
		for (i, ((case, rule), stock)) in cases.iter().zip(&rules).zip(&stocks).enumerate() {
			ensure!(
				number(rule, "index")? == i && number(&baseline[i], "index")? == i,
				"emission row index changed"
			);
			let stock_digest = common::hash_bytes(&canon_bytes(stock)?);
			ensure!(
				stock_digest == string(rule, "stock_row_sha256")?,
				"stock policy digest changed: {} {i}",
				module.name
			);
			let dir = root.join(format!("{i:03}"));
			let mut env = vec![
				(
					"D1E_TRACE_OUT",
					dir.join("logical.json").display().to_string(),
				),
				(
					"C18E_OPS_RESULT",
					dir.join("emission.code").display().to_string(),
				),
			];
			let keys = [
				format!("{}_ENGINE", module.prefix),
				format!("{}_REPLAY", module.prefix),
				format!("{}_CASE", module.prefix),
				format!("{}_RESULT", module.prefix),
			];
			env.extend([
				(&*keys[0], "fresh".to_owned()),
				(&*keys[1], "1".to_owned()),
				(&*keys[2], i.to_string()),
				(
					&*keys[3],
					dir.join("observation.json").display().to_string(),
				),
			]);
			child(
				ctx,
				binary,
				&dir,
				&format!("experimental_core::tests::gap_hunt::{}::child", module.name),
				&env,
				20,
				true,
			)?;
			ensure!(
				fs::metadata(dir.join("observation.json"))?.len() <= 2097152,
				"oversize observation"
			);
			let actual = read(&dir.join("observation.json"))?;
			ensure!(
				number(&actual, "index")? == i
					&& actual["label"] == case["label"]
					&& actual["source_sha256"] == case["source_sha256"]
					&& actual["source_sha256"] == rule["source_sha256"]
					&& actual["source"] == case["source"],
				"emission case identity changed"
			);
			let audit = &actual["fresh_audit"];
			for key in ["row_idle", "executor_idle"] {
				ensure!(audit[key] == true, "audit {key} failed");
			}
			ensure!(audit["cleanup_error"] == false, "cleanup failed");
			for key in [
				"row_live_entries",
				"row_payload_bytes",
				"live_entries",
				"payload_bytes",
			] {
				ensure!(audit[key] == 0, "audit {key} nonzero");
			}
			let before = admitted(rule);
			let now = audit["invoked"]
				.as_bool()
				.context("missing invoked audit")?;
			old_invoked += usize::from(before);
			now_invoked += usize::from(now);
			if now {
				let c = canonical(&actual)?;
				let s = canonical(stock)?;
				if before {
					ensure!(
						c == if rule["classification"] == "Exact" {
							s
						} else {
							rule["fresh"].clone()
						},
						"prior admission observation changed: {} {i}",
						module.name
					);
				} else {
					ensure!(
						c == s
							&& hex(&fs::read(dir.join("stdout"))?) == string(stock, "stdout_hex")?
							&& hex(&fs::read(dir.join("stderr"))?) == string(stock, "stderr_hex")?,
						"new admission not exact: {} {i}",
						module.name
					);
					changes.push(
						json!({"index":i,"legacy":rule,"admitted":{"classification":"Exact"}}),
					);
				}
			} else {
				ensure!(!before, "prior admission refused: {} {i}", module.name);
				ensure!(
					actual["observation"]["outcome"]["status"] == "refused"
						&& !actual["observation"]["outcome"]["location"].is_null(),
					"invalid refusal"
				);
				ensure!(
					actual["observation"]["outcome"] == rule["fresh"]
						&& actual["diagnostics"] == rule["diagnostics"],
					"unapproved refusal or diagnostic change"
				);
			}
			if baseline[i]["invoked"] == true {
				let old = ctx
					.fixtures
					.join(&module.baseline_progress)
					.parent()
					.context("baseline parent")?
					.join(format!("{i:03}/emission.code"));
				ensure!(
					common::hash_file(&old)? == string(&baseline[i], "emission_sha256")?,
					"baseline emission hash changed"
				);
				exact(&old, &dir.join("emission.code"))?;
				byte_comparisons += 1;
			}
			count += 1;
		}
		assessments.push(json!({"module":module.name,"rows":cases.len(),"old_invoked":old_invoked,"now_invoked":now_invoked,"new_Exact":changes.len(),"proposed_overlay":changes,"issues":[]}));
	}
	ensure!(
		count == m.counts["emissions"] && byte_comparisons == m.counts["admitted_emissions"],
		"emission record/admission count changed"
	);
	Ok(
		json!({"records":count,"old_admissions_identical":byte_comparisons,"assessments":assessments}),
	)
}
fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn projects_only_named_process_addresses() {
		assert_eq!(
			stable("AnyTypeInfo { full_name: 0xAf32, other: 0x42"),
			"AnyTypeInfo { full_name: 0x<process-local>, other: 0x42"
		);
		assert_eq!(stable("0xAf32"), "0xAf32");
	}
	#[test]
	fn projection_preserves_error_policy_and_float_bits() {
		let input = json!({"observation":{"outcome":{"status":"error","raw":"unstable","kind":"AnyTypeInfo { full_name: 0x123, x}","class":"Error","sites":[{"frames":[1],"location":7}],"chain":[{"kind":"retained"}]},"float_bits":"7ff8000000000001"},"logical":["tap"],"diagnostics":["retained"]});
		let projected = canonical(&input).unwrap();
		assert!(projected["observation"]["outcome"].get("raw").is_none());
		assert!(projected["observation"]["outcome"]["sites"][0]
			.get("frames")
			.is_none());
		assert_eq!(
			projected["observation"]["outcome"]["sites"][0]["location"],
			7
		);
		assert_eq!(projected["observation"]["float_bits"], "7ff8000000000001");
		assert_eq!(projected["logical"], input["logical"]);
		assert_eq!(projected["diagnostics"], input["diagnostics"]);
	}

	#[test]
	fn authentic_stock_null_logical_is_preserved_but_missing_keys_fail() {
		let rows:Value=serde_json::from_str(include_str!("../../../../crates/rune/src/experimental_core/tests/gap_hunt/static_methods/stock.json")).unwrap();
		let mut stock = rows[0].clone();
		assert_eq!(stock["label"], "receiver/literal-named");
		assert!(stock["logical"].is_null());
		let projected = canonical(&stock).unwrap();
		assert_eq!(projected["logical"], Value::Null);
		assert_eq!(projected["diagnostics"], stock["diagnostics"]);
		stock.as_object_mut().unwrap().remove("logical");
		assert!(canonical(&stock).is_err());
		let mut stock = rows[0].clone();
		stock.as_object_mut().unwrap().remove("diagnostics");
		assert!(canonical(&stock).is_err());
	}
	#[test]
	fn missing_stock_or_policy_row_fails_identity() {
		assert!(number(&json!({"label":"x"}), "index").is_err());
	}
}
