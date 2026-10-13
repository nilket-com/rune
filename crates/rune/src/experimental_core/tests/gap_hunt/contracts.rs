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
			"comparison-stock",
			"gap_hunt::comparison_contract",
			"stock",
			3594,
		),
		(
			"comparison-fresh",
			"gap_hunt::comparison_contract",
			"fresh",
			3594,
		),
		("index-stock", "gap_hunt::index_contract", "stock", 1035),
		("index-fresh", "gap_hunt::index_contract", "fresh", 1035),
		(
			"field-stock",
			"gap_hunt::field_iter::contract",
			"stock",
			228,
		),
		(
			"field-fresh",
			"gap_hunt::field_iter::contract",
			"fresh",
			228,
		),
		(
			"len-stock",
			"gap_hunt::len_clone::stock_contract",
			"stock",
			310,
		),
		(
			"len-fresh",
			"gap_hunt::len_clone::fresh_contract",
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
			"range-stock",
			"gap_hunt::range_index::stock_contract",
			"stock",
			522,
		),
		(
			"range-fresh",
			"gap_hunt::range_index::fresh_contract",
			"fresh",
			522,
		),
		(
			"endpoints-stock",
			"gap_hunt::range_index::endpoint_stock_contract",
			"stock",
			100,
		),
		(
			"endpoints-fresh",
			"gap_hunt::range_index::endpoint_fresh_contract",
			"fresh",
			100,
		),
		(
			"assertions-stock",
			"gap_hunt::assertions::stock_contract",
			"stock",
			178,
		),
		("s0", "gap_hunt::corpus", "both", 639),
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
