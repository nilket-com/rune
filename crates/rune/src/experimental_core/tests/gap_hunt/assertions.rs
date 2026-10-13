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
		let site = if phase == "error" && engine == "stock" {
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
