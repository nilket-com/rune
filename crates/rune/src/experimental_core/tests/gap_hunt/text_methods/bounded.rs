#[test]
#[cfg(target_os = "linux")]
#[ignore = "C14 strict execution contract against live stock; explicit new C14_OUT, bounded children"]
fn contract() {
	let out = PathBuf::from(std::env::var_os("C14_OUT").expect("new C14_OUT"));
	fs::create_dir(&out).unwrap();
	let cases: Vec<Json> = serde_json::from_str(CASES).unwrap();
	let frozen: Json = serde_json::from_str(DISPOSITIONS).unwrap();
	let start = Instant::now();
	for (i, case) in cases.iter().enumerate() {
		assert!(start.elapsed() < Duration::from_secs(1700));
		let label = case["label"].as_str().unwrap();
		let source = case["source"].as_str().unwrap();
		let dir = out.join(format!("{i:03}"));
		let stock = launch_case("stock", i, case, &dir.join("stock"));
		assert_eq!(
			stock.observation["observation"]["reuse"]["status"], "value",
			"{label}"
		);
		let expected = expected_stock(case, &stock.observation);
		let complete = if i == 184 {
			let mut counterfactual_case = case.clone();
			counterfactual_case["fuel"] = json!(200000);
			let complete = launch_case_inner(
				"stock",
				i,
				&counterfactual_case,
				&dir.join("stock-complete"),
				true,
			);
			assert!(complete.stderr.is_empty());
			Some(expected_stock(&counterfactual_case, &complete.observation))
		} else {
			None
		};
		let fresh = launch_case("fresh", i, case, &dir.join("fresh"));
		crate::experimental_core::tests::gap_hunt::assert_warning_multisets(
			&stock.observation,
			&fresh.observation,
		);
		let mut actual = fresh.observation;
		actual["observation"]["outcome"] = super::d1c_diagnostics::before(
			"text_methods",
			label,
			source,
			"fresh",
			&actual["observation"]["outcome"],
		);
		compare(
			case,
			&frozen[format!("{i:03}")],
			&expected,
			&actual,
			complete.as_ref(),
		);
		assert!(stock.stderr.is_empty(), "C14 stock stderr {i}");
		assert!(fresh.stderr.is_empty(), "C14 stderr {i}");
	}
	fs::write(
		out.join("COMPLETE"),
		"execution190 strict against live stock; P1 pointers and fuel policy only\n",
	)
	.unwrap();
	println!("\nCONTRACT ENGINE fresh\nCONTRACT ROWS {}", cases.len());
}

/// Run case `index` on `engine` in a bounded child and check its envelope.
#[cfg(target_os = "linux")]
fn launch_case(engine: &str, index: usize, case: &Json, folder: &Path) -> super::live_oracle::Run {
	launch_case_inner(engine, index, case, folder, false)
}

#[cfg(target_os = "linux")]
fn launch_case_inner(
	engine: &str,
	index: usize,
	case: &Json,
	folder: &Path,
	counterfactual: bool,
) -> super::live_oracle::Run {
	let index_text = index.to_string();
	let mut env = vec![
		("C14_ENGINE", engine.as_ref()),
		("C14_CASE", index_text.as_ref()),
	];
	if counterfactual {
		env.push(("C14_COUNTERFACTUAL_FUEL", std::ffi::OsStr::new("200000")));
	}
	let run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::text_methods::child",
		case["label"].as_str().unwrap(),
		folder,
		"C14_RESULT",
		&env,
	);
	let observed = &run.observation;
	assert_eq!(observed["schema"], 1);
	assert_eq!(observed["engine"], engine);
	assert_eq!(observed["index"], index);
	for key in ["label", "source_sha256", "fixture", "fuel"] {
		assert_eq!(observed[key], case[key], "{key}");
	}
	run
}
