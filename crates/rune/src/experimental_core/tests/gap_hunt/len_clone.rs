//! Frozen, bounded stock method/clone/invalidation oracle. No production changes.
use super::*;

fn cases() -> Vec<(String, String)> {
	let mut out = Vec::new();
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
	for (label, target) in targets {
		for (op, call) in [
			("len", "x.len()"),
			("clone", "x.clone()"),
			("push-integer", "x.push(1)"),
			("push-string", "x.push(\"s\")"),
			("push-effect", "x.push(effect())"),
			("len-extra", "x.len(effect())"),
			("clone-extra", "x.clone(effect())"),
		] {
			out.push((format!("method/{label}/{op}"), format!("struct P {{x}} struct T(a,b); struct U; fn g() {{ 1 }} fn effect() {{ println!(\"rhs\"); 1 }} pub fn main() {{ let x={target}; {call} }}")));
		}
	}
	for (label, body) in [
		(
			"tuple-vec",
			"let a=([1],2); let b=a.clone(); b.0.push(9); (a,b)",
		),
		(
			"tuple-option-vec",
			"let a=(Some([1]),); let b=a.clone(); b.0.0.push(9); (a,b)",
		),
		(
			"nested-tuple",
			"let a=((([1],),),); let b=a.clone(); b.0.0.0.push(9); (a,b)",
		),
		(
			"option-tuple",
			"let a=Some(([1],)); let b=a.clone(); b.0.0.push(9); (a,b)",
		),
		(
			"branching",
			"let a=([1],([2],)); let b=a.clone(); b.0.push(9); b.1.0.push(8); (a,b)",
		),
		(
			"tuple-object",
			"let a=(#{k:[1]},); let b=a.clone(); b.0.k.push(9); (a,b)",
		),
		(
			"vec-shallow",
			"let a=[[1]]; let b=a.clone(); b[0].push(9); b.push([2]); (a,b)",
		),
		(
			"object-shallow",
			"let a=#{k:[1]}; let b=a.clone(); b.k.push(9); b.z=2; (a.k,b.k,b.z)",
		),
		(
			"string-copy",
			"let a=\"a\"; let b=a.clone(); b+=\"b\"; (a,b)",
		),
		(
			"option-struct-child",
			"let a=Some(P{x:[1]}); let b=a.clone(); b.0.x.push(9); (a,b)",
		),
		(
			"option-struct-field",
			"let a=Some(P{x:1}); let b=a.clone(); b.0.x=5; (a,b)",
		),
		(
			"tuple-struct-child",
			"let a=(P{x:[1]},); let b=a.clone(); b.0.x.push(9); (a,b)",
		),
		(
			"tuple-struct-field",
			"let a=(P{x:1},); let b=a.clone(); b.0.x=5; (a,b)",
		),
		(
			"result-ok",
			"let a=Ok([1]); let b=a.clone(); b.0.push(9); (a,b)",
		),
		(
			"result-err",
			"let a=Err([1]); let b=a.clone(); b.0.push(9); (a,b)",
		),
		("function-call", "let a=g; let b=a.clone(); b()"),
		("function-identity", "let a=g; let b=a.clone(); a==b"),
		(
			"repeated-dag",
			"let x=Some([1]); let a=(x,x); let b=a.clone(); b.0.0.push(9); (a,b)",
		),
		(
			"vec-cycle-shallow",
			"let a=[()]; a[0]=a; let b=a.clone(); b.push(2); (a.len(),b.len(),b[0].len())",
		),
		(
			"object-cycle-shallow",
			"let a=#{x:()}; a.x=a; let b=a.clone(); b.y=2; (a.len(),b.len(),b.x.len())",
		),
	] {
		out.push((
			format!("clone/{label}"),
			format!("struct P {{x}} fn g() {{ 7 }} pub fn main() {{ {body} }}"),
		));
	}
	for depth in [1, 2, 8, 64] {
		for wrapper in ["Some", "Ok", "Err"] {
			let mut expr = String::from("[1]");
			for _ in 0..depth {
				expr = format!("{wrapper}({expr})");
			}
			out.push((
				format!("clone/depth-{depth}-{wrapper}"),
				format!("pub fn main() {{ let a={expr}; let b=a.clone(); b{access}.push(9); (a{access}.len(),b{access}.len()) }}", access=".0".repeat(depth)),
			));
		}
	}
	for (label, target, child) in [
		("vec", "[[1]]", "a[0]"),
		("object", "#{f:[1]}", "a.f"),
		("tuple", "([1],)", "a.0"),
		("string", "\"ab\"", "a"),
		("option", "Some([1])", "a.0"),
		("struct", "P{f:[1]}", "a.f"),
	] {
		let prefix =
			format!("struct P {{f}} fn pass(x) {{ 1 }} pub fn main() {{ let a={target}; let b=a;");
		let mut add = |id: &str, action: String| {
			out.push((
				format!("moved/{label}/{id}"),
				format!("{prefix} {action} }}"),
			))
		};
		add("alias-live", "a".into());
		for (op, action) in [
			("field-read", "a.f"),
			("index-read", "a[0]"),
			("len", "a.len()"),
			("clone", "a.clone()"),
			("pass", "pass(a)"),
			("debug", "println!(\"{:?}\",a); ()"),
			("display", "println!(\"{}\",a); ()"),
			("field-write", "a.f=2; ()"),
			("index-write", "a[0]=2; ()"),
			("push", "a.push(1)"),
			("field-add", "a.f+=1; ()"),
		] {
			add(op, format!("drop(b); {action}"));
		}
		add("double-drop", "drop(b); drop(a); ()".into());
		add(
			"nested-parent",
			format!(
				"let c={child}; drop(c); {}",
				if label == "object" { "a.len()" } else { "a" }
			),
		);
		add("nested-child", format!("let c={child}; drop(c); {child}"));
		add("revive-live", format!("drop(b); a={target}; a"));
		add("revive-alias-moved", format!("drop(b); a={target}; b"));
	}
	out.push(("moved/take-argument".into(), "pub fn main() { let object=#{field:1}; let object2=object; println!(\"field: {}\",object.field); drop(object2); println!(\"field: {}\",object.field); }".into()));
	assert_eq!(out.len(), 310);
	out
}

// Reject process addresses rather than normalizing them. Function instruction
// offsets (e.g. 0x5) are short; missing-instance-function hashes are stable IDs.
pub(super) fn assert_no_addresses(label: &str, record: &str, stdout: &[u8], stderr: &[u8]) {
	let fields: Vec<_> = record.split('\t').collect();
	assert_eq!(fields.len(), 5);
	let texts = [
		unhex(fields[2]),
		unhex(fields[3]),
		unhex(fields[4]),
		String::from_utf8_lossy(stdout).into_owned(),
		String::from_utf8_lossy(stderr).into_owned(),
	];
	for (field, text) in texts.iter().enumerate() {
		for (offset, _) in text.match_indices("0x") {
			let digits = text[offset + 2..]
				.bytes()
				.take_while(u8::is_ascii_hexdigit)
				.count();
			if digits < 8 {
				continue;
			}
			assert!(
				field == 1
					&& fields[0] == "error"
					&& fields[1] == "Type"
					&& text.starts_with("Missing instance function `0x")
					&& offset == "Missing instance function `".len()
					&& text[offset + 2 + digits..].starts_with("` for `"),
				"process address in {label}: {text}"
			);
		}
	}
}

#[test]
#[ignore = "explicit bounded S-len/clone stock study; requires new GAP_OUT"]
fn stock_probe() {
	probe("stock");
}

#[test]
#[ignore = "explicit bounded S-len/clone fresh inventory; requires new GAP_OUT"]
fn fresh_probe() {
	probe("fresh");
}

fn probe(engine: &str) {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-len-clone-probe-{}", std::process::id())));
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
		assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		let phase = run.record.split('\t').next().unwrap();
		assert!(
			matches!(
				phase,
				"value" | "error" | "compile-error" | "observation-error"
			) || (engine == "fresh" && phase == "refused"),
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
		fs::write(out.join(format!("{engine}-len-clone.tsv")), &table).unwrap();
	}
	fs::write(out.join("summary.txt"), format!("{} cases: {counts:?}\nBounded M/C/V matrix. Full phase/class/message/value/stdout/stderr retained. Deep and recursive-cycle stock probes excluded.\n", cases.len())).unwrap();
}
#[test]
#[ignore = "explicit frozen stock method/clone/invalidation contract; bounded children"]
fn stock_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-len-clone-contract-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let generated = cases();
	let rows: Vec<_> = include_str!("../len_clone_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), generated.len());
	for (n, (line, (label, source))) in rows.iter().zip(&generated).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 5);
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source, "source drift: {label}");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", "stock", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {label}: {}", run.record);
		assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		assert_no_addresses(label, &unhex(row[2]), &[], &[]);
		assert_eq!(run.record, unhex(row[2]), "observation drift: {label}");
		assert_eq!(hex(&run.stdout), row[3], "stdout drift: {label}");
		assert_eq!(hex(&run.stderr), row[4], "stderr drift: {label}");
	}
	super::contract_completed("stock", rows.len());
}

#[test]
#[ignore = "explicit fresh method/clone/invalidation contract with exact coverage refusals; bounded children"]
fn fresh_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch = Scratch(std::env::temp_dir().join(format!(
		"rune-len-clone-fresh-contract-{}",
		std::process::id()
	)));
	fs::create_dir(&scratch.0).unwrap();
	let generated = cases();
	let rows: Vec<_> = include_str!("../len_clone_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), generated.len());
	let mut coverage = 0;
	let mut retained = 0;
	for (n, (line, (label, source))) in rows.iter().zip(&generated).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 5);
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source, "source drift: {label}");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {label}: {}", run.record);
		assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		assert_no_addresses(label, &unhex(row[2]), &[], &[]);
		let actual: Vec<_> = run.record.split('\t').collect();
		let pieces: Vec<_> = label.split('/').collect();
		let refusal = if pieces[0] == "method" {
			if pieces[1] == "bytes" {
				Some("Some(ByteStr)")
			} else if matches!(pieces[2], "len-extra" | "clone-extra")
				&& !matches!(pieces[1], "tuple-struct" | "unit-struct")
			{
				Some("Some(Method)")
			} else {
				None
			}
		} else {
			None
		};
		if let Some(class) = refusal {
			assert_eq!(
				(&actual[0], &actual[1]),
				(&"refused", &class),
				"coverage drift: {label}"
			);
			coverage += 1;
			continue;
		}
		if label == "method/function/clone" {
			assert_eq!((&actual[0], &actual[1]), (&"observation-error", &"Debug"));
			assert_eq!(unhex(actual[3]), "value formatting form: Unsupported");
			assert_eq!(unhex(actual[4]), "");
			retained += 1;
		} else {
			let expected = unhex(row[2]);
			let expected: Vec<_> = expected.split('\t').collect();
			assert_eq!(
				(&actual[..2], &actual[3..]),
				(&expected[..2], &expected[3..]),
				"observation drift: {label}"
			);
		}
		assert_eq!(hex(&run.stdout), row[3], "stdout drift: {label}");
		assert_eq!(hex(&run.stderr), row[4], "stderr drift: {label}");
	}
	assert_eq!(coverage, 51);
	assert_eq!(retained, 1);
	super::contract_completed("fresh", rows.len());
}

#[test]
#[ignore = "bounded normal-stack lowering boundary study; requires new GAP_OUT"]
fn lowering_stack_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-stack-contract-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut summary = String::new();
	for depth in [64, 128, 129] {
		for shape in ["some", "binary", "tuple", "call", "block"] {
			let mut expr = String::from("1");
			for _ in 0..depth {
				expr = match shape {
					"some" => format!("Some({expr})"),
					"binary" => format!("1+({expr})"),
					"tuple" => format!("({expr},)"),
					"call" => format!("id({expr})"),
					"block" => format!("{{ {expr} }}"),
					_ => unreachable!(),
				};
			}
			let source = format!("fn id(x) {{x}} pub fn main() {{ let x={expr}; () }}");
			let id = format!("{shape}-{depth}");
			let path = out.join(format!("{id}.source.rn"));
			fs::write(&path, source).unwrap();
			let run = launch(&path, &path, "compile", "fresh", &scratch.0, &out, &id);
			assert!(run.complete, "{id}: {}", run.record);
			let fields: Vec<_> = run.record.split('\t').collect();
			assert!(
				fields[0] == "compiled"
					|| (fields[0] == "refused"
						&& (fields[1] == "Some(DepthLimit)"
							|| (fields[1] == "Some(FrontendDiagnostic)"
								&& unhex(fields[3]).contains("MaxItemDepth { max: 128 }")))),
				"{id}: {}",
				run.record
			);
			if shape == "some" && depth == 64 {
				assert_eq!(fields[0], "compiled");
			}
			summary.push_str(&format!("{id}	{}	{}\n", fields[0], fields[1]));
		}
	}
	fs::write(out.join("summary.tsv"), summary).unwrap();
}

fn global_cases() -> Vec<(String, String)> {
	let mut out = Vec::new();
	for (label, value) in [
		("unit", "()"),
		("bool", "true"),
		("char", "'a'"),
		("signed", "7"),
		("unsigned", "7u64"),
		("float", "7.0"),
		("vec", "[1]"),
		("tuple", "(1,2)"),
		("object", "#{x:1}"),
		("string", "\"ab\""),
		("none", "None"),
		("some", "Some([1])"),
		("ok", "Ok([1])"),
		("err", "Err([1])"),
		("type", "i64"),
		("range", "0..2"),
	] {
		for (mode, call) in [
			("global", "std::clone::clone(x)"),
			("associated", "x.clone()"),
		] {
			let body = if label == "type" && mode == "global" {
				format!("let y={call}; y==x")
			} else {
				call.into()
			};
			out.push((
				format!("{mode}/{label}"),
				format!("pub fn main() {{ let x={value}; {body} }}"),
			));
		}
	}
	for (label, body) in [
		("global/struct", "let x=P{x:1}; let y=std::clone::clone(x); y.x=7; x.x"),
		("associated/struct", "let x=P{x:1}; x.clone()"),
		("global/function", "let x=g; let y=std::clone::clone(x); y(7)"),
		("associated/function", "let x=g; let y=x.clone(); y(7)"),
		("global/nested", "let x=Some((Ok([1]),)); let y=std::clone::clone(x); y.0.0.0.push(9); (x.0.0.0.len(),y.0.0.0.len())"),
		("associated/nested", "let x=Some((Ok([1]),)); let y=x.clone(); y.0.0.0.push(9); (x.0.0.0.len(),y.0.0.0.len())"),
		("user/clone", "clone(true)"),
	] {
		out.push((label.into(),format!("struct P{{x}} fn g(x){{x+1}} fn clone(x){{17}} pub fn main(){{ {body} }}")));
	}
	out
}

#[test]
#[ignore = "bounded stock-first global/associated clone witnesses; requires new GAP_OUT"]
fn global_stock_probe() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-global-clone-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut table =
		String::from("case\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\n");
	for (n, (label, source)) in global_cases().iter().enumerate() {
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", "stock", &scratch.0, &out, &id);
		assert!(run.complete, "{label}: {}", run.record);
		assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		table.push_str(&format!(
			"{label}\t{}\t{}\t{}\t{}\n",
			hex(source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr)
		));
		fs::write(out.join("global-clone-stock.tsv"), &table).unwrap();
	}
}

#[test]
fn clone_op_layout() {
	assert_eq!(core::mem::size_of::<super::super::super::code::Op>(), 24);
}

#[test]
#[ignore = "stock/fresh supplementary global clone contract; bounded children"]
fn global_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-global-contract-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let cases = global_cases();
	let rows: Vec<_> = include_str!("../global_clone_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), cases.len());
	for (n, (row, (label, source))) in rows.iter().zip(&cases).enumerate() {
		let row: Vec<_> = row.split('\t').collect();
		assert_eq!(row[0], label);
		assert_eq!(unhex(row[1]), *source);
		let path = out.join(format!("{n:05}.source.rn"));
		fs::write(&path, source).unwrap();
		for engine in ["stock", "fresh"] {
			let run = launch(
				&path,
				&path,
				"main",
				engine,
				&scratch.0,
				&out,
				&format!("{n:05}"),
			);
			assert!(run.complete, "{label}/{engine}: {}", run.record);
			assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
			let expected = unhex(row[2]);
			let expected: Vec<_> = expected.split('\t').collect();
			let actual: Vec<_> = run.record.split('\t').collect();
			assert_eq!(
				(&actual[..2], &actual[3..]),
				(&expected[..2], &expected[3..]),
				"{label}/{engine}"
			);
			assert_eq!(hex(&run.stdout), row[3]);
			assert_eq!(hex(&run.stderr), row[4]);
		}
	}
	super::contract_completed("both", rows.len());
}

#[test]
fn lowering_runs_at_the_normal_stack_boundary() {
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-stack-regression-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let out = scratch.0.join("out");
	fs::create_dir(&out).unwrap();
	for shape in ["some", "binary", "tuple", "call", "block"] {
		let last = if shape == "block" { 126 } else { 127 };
		for depth in [last, last + 1] {
			let mut expr = String::from("1");
			for _ in 0..depth {
				expr = match shape {
					"some" => format!("Some({expr})"),
					"binary" => format!("{expr}+1"),
					"tuple" => format!("({expr},)"),
					"call" => format!("id({expr})"),
					"block" => format!("{{ {expr} }}"),
					_ => unreachable!(),
				};
			}
			let id = format!("{shape}-{depth}");
			let path = out.join(format!("{id}.rn"));
			fs::write(&path, format!("fn id(x){{x}} pub fn main(){{ {expr}; 1 }}")).unwrap();
			let run = launch(&path, &path, "stack-main", "fresh", &scratch.0, &out, &id);
			assert!(run.complete, "{id}: {}", run.record);
			let fields: Vec<_> = run.record.split('\t').collect();
			if depth == last {
				assert_eq!(fields[0], "value", "{id}: {}", run.record);
				assert_eq!(unhex(fields[4]), "i64:1:1");
			} else {
				assert_eq!(fields[0], "refused", "{id}: {}", run.record);
				assert!(
					fields[1] == "Some(DepthLimit)"
						|| (fields[1] == "Some(FrontendDiagnostic)"
							&& unhex(fields[3]).contains("MaxItemDepth { max: 128 }")),
					"{id}: {}",
					run.record
				);
			}
		}
	}
}
