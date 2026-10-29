//! Check explicit layout invariants against fresh DWARF observations.
use crate::common::{write_json, Context};
use anyhow::{ensure, Context as _, Result};
use serde_json::{json, Value};
use std::{fs, process::Command};

pub fn run(ctx: &Context) -> Result<Value> {
	let binary = ctx.test_binary()?;
	let script = ctx.repo.join("tools/xtask/src/layout.gdb");
	let mut cmd = Command::new("/usr/bin/gdb");
	cmd.args(["-batch", "-nx", "-x"]).arg(&script).arg(&binary);
	let r = ctx.checked("layout-gdb", &mut cmd, 900)?;
	let text = fs::read_to_string(r.stdout)?;
	let lines: Vec<_> = text
		.lines()
		.filter_map(|l| l.strip_prefix("LAYOUT_JSON "))
		.collect();
	ensure!(
		lines.len() == 1,
		"expected exactly one GDB layout observation"
	);
	let actual: Value = serde_json::from_str(lines[0])?;
	check_layout_invariants(&actual)?;
	for kind in ["layout_probe", "layout_variants"] {
		write_json(&ctx.out.join(format!("{kind}.json")), &actual[kind])?;
	}
	Ok(
		json!({"status":"PASS","rule":"explicit size, alignment, field offset and discriminant invariants"}),
	)
}

fn check_layout_invariants(actual: &Value) -> Result<()> {
	let probe = &actual["layout_probe"];
	let variants = &actual["layout_variants"];
	let sizes = [
        (probe, "rune::experimental_core::detail::Detail", 32, 8),
        (probe, "rune::experimental_core::error::Error", 88, 8),
        (probe, "rune::experimental_core::execute::Failure", 272, 8),
        (probe, "core::option::Option<rune::experimental_core::error::Error>", 88, 8),
        (probe, "core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::error::Error>", 88, 8),
        (probe, "core::result::Result<(), rune::experimental_core::error::Error>", 88, 8),
        (probe, "core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::execute::Failure>", 272, 8),
        (variants, "core::option::Option<rune::experimental_core::detail::Detail>", 32, 8),
        (variants, "rune::experimental_core::detail::Detail::Coerce", 32, 8),
        (variants, "rune::experimental_core::detail::Detail::NumericArgumentCoerce", 32, 8),
        (variants, "rune::experimental_core::detail::IntTarget", 1, 1),
        (variants, "rune::experimental_core::detail::IntValue", 16, 8),
    ];
	for (group, name, size, align) in sizes {
		ensure!(
			group[name]["size"] == size && group[name]["align"] == align,
			"layout size or alignment differs: {name}"
		);
	}
	for (group, name, fields) in [
		(
			probe,
			"rune::experimental_core::error::Error",
			&[
				("kind", 640),
				("location", 0),
				("operation", 512),
				("refusal", 648),
				("detail", 256),
			][..],
		),
		(
			probe,
			"rune::experimental_core::execute::Failure",
			&[
				("remaining_work", 0),
				("numeric_context", 2112),
				("error", 128),
				("missing_key", 1728),
				("panic_message", 1856),
				("host_message", 1984),
				("call_chain", 1536),
				("cleanup_error", 832),
			][..],
		),
		(
			variants,
			"rune::experimental_core::detail::Detail::Coerce",
			&[("value", 64), ("to", 8)][..],
		),
		(
			variants,
			"rune::experimental_core::detail::Detail::NumericArgumentCoerce",
			&[("cause", 16), ("arg", 8), ("value", 64), ("to", 24)][..],
		),
	] {
		let actual_fields = group[name]["fields"]
			.as_array()
			.context("layout fields missing")?;
		ensure!(
			actual_fields.len() == fields.len(),
			"layout field count differs: {name}"
		);
		for (field, (expected_name, bitpos)) in actual_fields.iter().zip(fields) {
			ensure!(
				field["name"] == *expected_name && field["bitpos"] == *bitpos,
				"layout field differs: {name}.{expected_name}"
			);
		}
	}
	for (group, name, kind) in [
        (probe, "rune::experimental_core::detail::Detail", "u8"),
        (probe, "core::option::Option<rune::experimental_core::error::Error>", "u64"),
        (probe, "core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::error::Error>", "u64"),
        (probe, "core::result::Result<(), rune::experimental_core::error::Error>", "u64"),
        (probe, "core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::execute::Failure>", "u64"),
        (variants, "core::option::Option<rune::experimental_core::detail::Detail>", "u8"),
    ] {
        let fields = group[name]["fields"].as_array().context("discriminant fields missing")?;
        let first = fields.first().context("discriminant absent")?;
        ensure!(first["artificial"] == true && first["type"] == kind && first["bitpos"] == 0,
                "layout discriminant differs: {name}");
    }

	Ok(())
}
