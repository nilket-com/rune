//! Frozen DWARF layouts: GDB observes types; this Rust module decides equivalence.
use crate::common::{read_json, write_json, Context};
use anyhow::{ensure, Context as _, Result};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::{fs, process::Command};

const TYPES: [&str; 8] = [
	"rune::experimental_core::detail::Detail",
	"rune::experimental_core::error::Error",
	"rune::experimental_core::execute::Failure",
	"Option<rune::experimental_core::error::Error>",
	"Result<rune::experimental_core::value::Value, rune::experimental_core::error::Error>",
	"Result<(), rune::experimental_core::error::Error>",
	"Option<rune::experimental_core::detail::Detail>",
	"Result<rune::experimental_core::value::Value, rune::experimental_core::execute::Failure>",
];

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
	let mut actual: Value = serde_json::from_str(lines[0])?;
	let mut niche = Map::new();
	for (i, name) in TYPES.iter().enumerate() {
		let short = if name.starts_with("Option<") || name.starts_with("Result<") {
			*name
		} else {
			name.rsplit("::").next().unwrap()
		};
		let mut cmd = Command::new("llvm-dwarfdump-21");
		cmd.arg("-c").arg(format!("--name={short}")).arg(&binary);
		let r = ctx.checked(&format!("layout-niche-{i}"), &mut cmd, 300)?;
		let text = fs::read_to_string(r.stdout)?;
		niche.insert((*name).to_owned(), niche_record(&text, short)?);
	}
	actual["layout_niche"] = Value::Object(niche);
	let mut rows = vec![];
	for kind in ["layout_probe", "layout_variants", "layout_niche"] {
		let expected: Value = read_json(&ctx.repo.join(format!(
			"tools/xtask/fixtures/native/LAYOUT-FIXED-tip-{kind}.json"
		)))?;
		write_json(&ctx.out.join(format!("{kind}.json")), &actual[kind])?;
		ensure!(actual[kind] == expected, "frozen DWARF mismatch: {kind}");
		rows.push(json!({"kind":kind,"identical":true,"records":expected.as_object().context("layout object absent")?.len()}));
	}
	Ok(
		json!({"status":"PASS","rule":"Byte-identical frozen DWARF records against published M5; no variant allowance","results":rows}),
	)
}

fn hex(s: &str) -> Result<u64> {
	Ok(u64::from_str_radix(s.trim_start_matches("0x"), 16)?)
}

fn niche_record(text: &str, short: &str) -> Result<Value> {
	// Split only top-level structure/union/enumeration DIE boundaries, as the
	// reviewed observer did; nested members remain available to summarize.
	let start =
		Regex::new(r"(?m)^0x[0-9a-f]+: DW_TAG_(?:structure_type|union_type|enumeration_type)")?;
	let offsets: Vec<_> = start.find_iter(text).map(|m| m.start()).collect();
	for (i, &a) in offsets.iter().enumerate() {
		let b = &text[a..offsets.get(i + 1).copied().unwrap_or(text.len())];
		let first = b.split("\n\n").next().unwrap_or("");
		let line = b.lines().next().unwrap_or("");
		if !(line.contains("DW_TAG_structure_type") || line.contains("DW_TAG_union_type"))
			|| !first.contains(&format!("DW_AT_name\t(\"{short}\")"))
		{
			continue;
		}
		if short == "Error"
			&& !(b.contains("DW_AT_name\t(\"kind\")") && b.contains("DW_AT_name\t(\"detail\")"))
		{
			continue;
		}
		return summarize(b, short);
	}
	Ok(Value::Null)
}
fn summarize(b: &str, short: &str) -> Result<Value> {
	let first = b.split("\n\n").next().unwrap();
	let size_re = Regex::new(r"DW_AT_byte_size\t\((0x[0-9a-f]+)\)")?;
	let size = hex(&size_re.captures(first).context("DWARF byte size absent")?[1])?;
	let align_re = Regex::new(r"DW_AT_alignment\t\((\d+)\)")?;
	let align = align_re
		.captures(first)
		.map(|c| c[1].parse::<u64>())
		.transpose()?;
	let vp_re = Regex::new(r"\n0x[0-9a-f]+:(\s+)DW_TAG_variant_part\n")?;
	let mut disc = Value::Null;
	let mut variants = vec![];
	if let Some(vp) = vp_re.captures(b) {
		let ind = vp[1].len();
		let rest = &b[vp.get(0).unwrap().end()..];
		let member = Regex::new(
			r#"DW_TAG_member\n(?:.*\n)*?\s+DW_AT_type\t\(0x[0-9a-f]+ "([^"]+)"\)\n(?:.*\n)*?\s+DW_AT_data_member_location\t\((0x[0-9a-f]+)\)"#,
		)?;
		let m = member
			.captures(rest)
			.context("DWARF discriminant member absent")?;
		disc = json!({"type":&m[1],"offset":hex(&m[2])?});
		let pattern = format!(
			r#"\n0x[0-9a-f]+:{}DW_TAG_variant\n(?:(?:\s+DW_AT_discr_value\t\((0x[0-9a-f]+)\)\n))?\n0x[0-9a-f]+:\s+DW_TAG_member\n\s+DW_AT_name\t\("([^"]+)"\)"#,
			" ".repeat(ind + 2)
		);
		for v in Regex::new(&pattern)?.captures_iter(rest) {
			variants.push(json!({"name":&v[2],"discr":v.get(1).map(|m|m.as_str())}));
		}
	}
	let mut result = json!({"size":size,"align":align,"discriminant":disc,"variants":variants});
	if short == "Error" {
		let indent = Regex::new(r"^0x[0-9a-f]+:(\s+)")?
			.captures(b)
			.context("DIE indentation absent")?[1]
			.len();
		let pattern = format!(
			r#"\n0x[0-9a-f]+:{}DW_TAG_member\n\s+DW_AT_name\t\("(\w+)"\)\n\s+DW_AT_type\t\(0x[0-9a-f]+ "([^"]+)"\)\n(?:\s+DW_AT_alignment\t\(\d+\)\n)?\s+DW_AT_data_member_location\t\((0x[0-9a-f]+)\)"#,
			" ".repeat(indent + 2)
		);
		result["members"] = Value::Array(
			Regex::new(&pattern)?
				.captures_iter(b)
				.map(|m| json!([&m[1], &m[2], &m[3]]))
				.collect(),
		);
	}
	Ok(result)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn missing_type_is_explicit_null() {
		assert_eq!(niche_record("not a DIE", "Error").unwrap(), Value::Null);
	}
	#[test]
	fn direct_members_only() {
		let b="0x01: DW_TAG_structure_type\n DW_AT_name\t(\"Error\")\n DW_AT_byte_size\t(0x58)\n\n0x02:   DW_TAG_member\n DW_AT_name\t(\"kind\")\n DW_AT_type\t(0x01 \"u8\")\n DW_AT_data_member_location\t(0x50)\n\n0x03:     DW_TAG_member\n DW_AT_name\t(\"detail\")\n DW_AT_type\t(0x01 \"u8\")\n DW_AT_data_member_location\t(0x00)\n";
		let r = niche_record(b, "Error").unwrap();
		assert_eq!(r["members"].as_array().unwrap().len(), 1);
	}
}
