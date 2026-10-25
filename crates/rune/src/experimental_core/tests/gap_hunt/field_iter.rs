//! S-field stock oracle; production remains unchanged until the table is frozen.
use super::*;

fn cases() -> Vec<(String, String)> {
	let targets = [
		("unit", "()"),
		("bool", "true"),
		("char", "'a'"),
		("signed", "7"),
		("unsigned", "7u64"),
		("float", "7.0"),
		("vec", "[10,20]"),
		("empty-vec", "[]"),
		("tuple", "(10,20)"),
		("empty-tuple-unit", "()"),
		("empty-object", "#{}"),
		("object", "#{x:10}"),
		("three-key-object", "#{x:10,y:20,z:30}"),
		("struct", "P{x:10}"),
		("tuple-struct", "T(10,20)"),
		("unit-struct", "U"),
		("none", "None"),
		("some", "Some(10)"),
		("ok", "Ok(10)"),
		("err", "Err(10)"),
		("string", "\"ab\""),
		("bytes", "b\"ab\""),
		("function", "g"),
		("type", "i64"),
		("range", "0..2"),
	];
	let mut out = Vec::new();
	for (label, target) in targets {
		for field in ["x", "missing"] {
			for (op, body) in [
				("get", format!("x.{field}")),
				("set", format!("x.{field} = 3; x.{field}")),
				("add", format!("x.{field} += 3; x.{field}")),
			] {
				out.push((
					format!("field/{label}/{field}/{op}"),
					format!("let x={target}; {body}"),
				));
			}
		}
		let body = if label == "three-key-object" {
			// This observes the key/value map and count in source, independent of hash order.
			"let a=[0,0,0,0]; let n=0; for pair in x { n+=1; if pair.0==\"x\" { a[0]=pair.1; } else if pair.0==\"y\" { a[1]=pair.1; } else if pair.0==\"z\" { a[2]=pair.1; } else { a[3]+=1; } } (a,n)"
		} else {
			"let a=[]; for v in x { a.push(v); } a"
		};
		out.push((
			format!("iterate/{label}"),
			format!("let x={target}; {body}"),
		));
	}
	let prefix = "struct P { x } fn g() { 1 }";
	let mut out: Vec<_> = out
		.into_iter()
		.map(|(id, body)| {
			let extra = if body.contains("T(10,20)") {
				"struct T(a,b);"
			} else if body.contains("let x=U;") {
				"struct U;"
			} else {
				""
			};
			(id, format!("{prefix} {extra} pub fn main() {{ {body} }}"))
		})
		.collect();
	for (label, target, rhs) in [
		("object-present", "#{x:10}", "3"),
		("object-missing", "#{}", "3"),
		("scalar", "7", "3"),
		("heap", "[10]", "3"),
		("binary-fails", "#{x:10}", "\"bad\""),
		("target-fails", "1 / 0", "3"),
		("rhs-fails", "#{x:10}", "1 / 0"),
	] {
		for (op, body) in [
			("get", "target().x"),
			("set", "target().x=rhs(); ()"),
			("add", "target().x+=rhs(); ()"),
		] {
			out.push((format!("witness/{label}/{op}"),format!("fn target() {{ println!(\"target\"); {target} }} fn rhs() {{ println!(\"rhs\"); {rhs} }} pub fn main() {{ {body} }}")));
		}
	}
	for (label, body) in [
		("object-update", "let x=#{x:10}; for p in x { x.x=2; } x.x"),
		("object-insert", "let x=#{x:10}; for p in x { x.y=2; } x.x"),
		(
			"object-alias-insert",
			"let x=#{x:10}; let y=x; for p in x { y.y=2; } x.x",
		),
		("vec-push", "let x=[10]; for v in x { x.push(2); } x.len()"),
		("tuple-write", "let x=(10,20); for v in x { x.0=2; } x.0"),
		("option-write", "let x=Some(10); for v in x { x.0=2; } x.0"),
		(
			"object-break-release",
			"let x=#{x:10}; for p in x { break; } x.y=2; x.y",
		),
		(
			"object-nested-break-release",
			"let x=#{x:10}; for p in x { for q in x { break; } break; } x.y=2; x.y",
		),
	] {
		out.push((
			format!("iterate-mutation/{label}"),
			format!("pub fn main() {{ {body} }}"),
		));
	}
	for (label, source) in [
		("option-payload-snapshot", "pub fn main() { let x=Some([10]); let old=x.0; for v in x { x.0=[20]; v.push(30); } (x.0,old) }"),
		("option-wrapper-replace", "pub fn main() { let x=Some(10); let a=[]; for v in x { x=None; a.push(v); } (x,a) }"),
		("object-return-release", "fn visit(x) { for p in x { return p.1; } 0 } pub fn main() { let x=#{x:10}; let n=visit(x); x.y=2; (n,x.y) }"),
		("object-body-error", "pub fn main() { let x=#{x:10}; for p in x { let n=1/0; } () }"),
	] {
		out.push((format!("lifecycle/{label}"),source.into()));
	}
	for (label, source) in [
		("scalar-replace", "pub fn main() { let x=Some(10); let a=[]; for v in x { x.0=2; a.push(v); } (a,x.0) }"),
		("heap-replace", "pub fn main() { let x=Some([1]); let a=[]; for v in x { x.0=[9]; a.push(v); } (a,x) }"),
		("payload-shared", "pub fn main() { let x=Some([1]); for v in x { v.push(2); } x }"),
		("wrapped-payload-shared", "pub fn main() { let x=Some([1]); for v in x { x.0.push(2); } x }"),
		("heap-break", "pub fn main() { let x=Some([1]); for v in x { break; } x.0.push(2); x }"),
		("heap-return", "fn visit(x) { for v in x { return v; } [] } pub fn main() { let x=Some([1]); let a=visit(x); a.push(2); x }"),
		("heap-body-error", "pub fn main() { let x=Some([1]); for v in x { let n=1/0; } () }"),
		("none-empty", "pub fn main() { let x=None; let a=[]; for v in x { a.push(v); } (x,a) }"),
		("none-unreachable-mutation", "pub fn main() { let x=None; let a=[]; for v in x { x=Some(2); a.push(v); } (x,a) }"),
	] {
		out.push((format!("option-snapshot/{label}"),source.into()));
	}
	for (label, body) in [
		("integer", "let x=#{x:10}; for p in x { x.x+=1; } x.x"),
		(
			"integer-bad-rhs",
			"let x=#{x:10}; for p in x { x.x+=\"bad\"; } x.x",
		),
		("string", "let x=#{x:\"a\"}; for p in x { x.x+=\"b\"; } x.x"),
		(
			"string-bad-rhs",
			"let x=#{x:\"a\"}; for p in x { x.x+=1; } x.x",
		),
	] {
		out.push((
			format!("field-compound-reader/{label}"),
			format!("pub fn main() {{ {body} }}"),
		));
	}
	for (label, target) in [("struct-present", "P{x:10}"), ("struct-missing", "P{y:10}")] {
		for (op, body) in [
			("get", "target().x"),
			("set", "target().x=rhs(); ()"),
			("add", "target().x+=rhs(); ()"),
		] {
			let declaration = if label == "struct-present" {
				"struct P {x}"
			} else {
				"struct P {y}"
			};
			out.push((format!("witness/{label}/{op}"),format!("{declaration} fn target() {{ println!(\"target\"); {target} }} fn rhs() {{ println!(\"rhs\"); 3 }} pub fn main() {{ {body} }}")));
		}
	}
	out.push((
		"field-overflow".into(),
		"pub fn main() { let x=#{x:9223372036854775807}; x.x+=1; x.x }".into(),
	));
	assert_eq!(out.len(), 228);
	out
}

#[test]
#[ignore = "explicit bounded S-field stock study; requires new GAP_OUT"]
fn legacy_probe() {
	run_probe("stock");
}

#[test]
#[ignore = "explicit bounded S-field execution inventory; requires new GAP_OUT"]
fn probe() {
	run_probe("fresh");
}

fn run_probe(engine: &str) {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-field-probe-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let cases = cases();
	let mut table =
		String::from("case\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\n");
	let mut counts = BTreeMap::new();
	for (n, (label, source)) in cases.iter().enumerate() {
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", engine, &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {label}: {}", run.record);
		let phase = run.record.split('\t').next().unwrap();
		assert!(
			matches!(phase, "value" | "error") || (engine == "fresh" && phase == "refused"),
			"stock admission {label}: {}",
			run.record
		);
		*counts.entry(phase.to_owned()).or_insert(0usize) += 1;
		table.push_str(&format!(
			"{label}\t{}\t{}\t{}\t{}\n",
			hex(source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr)
		));
		fs::write(out.join(format!("{engine}-field-iterate.tsv")), &table).unwrap();
	}
	fs::write(out.join("summary.txt"),format!("{} cases: {counts:?}\n25 receivers x(2 fields x3 ops + iteration),27 operand witnesses,25 mutation/lease/lifecycle cases,1 overflow case. Object multi-key observations are order-independent in source.\n",cases.len())).unwrap();
}

#[test]
#[ignore = "explicit frozen field/iteration contract; bounded children"]
fn contract() {
	let engine = std::env::var("GAP_COMPARE_ENGINE").unwrap_or_else(|_| "stock".into());
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-field-contract-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let generated = cases();
	let rows: Vec<_> = include_str!("../field_iter_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), generated.len());
	let mut coverage = 0;
	for (n, (line, (label, source))) in rows.iter().zip(&generated).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 5);
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source, "source drift: {label}");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let mut run = launch(&path, &path, "main", &engine, &scratch.0, &out, &id);
		assert!(
			run.complete,
			"incomplete {engine} case {label}: {}",
			run.record
		);
		run.record = super::method_diagnostics::intentional_record_before(
			"field_iter",
			label,
			source,
			&engine,
			&run.record,
		);
		let actual: Vec<_> = run.record.split('\t').collect();
		assert_eq!(actual.len(), 5);
		let mut pieces = label.split('/');
		let root = pieces.next().unwrap();
		let target = pieces.next().unwrap_or("");
		let refusal = if matches!(root, "field" | "iterate") {
			match target {
				"bytes" => Some("Some(ByteStr)"),
				_ => None,
			}
		} else {
			None
		};
		if engine == "fresh" {
			if let Some(refusal) = refusal {
				assert_eq!(
					(&actual[0], &actual[1]),
					(&"refused", &refusal),
					"coverage refusal: {label}"
				);
				coverage += 1;
				continue;
			}
		}
		let expected = unhex(row[2]);
		let expected: Vec<_> = expected.split('\t').collect();
		assert_eq!(expected.len(), 5);
		// Operation labels are engine-specific; exact metadata has focused tests.
		assert_eq!(
			(&actual[..2], &actual[3..]),
			(&expected[..2], &expected[3..]),
			"{engine} observation {label}"
		);
		assert_eq!(hex(&run.stdout), row[3], "{engine} stdout {label}");
		assert_eq!(hex(&run.stderr), row[4], "{engine} stderr {label}");
	}
	assert_eq!(coverage, if engine == "fresh" { 7 } else { 0 });
	super::contract_completed(&engine, rows.len());
}
