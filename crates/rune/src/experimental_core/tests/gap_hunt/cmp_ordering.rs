//! C9 live stock oracle and source-pinned admission overlays.
use super::*;
use serde_json::{json, Value as Json};
include!("cmp_ordering/cases.rs");
include!("cmp_ordering/fixture.rs");
const C9_STALE_REFUSAL_ADMISSIONS: &str =
	include_str!("cmp_ordering/stale_refusal_admissions.json");

include!("cmp_ordering/stock_observer.rs");
include!("cmp_ordering/control_stock.rs");

#[test]
#[ignore = "C9 stock oracle child; externally bounded controller only"]
fn child() {
	assert_eq!(CASES.len(), COUNT);
	let index: usize = std::env::var("C9_CASE").unwrap().parse().unwrap();
	let (label, source, mode, policy) = CASES[index];
	let engine = std::env::var("C9_ENGINE").unwrap();
	let observation = match engine.as_str() {
		"stock" => legacy_observe(source, mode),
		"fresh" => observe(source, mode),
		_ => panic!("unknown C9 engine"),
	};
	let envelope = json!({"schema":1,"engine":engine,"index":index,"label":label,"source_hex":hex(source.as_bytes()),"mode":mode,"policy":policy,"fuel":FUEL,"observation":observation});
	fs::write(
		std::env::var_os("C9_RESULT").unwrap(),
		serde_json::to_vec_pretty(&envelope).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

include!("cmp_ordering/p1.rs");
include!("cmp_ordering/pins.rs");
include!("cmp_ordering/p1_controls.rs");
include!("cmp_ordering/observe.rs");
include!("cmp_ordering/contract.rs");

include!("cmp_ordering/retained.rs");

include!("cmp_ordering/site_enforce.rs");
