// Single predeclared Retained-frame. Frozen stock bytes are never rewritten.
fn retained_frame_matches(source: &str, stock: &Json, execution: &Json, chain: &Json) -> bool {
	if source != "fn calculate(x){x.powf(\"x\")} pub fn main(){calculate(2.0)}" {
		return false;
	}
	if chain != &json!([{"source":0,"start":43,"end":57}])
		|| source.as_bytes().get(43..57) != Some(b"calculate(2.0)".as_slice())
	{
		return false;
	}
	let (Some(oracle_sites), Some(sites)) =
		(stock["sites"].as_array(), execution["sites"].as_array())
	else {
		return false;
	};
	if oracle_sites.len() != 1 || sites.len() != 1 {
		return false;
	}
	if oracle_sites[0]
		!= json!({"source":0,"start":16,"end":27,"frames":"[CallFrame { ip: 3, top: 0, isolated: None, out: keep(1) }]"})
		|| sites[0]["frames"] != serde_json::to_string(chain).unwrap()
	{
		return false;
	}
	let mut stock = stock.clone();
	let mut execution = execution.clone();
	let (Some(oracle_site), Some(site)) = (
		stock["sites"][0].as_object_mut(),
		execution["sites"][0].as_object_mut(),
	) else {
		return false;
	};
	oracle_site.remove("frames");
	site.remove("frames");
	stock == execution
}

#[test]
fn retained_frame_rule_rejects_drift_and_extra_callers() {
	let source = "fn calculate(x){x.powf(\"x\")} pub fn main(){calculate(2.0)}";
	let chain = json!([{"source":0,"start":43,"end":57}]);
	let stock = json!({"status":"error","class":"Type","display":"Expected std::f64","chain_displays":["Bad argument #1"],"sites":[{"source":0,"start":16,"end":27,"frames":"[CallFrame { ip: 3, top: 0, isolated: None, out: keep(1) }]"}]});
	let mut execution = stock.clone();
	execution["sites"][0]["frames"] = json!(serde_json::to_string(&chain).unwrap());
	assert!(retained_frame_matches(source, &stock, &execution, &chain));
	assert!(!retained_frame_matches("changed", &stock, &execution, &chain));
	for bad in [
		json!([]),
		json!([{"source":0,"start":43,"end":56}]),
		json!([{"source":1,"start":43,"end":57}]),
		json!([{"source":0,"start":43,"end":57},{"source":0,"start":43,"end":57}]),
	] {
		assert!(!retained_frame_matches(source, &stock, &execution, &bad));
	}
	let mut bad = execution.clone();
	bad["display"] = json!("changed");
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = execution.clone();
	bad["sites"][0]["end"] = json!(28);
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = execution.clone();
	bad["sites"][0]["frames"] = json!("[]");
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = stock.clone();
	bad["sites"][0]["frames"] = json!("[]");
	assert!(!retained_frame_matches(source, &bad, &execution, &chain));
	let mut bad = execution.clone();
	bad["chain_displays"] = json!([]);
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
}
