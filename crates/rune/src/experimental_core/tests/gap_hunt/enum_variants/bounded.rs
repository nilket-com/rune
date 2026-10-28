#[test]
#[cfg(target_os = "linux")]
#[ignore = "C15b strict execution contract against live stock; explicit new C15B_OUT, bounded children"]
fn contract() {
	let out = PathBuf::from(std::env::var_os("C15B_OUT").expect("new C15B_OUT"));
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
		let raw = &stock.observation["observation"];
		if raw["outcome"]["status"] == "compile-error" {
			assert_eq!(raw["reuse"], Json::Null);
			assert_eq!(frozen[format!("{i:03}")]["disposition"], "Refused-frontend");
		} else {
			assert_eq!(raw["reuse"]["status"], "value");
		}
		let complete = if i == 148 {
			// The stock fuel stop keeps the first event of the complete tape.
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
			let complete = expected_stock(&counterfactual_case, &complete.observation);
			let tape = complete["observation"]["events"].as_array().unwrap();
			assert_eq!(raw["events"], json!(&tape[..1]));
			Some(complete)
		} else {
			None
		};
		let expected = expected_stock(case, &stock.observation);
		let fresh = launch_case("fresh", i, case, &dir.join("fresh"));
		let mut actual = fresh.observation;
		actual["observation"]["outcome"] = super::d1c_diagnostics::before(
			"enum_variants",
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
		assert!(stock.stderr.is_empty(), "C15b stock stderr {i}");
		assert!(fresh.stderr.is_empty(), "C15b stderr {i}");
	}
	fs::write(
		out.join("COMPLETE"),
		"execution166 strict against live stock; P1, pre-effect refusal and fuel policy only\n",
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
		("C15B_ENGINE", engine.as_ref()),
		("C15B_CASE", index_text.as_ref()),
	];
	if counterfactual {
		env.push(("C15B_COUNTERFACTUAL_FUEL", std::ffi::OsStr::new("200000")));
	}
	let run = super::live_oracle::run_child(
		"experimental_core::tests::gap_hunt::enum_variants::child",
		case["label"].as_str().unwrap(),
		folder,
		"C15B_RESULT",
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
