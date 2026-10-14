// Single predeclared Retained-frame. Frozen stock bytes are never rewritten.
fn retained_frame_matches(source: &str, stock: &Json, fresh: &Json, chain: &Json) -> bool {
	if source != "fn calculate(x){x.powf(\"x\")} pub fn main(){calculate(2.0)}" {
		return false;
	}
	if chain != &json!([{"source":0,"start":43,"end":57}])
		|| source.as_bytes().get(43..57) != Some(b"calculate(2.0)".as_slice())
	{
		return false;
	}
	let (Some(stock_sites), Some(fresh_sites)) =
		(stock["sites"].as_array(), fresh["sites"].as_array())
	else {
		return false;
	};
	if stock_sites.len() != 1 || fresh_sites.len() != 1 {
		return false;
	}
	if stock_sites[0]
		!= json!({"source":0,"start":16,"end":27,"frames":"[CallFrame { ip: 3, top: 0, isolated: None, out: keep(1) }]"})
		|| fresh_sites[0]["frames"] != serde_json::to_string(chain).unwrap()
	{
		return false;
	}
	let mut stock = stock.clone();
	let mut fresh = fresh.clone();
	let (Some(stock_site), Some(fresh_site)) = (
		stock["sites"][0].as_object_mut(),
		fresh["sites"][0].as_object_mut(),
	) else {
		return false;
	};
	stock_site.remove("frames");
	fresh_site.remove("frames");
	stock == fresh
}

#[test]
fn retained_frame_rule_rejects_drift_and_extra_callers() {
	let source = "fn calculate(x){x.powf(\"x\")} pub fn main(){calculate(2.0)}";
	let chain = json!([{"source":0,"start":43,"end":57}]);
	let stock = json!({"status":"error","class":"Type","display":"Expected std::f64","chain_displays":["Bad argument #1"],"sites":[{"source":0,"start":16,"end":27,"frames":"[CallFrame { ip: 3, top: 0, isolated: None, out: keep(1) }]"}]});
	let mut fresh = stock.clone();
	fresh["sites"][0]["frames"] = json!(serde_json::to_string(&chain).unwrap());
	assert!(retained_frame_matches(source, &stock, &fresh, &chain));
	assert!(!retained_frame_matches("changed", &stock, &fresh, &chain));
	for bad in [
		json!([]),
		json!([{"source":0,"start":43,"end":56}]),
		json!([{"source":1,"start":43,"end":57}]),
		json!([{"source":0,"start":43,"end":57},{"source":0,"start":43,"end":57}]),
	] {
		assert!(!retained_frame_matches(source, &stock, &fresh, &bad));
	}
	let mut bad = fresh.clone();
	bad["display"] = json!("changed");
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = fresh.clone();
	bad["sites"][0]["end"] = json!(28);
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = fresh.clone();
	bad["sites"][0]["frames"] = json!("[]");
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
	let mut bad = stock.clone();
	bad["sites"][0]["frames"] = json!("[]");
	assert!(!retained_frame_matches(source, &bad, &fresh, &chain));
	let mut bad = fresh.clone();
	bad["chain_displays"] = json!([]);
	assert!(!retained_frame_matches(source, &stock, &bad, &chain));
}
