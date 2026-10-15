//! C3 stock pattern inventory. No production lowering or admission changes.
//! This source checkpoint must be peer-reviewed before its ignored probe runs.
use super::*;

struct Case {
	label: String,
	source: String,
	mode: String,
}

fn add(out: &mut Vec<Case>, family: &str, name: &str, body: &str) {
	out.push(Case {
		label: format!("{family}/{name}"),
		source: format!("pub fn main() {{ {body} }}"),
		mode: "c3-main".into(),
	});
}

fn cases() -> Vec<Case> {
	let mut out = Vec::new();
	for (name, pattern, values) in [
		("signed", "5", ["5", "6", "5u64", "true", "\"5\""]),
		("negative", "-5", ["-5", "5", "5u64", "false", "[]"]),
		("unsigned", "5u64", ["5u64", "6u64", "5", "true", "\"5\""]),
		("byte", "b'a'", ["97u64", "98u64", "97", "'a'", "\"a\""]),
		("true", "true", ["true", "false", "1", "1u64", "[]"]),
		("false", "false", ["false", "true", "0", "0u64", "None"]),
		("char", "'α'", ["'α'", "'β'", "945", "945u64", "\"α\""]),
		("string", "\"α\"", ["\"α\"", "\"β\"", "'α'", "1", "[]"]),
	] {
		for (n, value) in values.iter().enumerate() {
			add(
				&mut out,
				"lit",
				&format!("{name}/{n}"),
				&format!("match {value} {{ {pattern} => 11, _ => 22 }}"),
			);
		}
	}
	for (name, pattern) in [("empty", "()"), ("rest", "(..)"), ("prefix", "(x, ..)")] {
		for (target, value) in [
			("unit", "()"),
			("one", "(1,)"),
			("two", "(1,2)"),
			("vec", "[]"),
			("scalar", "1"),
		] {
			add(
				&mut out,
				"unit",
				&format!("{name}/{target}"),
				&format!("match {value} {{ {pattern} => 11, _ => 22 }}"),
			);
		}
	}
	for (name, body) in [
		("nested", "match ((),7) { ((),x) => x, _ => 22 }"),
		("nested-rest", "match ((1,2),7) { ((..),x) => x, _ => 22 }"),
		(
			"unit-expression",
			"match { let x=1; () } { () => 11, _ => 22 }",
		),
		("empty-vec-rest", "match [] { [..] => 11, _ => 22 }"),
		("empty-vec-closed", "match [] { [] => 11, _ => 22 }"),
	] {
		add(&mut out, "unit", name, body);
	}
	for (name, pattern) in [
		("tuple-closed", "(x,5)"),
		("tuple-open", "(x, ..)"),
		("tuple-ignore", "(_,5)"),
		("tuple-nested", "((_,x),5)"),
		("vec-closed", "[x,5]"),
		("vec-open", "[x, ..]"),
		("vec-ignore", "[_,5]"),
		("vec-nested", "[[_,x],5]"),
	] {
		for (target, value) in [
			("zero", "[]"),
			("one", "[1]"),
			("two", "[1,5]"),
			("three", "[1,5,9]"),
			("tuple", "(1,5)"),
			("nested-tuple", "((\"a\",2),5)"),
			("nested-vec", "[[\"a\",2],5]"),
			("wrong", "7"),
		] {
			add(
				&mut out,
				"seq",
				&format!("{name}/{target}"),
				&format!("match {value} {{ {pattern} => 11, _ => 22 }}"),
			);
		}
	}
	for (name, pattern, result) in [
		("full", "P{a:x,b:y}", "(x,y)"),
		("partial", "P{a:x,..}", "x"),
		("ignore", "P{a:_,..}", "11"),
		("nested", "P{a:Some(x),..}", "x"),
	] {
		for (target, value) in [
			("p", "P{a:Some(3),b:5}"),
			("q", "Q{a:Some(3),b:5}"),
			("object", "#{a:Some(3),b:5}"),
			("scalar", "5"),
		] {
			out.push(Case { label:format!("struct/{name}/{target}"), source:format!("struct P{{a,b}} struct Q{{a,b}} pub fn main() {{ match {value} {{ {pattern} => {result}, _ => 22 }} }}"), mode:"c3-main".into() });
		}
	}
	for (name, pattern, result) in [
		("some", "Some(x)", "x"),
		("ignore", "Some(_)", "11"),
		("none", "None", "11"),
		("nested", "Some(Some(x))", "x"),
	] {
		for (target, value) in [
			("scalar", "7"),
			("none", "None"),
			("some", "Some(7)"),
			("heap", "Some([7])"),
			("nested", "Some(Some(7))"),
		] {
			add(
				&mut out,
				"option",
				&format!("{name}/{target}"),
				&format!("match {value} {{ {pattern} => {result}, _ => 22 }}"),
			);
		}
	}
	for (name, body) in [
		("true", "match Some(7) { Some(x) if true => x, _ => 22 }"),
		("false", "match Some(7) { Some(x) if false => x, _ => 22 }"),
		("non-bool", "match 7 { x if 1 => x, _ => 22 }"),
		("uses-binding", "match Some(7) { Some(x) if x==7 => x, _ => 22 }"),
		("mutate-vec", "let a=[1]; match a { [x] if { a[0]=2; false } => x, [y] => y, _ => 22 }"),
		("mutate-struct", "let a=P{a:1,b:2}; match a { P{a:x,..} if {a.a=3;false} => x, P{a:y,..} => y, _ => 22 }"),
		("mutate-option", "let a=Some(1); match a { Some(x) if {a.0=3;false} => x, Some(y) => y, _ => 22 }"),
		("error", "match Some(7) { Some(x) if 1/0==1 => x, _ => 22 }"),
		("return", "match Some(7) { Some(x) if {return x;} => 11, _ => 22 }"),
		("break", "loop { match Some(7) { Some(x) if {break x;} => 11, _ => 22 }; }"),
		("output", "match 7 { x if {println!(\"first\");false} => x, y if {println!(\"second\");true} => y, _ => 22 }"),
		("pattern-before-guard", "match None { Some(x) if {println!(\"never\");true} => x, _ => 22 }"),
	] {
		out.push(Case {label:format!("guard/{name}"), source:format!("struct P{{a,b}} pub fn main(){{{body}}}"), mode:"c3-main".into()});
	}
	for (name, body) in [
		("no-arm-value", "let a=match 7 { 8=>11 }; (a,9)"),
		("no-arm-discard", "match 7 { 8=>11 }; 9"),
		("empty", "match 7 {}"),
		("first-match", "match 7 { x=>x, 7=>22 }"),
		("arm-order", "match 7 { 8=>11, 7=>22, _=>33 }"),
		(
			"single-eval",
			"let a=[]; let r=match {a.push(1);Some(7)} {Some(x)=>x,_=>22}; (r,a)",
		),
		("body-return", "match Some([1]) {Some(x)=>return x,_=>[]}"),
		(
			"body-break",
			"loop {match Some([1]) {Some(x)=>break x,_=>[]};}",
		),
		(
			"body-error",
			"match Some([1]) {Some(x)=>{let a=x;1/0},_=>22}",
		),
		("mixed-result", "match Some(7) {Some(x)=>x,Ok(x)=>x,_=>22}"),
		("mixed-object", "match 7 {7=>11,#{x}=>x,_=>22}"),
		("top-ignore", "let a=[]; let _={a.push(1);7}; a"),
	] {
		add(&mut out, "flow", name, body);
	}
	for (name, pattern, value, result) in [
		("some", "Some(x)", "Some(7)", "x"),
		("none", "Some(x)", "None", "x"),
		("tuple", "(x,5)", "(7,5)", "x"),
		("tuple-miss", "(x,5)", "(7,6)", "x"),
		("vec", "[x,..]", "[7,8]", "x"),
		("vec-miss", "[x,..]", "[]", "x"),
		("unit", "()", "()", "11"),
		("nested", "Some((_,x))", "Some(([1],7))", "x"),
	] {
		add(
			&mut out,
			"iflet",
			name,
			&format!("if let {pattern}={value} {{ {result} }} else {{22}}"),
		);
		add(
			&mut out,
			"let",
			name,
			&format!("let {pattern}={value}; {result}"),
		);
	}
	for (name, body) in [
		("option", "let a=Some(1); let n=0; while let Some(x)=a {n+=x;a=None;} n"),
		("tuple", "let a=(1,5); let n=0; while let (x,5)=a {n+=x;a=(2,6);} n"),
		("vec", "let a=[1]; let n=0; while let [x]=a {n+=x;a=[];} n"),
		("false-entry", "let n=0; while let Some(x)=None {n+=1;} n"),
		("break", "let a=Some([1]); while let Some(x)=a {break;} a"),
		("return", "while let Some(x)=Some([1]) {return x;} []"),
		("error", "while let Some(x)=Some([1]) {let y=x;1/0;} ()"),
		("shadow", "let x=7; let a=Some(1); while let Some(x)=a {a=None;} x"),
		("repeated", "let a=[Some([1]),Some([2]),None]; let i=0; let sum=0; while let Some(x)=a[i] {sum+=x[0];i+=1;} sum"),
		("reeval", "let a=[]; let i=0; while let Some(x)={a.push(i);i+=1;if i<3 {Some(i)} else {None}} {} a"),
	] { add(&mut out,"whilelet",name,body); }
	for (name, pattern) in [
		("scalar", "5"),
		("string", "\"held\""),
		("tuple", "(_,)"),
		("vec", "[_]"),
		("option", "Some(_)"),
		("wildcard", "_"),
	] {
		for (target, setup, value) in [
			("string", "let a=\"held\";let b=a;drop(a);", "b"),
			("vec", "let a=[1];let b=a;drop(a);", "b"),
			("option", "let a=Some([1]);let b=a;a.unwrap();", "b"),
			(
				"child",
				"let a=\"held\";let b=a;drop(a);let c=Some(b);",
				"c",
			),
			(
				"tuple-child",
				"let a=\"held\";let b=a;drop(a);let c=(b,);",
				"c",
			),
		] {
			add(
				&mut out,
				"access",
				&format!("{target}/{name}"),
				&format!("{setup} match {value} {{ {pattern}=>11,_=>22 }}"),
			);
		}
	}
	for (name, body) in [
		("W1-parent-move", "let child=[1];let parent=Some(child);let alias=parent;let r=match parent {Some(x) if false=>0,_=>{alias.unwrap();child.push(2);child.len()}};(r,child)"),
		("W1-child-move", "let child=[1];let parent=Some(child);match parent {Some(x) if false=>0,_=>{drop(child);11}}"),
		("W2-partial", "let a=[[1],5,6];match a {[x,5,7]=>0,_=>{a[0].push(2);a.push(9);a}}"),
		("W2-tuple-partial", "let a=([1],5,6);match a {(x,5,7)=>0,_=>{a.0.push(2);a}}"),
		("W3-mutate", "let a=Some([1]);match a {Some(x) if {a.0=[2];false}=>x,Some(y)=>y,_=>[]}"),
		("W3-ignore-later", "let a=Some([1]);match a {Some(x) if {drop(a);false}=>0,Some(_)=>11,_=>22}"),
		("W4-repeat", "let a=[Some([1]),Some([2]),None];let i=0;while let Some(x)=a[i] {x.push(3);i+=1;} a"),
		("W5-guard-return", "let a=Some([1]);match a {Some(x) if {return x;}=>[],_=>[]}"),
		("W5-guard-break", "loop {match Some([1]) {Some(x) if {break x;}=>[],_=>[]};}"),
		("W5-guard-error", "match Some([1]) {Some(x) if 1/0==1=>x,_=>[]}"),
		("W6-output", "match Some([1]) {Some(x) if {println!(\"first {:?}\",x);false}=>0,Some(y)=>{println!(\"second {:?}\",y);11},_=>22}"),
		("shadow-restore", "let x=[9];let r=match Some([1]) {Some(x) if false=>x,_=>[2]};(x,r)"),
	] { add(&mut out,"life",name,body); }
	for (name, body) in [
		("result-ok", "match Ok(7) {Ok(x)=>x,Err(x)=>x}"),
		("result-err", "match Err(7) {Ok(x)=>x,Err(x)=>x}"),
		("object-closed", "match #{a:7} {#{a}=>a,_=>22}"),
		("object-open", "match #{a:7,b:8} {#{a,..}=>a,_=>22}"),
		("object-missing", "match #{} {#{a,..}=>a,_=>22}"),
		("float", "match 1.0 {1.0=>11,_=>22}"),
		("bytes", "match b\"a\" {b\"a\"=>11,_=>22}"),
	] {
		add(&mut out, "deferred", name, body);
	}
	for (name, prefix, body) in [
		(
			"enum",
			"enum E{A(x),B}",
			"match E::A(7) {E::A(x)=>x,E::B=>22}",
		),
		("tuple-struct", "struct T(x);", "match T(7) {T(x)=>x,_=>22}"),
		("constant", "const N=7;", "match 7 {N=>11,_=>22}"),
		(
			"qualified-constant",
			"mod m {pub const N=7;}",
			"match 7 {m::N=>11,_=>22}",
		),
		(
			"constant-tuple",
			"const T=(1,2);",
			"match (1,2) {T=>11,_=>22}",
		),
	] {
		out.push(Case {
			label: format!("paths/{name}"),
			source: format!("{prefix} pub fn main(){{{body}}}"),
			mode: "c3-main".into(),
		});
	}
	out.push(Case {
		label: "test/entry".into(),
		source: "#[test] fn specimen(){match Some(7){Some(x)=>x,_=>22}}".into(),
		mode: "c3-test:specimen".into(),
	});
	for fuel in 0..=20 {
		for (name, body) in [
			("tests", "match {println!(\"scrutinee\");Some((1,2))} {Some((x,3))=>x,Some((x,y))=>{println!(\"arm\");x+y},_=>22}"),
			("guards", "match 7 {x if {println!(\"first\");false}=>x,y if {println!(\"second\");true}=>y,_=>22}"),
			("loop", "let a=Some(1);while let Some(x)=a {println!(\"body\");a=None;} 7"),
		] {
			out.push(Case{label:format!("fuel/{name}/{fuel}"),source:format!("pub fn main(){{{body}}}"),mode:format!("c3-fuel-{fuel}")});
		}
	}
	out
}

pub(super) fn observe_patterns(path: &Path, mode: &str, engine: &str) -> Observation {
	assert!(matches!(engine, "stock" | "fresh"));
	let fuel = if engine == "fresh" {
		FUEL // Frozen stock budgets are not fresh budgets.
	} else if let Some(fuel) = mode.strip_prefix("c3-fuel-") {
		fuel.parse::<usize>().unwrap()
	} else {
		FUEL
	};
	let observation_mode = if let Some(entry) = mode.strip_prefix("c3-test:") {
		format!("patterns-test:{entry}")
	} else {
		assert!(mode == "c3-main" || mode.starts_with("c3-fuel-"));
		"patterns-main".into()
	};
	let context = Context::with_default_modules().unwrap();
	observe_with_context(path, &observation_mode, engine, &context, None, fuel)
}

fn supplement_cases() -> Vec<Case> {
	let mut out = Vec::new();
	for mut case in cases().into_iter().filter(|case| {
		case.label.starts_with("struct/")
			&& (case.label.ends_with("/p") || case.label.ends_with("/q"))
	}) {
		let value = if case.label.ends_with("/p") {
			"P{a:Some(3),b:5}"
		} else {
			"Q{a:Some(3),b:5}"
		};
		let from = format!("match {value} {{");
		let to = format!("match ({value}) {{");
		assert_eq!(case.source.matches(&from).count(), 1);
		case.source = case.source.replacen(&from, &to, 1);
		case.label = format!("supplement/parenthesized/{}", case.label);
		out.push(case);
	}
	assert_eq!(out.len(), 8);
	for (name, pattern) in [
		("some-ignore", "Some(_)"),
		("none", "None"),
		("scalar", "5"),
		("wildcard", "_"),
	] {
		add(
			&mut out,
			"supplement/dropped-option",
			name,
			&format!("let a=Some([1]); let b=a; drop(a); match b {{ {pattern}=>11,_=>22 }}"),
		);
	}
	for (name, value) in [("match", "P{a:7,b:8}"), ("mismatch", "Q{a:7,b:8}")] {
		out.push(Case {
			label: format!("supplement/struct-let/{name}"),
			source: format!(
				"struct P{{a,b}} struct Q{{a,b}} pub fn main(){{let P{{a:x,..}}=({value});x}}"
			),
			mode: "c3-main".into(),
		});
	}
	out
}

#[test]
#[ignore = "C3 stock oracle SOURCE CHECKPOINT; peer GO required, new GAP_OUT"]
fn stock_probe() {
	probe(cases(), 328);
}

#[test]
#[ignore = "C3 exact 14-row supplement; peer GO required, separate new GAP_OUT"]
fn stock_supplement_probe() {
	probe(supplement_cases(), 14);
}

fn probe(generated: Vec<Case>, expected: usize) {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c3-oracle-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	assert_eq!(generated.len(), expected, "C3 source matrix drift");
	let mut seen = std::collections::BTreeSet::new();
	let mut counts = BTreeMap::new();
	let mut table = String::from("case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tstock_site_hex\tdiagnostics_hex\tpolicy\n");
	for (n, case) in generated.iter().enumerate() {
		assert!(seen.insert(&case.label), "duplicate {}", case.label);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		for extension in ["site", "diagnostics"] {
			let previous = scratch.0.join(format!("result.{extension}"));
			if previous.exists() {
				fs::remove_file(previous).unwrap();
			}
		}
		let run = launch(&path, &path, &case.mode, "stock", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {}: {}", case.label, run.record);
		let phase = run.record.split('\t').next().unwrap();
		assert!(
			matches!(
				phase,
				"value" | "error" | "compile-error" | "observation-error"
			),
			"{}: {}",
			case.label,
			run.record
		);
		let site = if phase == "error" {
			fs::read(scratch.0.join("result.site")).unwrap()
		} else {
			Vec::new()
		};
		let diagnostics = fs::read(scratch.0.join("result.diagnostics")).unwrap();
		assert!(diagnostics.len() <= CAP, "diagnostics cap");
		let address = [
			run.record.as_bytes(),
			run.stdout.as_slice(),
			run.stderr.as_slice(),
			diagnostics.as_slice(),
		]
		.iter()
		.any(|bytes| String::from_utf8_lossy(bytes).contains("0x"));
		let policy = if address {
			"Observation(address-unmodified)"
		} else {
			"Exact"
		};
		*counts.entry(phase.to_owned()).or_insert(0usize) += 1;
		table.push_str(&format!(
			"{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{policy}\n",
			case.label,
			case.mode,
			hex(case.source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr),
			hex(&site),
			hex(&diagnostics)
		));
		fs::write(out.join(format!("{id}.stock.site")), &site).unwrap();
		fs::write(out.join(format!("{id}.stock.diagnostics")), &diagnostics).unwrap();
		fs::write(out.join("stock-patterns.tsv"), &table).unwrap();
	}
	fs::write(out.join("summary.txt"),format!("{} stock rows: {counts:?}\nCompile failures are separate from eligible runtime cases. Address observations are retained raw, never normalized. Stock fuel is not fresh fuel parity.\n",generated.len())).unwrap();
	contract_completed("stock", generated.len());
}

#[test]
#[ignore = "C3 exact 348-row frozen stock contract; bounded children, new GAP_OUT"]
fn stock_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).expect("new GAP_OUT required");
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-c3-stock-contract-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let mut generated = cases();
	generated.extend(supplement_cases());
	generated.extend(supplement2_cases());
	let mut frozen = include_str!("../patterns_stock.tsv").lines();
	assert_eq!(frozen.next(), Some("case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tstock_site_hex\tdiagnostics_hex\tpolicy"));
	let frozen: Vec<_> = frozen.collect();
	assert_eq!(frozen.len(), 348, "frozen C3 row count");
	assert_eq!(generated.len(), frozen.len(), "C3 source count");
	let mut seen = std::collections::BTreeSet::new();
	for (n, (line, case)) in frozen.iter().zip(&generated).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row.len(), 9, "C3 schema {}", case.label);
		assert!(seen.insert(row[0]), "duplicate frozen row {}", row[0]);
		assert_eq!(row[0], case.label, "C3 label drift");
		assert_eq!(row[1], case.mode, "C3 mode drift {}", case.label);
		assert_eq!(
			row[2],
			hex(case.source.as_bytes()),
			"C3 source drift {}",
			case.label
		);
		assert_eq!(row[8], "Exact", "C3 policy drift {}", case.label);
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		for extension in ["site", "diagnostics"] {
			let previous = scratch.0.join(format!("result.{extension}"));
			if previous.exists() {
				fs::remove_file(previous).unwrap();
			}
		}
		let run = launch(&path, &path, &case.mode, "stock", &scratch.0, &out, &id);
		assert!(run.complete, "incomplete {}: {}", case.label, run.record);
		let site = if run.record.starts_with("error\t") {
			fs::read(scratch.0.join("result.site")).unwrap()
		} else {
			Vec::new()
		};
		let diagnostics = fs::read(scratch.0.join("result.diagnostics")).unwrap();
		assert!(diagnostics.len() <= CAP, "diagnostics cap");
		fs::write(out.join(format!("{id}.stock.site")), &site).unwrap();
		fs::write(out.join(format!("{id}.stock.diagnostics")), &diagnostics).unwrap();
		assert_eq!(
			row[3],
			hex(run.record.as_bytes()),
			"C3 observation drift {}",
			case.label
		);
		assert_eq!(row[4], hex(&run.stdout), "C3 stdout drift {}", case.label);
		assert_eq!(row[5], hex(&run.stderr), "C3 stderr drift {}", case.label);
		assert_eq!(row[6], hex(&site), "C3 error site drift {}", case.label);
		assert_eq!(
			row[7],
			hex(&diagnostics),
			"C3 diagnostics/warnings drift {}",
			case.label
		);
	}
	contract_completed("stock", frozen.len());
}

fn supplement2_cases() -> Vec<Case> {
	[
		("supplement2/o1/bare-guard-rebind", "pub fn main(){let a=Some(1);match a{_ if {a=Some(9);false}=>0,Some(y)=>y,_=>22}}"),
		("supplement2/o1/block-variable-guard-rebind", "pub fn main(){let a=Some(1);match {a}{_ if {a=Some(9);false}=>0,Some(y)=>y,_=>22}}"),
		("supplement2/o1/effectful-block-variable-guard-rebind", "pub fn main(){let a=Some(1);let n=0;match {n+=1;a}{_ if {a=Some(9);false}=>(0,n),Some(y)=>(y,n),_=>(22,n)}}"),
		("supplement2/o1/while-let-identifier-rebind", "pub fn main(){let a=1;let out=[];let i=0;while let x=a{a=a+10;out.push(x);i+=1;if i>=3{break;}}(out,a)}"),
		("supplement2/moved-tuple/empty", "pub fn main(){let a=(1,);let b=a;drop(a);match b{()=>11,_=>22}}"),
		("supplement2/moved-tuple/rest", "pub fn main(){let a=(1,);let b=a;drop(a);match b{(..)=>11,_=>22}}"),
	]
	.into_iter()
	.map(|(label, source)| Case {
		label: label.into(),
		source: source.into(),
		mode: "c3-main".into(),
	})
	.collect()
}

#[test]
#[ignore = "C3 exact six-row alias/access supplement; peer GO required, new GAP_OUT"]
fn stock_supplement2_probe() {
	probe(supplement2_cases(), 6);
}

#[test]
#[ignore = "C3 development inventory only: source reviewed before final fresh contract"]
fn fresh_inventory() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-c3-fresh-inventory-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let mut generated = cases();
	generated.extend(supplement_cases());
	generated.extend(supplement2_cases());
	let frozen: Vec<_> = include_str!("../patterns_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(generated.len(), 348);
	assert_eq!(frozen.len(), 348);
	let mut report =
		String::from("case\tverdict\tstock_hex\tfresh_hex\tstock_site_hex\tfresh_site_hex\n");
	let mut counts = BTreeMap::new();
	for (n, (case, line)) in generated.iter().zip(&frozen).enumerate() {
		if case.label.starts_with("fuel/") {
			continue;
		} // Numeric budgets are NOT parity.
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row[0], case.label);
		assert_eq!(row[2], hex(case.source.as_bytes()));
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		let previous = scratch.0.join("result.site");
		if previous.exists() {
			fs::remove_file(previous).unwrap();
		}
		let run = launch(&path, &path, &case.mode, "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{}: {}", case.label, run.record);
		let fresh: Vec<_> = run.record.split('\t').collect();
		let stock = unhex(row[3]);
		let stock: Vec<_> = stock.split('\t').collect();
		let site = if fresh[0] == "error" {
			fs::read(scratch.0.join("result.site")).unwrap()
		} else {
			Vec::new()
		};
		fs::write(out.join(format!("{id}.fresh.site")), &site).unwrap();
		let verdict = if fresh[0] == "refused" {
			"refused"
		} else if fresh[..2] == stock[..2]
			&& fresh[3..] == stock[3..]
			&& hex(&run.stdout) == row[4]
			&& hex(&run.stderr) == row[5]
			&& hex(&site) == row[6]
		{
			"equal"
		} else {
			"divergence"
		};
		*counts.entry(verdict).or_insert(0usize) += 1;
		report.push_str(&format!(
			"{}\t{verdict}\t{}\t{}\t{}\t{}\n",
			case.label,
			row[3],
			hex(run.record.as_bytes()),
			row[6],
			hex(&site)
		));
		fs::write(out.join("inventory.tsv"), &report).unwrap();
	}
	fs::write(
		out.join("summary.txt"),
		format!("{counts:?};63 stock-only fuel rows skipped (fresh invariants separate)\n"),
	)
	.unwrap();
}

fn expected_fresh_refusal(label: &str) -> Option<&'static str> {
	if matches!(
		label,
		"flow/mixed-result" | "deferred/result-ok" | "deferred/result-err"
	) {
		return None;
	}
	if (label.starts_with("struct/") && (label.ends_with("/p") || label.ends_with("/q")))
		|| label == "paths/constant-tuple"
	{
		Some("Some(FrontendDiagnostic)")
	} else if (label.starts_with("let/") && label != "let/unit")
		|| label.starts_with("supplement/struct-let/")
	{
		Some("Some(Pattern)")
	} else if label.starts_with("access/option/") || label == "life/W1-parent-move" {
		Some("Some(Method)")
	} else if label == "deferred/bytes" {
		Some("Some(ByteStr)")
	} else if label.starts_with("deferred/")
		|| label.starts_with("flow/mixed-")
		|| matches!(label, "paths/enum" | "paths/tuple-struct")
	{
		Some("Some(Match)")
	} else {
		None
	}
}

#[test]
#[ignore = "C3 exact fresh contract plus declared refusals; new GAP_OUT; stock budgets never reused"]
fn fresh_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-c3-fresh-contract-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let mut generated = cases();
	generated.extend(supplement_cases());
	generated.extend(supplement2_cases());
	let frozen: Vec<_> = include_str!("../patterns_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(generated.len(), 348);
	assert_eq!(frozen.len(), 348);
	let mut equal = 0;
	let mut refused = 0;
	let mut full_budget = 0;
	for (n, (case, line)) in generated.iter().zip(&frozen).enumerate() {
		let row: Vec<_> = line.split('\t').collect();
		assert_eq!(row[0], case.label);
		assert_eq!(row[1], case.mode);
		assert_eq!(row[2], hex(case.source.as_bytes()));
		assert_eq!(row[8], "Exact");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		let old = scratch.0.join("result.site");
		if old.exists() {
			fs::remove_file(old).unwrap();
		}
		let run = launch(&path, &path, &case.mode, "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{}: {}", case.label, run.record);
		let actual: Vec<_> = run.record.split('\t').collect();
		assert_eq!(actual.len(), 5);
		if let Some(class) = expected_fresh_refusal(&case.label) {
			assert_eq!(
				(&actual[0], &actual[1]),
				(&"refused", &class),
				"C3 refusal {}",
				case.label
			);
			assert_eq!(run.stdout, b"\nrunning 1 test\n");
			assert!(run.stderr.is_empty());
			refused += 1;
			continue;
		}
		if case.label.starts_with("fuel/") {
			// This checks the same SOURCE at full fresh fuel. Exact fresh frontiers
			// are covered separately; the stock N is intentionally ignored here.
			let family = case.label.split('/').nth(1).unwrap();
			let (value, stdout) = match family {
				"tests" => ("i64:3:3", b"\nrunning 1 test\nscrutinee\narm\n".as_slice()),
				"guards" => ("i64:7:7", b"\nrunning 1 test\nfirst\nsecond\n".as_slice()),
				"loop" => ("i64:7:7", b"\nrunning 1 test\nbody\n".as_slice()),
				_ => panic!("unknown stock fuel source"),
			};
			assert_eq!(actual[0], "value");
			assert_eq!(unhex(actual[4]), value);
			assert_eq!(run.stdout, stdout);
			assert!(run.stderr.is_empty());
			full_budget += 1;
			continue;
		}
		let expected = unhex(row[3]);
		let expected: Vec<_> = expected.split('\t').collect();
		assert_eq!(actual[..2], expected[..2], "C3 class {}", case.label);
		assert_eq!(
			actual[3..],
			expected[3..],
			"C3 exact message/value {}",
			case.label
		);
		assert_eq!(hex(&run.stdout), row[4], "C3 stdout {}", case.label);
		assert_eq!(hex(&run.stderr), row[5], "C3 stderr {}", case.label);
		let site = if actual[0] == "error" {
			fs::read(scratch.0.join("result.site")).unwrap()
		} else {
			Vec::new()
		};
		fs::write(out.join(format!("{id}.fresh.site")), &site).unwrap();
		assert_eq!(hex(&site), row[6], "C3 exact error site {}", case.label);
		equal += 1;
	}
	assert_eq!((equal, refused, full_budget), (252, 33, 63));
	fs::write(out.join("summary.txt"),"252 exact stock parity;33 declared refusals;63 full-fresh-budget source checks (no numeric budget parity)\n").unwrap();
	contract_completed("fresh", 348);
}

#[test]
#[ignore = "C3 2MiB compile-stack depth boundary; isolated bounded children, new GAP_OUT"]
fn pattern_depth_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c3-depth-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut summary = String::new();
	for depth in [32, 64, 120, 126, 127, 128] {
		let mut pattern = String::from("_");
		for _ in 0..depth {
			pattern = format!("({pattern},)");
		}
		let source = format!("pub fn main(){{let a=0;match a{{{pattern}=>11,_=>22}}}}");
		let id = format!("depth-{depth}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &source).unwrap();
		let run = launch(&path, &path, "stack-main", "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{id}: {}", run.record);
		let fields: Vec<_> = run.record.split('\t').collect();
		if depth <= 126 {
			assert_eq!(fields[0], "value", "{id}: {}", run.record);
			assert_eq!(unhex(fields[4]), "i64:22:22");
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
		summary.push_str(&format!("{depth}\t{}\t{}\n", fields[0], fields[1]));
	}
	fs::write(out.join("summary.tsv"), summary).unwrap();
}

#[path = "patterns/result_supplement.rs"]
mod result_supplement;
