//! Live boundary classes authorized by rune-e1 seq23661 and seq23666.
use super::*;
use crate::experimental_core::error::{ErrorClass, ErrorProjection};
use serde_json::{json, Value as Json};
use std::{
	fs,
	io::Write,
	path::{Path, PathBuf},
	process::Command,
	sync::Mutex,
};

fn text(case: &str) -> &'static str {
	match case {
		"panic" => "pub fn main(){panic!(\"boom\")}",
		"index" => "pub fn main(){let a=[1];a[9]}",
		"success" => "pub fn main(){println!(\"done\");42}",
		"error-effects" => {
			"pub fn main(){probe::mark(0);probe::err(0);println!(\"before\");panic!(\"boom\")}"
		}
		"print" => "pub fn main(){println!(\"one\");loop{}}",
		"effects" => {
			"pub fn main(){let n=0;loop{probe::mark(n);probe::err(n);println!(\"{}\",n);n+=1;}}"
		}
		_ => panic!("unknown budget-edge input"),
	}
}

fn observe(case: &str, engine: &str, work: usize) -> Json {
	let mut context = Context::with_default_modules().unwrap();
	let effects = Arc::new(Mutex::new(std::vec::Vec::<std::string::String>::new()));
	let mut module = crate::Module::with_crate("probe").unwrap();
	let log = effects.clone();
	module
		.host_function("mark", move |n: i64| {
			log.lock().unwrap().push(std::format!("mark:{n}"));
		})
		.unwrap();
	let log = effects.clone();
	module
		.host_function("err", move |n: i64| {
			log.lock().unwrap().push(std::format!("err:{n}"));
			std::eprintln!("err:{n}");
		})
		.unwrap();
	context.install(module).unwrap();
	let text = text(case);
	let mut input = sources(text);
	let mut result;
	if engine == "legacy" {
		let mut vm = crate::prepare(&mut input)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let (out, remaining) = crate::runtime::budget::with(work, || {
			let out = vm.call(["main"], ());
			(out, crate::runtime::budget::remaining().unwrap())
		})
		.call();
		result = match out {
			Ok(value) => {
				json!({"phase":"success","value":crate::from_value::<i64>(value).unwrap(),"remaining":remaining})
			}
			Err(error)
				if matches!(
					error.error_class(),
					ErrorProjection::Class(ErrorClass::BudgetExhausted)
				) =>
			{
				json!({"phase":"budget","remaining":remaining})
			}
			Err(error) => {
				json!({"phase":"error","remaining":remaining,"primary":error.diagnostic(&input).to_string()})
			}
		};
	} else {
		assert_eq!(engine, "execution");
		let program = frontend::compile(&mut input, &context, &mut Diagnostics::new(), 1).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = executor(&context);
		executor.set_format_limits(FormatLimits::permissive());
		let out = executor
			.prepare(&program, &mut store, main(&program), &[])
			.unwrap()
			.invoke(work);
		result = match out {
			Ok(root) => {
				let value = store.root_scalar(&root).unwrap();
				store.root_release(root).unwrap();
				json!({"phase":"success","value":match value{Some(Scalar::I64(n))=>n,_=>panic!("unexpected result")}})
			}
			Err(failure) => {
				let phase = if failure.error.kind == ErrorKind::FuelExhausted {
					"budget"
				} else {
					"error"
				};
				json!({"phase":phase,"remaining":failure.remaining_work,"primary":failure.diagnostic(&program,&input).to_string()})
			}
		};
		store.close().unwrap();
	}
	result["effects"] = json!(*effects.lock().unwrap());
	result
}

#[test]
#[ignore = "isolated budget-edge child; the parent captures real streams"]
fn child() {
	let case = std::env::var("E1_EDGE_CASE").unwrap();
	let engine = std::env::var("E1_EDGE_ENGINE").unwrap();
	let work = std::env::var("E1_EDGE_WORK").unwrap().parse().unwrap();
	let result = observe(&case, &engine, work);
	fs::write(
		std::env::var_os("E1_EDGE_RESULT").unwrap(),
		serde_json::to_vec(&result).unwrap(),
	)
	.unwrap();
	std::io::stdout().flush().unwrap();
	std::io::stderr().flush().unwrap();
	std::process::exit(0);
}

struct Observation {
	metadata: Json,
	stdout: std::vec::Vec<u8>,
	stderr: std::vec::Vec<u8>,
}

fn run(root: &Path, case: &str, engine: &str, work: usize) -> Observation {
	let directory = root.join(std::format!("{case}-{engine}-{work}"));
	fs::create_dir(&directory).unwrap();
	fs::write(directory.join("source.rn"), text(case)).unwrap();
	let mut command = Command::new("/usr/bin/prlimit");
	command
		.args(["--cpu=10", "--as=2147483648", "--core=0", "--"])
		.arg(std::env::current_exe().unwrap());
	command.args([
		"--exact",
		"experimental_core::tests::embedding_boundary::budget_edges::child",
		"--ignored",
		"--nocapture",
		"--quiet",
		"--test-threads=1",
	]);
	command
		.env("E1_EDGE_CASE", case)
		.env("E1_EDGE_ENGINE", engine)
		.env("E1_EDGE_WORK", work.to_string())
		.env("E1_EDGE_RESULT", directory.join("result.json"));
	let output = command.output().unwrap();
	fs::write(directory.join("stdout"), &output.stdout).unwrap();
	fs::write(directory.join("stderr"), &output.stderr).unwrap();
	assert!(
		output.status.success(),
		"{case}/{engine}/{work}: {}",
		std::string::String::from_utf8_lossy(&output.stderr)
	);
	let metadata =
		serde_json::from_slice(&fs::read(directory.join("result.json")).unwrap()).unwrap();
	Observation {
		metadata,
		stdout: output.stdout,
		stderr: output.stderr,
	}
}

fn prefix(a: &Observation, b: &Observation) {
	assert!(b.stdout.starts_with(&a.stdout), "stdout prefix failed");
	assert!(b.stderr.starts_with(&a.stderr), "stderr prefix failed");
	let a = a.metadata["effects"].as_array().unwrap();
	let b = b.metadata["effects"].as_array().unwrap();
	assert!(b.starts_with(a), "ordered host-effect prefix failed");
}

fn same_progress(a: &Observation, b: &Observation) -> bool {
	a.stdout == b.stdout && a.stderr == b.stderr && a.metadata["effects"] == b.metadata["effects"]
}

#[test]
fn live_budget_edges_obey_all_four_classes_and_two_way_progress_prefixes() {
	let root: PathBuf = std::env::var_os("E1_EDGE_OUTPUT")
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			std::env::temp_dir().join(std::format!(
				"rune-e1-budget-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			))
		});
	fs::create_dir(&root).unwrap();
	let mut counts = std::collections::BTreeMap::<&str, usize>::new();
	for case in [
		"success",
		"panic",
		"index",
		"print",
		"effects",
		"error-effects",
	] {
		let reference = run(&root, case, "legacy", 1024);
		for work in 0..20 {
			let legacy = run(&root, case, "legacy", work);
			let new = run(&root, case, "execution", work);
			let a = legacy.metadata["phase"].as_str().unwrap();
			let b = new.metadata["phase"].as_str().unwrap();
			for observation in [&legacy, &new] {
				if observation.metadata["phase"] == "budget" {
					assert_eq!(observation.metadata["remaining"], 0);
				}
			}
			// A natural outcome has complete streams, not merely a valid prefix.
			if b != "budget" {
				assert!(
					same_progress(&new, &reference),
					"natural-outcome output differs"
				);
			}
			if a != "budget" {
				assert!(
					same_progress(&legacy, &new),
					"completed legacy output differs"
				);
			}
			let class = match (a, b) {
				("budget", "success") => {
					assert_eq!(new.metadata["value"], reference.metadata["value"]);
					"reaches-further/success"
				}
				("budget", "error") => {
					assert_eq!(reference.metadata["phase"], "error");
					assert_eq!(new.metadata["primary"], reference.metadata["primary"]);
					"reaches-further/error"
				}
				("budget", "budget") => {
					if same_progress(&legacy, &new) {
						"equal-budget"
					} else {
						"both-halt/further"
					}
				}
				("success", "success") => {
					assert_eq!(legacy.metadata["value"], new.metadata["value"]);
					"equal-success"
				}
				("error", "error") => {
					assert_eq!(legacy.metadata["primary"], new.metadata["primary"]);
					let a = legacy.metadata["remaining"].as_u64().unwrap();
					let b = new.metadata["remaining"].as_u64().unwrap();
					assert!(b >= a, "execution used more work before the same error");
					if a == 0 && b > 0 {
						"budget-annotation"
					} else {
						"equal-error"
					}
				}
				_ => panic!(
					"legacy reached {a}, execution reached {b}: {}",
					new.metadata
				),
			};
			prefix(&legacy, &new);
			prefix(&new, &reference);
			*counts.entry(class).or_default() += 1;
		}
	}
	for class in [
		"reaches-further/success",
		"reaches-further/error",
		"budget-annotation",
		"both-halt/further",
	] {
		assert!(counts.get(class).is_some_and(|n| *n > 0), "missing {class}");
	}
	fs::write(
		root.join("COUNTS.json"),
		serde_json::to_vec_pretty(&counts).unwrap(),
	)
	.unwrap();
	std::println!(
		"E1 live budget classes {counts:?}; raw observations {}",
		root.display()
	);
}
