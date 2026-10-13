//! S-13b: dynamic half-open Range indices, separately from range construction.
use super::*;

fn cases() -> Vec<(String, String)> {
	let mut cases = Vec::new();
	let mut add = |id: String, body: String| {
		cases.push((id, format!("struct P {{x}} fn g() {{7}} fn make_range() {{0..2}} fn index() {{println!(\"index\"); 0..2}} fn rhs() {{println!(\"rhs\"); [7,8]}} pub fn main() {{{body}}}")));
	};
	let receivers = [
		("unit", "()"),
		("bool", "true"),
		("signed", "7"),
		("unsigned", "7u64"),
		("float", "7.0"),
		("vec", "[10,20,30]"),
		("empty-vec", "[]"),
		("tuple", "(10,20,30)"),
		("object", "#{x:10}"),
		("struct", "P{x:10}"),
		("none", "None"),
		("some", "Some(10)"),
		("ok", "Ok(10)"),
		("err", "Err(10)"),
		("string", "\"abc\""),
		("function", "g"),
		("type", "i64"),
		("range", "0..3"),
	];
	for (name, receiver) in receivers {
		for (bound, lo, hi) in [
			("normal", "0", "2"),
			("empty", "1", "1"),
			("reversed", "2", "1"),
			("past-end", "0", "4"),
			("start-past-end", "4", "5"),
			("negative-start", "-1", "1"),
			("negative-end", "0", "-1"),
			("full", "0", "3"),
		] {
			for (op, action) in [
				("get", "v[r]"),
				("set", "v[r]=[7,8]; ()"),
				("add", "v[r]+=[7,8]; ()"),
			] {
				add(
					format!("dynamic/{name}/{bound}/{op}"),
					format!("let v={receiver}; let lo={lo}; let hi={hi}; let r=lo..hi; {action}"),
				);
			}
		}
	}
	for (name, receiver) in [
		("vec", "[10,20,30]"),
		("string", "\"abc\""),
		("tuple", "(10,20,30)"),
		("struct", "P{x:10}"),
	] {
		for (origin, make) in [("return", "make_range()"), ("field", "P{x:0..2}.x")] {
			for (op, action) in [
				("get", "v[r]"),
				("set", "v[r]=[7,8]; ()"),
				("add", "v[r]+=[7,8]; ()"),
			] {
				add(
					format!("provenance/{name}/{origin}/{op}"),
					format!("let v={receiver}; let r={make}; {action}"),
				);
			}
		}
	}
	for (name, lo, hi) in [
		("u64", "0u64", "2u64"),
		("f64", "0.0", "2.0"),
		("string", "\"a\"", "\"b\""),
		("unit", "()", "()"),
		("i64-u64", "0", "2u64"),
		("u64-i64", "0u64", "2"),
		("i64-f64", "0", "2.0"),
		("f64-i64", "0.0", "2"),
	] {
		for (op, action) in [
			("get", "v[r]"),
			("set", "v[r]=[7,8]; ()"),
			("add", "v[r]+=[7,8]; ()"),
		] {
			add(
				format!("construction/{name}/{op}"),
				format!("let v=[10,20,30]; let lo={lo}; let hi={hi}; let r=lo..hi; {action}"),
			);
		}
	}
	for (name, receiver) in [
		("vec", "[10,20,30]"),
		("string", "\"abc\""),
		("tuple", "(10,20,30)"),
	] {
		for (op, action) in [
			("get", "v[0..2]"),
			("set", "v[0..2]=[7,8]; ()"),
			("add", "v[0..2]+=[7,8]; ()"),
		] {
			add(
				format!("literal/{name}/{op}"),
				format!("let v={receiver}; {action}"),
			);
		}
	}
	for (name, expr) in [
		("inclusive", "0..=2"),
		("from", "0.."),
		("to", "..2"),
		("to-inclusive", "..=2"),
		("full", ".."),
	] {
		add(
			format!("refusal/{name}"),
			format!("let v=[10,20,30]; let r={expr}; v[r]"),
		);
	}
	for (name, body) in [
		("vec-child", "let v=[[1],[2],[3]]; let r=0..2; let s=v[r]; s[0].push(9); (v[0].len(),s[0].len(),v.len(),s.len())"),
		("vec-replace", "let v=[[1],[2],[3]]; let r=0..2; let s=v[r]; s[0]=[8,9]; (v[0][0],s[0][0],v[0].len(),s[0].len())"),
		("vec-outer-push", "let v=[[1],[2],[3]]; let r=0..2; let s=v[r]; s.push([4]); (v.len(),s.len())"),
		("vec-source-replace", "let v=[[1],[2],[3]]; let r=0..2; let s=v[r]; v[0]=[8,9]; (v[0][0],s[0][0])"),
		("vec-reuse", "let v=[1,2,3]; let r=0..2; let s=v[r]; (v.len(),s.len(),v[r].len())"),
		("string-copy", "let v=\"abc\"; let r=0..2; let s=v[r]; s.push('z'); (v,s)"),
		("utf8-whole", "let v=\"aéz\"; let r=1..3; v[r]"),
		("utf8-split-start", "let v=\"aéz\"; let r=2..3; v[r]"),
		("utf8-split-end", "let v=\"aéz\"; let r=1..2; v[r]"),
		("utf8-empty-split", "let v=\"aéz\"; let r=2..2; v[r]"),
		("utf8-scalar-count", "let v=\"é😀z\"; let r=0..2; v[r]"),
		("empty-string", "let v=\"\"; let r=0..0; v[r]"),
		("effects-get", "let v=[1,2,3]; v[index()]"),
		("effects-set", "let v=[1,2,3]; v[index()]=rhs(); ()"),
		("effects-add", "let v=[1,2,3]; v[index()]+=rhs(); ()"),
		("effects-wrong-receiver", "let v=7; v[index()]=rhs(); ()"),
		("moved-vec-get", "let v=[1,2,3]; let a=v; drop(a); let r=0..2; v[r]"),
		("moved-vec-set", "let v=[1,2,3]; let a=v; drop(a); v[index()]=rhs(); ()"),
		("moved-vec-add", "let v=[1,2,3]; let a=v; drop(a); v[index()]+=rhs(); ()"),
		("moved-string-get", "let v=\"abc\"; let a=v; drop(a); let r=0..2; v[r]"),
		("moved-range-get", "let v=[1,2,3]; let r=0..2; let a=r; drop(a); v[r]"),
		("moved-range-set", "let v=[1,2,3]; let r=0..2; let a=r; drop(a); v[r]=rhs(); ()"),
		("moved-both", "let v=[1,2,3]; let a=v; drop(a); let r=0..2; let a=r; drop(a); v[r]"),
		("leased-vec-get", "let v=[1,2,3]; let r=0..2; for x in v { v[r]; } ()"),
		("leased-vec-set", "let v=[1,2,3]; let r=0..2; for x in v { v[r]=rhs(); } ()"),
		("leased-vec-add", "let v=[1,2,3]; let r=0..2; for x in v { v[r]+=rhs(); } ()"),
		("leased-range-get", "let v=[1,2,3]; let r=0..2; for x in r { v[r]; } ()"),
		("leased-range-set", "let v=[1,2,3]; let r=0..2; for x in r { v[r]=rhs(); } ()"),
	] {
		add(format!("witness/{name}"), body.into());
	}
	assert_eq!(cases.len(), 522);
	cases
}

#[test]
fn fixed_case_list() {
	let cases = cases();
	let mut ids = std::collections::BTreeSet::new();
	for (id, _) in &cases {
		assert!(ids.insert(id), "duplicate {id}");
	}
}

#[test]
#[ignore = "bounded range-index oracle study; requires new GAP_OUT and RANGE_ENGINE"]
fn probe() {
	let engine = std::env::var("RANGE_ENGINE").expect("RANGE_ENGINE");
	assert!(matches!(engine.as_str(), "stock" | "fresh"));
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("GAP_OUT"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-range-index-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut table =
		String::from("case\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\n");
	for (n, (label, source)) in cases().iter().enumerate() {
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", &engine, &scratch.0, &out, &id);
		assert!(run.complete, "{label}: {}", run.record);
		super::len_clone::assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		assert!(
			!run.record.split('\t').nth(1).unwrap().contains("0x"),
			"unstable class: {}",
			run.record
		);
		table.push_str(&format!(
			"{label}\t{}\t{}\t{}\t{}\n",
			hex(source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr)
		));
		fs::write(out.join(format!("{engine}-range-index.tsv")), &table).unwrap();
	}
}

#[test]
#[ignore = "bounded frozen stock range-index contract; requires new GAP_OUT"]
fn stock_contract() {
	contract("stock");
}

#[test]
#[ignore = "bounded fresh range-index convergence contract; requires new GAP_OUT"]
fn fresh_contract() {
	contract("fresh");
}

fn contract(engine: &str) {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("GAP_OUT"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-range-contract-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let generated = cases();
	let rows: Vec<_> = include_str!("../range_index_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(generated.len(), rows.len());
	for (n, ((label, source), row)) in generated.iter().zip(rows).enumerate() {
		let row: Vec<_> = row.split('\t').collect();
		assert_eq!(row.len(), 5);
		assert_eq!(label, row[0]);
		assert_eq!(*source, unhex(row[1]), "source drift: {label}");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let run = launch(&path, &path, "main", engine, &scratch.0, &out, &id);
		assert!(run.complete, "{label}: {}", run.record);
		super::len_clone::assert_no_addresses(label, &run.record, &run.stdout, &run.stderr);
		let actual: Vec<_> = run.record.split('\t').collect();
		assert!(!actual[1].contains("0x"), "unstable class: {}", run.record);
		if engine == "fresh"
			&& ((label.starts_with("literal/") && label.ends_with("/get"))
				|| label.starts_with("refusal/"))
		{
			assert_eq!(
				&actual[..2],
				&["refused", "Some(Range)"],
				"coverage drift: {label}"
			);
			continue;
		}
		let expected = unhex(row[2]);
		let expected: Vec<_> = expected.split('\t').collect();
		// The engine-specific source-site label is retained, not equated.
		assert_eq!(
			(&actual[..2], &actual[3..]),
			(&expected[..2], &expected[3..]),
			"observation drift: {label}"
		);
		assert_eq!(hex(&run.stdout), row[3], "stdout drift: {label}");
		assert_eq!(hex(&run.stderr), row[4], "stderr drift: {label}");
	}
}
