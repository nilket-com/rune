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
fn assert_no_addresses(label: &str, record: &str, stdout: &[u8], stderr: &[u8]) {
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
}
