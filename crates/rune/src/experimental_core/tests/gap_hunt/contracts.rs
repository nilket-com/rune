//! One operator entry for every frozen contract, including both engine modes.
//! Requires a new GAP_OUT and the intended GAP_EXTERNAL corpus root.
//! Run this ignored test by its exact name with --nocapture. Each subprocess
//! retains stdout/stderr and must echo its actual engine and frozen row count.

use std::{env, fs, path::PathBuf, process::Command};

#[test]
#[ignore = "full frozen contracts and S0: explicit operator run, requires GAP_OUT and GAP_EXTERNAL"]
fn all_frozen_contracts() {
	let out = PathBuf::from(env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	let external = env::var_os("GAP_EXTERNAL").expect("GAP_EXTERNAL required");
	assert!(PathBuf::from(&external).is_dir(), "GAP_EXTERNAL directory");
	fs::create_dir(&out).expect("new GAP_OUT required");
	let executable = env::current_exe().unwrap();
	let mut completed = std::string::String::new();
	let checks = [
		(
			"differential",
			"differential::differential_full_corpus",
			"both",
			675,
		),
		(
			"comparison-execution",
			"gap_hunt::comparison_contract",
			"fresh",
			3594,
		),
		("index-execution", "gap_hunt::index_contract", "fresh", 1035),
		(
			"field-execution",
			"gap_hunt::field_iter::contract",
			"fresh",
			228,
		),
		(
			"len-execution",
			"gap_hunt::len_clone::contract",
			"fresh",
			310,
		),
		(
			"global-both",
			"gap_hunt::len_clone::global_contract",
			"both",
			39,
		),
		(
			"range-execution",
			"gap_hunt::range_index::contract",
			"fresh",
			522,
		),
		(
			"endpoints-execution",
			"gap_hunt::range_index::endpoint_contract",
			"fresh",
			100,
		),
		(
			"assertions-execution",
			"gap_hunt::assertions::contract",
			"fresh",
			178,
		),
		(
			"result-patterns-execution",
			"gap_hunt::patterns::result_supplement::contract",
			"fresh",
			264,
		),
		(
			"patterns-execution",
			"gap_hunt::patterns::contract",
			"fresh",
			348,
		),
		(
			"host-execution",
			"gap_hunt::host_bridge::contract::contract",
			"fresh",
			144,
		),
		(
			"format-specs-execution",
			"gap_hunt::format_specs::contract",
			"fresh",
			266,
		),
		(
			"scalar-methods-execution",
			"gap_hunt::scalar_methods::contract",
			"fresh",
			199,
		),
		(
			"scalar-supplement-execution",
			"gap_hunt::scalar_methods_supplement::contract",
			"fresh",
			7,
		),
		(
			"deferred-index-execution",
			"gap_hunt::deferred_index::contract",
			"fresh",
			318,
		),
		(
			"tuple-let-execution",
			"gap_hunt::tuple_let::contract",
			"fresh",
			98,
		),
		("s0", "gap_hunt::corpus", "both", 639),
		(
			"output-execution",
			"gap_hunt::output_macros::contract",
			"fresh",
			367,
		),
	];
	for (name, test, engine, rows) in checks {
		let full_name = std::format!("experimental_core::tests::{test}");
		let result = Command::new(&executable)
			.args(["--include-ignored", "--exact", &full_name, "--nocapture"])
			.env("GAP_OUT", out.join(name))
			.env("GAP_EXTERNAL", &external)
			.env("GAP_COMPARE_ENGINE", engine)
			.output()
			.unwrap();
		fs::write(out.join(std::format!("{name}.stdout")), &result.stdout).unwrap();
		fs::write(out.join(std::format!("{name}.stderr")), &result.stderr).unwrap();
		// A renamed test must not silently become a successful zero-test run.
		let stdout = std::string::String::from_utf8_lossy(&result.stdout);
		assert!(
			stdout.contains("1 passed; 0 failed"),
			"{name}: no passing test"
		);
		assert!(result.status.success(), "{name}: {:?}", result.status);
		assert_eq!(
			stdout
				.lines()
				.filter(|line| *line == std::format!("CONTRACT ENGINE {engine}"))
				.count(),
			1,
			"{name}: engine echo"
		);
		assert_eq!(
			stdout
				.lines()
				.filter(|line| *line == std::format!("CONTRACT ROWS {rows}"))
				.count(),
			1,
			"{name}: row count echo"
		);
		completed.push_str(name);
		completed.push('\n');
		fs::write(out.join("COMPLETED"), &completed).unwrap();
		std::println!("PASS {name}");
	}
}
