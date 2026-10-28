// Independent literal pins. This module has no discovery or wildcard path.
const C12_RETAINED_PINS: &str = r########"{"schema":1,"diagnostics":[{"index":5,"label":"not/wrong-string","source_sha256":"966cd82d13e3a242d2aae81f6e9e0432f1e2c26a28e90ecca33b9db21c77d5e6","leaf":"outcome.sites","fresh":[{"end":10,"frames":"[{\"end\":32,\"source\":0,\"start\":26}]","source":0,"start":8}],"category":"Retained-frame"},{"index":10,"label":"binary/and/wrong-rhs","source_sha256":"361c1244467f904d46298a62b92f0cb615a3f38c217f071f614f739e616890cb","leaf":"outcome.sites","fresh":[{"end":13,"frames":"[{\"end\":38,\"source\":0,\"start\":29}]","source":0,"start":10}],"category":"Retained-frame"},{"index":15,"label":"binary/or/wrong-rhs","source_sha256":"cb087df0df5242e8718ab59d8f46b5078ffac39b82e57f79f86bd27d8e702e99","leaf":"outcome.sites","fresh":[{"end":13,"frames":"[{\"end\":38,\"source\":0,\"start\":29}]","source":0,"start":10}],"category":"Retained-frame"},{"index":20,"label":"binary/xor/wrong-rhs","source_sha256":"06155903fb98d03c36aaea5acd5db85b0d8161dc4cbaf446233c3e2048efabc5","leaf":"outcome.sites","fresh":[{"end":13,"frames":"[{\"end\":38,\"source\":0,\"start\":29}]","source":0,"start":10}],"category":"Retained-frame"},{"index":25,"label":"binary/shl/wrong-rhs","source_sha256":"8ebf0bf8131b84b9be173f522bece5ff65edc68bb3c387a8453e9f747e10172f","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":39,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":30,"label":"binary/shr/wrong-rhs","source_sha256":"876d8d4f2274d13ce899c4a541ec61373a6ea46975502c5b5f30d28e735d65a7","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":39,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":33,"label":"shift/shl/-1","source_sha256":"73ba557c46e802103e44cd8c869be36975f768dced40160cbe1999ef416814d4","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":36,"label":"shift/shl/64","source_sha256":"33c03861407cc0efdbf6fd3868b8289966156e27a8eb2a0c870d060e47ca2786","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":37,"label":"shift/shl/65","source_sha256":"8f1aa1f54dea45d944792d3413229275f9a61156b398fe1d8a3ac82a8de7ed08","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":38,"label":"shift/shr/-1","source_sha256":"481dab6cd2bbb93e4f0298acf3370a591520e2062f091e65ecb6b454cfa7889b","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":41,"label":"shift/shr/64","source_sha256":"cab4aa8f3cc54b05c5a853544b65dbad372965c7b45199e58b8e9d601d18d862","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":42,"label":"shift/shr/65","source_sha256":"370658c7babb80a3ba44a34d26e8e8c8cdce79fc01fbdc4f022fa1103efdc4be","leaf":"outcome.sites","fresh":[{"end":14,"frames":"[{\"end\":37,\"source\":0,\"start\":30}]","source":0,"start":10}],"category":"Retained-frame"},{"index":97,"label":"any/predicate-error","source_sha256":"ec81c154bfac88bd441f8a6b428691df843e7396a33213c5867e5fe9d58325c9","leaf":"outcome.sites","fresh":[{"end":29,"frames":"[{\"end\":69,\"source\":0,\"start\":54}]","source":0,"start":22}],"category":"Retained-frame/chain"},{"index":102,"label":"any/cycle-parent-write","source_sha256":"c2834475f049a2eba206d7b385bec251ab6fd05fc8132b42717e2594db8dd6e3","leaf":"outcome.sites","fresh":[{"end":17,"frames":"[{\"end\":61,\"source\":0,\"start\":46}]","source":0,"start":8}],"category":"Retained-frame/chain"},{"index":102,"label":"any/cycle-parent-write","source_sha256":"c2834475f049a2eba206d7b385bec251ab6fd05fc8132b42717e2594db8dd6e3","leaf":"outcome.kind","fresh":"SharedArgument { arg: 0, readers: 1 }","category":"Intentional-more-context","ruling_message":"01a14f55-a78a-7294-b0f4-e7662161c89a","ruling_seq":19886},{"index":102,"label":"any/cycle-parent-write","source_sha256":"c2834475f049a2eba206d7b385bec251ab6fd05fc8132b42717e2594db8dd6e3","leaf":"outcome.chain","fresh":[],"category":"Retained-chain"}],"resources":[{"index":108,"label":"any/fuel-cut","source_sha256":"4e7b128e4a28408c64b01406aa87bac3f3dc2accf3bc888b1fb109e255675a30","fuel":100,"outcome":{"chain":[],"class":"FuelExhausted","display":"execute: FuelExhausted","kind":"FuelExhausted","sites":[{"end":14,"frames":"[{\"end\":52,\"source\":0,\"start\":37}]","source":0,"start":8}],"status":"error"},"category":"Retained-fuel","ruling_peer":16979}],"base_sha256":"def79350cf9e15877a7879aa63a4c2a660d830f00c49bd25e1e49f6035d09801","candidate_sha256":"7958080e783bd1bd7edab01ff2cce4a9246faa9046b33c64148b5382107ca045","twin_sha256":"53eda7ccb1897384cc504e2c122e6631c2b6d9e60ffa0ab25994f22afb82a0f4"}"########;
fn c12_retained_manifest() -> Result<Json, String> {
	let manifest: Json =
		serde_json::from_str(include_str!("retained.json")).map_err(|_| "retained JSON")?;
	let literal: Json = serde_json::from_str(C12_RETAINED_PINS).map_err(|_| "literal pins")?;
	if manifest != literal {
		return Err("retained manifest drift".into());
	}
	Ok(manifest)
}
fn c12_reconcile_leaf(leaf: &str, stock: &Json, actual: &Json, fresh: &Json) -> Result<Json, String> {
    if actual != fresh { return Err("retained fresh diagnostic drift".into()); }
    match leaf {
        "sites" => {
            let stock_sites = stock.as_array().ok_or("stock sites")?;
            let fresh_sites = fresh.as_array().ok_or("fresh sites")?;
            if stock_sites.is_empty() || fresh_sites.len() != 1 { return Err("diagnostic site count".into()); }
            let mut primary = fresh_sites[0].clone();
            let frames: Json = serde_json::from_str(primary["frames"].as_str().ok_or("fresh frames")?).map_err(|_| "fresh frame JSON")?;
            let callers = frames.as_array().ok_or("fresh caller array")?;
            primary["frames"] = stock_sites[0]["frames"].clone();
            if primary != stock_sites[0] { return Err("diagnostic primary drift".into()); }
            let legacy = stock_sites[0]["frames"].as_str().ok_or("stock frames")?;
            if stock_sites.len() == 1 {
                let count = if legacy == "[]" { 0 } else if legacy.starts_with("[CallFrame {") && legacy.ends_with(" }]") { legacy.matches("CallFrame {").count() } else { return Err("stock frame shape".into()); };
                if count != callers.len() { return Err("stock frame count".into()); }
            } else {
                if legacy != "[]" || stock_sites.len() != callers.len() + 1 { return Err("stock caller count".into()); }
                for (site, caller) in stock_sites[1..].iter().zip(callers) {
                    let mut projected = caller.clone(); projected["frames"] = json!("[]");
                    if *site != projected { return Err("stock caller site drift".into()); }
                }
            }
        }
        "kind" => if stock != "AccessError { error: AccessError { kind: NotAccessibleMut(Snapshot(--000001)) } }" { return Err("stock access kind drift".into()); },
        "chain" => if stock != &json!([{"display":"Bad argument #0","kind":"BadArgument { arg: 0 }"}]) { return Err("stock access chain drift".into()); },
        _ => return Err("unapproved retained leaf".into()),
    }
    Ok(stock.clone())
}

fn c12_enforce_diagnostics(i: usize, frozen: &Json, actual: &Json) -> Result<Json, String> {
	let manifest = c12_retained_manifest()?;
	let mut effective = actual.clone();
	for row in manifest["diagnostics"]
		.as_array()
		.ok_or("diagnostic rows")?
	{
		if row["index"] != i {
			continue;
		}
		if row["label"] != frozen["label"] || row["source_sha256"] != frozen["source_sha256"] {
			return Err("retained source identity drift".into());
		}
		let leaf = row["leaf"]
			.as_str()
			.ok_or("leaf")?
			.strip_prefix("outcome.")
			.ok_or("outcome leaf")?;
		if !matches!(leaf, "sites" | "kind" | "chain") || leaf != "sites" && i != 102 {
			return Err("unapproved retained leaf".into());
		}
		let stock = &frozen["stock"]["observation"]["outcome"][leaf];
		let reconciled = c12_reconcile_leaf(leaf, stock, &actual["outcome"][leaf], &row["fresh"])?;
		if i == 102
			&& leaf == "kind"
			&& (row["category"] != "Intentional-more-context"
				|| row["ruling_message"] != "01a14f55-a78a-7294-b0f4-e7662161c89a"
				|| row["ruling_seq"] != 19886)
		{
			return Err("D1c receiver-context ruling drift".into());
		}
		effective["outcome"][leaf] = reconciled;
	}
	Ok(effective)
}
fn c12_enforce_resource(i: usize, frozen: &Json, actual: &Json) -> Result<(), String> {
	let manifest = c12_retained_manifest()?;
	let rows = manifest["resources"].as_array().ok_or("resource rows")?;
	let row = rows
		.iter()
		.find(|r| r["index"] == i)
		.ok_or("unlisted resource row")?;
	if i != 108
		|| row["label"] != frozen["label"]
		|| row["source_sha256"] != frozen["source_sha256"]
		|| row["fuel"] != frozen["fuel"]
		|| row["outcome"] != c12_without_raw(actual["outcome"].clone())
	{
		return Err("frozen resource outcome drift".into());
	}
	Ok(())
}
#[test]
fn retained_pins_and_negative_controls() {
	let m = c12_retained_manifest().unwrap();
	let rows = m["diagnostics"].as_array().unwrap();
	assert_eq!(rows.len(), 16);
	assert_eq!(
		rows.iter()
			.map(|r| r["index"].as_u64().unwrap())
			.collect::<std::collections::BTreeSet<_>>()
			.len(),
		14
	);
	assert_eq!(m["resources"].as_array().unwrap().len(), 1);
	let (_, frozen, _) = c12_data();
	let stock = json!([{"source":0,"start":1,"end":2,"frames":"[]"},{"source":0,"start":3,"end":4,"frames":"[]"}]);
    let fresh = json!([{"source":0,"start":1,"end":2,"frames":"[{\"end\":4,\"source\":0,\"start\":3}]"}]);
    assert_eq!(c12_reconcile_leaf("sites", &stock, &fresh, &fresh).unwrap(), stock);
    let mut bad = fresh.clone(); bad[0]["start"] = json!(9);
    assert!(c12_reconcile_leaf("sites", &stock, &bad, &fresh).is_err());
    let mut bad = stock.clone(); bad[0]["start"] = json!(9);
    assert!(c12_reconcile_leaf("sites", &bad, &fresh, &fresh).is_err());
	let resource = json!({"outcome":m["resources"][0]["outcome"]});
	c12_enforce_resource(108, &frozen[108], &resource).unwrap();
	let mut bad = resource.clone();
	bad["outcome"]["sites"][0]["start"] = json!(9);
	assert!(c12_enforce_resource(108, &frozen[108], &bad).is_err());
	let mut bad = resource.clone();
	bad["outcome"]["display"] = json!("changed");
	assert!(c12_enforce_resource(108, &frozen[108], &bad).is_err());
}
