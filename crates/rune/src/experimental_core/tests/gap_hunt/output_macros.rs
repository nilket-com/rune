//! C2 stock oracle: no production macro or native admission changes.
use super::*;
use crate as rune;

struct Case {
	label: String,
	source: String,
	mode: String,
}

#[derive(crate::Any)]
#[rune(item = ::c2)]
struct DebugFailure;

fn debug_failure(
	_: &DebugFailure,
	_: &mut crate::runtime::Formatter,
) -> Result<(), crate::runtime::VmError> {
	Err(crate::runtime::VmError::panic("c2 debug failure"))
}

std::thread_local! {
	static SEED: std::cell::RefCell<Option<crate::runtime::Value>> = const {std::cell::RefCell::new(None)};
}

fn cases() -> Vec<Case> {
	let mut out = Vec::new();
	let mut add = |label: &str, body: &str| {
		out.push(Case {
			label: label.into(),
			source: body.into(),
			mode: "c2-default".into(),
		})
	};
	for macro_ in ["panic", "print"] {
		for (name, args) in [
			("empty", ""),
			("literal", "\"hello α\""),
			("trailing", "\"hello\","),
			("display", "\"{}\", 7"),
			("debug", "\"{:?}\", [1,2]"),
			("named", "\"{x}\", x=7"),
			("escaped", "\"{{}}\\n\\\"\""),
			("width", "\"{:04}\", 7"),
			("hex", "\"{:x}\", 255"),
			("precision", "\"{:.2}\", 1.25"),
			("nonliteral", "message"),
		] {
			add(
				&format!("forms/{macro_}/{name}"),
				&format!("pub fn main() {{ let message=\"hello\"; {macro_}!({args}); 42 }}"),
			);
		}
	}
	for (name, value) in [
		("unit", "()"),
		("bool", "true"),
		("signed", "-7"),
		("unsigned", "7u64"),
		("float", "1.25"),
		("char", "'α'"),
		("string", "\"α\\n\\t\\\"\\\\\""),
		("vec", "[1,2]"),
		("tuple", "(1,\"x\")"),
		("object", "#{x:1}"),
		("struct", "P{x:1}"),
		("some", "Some(7)"),
		("none", "None"),
		("ok", "Ok(7)"),
		("err", "Err(\"bad\")"),
		("nested", "[Some(#{x:(1,2)})]"),
		("function", "f"),
		("range", "1..3"),
		("range-inclusive", "1..=3"),
	] {
		add(
			&format!("dbg/value/{name}"),
			&format!("struct P{{x}} fn f(){{7}} pub fn main() {{ dbg!({value}); 42 }}"),
		);
	}
	for (label, source) in [
		("dbg/empty", "pub fn main() { dbg!(); 42 }"),
		("dbg/trailing", "pub fn main() { dbg!(1,2,); 42 }"),
		("dbg/return-unit", "pub fn main() { let x=dbg!(1); x }"),
		("dbg/direct", "pub fn main() { ::std::io::dbg(1,2); 42 }"),
		("dbg/alias", "use std::io::dbg as d; pub fn main() { d!(1); 42 }"),
		("print/alias", "use std::io::print as p; pub fn main() { p!(\"x\"); 42 }"),
		("panic/alias", "use std::panic as p; pub fn main() { p!(\"x\"); 42 }"),
		("print/direct", "pub fn main() { ::std::io::print(\"x\"); 42 }"),
		("panic/direct", "pub fn main() { ::std::panic(\"x\"); 42 }"),
		("interleave", "pub fn main() { print!(\"a\"); dbg!(1,2); println!(\"b\"); print!(\"c\"); 42 }"),
		("order/left-to-right", "fn mark(n) { print!(\"{}\",n); n } pub fn main() { dbg!(mark(1),mark(2)); 42 }"),
		("order/alias-mutation", "pub fn main() { let a=[1]; dbg!(a,{a.push(2);3}); a }"),
		("order/owned-once", "fn mark(a,n) { a.push(n); [n] } pub fn main() { let a=[]; dbg!(mark(a,1),mark(a,2)); a }"),
		("error/first-argument", "pub fn main() { dbg!(1/0,{println!(\"late\");2}); 42 }"),
		("error/later-argument", "pub fn main() { dbg!({print!(\"early\");1},1/0); 42 }"),
		("error/panic-argument", "pub fn main() { dbg!(1,{panic!(\"nested\");2}); 42 }"),
		("error/print-argument", "pub fn main() { print!(\"{}\",1/0); 42 }"),
		("error/panic-argument-format", "pub fn main() { panic!(\"{}\",1/0); 42 }"),
		("panic/before-output", "pub fn main() { print!(\"before\"); panic!(\"stop\"); 42 }"),
		("panic/branch-not-taken", "pub fn main() { if false { panic!(\"never\"); } 42 }"),
		("panic/branch-taken", "pub fn main() { if true { panic!(\"taken\"); } 42 }"),
		("panic/nested-call", "fn inner(){panic!(\"nested\")} fn outer(){inner()} pub fn main(){outer()}"),
		("test/dbg", "#[test] fn main() { dbg!(1); 42 }"),
		("test/print", "#[test] fn main() { print!(\"x\"); 42 }"),
		("test/panic", "#[test] fn main() { panic!(\"x\"); }"),
		("source-shadow/dbg", "mod std { pub mod io { pub fn dbg(a) { a } } } pub fn main() { std::io::dbg(7) }"),
		("source-shadow/print", "mod std { pub mod io { pub fn print(a) { a } } } pub fn main() { std::io::print(7) }"),
		("source-shadow/panic", "mod std { pub fn panic(a) { a } } pub fn main() { std::panic(7) }"),
		("closed/string", "pub fn main() { let a=\"x\"; let b=a; drop(a); dbg!(b); 42 }"),
		("moved/option", "pub fn main() { let a=Some([1]); let b=a; a.unwrap(); dbg!(b); 42 }"),
		("lease/vec", "pub fn main() { let a=[1]; let b=a; a.iter().map(|x| {dbg!(b);x}).collect::<Vec>() }"),
	] { add(label, source); }
	for depth in [31, 32, 33, 62, 63, 64, 65] {
		for (name, wrap) in [
			("vec", "[a]"),
			("tuple", "(a,)"),
			("object", "#{x:a}"),
			("option", "Some(a)"),
			("result", "Ok(a)"),
		] {
			add(
				&format!("depth/{name}/{depth}"),
				&format!(
					"pub fn main() {{ let a=1; for _ in 0..{depth} {{ a={wrap}; }} dbg!(a); 42 }}"
				),
			);
		}
	}
	add(
		"long/unicode",
		&format!(
			"pub fn main() {{ print!(\"{}\"); dbg!(\"{}\"); panic!(\"{}\") }}",
			"α".repeat(1024),
			"β".repeat(1024),
			"γ".repeat(1024)
		),
	);
	for mode in ["c2-capture", "c2-disable", "c2-none"] {
		for (macro_, args) in [("dbg", "1,2"), ("print", "\"x\""), ("panic", "\"stop\"")] {
			out.push(Case {
				label: format!("context/{mode}/{macro_}"),
				source: format!("pub fn main() {{ {macro_}!({args}); 42 }}"),
				mode: mode.into(),
			});
		}
	}
	for mode in ["c2-nan", "c2-inf", "c2-neg-inf"] {
		out.push(Case {
			label: format!("host-float/{mode}"),
			source: "pub fn main(a) { dbg!(a); 42 }".into(),
			mode: mode.into(),
		});
	}
	for mode in [
		"c2-lease-string",
		"c2-closed-string",
		"c2-moved-string",
		"c2-custom-debug",
	] {
		out.push(Case {
			label: format!("host-fixture/{mode}"),
			source: "pub fn main() { dbg!(::c2::seed()); 42 }".into(),
			mode: mode.into(),
		});
	}
	for fuel in 0..=80 {
		for (name, call) in [
			("dbg", "dbg!(mark(1),mark(2))"),
			("print", "print!(\"{}{}\",mark(1),mark(2))"),
			("panic", "panic!(\"{}{}\",mark(1),mark(2))"),
		] {
			out.push(Case {
				label: format!("fuel/{name}/{fuel}"),
				source: format!(
					"fn mark(n) {{ println!(\"{{}}\",n); n }} pub fn main() {{ {call}; 42 }}"
				),
				mode: format!("c2-fuel-{fuel}"),
			});
		}
	}
	out
}

pub(super) fn observe_c2(path: &Path, mode: &str, engine: &str) -> Observation {
	let mut context =
		Context::with_config(!matches!(mode, "c2-capture" | "c2-disable" | "c2-none")).unwrap();
	#[cfg(feature = "capture-io")]
	let capture = if mode == "c2-capture" {
		let io = crate::modules::capture_io::CaptureIo::new();
		context
			.install(crate::modules::capture_io::module(&io).unwrap())
			.unwrap();
		Some(io)
	} else {
		None
	};
	#[cfg(not(feature = "capture-io"))]
	assert_ne!(mode, "c2-capture", "C2 oracle requires capture-io");
	#[cfg(feature = "disable-io")]
	if mode == "c2-disable" {
		context
			.install(crate::modules::disable_io::module().unwrap())
			.unwrap();
	}
	#[cfg(not(feature = "disable-io"))]
	assert_ne!(mode, "c2-disable", "C2 oracle requires disable-io");
	let input = match mode {
		"c2-nan" => Some(f64::NAN),
		"c2-inf" => Some(f64::INFINITY),
		"c2-neg-inf" => Some(f64::NEG_INFINITY),
		_ => None,
	};
	let seed =
		crate::runtime::Value::try_from(crate::alloc::String::try_from("held").unwrap()).unwrap();
	let lease = if mode == "c2-lease-string" {
		Some(seed.borrow_mut::<crate::alloc::String>().unwrap())
	} else {
		None
	};
	if mode == "c2-closed-string" {
		crate::runtime::Value::drop(seed.clone()).unwrap();
	}
	let moved = if mode == "c2-moved-string" {
		Some(seed.clone().move_().unwrap())
	} else {
		None
	};
	if matches!(
		mode,
		"c2-lease-string" | "c2-closed-string" | "c2-moved-string" | "c2-custom-debug"
	) {
		let mut module = crate::Module::with_crate("c2").unwrap();
		let value = if mode == "c2-custom-debug" {
			module.ty::<DebugFailure>().unwrap();
			module
				.associated_function(&crate::runtime::Protocol::DEBUG_FMT, debug_failure)
				.unwrap();
			crate::runtime::Value::new(DebugFailure).unwrap()
		} else {
			seed.clone()
		};
		SEED.with(|seed| *seed.borrow_mut() = Some(value));
		module
			.function("seed", || {
				SEED.with(|seed| seed.borrow().as_ref().unwrap().clone())
			})
			.build()
			.unwrap();
		context.install(module).unwrap();
	}
	let fuel = mode
		.strip_prefix("c2-fuel-")
		.map(|n| n.parse().unwrap())
		.unwrap_or(FUEL);
	let observe_mode = if mode == "c2-depth64" {
		"output-main-depth64"
	} else {
		"output-main"
	};
	let observed = observe_with_context(path, observe_mode, engine, &context, input, fuel);
	SEED.with(|seed| seed.borrow_mut().take());
	drop(lease);
	drop(moved);
	#[cfg(feature = "capture-io")]
	if let Some(io) = capture {
		let result = PathBuf::from(std::env::var_os("GAP_RESULT").unwrap());
		fs::write(result.with_extension("capture"), io.drain()).unwrap();
	}
	observed
}

#[test]
#[ignore = "C2 stock oracle, explicit new GAP_OUT; capture-io and disable-io required"]
#[allow(clippy::assertions_on_constants)]
fn stock_probe() {
	// Runtime admission for an ignored oracle: ordinary feature configurations
	// must compile, while an explicit invocation without the two contexts fails.
	assert!(
		cfg!(all(feature = "capture-io", feature = "disable-io")),
		"both IO features required"
	);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT required"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c2-oracle-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let cases = cases();
	let mut table=String::from("case\tmode\tsource_hex\tstock_observation_hex\tstdout_hex\tstderr_hex\tstock_site\tcaptured_hex\tpolicy\n");
	for (n, case) in cases.iter().enumerate() {
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		for extension in ["site", "capture"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{extension}")));
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
			fs::read_to_string(scratch.0.join("result.site")).unwrap()
		} else {
			String::new()
		};
		let captured = fs::read(scratch.0.join("result.capture")).unwrap_or_default();
		let address = String::from_utf8_lossy(&run.stdout).contains("object at 0x")
			|| String::from_utf8_lossy(&captured).contains("object at 0x");
		let policy = if address {
			"Observation(address-fallback)"
		} else {
			"Exact"
		};
		table.push_str(&format!(
			"{}\t{}\t{}\t{}\t{}\t{}\t{site}\t{}\t{policy}\n",
			case.label,
			case.mode,
			hex(case.source.as_bytes()),
			hex(run.record.as_bytes()),
			hex(&run.stdout),
			hex(&run.stderr),
			hex(&captured)
		));
		fs::write(out.join("stock-output-macros.tsv"), &table).unwrap();
	}
	fs::write(
		out.join("summary.txt"),
		format!(
			"{} stock C2 cases complete; exact rows and explicit address observations\n",
			cases.len()
		),
	)
	.unwrap();
	contract_completed("stock", cases.len());
}

// These are explicitly observation rows, never exact-address equivalence.
// Extract one address field only on a row already frozen with that policy;
// compare every other byte and retain both original streams unmodified.
fn address_fields(bytes: &[u8]) -> (&[u8], &[u8]) {
	let marker = b"object at 0x";
	let start = bytes
		.windows(marker.len())
		.position(|w| w == marker)
		.unwrap()
		+ marker.len();
	let end = start
		+ bytes[start..]
			.iter()
			.take_while(|b| b.is_ascii_hexdigit())
			.count();
	assert!(end > start);
	assert!(!bytes[end..].windows(marker.len()).any(|w| w == marker));
	(&bytes[..start], &bytes[end..])
}

#[test]
#[ignore = "C2 frozen stock replay, explicit new GAP_OUT; both IO features required"]
fn stock_contract() {
	stock_probe();
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	let actual = fs::read_to_string(out.join("stock-output-macros.tsv")).unwrap();
	let expected = include_str!("../output_macros_stock.tsv");
	let actual: Vec<_> = actual.lines().skip(1).collect();
	let expected: Vec<_> = expected.lines().skip(1).collect();
	assert_eq!(actual.len(), 367);
	assert_eq!(actual.len(), expected.len());
	let mut exact = 0;
	let mut observations = 0;
	for (current, frozen) in actual.iter().zip(&expected) {
		let a: Vec<_> = current.split('\t').collect();
		let b: Vec<_> = frozen.split('\t').collect();
		assert_eq!(a.len(), 9);
		assert_eq!(b.len(), 9);
		if b[8] == "Exact" {
			assert_eq!(current, frozen, "{}", b[0]);
			exact += 1;
		} else {
			assert_eq!(b[8], "Observation(address-fallback)");
			for i in [0, 1, 2, 3, 5, 6, 7, 8] {
				assert_eq!(a[i], b[i], "{} column {i}", b[0]);
			}
			let a = unhex(a[4]);
			let b = unhex(b[4]);
			assert_eq!(address_fields(a.as_bytes()), address_fields(b.as_bytes()));
			observations += 1;
		}
	}
	assert_eq!((exact, observations), (351, 16));
	#[cfg(target_os = "linux")]
	{
		crate::modules::io::stdout_tests::stock_c2_at(&out.join("descriptors"));
		assert_eq!(
			fs::read_to_string(out.join("descriptors/stock-output-descriptors.tsv")).unwrap(),
			include_str!("../output_descriptors_stock.tsv")
		);
	}
	println!(
		"C2 STOCK EXACT {exact}; ADDRESS OBSERVATIONS {observations}; Linux descriptor rows 7"
	);
}

fn observation(record: &str) -> (&str, &str, String, String) {
	let fields: Vec<_> = record.split('\t').collect();
	assert_eq!(fields.len(), 5);
	(fields[0], fields[1], unhex(fields[3]), unhex(fields[4]))
}

#[test]
#[ignore = "C2 fresh contract with explicit retained/refusal/fuel policies; new GAP_OUT"]
fn fresh_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c2-fresh-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let expected: Vec<_> = include_str!("../output_macros_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	let cases = cases();
	assert_eq!(expected.len(), cases.len());
	let mut policies = String::from("case\tdisposition\n");
	let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
	for (n, (case, row)) in cases.iter().zip(&expected).enumerate() {
		let columns: Vec<_> = row.split('\t').collect();
		assert_eq!(columns.len(), 9);
		assert_eq!(case.label, columns[0]);
		assert_eq!(hex(case.source.as_bytes()), columns[2]);
		assert_eq!(case.mode, columns[1]);
		let path = out.join(format!("{n:05}.source.rn"));
		fs::write(&path, &case.source).unwrap();
		for extension in ["site", "capture"] {
			let _ = fs::remove_file(scratch.0.join(format!("result.{extension}")));
		}
		let run = launch(
			&path,
			&path,
			&case.mode,
			"fresh",
			&scratch.0,
			&out,
			&format!("{n:05}"),
		);
		assert!(run.complete, "{} {}", case.label, run.record);
		let actual = observation(&run.record);
		let frozen_record = unhex(columns[3]);
		let stock = observation(&frozen_record);
		let captured = fs::read(scratch.0.join("result.capture")).unwrap_or_default();
		assert!(run.stderr.is_empty());
		let site = fs::read_to_string(scratch.0.join("result.site")).unwrap_or_default();
		let no_output = |run: &Run| {
			assert_eq!(run.stdout, b"\nrunning 1 test\n");
		};
		let disposition;
		if case.source.contains("dbg") && !case.label.starts_with("source-shadow/") {
			disposition = "dbg deferred: compiler or native refusal";
			assert_eq!(actual.0, "refused", "{}", case.label);
			assert!(matches!(
				actual.1,
				"Some(FrontendDiagnostic)" | "Some(Native)"
			));
			no_output(&run);
		} else if case.label.starts_with("fuel/print/") {
			disposition = "fresh print fuel invariants (not VM fuel parity)";
			let fuel: u32 = case.mode.strip_prefix("c2-fuel-").unwrap().parse().unwrap();
			assert!(matches!(actual.0, "value" | "error"));
			if actual.0 == "error" {
				assert_eq!(actual.1, "FuelExhausted");
				assert!(fuel < 80);
				assert!(!site.is_empty());
				assert!([
					&b"\nrunning 1 test\n"[..],
					&b"\nrunning 1 test\n1\n"[..],
					&b"\nrunning 1 test\n1\n2\n"[..],
					&b"\nrunning 1 test\n1\n2\n12"[..]
				]
				.contains(&run.stdout.as_slice()));
			} else {
				let high = expected
					.iter()
					.find(|r| r.starts_with("fuel/print/80\t"))
					.unwrap()
					.split('\t')
					.nth(3)
					.unwrap();
				let high_record = unhex(high);
				let high_obs = observation(&high_record);
				assert_eq!((&actual.2, &actual.3), (&high_obs.2, &high_obs.3));
				assert_eq!(
					hex(&run.stdout),
					expected
						.iter()
						.find(|r| r.starts_with("fuel/print/80\t"))
						.unwrap()
						.split('\t')
						.nth(4)
						.unwrap()
				);
			}
		} else if case.label.starts_with("fuel/") {
			disposition = "fresh fuel invariants (not VM fuel parity)";
			let fuel: u32 = case.mode.strip_prefix("c2-fuel-").unwrap().parse().unwrap();
			if fuel < 3 {
				assert_eq!((actual.0, actual.1), ("error", "FuelExhausted"));
				let (start, len) = if fuel == 0 {
					(case.source.find("mark(1)").unwrap() + 5, 1)
				} else {
					(case.source.find("println!(\"{}\",n)").unwrap(), 16)
				};
				assert_eq!(site, format!("0:{start}:{}", start + len));
				assert_eq!(
					run.stdout,
					if fuel == 2 {
						&b"\nrunning 1 test\n1\n"[..]
					} else {
						&b"\nrunning 1 test\n"[..]
					}
				);
			} else {
				let name = case.label.split('/').nth(1).unwrap();
				let high = expected
					.iter()
					.find(|r| r.starts_with(&format!("fuel/{name}/80\t")))
					.unwrap();
				let high: Vec<_> = high.split('\t').collect();
				let high_record = unhex(high[3]);
				let high_obs = observation(&high_record);
				assert_eq!(actual.0, high_obs.0);
				assert_eq!((&actual.2, &actual.3), (&high_obs.2, &high_obs.3));
				assert_eq!(hex(&run.stdout), high[4]);
				if name == "panic" {
					assert_eq!(actual.1, "Panic");
					assert_eq!(site, high[6]);
				} else {
					assert_eq!(actual.1, "");
				}
			}
		} else if stock.0 == "compile-error" {
			disposition = "existing compiler refusal";
			assert_eq!(
				(actual.0, actual.1),
				("refused", "Some(FrontendDiagnostic)")
			);
			no_output(&run);
		} else if case.label.starts_with("forms/") && case.label.ends_with("/hex") {
			disposition = "existing format-spec refusal";
			assert_eq!((actual.0, actual.1), ("refused", "Some(FormatSpec)"));
			no_output(&run);
		} else if case.label.starts_with("host-fixture/")
			|| (case.label.starts_with("context/") && !case.label.ends_with("/panic"))
		{
			disposition = "uncertified IO or host callee refusal";
			assert_eq!((actual.0, actual.1), ("refused", "Some(Native)"));
			no_output(&run);
		} else {
			disposition = "exact admitted behaviour";
			assert_eq!(actual.0, stock.0, "{}", case.label);
			if stock.1.starts_with("stock:Panic ") {
				assert_eq!(actual.1, "Panic");
			} else {
				assert_eq!(actual.1, stock.1, "{}", case.label);
			}
			assert_eq!(
				(&actual.2, &actual.3),
				(&stock.2, &stock.3),
				"{}",
				case.label
			);
			assert_eq!(hex(&run.stdout), columns[4], "{} stdout", case.label);
			assert_eq!(hex(&run.stderr), columns[5]);
			assert_eq!(site, columns[6], "{} site", case.label);
			assert_eq!(hex(&captured), columns[7]);
		}
		assert!(captured.is_empty());
		*counts.entry(disposition).or_default() += 1;
		policies.push_str(&format!("{}\t{disposition}\n", case.label));
		fs::write(out.join("dispositions.tsv"), &policies).unwrap();
	}
	assert_eq!(
		counts.get("fresh fuel invariants (not VM fuel parity)"),
		Some(&81)
	);
	assert_eq!(
		counts.get("fresh print fuel invariants (not VM fuel parity)"),
		Some(&81)
	);
	fs::write(
		out.join("summary.txt"),
		format!("{} C2 fresh rows; {counts:?}\n", cases.len()),
	)
	.unwrap();
	direct_print_supplement(&out.join("direct-print"));
	#[cfg(target_os = "linux")]
	crate::modules::io::stdout_tests::fresh_c2_at(&out.join("descriptors"));
	contract_completed("fresh", cases.len());
}

fn direct_print_supplement(out: &Path) {
	fs::create_dir(out).unwrap();
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-c2-direct-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	for (n, (args, arity)) in [
		("", true),
		("1", false),
		("()", false),
		("[1]", false),
		("\"a\",\"b\"", true),
		("\"α\"", false),
	]
	.into_iter()
	.enumerate()
	{
		let path = out.join(format!("{n}.source.rn"));
		fs::write(
			&path,
			format!("pub fn main() {{ ::std::io::print({args}); 42 }}"),
		)
		.unwrap();
		let stock = launch(
			&path,
			&path,
			"c2-default",
			"stock",
			&scratch.0,
			out,
			&format!("{n}-stock"),
		);
		let fresh = launch(
			&path,
			&path,
			"c2-default",
			"fresh",
			&scratch.0,
			out,
			&format!("{n}-fresh"),
		);
		assert!(stock.complete && fresh.complete);
		let a = observation(&stock.record);
		let b = observation(&fresh.record);
		assert!(stock.stderr.is_empty() && fresh.stderr.is_empty());
		if arity {
			assert_eq!(a.0, "error");
		}
		if arity {
			assert_eq!((b.0, b.1), ("refused", "Some(Arity)"));
			assert_eq!(fresh.stdout, b"\nrunning 1 test\n");
		} else {
			assert_eq!(b.0, a.0);
			assert_eq!((&b.2, &b.3), (&a.2, &a.3));
			assert_eq!(fresh.stdout, stock.stdout);
			if a.0 == "error" {
				assert_eq!(b.1, "Type");
			}
		}
	}
}

#[test]
#[ignore = "C2 both-engine frozen contract, both IO features; new GAP_OUT"]
fn contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	for (engine, test) in [("stock", "stock_contract"), ("fresh", "fresh_contract")] {
		let result = Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				&format!("experimental_core::tests::gap_hunt::output_macros::{test}"),
				"--ignored",
				"--nocapture",
			])
			.env("GAP_OUT", out.join(engine))
			.output()
			.unwrap();
		fs::write(out.join(format!("{engine}.stdout")), &result.stdout).unwrap();
		fs::write(out.join(format!("{engine}.stderr")), &result.stderr).unwrap();
		assert!(
			result.status.success(),
			"{engine}: {}",
			String::from_utf8_lossy(&result.stdout)
		);
		assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed; 0 failed"));
	}
	contract_completed("both", 367);
}
