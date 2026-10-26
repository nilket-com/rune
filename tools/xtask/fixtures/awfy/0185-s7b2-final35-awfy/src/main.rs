//! W1 AWFY runner: compiles the given Rune sources as one program and calls
//! `main(num_iterations, inner_iterations)`, which must return `true`.
//!
//! usage: awfy-runner NUM INNER FILE.rn...
//!
//! Built without features it runs the stock VM; with `fresh` or `native` it runs the
//! fresh core and prints the first refusal (`REFUSED ...`, exit 3) when the program is
//! outside the admitted subset. Nothing falls back.
use rune::termcolor::{ColorChoice, StandardStream};
use rune::{Context, Diagnostics, Source, Sources};

fn usage() -> ! {
	eprintln!("usage: awfy-runner NUM INNER FILE.rn...");
	std::process::exit(2);
}

fn load(paths: &[String]) -> Sources {
	let mut sources = Sources::new();
	for path in paths {
		let text = std::fs::read_to_string(path).expect("source readable");
		sources.insert(Source::new(path, &text).expect("source")).expect("source insertion");
	}
	sources
}

/// Only errors are shown: unused-item warnings from the shared library are expected.
fn emit(diagnostics: &Diagnostics, sources: &Sources) {
	if diagnostics.has_error() {
		let mut writer = StandardStream::stderr(ColorChoice::Never);
		diagnostics.emit(&mut writer, sources).expect("diagnostics emit");
	}
}

#[cfg(not(feature = "fresh"))]
fn run(num: i64, inner: i64, paths: &[String]) -> bool {
	use rune::sync::Arc;
	use rune::Vm;

	let context = Context::with_default_modules().expect("context");
	let runtime = Arc::try_new(context.runtime().expect("runtime")).expect("runtime arc");
	let mut sources = load(paths);
	let mut diagnostics = Diagnostics::new();
	let result = rune::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build();
	emit(&diagnostics, &sources);
	let unit = match result {
		Ok(unit) => unit,
		Err(_) => std::process::exit(4),
	};
	let mut vm = Vm::new(runtime, Arc::try_new(unit).expect("unit arc"));
	let output = match vm.call(["main"], (num, inner)) {
		Ok(output) => output,
		Err(error) => {
			eprintln!("error: {error}");
			std::process::exit(7);
		}
	};
	rune::from_value::<bool>(output).expect("main returns bool")
}

#[cfg(feature = "fresh")]
fn run(num: i64, inner: i64, paths: &[String]) -> bool {
	use rune::experimental_core::execute::Executor;
	use rune::experimental_core::frontend;
	use rune::experimental_core::heap::{Limits, Store};
	use rune::experimental_core::value::Scalar;
	use rune::Hash;

	let context = Context::with_default_modules().expect("context");
	let mut sources = load(paths);
	let mut diagnostics = Diagnostics::new();
	let program = match frontend::compile(&mut sources, &context, &mut diagnostics, 1) {
		Ok(program) => program,
		Err(error) => {
			let refusal = error.refusal.map_or("-", |r| r.name());
			let mut site = String::from("-");
			if let Some(l) = error.location {
				if let Some(path) = paths.get(l.source as usize) {
					let text = std::fs::read_to_string(path).expect("source readable");
					let start = l.start as usize;
					let line = text[..start].matches('\n').count() + 1;
					let column = start - text[..start].rfind('\n').map_or(0, |i| i + 1) + 1;
					let snippet: String = text[start..(l.end as usize).min(text.len())].chars().take(160).collect();
					site = format!("{path}:{line}:{column} {:?}", snippet);
				}
			}
			println!("REFUSED kind={refusal} operation={} error={:?} site={site}", error.operation, error.kind);
			emit(&diagnostics, &sources);
			std::process::exit(3);
		}
	};
	let main = program.function(Hash::type_hash(["main"])).expect("main");
	let limits = Limits::default();
	let mut store = Store::new(limits).expect("store");
	let mut invocation_limits = rune::experimental_core::execute::ExecutorLimits::default();
	if let Ok(n) = std::env::var("S6_FRAME_LIMIT") { invocation_limits.frames = n.parse().unwrap(); }
	if let Ok(n) = std::env::var("S6_SLOT_LIMIT") { invocation_limits.slots = n.parse().unwrap(); }
	let mut executor = Executor::with_limits(invocation_limits).expect("executor");
	executor.set_host_runtime(Some(rune::sync::Arc::try_new(context.runtime().expect("runtime")).expect("runtime arc"))).expect("host runtime");
	let a = store.root_i64(num);
	let b = store.root_i64(inner);
	let result = match executor.invoke(&program, &mut store, main, &[&a, &b], usize::MAX) {
		Ok(root) => root,
		Err(failure) => {
			eprintln!("error: {:?}", failure.error);
			if std::env::var_os("S6_FRAME_LIMIT").is_some() || std::env::var_os("S6_SLOT_LIMIT").is_some() {
				eprintln!("S6_CONTROL callers={} idle={} cleanup_error={}", failure.call_chain.len(), executor.is_idle(), failure.cleanup_error.is_some());
			}
			if let Some(m) = &failure.panic_message {
				eprintln!("panic: {m:?}");
			}
			std::process::exit(7);
		}
	};
	let ok = match store.root_scalar(&result).expect("result scalar") {
		Some(Scalar::Bool(ok)) => ok,
		other => {
			eprintln!("error: non-bool result {other:?}");
			std::process::exit(7);
		}
	};
	eprintln!("fresh live_entries={}", store.live_entries());
	ok
}

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	if args.len() < 3 {
		usage();
	}
	let num: i64 = args[0].parse().unwrap_or_else(|_| usage());
	let inner: i64 = args[1].parse().unwrap_or_else(|_| usage());
	if run(num, inner, &args[2..]) {
		println!("RESULT ok");
	} else {
		println!("RESULT FAILED");
		std::process::exit(1);
	}
}
