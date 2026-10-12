//! S0: local differential inventory, never an automatic routing or semantics change.
//! Run `gap_hunt::corpus` with --ignored --exact, GAP_EXTERNAL and GAP_OUT set.
//! Every subject/frontend runs in a separate child: 2M fuel, 10 seconds, 128KiB
//! per output stream. Corpus roles are fixed in gap_corpus.tsv; external roles
//! are inventoried explicitly at runtime. Missing host modules are coverage gaps.
//! Debug is the bounded value observation, not a proof of arbitrary value equality.
//! No ordering normalization is performed. Module loading differs: fresh refuses
//! filesystem module loading. Support/no-main files are compiled but not invoked.
use super::super::{
	execute::Executor,
	frontend,
	heap::{Limits, Store},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::{
	borrow::ToOwned,
	collections::BTreeMap,
	format, fs,
	io::{Read, Write},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	string::{String, ToString},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
	time::{Duration, Instant},
	vec,
	vec::Vec,
};

const MANIFEST: &str = include_str!("gap_corpus.tsv");
const CHILD: &str = "experimental_core::tests::gap_hunt::child";
const FUEL: usize = 2_000_000;
const CAP: usize = 128 * 1024;
const WALL: Duration = Duration::from_secs(10);

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
	phase: String,
	class: String,
	site: String,
	message: String,
	value: String,
}
impl Observation {
	fn new(phase: &str, class: &str, site: &str, message: String, value: String) -> Self {
		Self {
			phase: phase.into(),
			class: class.into(),
			site: site.into(),
			message,
			value,
		}
	}
	fn encode(&self) -> String {
		format!(
			"{}\t{}\t{}\t{}\t{}",
			self.phase,
			self.class,
			hex(self.site.as_bytes()),
			hex(self.message.as_bytes()),
			hex(self.value.as_bytes())
		)
	}
}

fn sources(path: &Path, mode: &str) -> Sources {
	let text = fs::read_to_string(path).unwrap();
	assert!(text.len() <= 2 * 1024 * 1024, "source size");
	let text = if mode == "script" {
		format!("pub fn main() {{\n{text}\n}}")
	} else {
		text
	};
	let mut sources = Sources::new();
	sources
		.insert(
			Source::with_path(
				"gap.rn",
				&text,
				std::env::var_os("GAP_ORIGIN").unwrap_or_else(|| path.as_os_str().to_owned()),
			)
			.unwrap(),
		)
		.unwrap();
	sources
}

fn stock_class(k: &crate::runtime::VmErrorKind) -> String {
	use crate::runtime::VmErrorKind as K;
	let class = match k {
		K::Overflow | K::Underflow => "Overflow",
		K::DivideByZero => "DivisionByZero",
		K::Halted {
			halt: crate::runtime::VmHaltInfo::Limited,
		} => "FuelExhausted",
		K::MissingField { .. } | K::MissingIndexInteger { .. } | K::OutOfRange { .. } => {
			"IndexOutOfBounds"
		}
		K::UnsupportedBinaryOperation { .. }
		| K::UnsupportedUnaryOperation { .. }
		| K::UnsupportedIndexGet { .. }
		| K::UnsupportedIndexSet { .. }
		| K::ExpectedNumber { .. }
		| K::ExpectedType { .. }
		| K::MissingInstanceFunction { .. }
		| K::MissingProtocolFunction { .. } => "Type",
		_ => return format!("stock:{k:?}"),
	};
	class.into()
}

fn stock_scalar(value: &crate::runtime::Value) -> String {
	use crate::runtime::Inline;
	match value.as_inline() {
		Some(Inline::Signed(n)) => format!("i64:{n}:"),
		Some(Inline::Unsigned(n)) => format!("u64:{n}:"),
		Some(Inline::Float(n)) => format!("f64:{:016x}:", n.to_bits()),
		Some(Inline::Bool(n)) => format!("bool:{n}:"),
		Some(Inline::Char(n)) => format!("char:{}:", *n as u32),
		_ => String::new(),
	}
}

fn fresh_scalar(store: &Store, root: &super::super::value::Root) -> String {
	use super::super::value::Scalar;
	match store.root_scalar(root).unwrap() {
		Some(Scalar::I64(n)) => format!("i64:{n}:"),
		Some(Scalar::U64(n)) => format!("u64:{n}:"),
		Some(Scalar::F64(n)) => format!("f64:{:016x}:", n.to_bits()),
		Some(Scalar::Bool(n)) => format!("bool:{n}:"),
		Some(Scalar::Char(n)) => format!("char:{}:", n as u32),
		_ => String::new(),
	}
}

fn observe(path: &Path, mode: &str, engine: &str) -> Observation {
	let context = Context::with_default_modules().unwrap();
	let mut sources = sources(path, mode);
	let mut diagnostics = Diagnostics::new();
	if engine == "stock" {
		let vm = crate::prepare(&mut sources)
			.with_context(&context)
			.with_diagnostics(&mut diagnostics)
			.build_vm();
		let mut vm = match vm {
			Ok(vm) => vm,
			Err(e) => {
				return Observation::new(
					"compile-error",
					"FrontendDiagnostic",
					"stock compiler",
					format!("{e:?}: {:?}", diagnostics.diagnostics()),
					String::new(),
				)
			}
		};
		if mode == "compile" {
			return Observation::new("compiled", "", "", String::new(), String::new());
		}
		let entry = mode.strip_prefix("test:").unwrap_or("main");
		let result = crate::runtime::budget::with(FUEL, || vm.call(entry_hash(entry), ())).call();
		return match result {
			Ok(value) => {
				let mut text = crate::alloc::String::new();
				let formatted = vm.with(|| {
					crate::runtime::Formatter::format_with(&mut text, |f| value.debug_fmt(f))
				});
				match formatted {
					Ok(()) => Observation::new(
						"value",
						"",
						"",
						String::new(),
						format!("{}{text}", stock_scalar(&value)),
					),
					Err(e) => Observation::new(
						"observation-error",
						"Debug",
						"stock Debug",
						e.to_string(),
						String::new(),
					),
				}
			}
			Err(e) => {
				let k = e.into_kind();
				Observation::new(
					"error",
					&stock_class(&k),
					"stock runtime",
					k.to_string(),
					String::new(),
				)
			}
		};
	}
	assert_eq!(engine, "fresh");
	let program = match frontend::compile(&mut sources, &context, &mut diagnostics, 1) {
		Ok(p) => p,
		Err(e) => {
			return Observation::new(
				"refused",
				&format!("{:?}", e.refusal),
				e.operation,
				format!("{e:?}: {:?}", diagnostics.diagnostics()),
				String::new(),
			)
		}
	};
	if mode == "compile" {
		return Observation::new("compiled", "", "", String::new(), String::new());
	}
	let entry = mode.strip_prefix("test:").unwrap_or("main");
	let Some(main) = program.function(entry_hash(entry)) else {
		return Observation::new(
			"entry-gap",
			"no-main",
			"entry",
			String::new(),
			String::new(),
		);
	};
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	match executor.invoke(&program, &mut store, main, &[], FUEL) {
		Ok(value) => {
			let observed = match store.root_format(&value, true) {
				Ok(text) => Observation::new(
					"value",
					"",
					"",
					String::new(),
					format!("{}{text}", fresh_scalar(&store, &value)),
				),
				Err(e) => Observation::new(
					"observation-error",
					"Debug",
					e.operation,
					e.message(&program).to_string(),
					String::new(),
				),
			};
			store.root_release(value).unwrap();
			assert!(executor.is_idle());
			observed
		}
		Err(f) => Observation::new(
			"error",
			&format!("{:?}", f.error.kind),
			f.error.operation,
			f.error.message(&program).to_string(),
			String::new(),
		),
	}
}

#[test]
#[ignore = "S0 child entry, launched only by the bounded parent"]
fn child() {
	let path = std::env::var_os("GAP_CASE").expect("child requires GAP_CASE");
	let mode = std::env::var("GAP_MODE").unwrap();
	let engine = std::env::var("GAP_ENGINE").unwrap();
	if mode == "timeout-control" {
		std::thread::sleep(Duration::from_secs(60));
	}
	let observation = observe(Path::new(&path), &mode, &engine);
	fs::write(
		std::env::var_os("GAP_RESULT").unwrap(),
		observation.encode(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	// Avoid libtest's elapsed-time footer contaminating the subject output.
	std::process::exit(0);
}

struct Scratch(PathBuf);
impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}

fn drain(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut saved = Vec::new();
	let mut buf = [0; 4096];
	loop {
		let n = pipe.read(&mut buf).unwrap();
		if n == 0 {
			break;
		}
		let room = CAP - saved.len();
		saved.extend_from_slice(&buf[..n.min(room)]);
		if n > room {
			exceeded.store(true, Ordering::Relaxed);
		}
	}
	saved
}

struct Run {
	record: String,
	stdout: Vec<u8>,
	stderr: Vec<u8>,
	complete: bool,
}
fn launch(
	path: &Path,
	origin: &Path,
	mode: &str,
	engine: &str,
	scratch: &Path,
	out: &Path,
	id: &str,
) -> Run {
	let result = scratch.join("result");
	let _ = fs::remove_file(&result);
	let mut child = Command::new(std::env::current_exe().unwrap())
		.args(["--exact", CHILD, "--ignored", "--nocapture", "--quiet"])
		.env("GAP_CASE", path)
		.env("GAP_ORIGIN", origin)
		.env("GAP_MODE", mode)
		.env("GAP_ENGINE", engine)
		.env("GAP_RESULT", &result)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let exceeded = Arc::new(AtomicBool::new(false));
	let flag = exceeded.clone();
	let pipe = child.stdout.take().unwrap();
	let stdout = std::thread::spawn(move || drain(pipe, flag));
	let flag = exceeded.clone();
	let pipe = child.stderr.take().unwrap();
	let stderr = std::thread::spawn(move || drain(pipe, flag));
	let start = Instant::now();
	let mut stopped = "";
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break status;
		}
		if exceeded.load(Ordering::Relaxed) || start.elapsed() >= WALL {
			stopped = if exceeded.load(Ordering::Relaxed) {
				"output-cap"
			} else {
				"timeout"
			};
			child.kill().unwrap();
			break child.wait().unwrap();
		}
		std::thread::sleep(Duration::from_millis(10));
	};
	let stdout = stdout.join().unwrap();
	let stderr = stderr.join().unwrap();
	let complete = stopped.is_empty()
		&& status.success()
		&& result.is_file()
		&& !exceeded.load(Ordering::Relaxed);
	let record = if complete {
		assert!(
			fs::metadata(&result).unwrap().len() <= CAP as u64,
			"result cap"
		);
		fs::read_to_string(&result).unwrap()
	} else {
		format!(
			"child-gap\t{stopped}\t\t{}\t",
			hex(format!("{status}").as_bytes())
		)
	};
	fs::write(out.join(format!("{id}.{engine}.out")), &stdout).unwrap();
	fs::write(out.join(format!("{id}.{engine}.err")), &stderr).unwrap();
	fs::write(out.join(format!("{id}.{engine}.result")), &record).unwrap();
	Run {
		record,
		stdout,
		stderr,
		complete,
	}
}

fn compare(stock: &Run, fresh: &Run) -> &'static str {
	if !stock.complete || !fresh.complete {
		return "coverage-child";
	}
	let s: Vec<_> = stock.record.split('\t').collect();
	let f: Vec<_> = fresh.record.split('\t').collect();
	assert_eq!(s.len(), 5);
	assert_eq!(f.len(), 5);
	if f[0] == "refused" {
		return "coverage-refusal";
	}
	if s[0] == "compile-error" {
		return "coverage-stock-compile";
	}
	if s[0] == "observation-error" || f[0] == "observation-error" {
		return "coverage-observation";
	}
	if s[0] == "compiled" && f[0] == "compiled" {
		return "coverage-no-entry";
	}
	if s[0] == f[0]
		&& s[1] == f[1]
		&& s[3..] == f[3..]
		&& stock.stdout == fresh.stdout
		&& stock.stderr == fresh.stderr
	{
		"equal"
	} else {
		"divergence"
	}
}

fn discover(dir: &Path, paths: &mut Vec<PathBuf>) {
	for entry in fs::read_dir(dir).unwrap() {
		let entry = entry.unwrap();
		let kind = entry.file_type().unwrap();
		if kind.is_symlink() {
			continue;
		}
		if kind.is_dir() {
			if matches!(
				entry.file_name().to_str(),
				Some(".git" | "target" | ".targets" | ".worktrees" | "node_modules")
			) {
				continue;
			}
			discover(&entry.path(), paths);
		} else if entry.path().extension().is_some_and(|e| e == "rn") {
			paths.push(entry.path());
			assert!(paths.len() <= 10_000, "external corpus cap");
		}
	}
}

/// Fixed receiver × operation matrix. Results are inventory, not expectations:
/// unsupported/reverse divergences must remain visible until the S rows land.
fn matrix() -> Vec<(String, String)> {
	let receivers = [
		("scalar", "1"),
		("vec", "[1]"),
		("tuple", "(1,)"),
		("object", "#{k:1}"),
		("struct", "P{x:1}"),
		("option", "Some(1)"),
		("result", "Ok(1)"),
		("string", "\"a\""),
		("function", "g"),
	];
	let mut rows = Vec::new();
	for (name, value) in receivers {
		let mut ops = vec![
			("field-present".into(), "x.x = 2; x".into()),
			("field-missing".into(), "x.missing = 2; x".into()),
			(
				"iterate".into(),
				"let out = []; for v in x { out.push(v); } out".into(),
			),
			("len".into(), "x.len()".into()),
			("clone".into(), "x.clone()".into()),
		];
		for (index, key) in [
			("int", "0"),
			("string", "\"k\""),
			("missing", "\"missing\""),
			("other", "true"),
		] {
			for (op, body) in [
				("get", format!("x[{key}]")),
				("set", format!("x[{key}] = 2; x")),
				("compound", format!("x[{key}] += 1; x")),
			] {
				ops.push((format!("index-{index}-{op}"), body));
			}
		}
		for (label, op) in [
			("eq", "=="),
			("ne", "!="),
			("lt", "<"),
			("le", "<="),
			("gt", ">"),
			("ge", ">="),
		] {
			ops.push((label.into(), format!("x {op} {value}")));
		}
		for (op, body) in ops {
			rows.push((
				format!("{name}-{op}"),
				format!(
					"struct P {{ x }} fn g() {{ 1 }} pub fn main() {{ let x = {value}; {body} }}"
				),
			));
		}
	}
	rows
}

#[test]
#[ignore = "S0 explicit local corpus run; requires GAP_EXTERNAL and new GAP_OUT"]
fn corpus() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.parent()
		.unwrap();
	let external = PathBuf::from(std::env::var_os("GAP_EXTERNAL").expect("GAP_EXTERNAL required"));
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("GAP_OUT required"));
	fs::create_dir(&out).expect("GAP_OUT must be new");
	let scratch = Scratch(std::env::temp_dir().join(format!("rune-gap-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let mut cases = Vec::new();
	for line in MANIFEST.lines().skip(1) {
		let fields: Vec<_> = line.split('\t').collect();
		assert_eq!(fields.len(), 3);
		assert!(matches!(
			fields[0],
			"executable" | "compile-fail" | "support" | "no-main-example" | "tests"
		));
		cases.push((
			format!("fork/{}", fields[2]),
			fields[0].to_owned(),
			if fields[1] == "test-fns" {
				"compile".into()
			} else {
				fields[1].to_owned()
			},
			root.join(fields[2]),
		));
	}
	let mut external_paths = Vec::new();
	discover(&external, &mut external_paths);
	external_paths.sort();
	for path in external_paths {
		let text = fs::read_to_string(&path).unwrap();
		let (role, mode) = if text.contains("fn main(") {
			("executable", "main")
		} else {
			("no-main-example", "compile")
		};
		cases.push((
			format!(
				"external/{}",
				path.strip_prefix(&external).unwrap().display()
			),
			role.into(),
			mode.into(),
			path,
		));
	}
	let file_cases = cases.clone();
	let mut test_manifest = String::from("file\tentry\targs\n");
	for (label, _, _, path) in file_cases {
		let text = fs::read_to_string(&path).unwrap();
		if !text.contains("#[test]") {
			continue;
		}
		let file =
			crate::parse::parse_all::<crate::ast::File>(&text, crate::SourceId::EMPTY, false);
		match file {
			Ok(file) => {
				let mut tests = Vec::new();
				test_items(&file, &text, "", &mut tests);
				for (entry, args) in tests {
					test_manifest.push_str(&format!("{label}\t{entry}\t{args}\n"));
					cases.push((
						format!("{label}::{entry}"),
						"test-function".into(),
						format!("test:{entry}"),
						path.clone(),
					));
				}
			}
			Err(error) => {
				test_manifest.push_str(&format!("{label}\tDISCOVERY_ERROR:{error:?}\t-\n"));
			}
		}
	}
	fs::write(out.join("test-functions.tsv"), test_manifest).unwrap();
	for (id, source) in matrix() {
		let path = scratch.0.join(format!("{id}.rn"));
		fs::write(&path, source).unwrap();
		cases.push((
			format!("matrix/{id}"),
			"executable".into(),
			"main".into(),
			path,
		));
	}
	let mut inventory = String::from("id\trole\tmode\tpath\n");
	let mut table = String::from("id\tcase\tclassification\tstock\tfresh\n");
	let mut counts = BTreeMap::new();
	let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
	for (n, (label, role, mode, path)) in cases.iter().enumerate() {
		let id = format!("{n:05}");
		inventory.push_str(&format!("{id}\t{role}\t{mode}\t{}\n", path.display()));
		let snapshot = out.join(format!("{id}.source.rn"));
		let input = fs::read(path).unwrap();
		assert!(input.len() <= 2 * 1024 * 1024, "source size");
		fs::write(&snapshot, input).unwrap();
		let stock = launch(&snapshot, path, mode, "stock", &scratch.0, &out, &id);
		let fresh = launch(&snapshot, path, mode, "fresh", &scratch.0, &out, &id);
		let class = compare(&stock, &fresh);
		*counts.entry(class).or_insert(0usize) += 1;
		if class == "divergence" {
			let site = fresh.record.split('\t').nth(2).unwrap().to_owned();
			groups.entry(site).or_default().push(label.clone());
		}
		table.push_str(&format!(
			"{id}\t{label}\t{class}\t{}\t{}\n",
			hex(stock.record.as_bytes()),
			hex(fresh.record.as_bytes())
		));
		fs::write(out.join("outcomes.tsv"), &table).unwrap();
	}
	fs::write(out.join("inventory.tsv"), inventory).unwrap();
	let summary = format!(
		"cases={}\ncounts={counts:#?}\ndivergences_by_fresh_site_hex={groups:#?}\n\
		Coverage: no-main/support files compiled only; each discovered Rune test has its own child; \
		Rust-doc-embedded Rune examples are not covered; external host modules absent; \
		typed scalars plus bounded Debug observation (heap equality not proven); \
		order is never normalized; manifest scripts wrapped identically on both engines; \
		2M fuel, 10s wall,128KiB streams.\n",
		cases.len()
	);
	fs::write(out.join("summary.txt"), summary).unwrap();
	println!(
		"S0 complete: {} cases, {counts:?}; {}",
		cases.len(),
		out.display()
	);
}

#[test]
#[ignore = "explicit S0 child-bound controls, including a 10-second timeout"]
fn controls() {
	let scratch =
		Scratch(std::env::temp_dir().join(format!("rune-gap-controls-{}", std::process::id())));
	fs::create_dir(&scratch.0).unwrap();
	let path = scratch.0.join("case.rn");
	fs::write(&path, "pub fn main() { println!(\"before\"); 42 }").unwrap();
	let stock = launch(
		&path, &path, "main", "stock", &scratch.0, &scratch.0, "good",
	);
	let fresh = launch(
		&path, &path, "main", "fresh", &scratch.0, &scratch.0, "good",
	);
	assert_eq!(compare(&stock, &fresh), "equal");
	assert!(stock.stdout.ends_with(b"before\n"));
	let changed = Run {
		record: fresh.record.clone(),
		stdout: b"different".to_vec(),
		stderr: fresh.stderr.clone(),
		complete: true,
	};
	assert_eq!(compare(&stock, &changed), "divergence");
	let changed = Run {
		record: fresh.record.clone(),
		stdout: fresh.stdout.clone(),
		stderr: b"different".to_vec(),
		complete: true,
	};
	assert_eq!(compare(&stock, &changed), "divergence");
	fs::write(&path, "pub fn main() { loop {} }").unwrap();
	for engine in ["stock", "fresh"] {
		let run = launch(&path, &path, "main", engine, &scratch.0, &scratch.0, "fuel");
		assert!(run.complete);
		assert!(
			run.record.starts_with("error\tFuelExhausted\t"),
			"{}",
			run.record
		);
	}
	fs::write(&path,"pub fn main() { loop { println!(\"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\"); } }").unwrap();
	let run = launch(&path, &path, "main", "fresh", &scratch.0, &scratch.0, "cap");
	assert!(!run.complete);
	assert_eq!(run.stdout.len(), CAP);
	let run = launch(
		&path,
		&path,
		"timeout-control",
		"fresh",
		&scratch.0,
		&scratch.0,
		"wall",
	);
	assert!(!run.complete);
	assert!(run.record.starts_with("child-gap\ttimeout\t"));
}

fn test_items(file: &crate::ast::File, text: &str, prefix: &str, tests: &mut Vec<(String, usize)>) {
	use crate::ast::{Item, ItemModBody, Spanned};
	for (item, _) in &file.items {
		match item {
			Item::Fn(f)
				if f.attributes
					.iter()
					.any(|a| text[a.path.span().range()].trim() == "test") =>
			{
				let name = &text[f.name.span.range()];
				tests.push((format!("{prefix}{name}"), f.args.len()));
			}
			Item::Mod(m) => {
				if let ItemModBody::InlineBody(body) = &m.body {
					let name = &text[m.name.span.range()];
					test_items(&body.file, text, &format!("{prefix}{name}::"), tests);
				}
			}
			_ => {}
		}
	}
}

fn entry_hash(entry: &str) -> Hash {
	let parts: Vec<_> = entry.split("::").collect();
	Hash::type_hash(parts.as_slice())
}
