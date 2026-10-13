//! C1 assertion oracle. Stock source, outcomes, streams and exact failure sites
//! are frozen before any production macro admission changes.
use super::*;

fn cases() -> Vec<(String, String)> {
	let mut out = Vec::new();
	for macro_ in ["assert", "assert_eq", "assert_ne"] {
		for passes in [true, false] {
			let args = match macro_ {
				"assert" => passes.to_string(),
				"assert_eq" => if passes { "1, 1" } else { "1, 2" }.into(),
				"assert_ne" => if passes { "1, 2" } else { "1, 1" }.into(),
				_ => unreachable!(),
			};
			for (form, message) in [
				("plain", ""),
				("trailing", ","),
				("literal", ", \"why\""),
				("format", ", \"why {} {:?}\", 7, [1, 2]"),
				("message-trailing", ", \"why {}\", 7,"),
			] {
				out.push((
					format!("forms/{macro_}/{passes}/{form}"),
					format!("pub fn main() {{ {macro_}!({args}{message}); 42 }}"),
				));
			}
		}
	}
	let pairs = [
		("unit", "()", "()"),
		("bool", "true", "false"),
		("char", "'a'", "'b'"),
		("signed", "7", "8"),
		("unsigned", "7u64", "8u64"),
		("float", "7.0", "8.0"),
		("string", "\"a\"", "\"b\""),
		("vec", "[1,2]", "[1,3]"),
		("tuple", "(1,2)", "(1,3)"),
		("object", "#{x:1}", "#{x:2}"),
		("struct", "P{x:1}", "P{x:2}"),
		("some", "Some(1)", "Some(2)"),
		("none", "None", "None"),
		("ok", "Ok(1)", "Ok(2)"),
		("err", "Err(1)", "Err(2)"),
		("variant", "Ok(1)", "Err(1)"),
		("mixed-int", "1", "1u64"),
		("mixed-float", "1", "1.0"),
		("nan", "f64::NAN", "f64::NAN"),
		("bad-pair", "1", "\"one\""),
	];
	for (label, lhs, rhs) in pairs {
		for macro_ in ["assert_eq", "assert_ne"] {
			for (shape, right) in [("same", lhs), ("different", rhs)] {
				out.push((
					format!("operands/{label}/{macro_}/{shape}"),
					format!("struct P {{x}} pub fn main() {{ {macro_}!({lhs}, {right}); 42 }}"),
				));
			}
		}
	}
	for (label, cond) in [
		("signed", "1"),
		("unsigned", "0u64"),
		("float", "1.0"),
		("string", "\"x\""),
		("unit", "()"),
		("vec", "[]"),
	] {
		out.push((
			format!("condition/{label}"),
			format!("pub fn main() {{ assert!({cond}); 42 }}"),
		));
	}
	for macro_ in ["assert", "assert_eq", "assert_ne"] {
		for passes in [true, false] {
			let operands = match macro_ {
				"assert" => format!("mark(\"condition\", {passes})"),
				"assert_eq" => format!(
					"mark(\"left\", 1), mark(\"right\", {})",
					if passes { 1 } else { 2 }
				),
				"assert_ne" => format!(
					"mark(\"left\", 1), mark(\"right\", {})",
					if passes { 2 } else { 1 }
				),
				_ => unreachable!(),
			};
			out.push((format!("order/{macro_}/{passes}"),format!("fn mark(name, value) {{ println!(\"{{}}\", name); value }} pub fn main() {{ println!(\"before\"); {macro_}!({operands}, \"why {{}}\", mark(\"message\", 9)); println!(\"after\"); 42 }}")));
		}
	}
	for (label, body) in [
		("condition-error", "assert!(1/0==0)"),
		("left-error", "assert_eq!(1/0, mark(\"right\", 1))"),
		("right-error", "assert_eq!(mark(\"left\", 1), 1/0)"),
		("message-error", "assert!(false, \"why {}\", 1/0)"),
		("message-skipped", "assert!(true, \"why {}\", 1/0)"),
		("eq-message-skipped", "assert_eq!(1,1, \"why {}\", 1/0)"),
		("ne-message-skipped", "assert_ne!(1,2, \"why {}\", 1/0)"),
		(
			"comparison-before-message",
			"assert_eq!(1,\"x\", \"why {}\", mark(\"message\", 9))",
		),
	] {
		out.push((format!("errors/{label}"),format!("fn mark(name, value) {{ println!(\"{{}}\", name); value }} pub fn main() {{ {body}; 42 }}")));
	}
	for (label, source) in [
		("once-owned-operands", "fn left(w) { w.push(1); [7] } fn right(w) { w.push(2); [7] } pub fn main() { let w=[]; assert_eq!(left(w),right(w)); w }"),
		("rhs-mutates-left", "fn right(v) { v.push(2); [1,2] } pub fn main() { let v=[1]; assert_eq!(v,right(v)); v }"),
		("message-mutates-left", "pub fn main() { let v=[1]; assert_eq!(v,[2],\"why {}\",{v.push(3); 9}); 42 }"),
	] {
		out.push((format!("ownership/{label}"), source.into()));
	}
	out.push((
		"ownership/unicode-long-message".into(),
		format!(
			"pub fn main() {{ assert!(false,\"{{}}終\",\"{}\"); 42 }}",
			"α".repeat(1024)
		),
	));
	for depth in [62, 63, 64, 65] {
		for (kind, wrap) in [
			("vec", "[a]"),
			("tuple", "(a,)"),
			("object", "#{x:a}"),
			("option", "Some(a)"),
			("result", "Ok(a)"),
		] {
			for macro_ in ["assert_eq", "assert_ne"] {
				out.push((format!("depth/{kind}/{depth}/{macro_}"),format!("pub fn main() {{ let a=1; for _ in 0..{depth} {{ a={wrap}; }} {macro_}!(a,a); 42 }}")));
			}
		}
	}
	for (label, body) in [
		(
			"pass",
			"assert!(true); assert_eq!(1,1); assert_ne!(1,2); 42",
		),
		("fail", "assert_eq!(1,2); 42"),
	] {
		out.push((
			format!("test-entry/{label}"),
			format!("#[test] fn main() {{ {body} }}"),
		));
	}
	for (label, body) in [
		("alias", "use std::test::assert_eq as check; pub fn main() { check!(1,1); 42 }"),
		("absolute", "pub fn main() { ::std::test::assert!(true); ::std::test::assert_eq!(1,1); ::std::test::assert_ne!(1,2); 42 }"),
	] { out.push((format!("resolution/{label}"),body.into())); }
	out
}

#[test]
#[ignore = "bounded C1 stock oracle; requires new GAP_OUT"]
fn stock_probe() {
	probe("stock");
}

#[test]
#[ignore = "bounded C1 refusal inventory; requires new GAP_OUT"]
fn fresh_probe() {
	probe("fresh");
}

fn probe(engine: &str) {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-assert-probe-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut table = String::from(
		"case\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tstock_site\n",
	);
	let cases = cases();
	for (n, (label, source)) in cases.iter().enumerate() {
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let site = scratch.0.join("result.site");
		let _ = fs::remove_file(&site);
		let run = launch(&path, &path, "assert-main", engine, &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {label}: {}", run.record);
		let phase = run.record.split('\t').next().unwrap();
		assert!(
			matches!(phase, "value" | "error") || (engine == "fresh" && phase == "refused"),
			"admission {label}: {}",
			run.record
		);
		let site = if phase == "error" {
			fs::read_to_string(site).unwrap()
		} else {
			String::new()
		};
		assert!(!run.record.contains("0x"), "unstable address {label}");
		table.push_str(&format!(
			"{label}\t{}\t{}\t{}\t{}\t{site}\n",
			hex(source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr)
		));
		fs::write(out.join(format!("{engine}-assertions.tsv")), &table).unwrap();
	}
	fs::write(
		out.join("summary.txt"),
		format!(
			"{} {engine} cases complete; source/outcome/streams/exact stock sites\n",
			cases.len()
		),
	)
	.unwrap();
}

#[test]
#[ignore = "explicit frozen C1 stock contract; bounded children"]
fn stock_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-assert-contract-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let cases = cases();
	let rows: Vec<_> = include_str!("../assertions_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), cases.len());
	for (n, (line, (label, source))) in rows.iter().zip(&cases).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 6);
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let site = scratch.0.join("result.site");
		let _ = fs::remove_file(&site);
		let run = launch(&path, &path, "assert-main", "stock", &scratch.0, &out, &id);
		assert!(run.complete, "{label}");
		assert_eq!(hex(run.record.as_bytes()), row[2], "observation {label}");
		assert_eq!(hex(&run.stdout), row[3], "stdout {label}");
		assert_eq!(hex(&run.stderr), row[4], "stderr {label}");
		let actual = if run.record.starts_with("error\t") {
			fs::read_to_string(site).unwrap()
		} else {
			String::new()
		};
		assert_eq!(actual, row[5], "site {label}");
	}
	super::contract_completed("stock", rows.len());
}

#[test]
#[ignore = "explicit C1 depth64 supplement; no other FormatLimits changed"]
fn raised_depth_probe() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-assert-depth64-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut table = String::from(
		"case\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tstock_site\n",
	);
	for (n, (label, source)) in cases()
		.into_iter()
		.enumerate()
		.filter(|(_, (label, _))| label.starts_with("depth/"))
	{
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &source).unwrap();
		let site = scratch.0.join("result.site");
		let _ = fs::remove_file(&site);
		let run = launch(
			&path,
			&path,
			"assert-main-depth64",
			"fresh",
			&scratch.0,
			&out,
			&id,
		);
		assert!(run.complete, "{label}");
		let site = if run.record.starts_with("error\t") {
			fs::read_to_string(site).unwrap()
		} else {
			String::new()
		};
		table.push_str(&format!(
			"{label}\t{}\t{}\t{}\t{}\t{site}\n",
			hex(source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr)
		));
	}
	fs::write(out.join("fresh-depth64.tsv"), table).unwrap();
}

fn expected_record(row: &[&str]) -> Vec<String> {
	let raw = unhex(row[2]);
	let mut fields: Vec<String> = raw.split('\t').map(str::to_owned).collect();
	assert_eq!(fields.len(), 5);
	if fields[1].starts_with("stock:Panic { reason: Panic { inner: ") && fields[1].ends_with(" } }")
	{
		fields[1] = "Panic".into();
	} else if fields[1] == "stock:MaxValueDepth { max: 64 }" {
		fields[1] = "ValueDepth".into();
	}
	fields
}
fn assert_exact(label: &str, row: &[&str], run: &Run, site: &str) {
	let actual: Vec<_> = run.record.split('\t').collect();
	let expected = expected_record(row);
	assert_eq!(actual.len(), 5);
	assert_eq!(actual[0], expected[0], "phase {label}");
	assert_eq!(actual[1], expected[1], "class {label}");
	assert_eq!(actual[3], expected[3], "message {label}");
	assert_eq!(actual[4], expected[4], "value {label}");
	assert_eq!(hex(&run.stdout), row[3], "stdout {label}");
	assert_eq!(hex(&run.stderr), row[4], "stderr {label}");
	assert_eq!(site, row[5], "location {label}");
}

#[test]
#[ignore = "explicit C1 fresh assertion contract and depth64 resource/semantic supplement"]
fn fresh_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-assert-fresh-contract-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let rows: Vec<_> = include_str!("../assertions_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	let generated = cases();
	assert_eq!(rows.len(), 178);
	assert_eq!(rows.len(), generated.len());
	let mut refused = 0;
	let mut budget = 0;
	let mut semantic = 0;
	let mut retained = String::from("case\tpolicy\n");
	for (n, (line, (label, source))) in rows.iter().zip(&generated).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 6);
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let site_path = scratch.0.join("result.site");
		let _ = fs::remove_file(&site_path);
		let run = launch(&path, &path, "assert-main", "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{label}");
		let actual: Vec<_> = run.record.split('\t').collect();
		assert_eq!(actual.len(), 5);
		if label.starts_with("operands/nan/") {
			assert_eq!(
				&actual[..2],
				&["refused", "Some(UnresolvedConstant)"],
				"{label}"
			);
			refused += 1;
			retained.push_str(&format!("{label}\tUnresolvedConstant\n"));
			continue;
		}
		let parts: Vec<_> = label.split('/').collect();
		let deep = parts.len() == 4
			&& parts[0] == "depth"
			&& matches!(parts[2], "62" | "63")
			&& parts[3] == "assert_ne";
		let site = if actual[0] == "error" {
			fs::read_to_string(&site_path).unwrap()
		} else {
			String::new()
		};
		if !deep {
			assert_exact(label, &row, &run, &site);
			continue;
		}
		// Existing host-configured depth32 budget is never raised for the default run.
		assert_eq!(&actual[..2], &["error", "FormatLimit"], "{label}");
		assert_eq!(unhex(actual[3]), "format bytes/depth/nodes: FormatLimit");
		assert!(actual[4].is_empty());
		let start = source.find("assert_ne!").unwrap();
		let end = start + "assert_ne!(a,a)".len();
		let expected_site = format!("0:{start}:{end}");
		assert_eq!(site, expected_site, "budget site {label}");
		assert_eq!(hex(&run.stdout), row[3]);
		assert_eq!(hex(&run.stderr), row[4]);
		let _ = fs::remove_file(&site_path);
		let raised = launch(
			&path,
			&path,
			"assert-main-depth64",
			"fresh",
			&scratch.0,
			&out,
			&format!("{id}-depth64"),
		);
		assert!(raised.complete, "{label}");
		let raised_site = fs::read_to_string(&site_path).unwrap();
		if parts[2] == "63" && parts[1] != "object" {
			// Independent formatter-walk semantics gap, NOT a budget-only proof.
			// Derive the complete depth63 message from the frozen depth62 stock
			// representation by adding exactly one source container wrapper.
			let lower = label.replace("/63/", "/62/");
			let lower_row: Vec<_> = rows
				.iter()
				.find(|r| r.starts_with(&format!("{lower}\t")))
				.unwrap()
				.split('\t')
				.collect();
			let lower_expected = expected_record(&lower_row);
			let message = unhex(&lower_expected[3]);
			let (_, rest) = message.split_once("\nleft: ").unwrap();
			let (left, right) = rest.split_once("\nright: ").unwrap();
			let (prefix, suffix) = match parts[1] {
				"vec" => ("[", "]"),
				"tuple" => ("(", ")"),
				"option" => ("Some(", ")"),
				"result" => ("Ok(", ")"),
				_ => unreachable!(),
			};
			let expected=format!("Panicked: assertion failed (left != right):\nleft: {prefix}{left}{suffix}\nright: {prefix}{right}{suffix}");
			let fields: Vec<_> = raised.record.split('\t').collect();
			assert_eq!(&fields[..2], &["error", "Panic"]);
			assert_eq!(unhex(fields[3]), expected);
			assert!(fields[4].is_empty());
			assert_eq!(raised_site, expected_site);
			assert_eq!(hex(&raised.stdout), row[3]);
			assert_eq!(hex(&raised.stderr), row[4]);
			semantic += 1;
			retained.push_str(&format!("{label}\tRetained(formatter-depth semantics)\n"));
		} else {
			assert_exact(label, &row, &raised, &raised_site);
			budget += 1;
			retained.push_str(&format!(
				"{label}\tRetained(format budget); exact at configured depth64\n"
			));
		}
	}
	assert_eq!((refused, budget, semantic), (4, 6, 4));
	fs::write(out.join("RETAINED.tsv"), retained).unwrap();
	super::contract_completed("fresh", rows.len());
}
