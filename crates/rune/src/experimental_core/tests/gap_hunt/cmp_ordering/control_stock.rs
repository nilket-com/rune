// Run the stock observer for diagnostic controls. Miri cannot spawn a child;
// native controls use the same bounded child as the paired contract.
fn diagnostic_stock(label: &str, source: &str, counts: &Json) -> Json {
	let outcome = {
		#[cfg(miri)]
		{
			assert!(matches!(label, "format/display/Less" | "format/display/Equal" | "format/display/Greater" | "format/display-nested" | "frames/missing" | "frames/invalid-rhs" | "access/leased-parent-mutate"));
			legacy_observe(source, "c9-main")["outcome"].clone()
		}
		#[cfg(not(miri))]
		{
			let index = CASES.iter().position(|case| case.0 == label && case.1 == source).unwrap();
			let out = std::env::temp_dir().join(format!("rune-c9-diagnostic-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
			let index_text = index.to_string();
			let run = super::live_oracle::run_child("experimental_core::tests::gap_hunt::cmp_ordering::child", label, &out, "C9_RESULT", &[("C9_ENGINE", "stock".as_ref()), ("C9_CASE", index_text.as_ref())]);
			assert_eq!(run.observation["source_hex"], hex(source.as_bytes()));
			let outcome = run.observation["observation"]["outcome"].clone();
			std::fs::remove_dir_all(out).unwrap();
			outcome
		}
	};
	let stock = semantic_projection(&project_p1(&outcome, counts).unwrap()).unwrap();
	#[cfg(not(miri))]
	let stock = super::d1b_diagnostics::intentional_json_before("cmp_ordering", label, source, "stock", &stock);
	#[cfg(miri)]
	let stock = d1b_diagnostics::intentional_json_before("cmp_ordering", label, source, "stock", &stock);
	stock
}
