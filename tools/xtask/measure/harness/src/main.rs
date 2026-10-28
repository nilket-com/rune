//! P2 first-probe repeat harness for the fresh core, v3 (derived from the P0 harness
//! `0184-p0-impl1/harness/rune_repeat_input.rs`; the changes are listed below).
//!
//! usage: p2-probe-harness KERNEL.rn K INPUTS.txt PHASES.txt|- [--transcript]
//!
//! Same shape as the Lua harness: read the first K inputs, compile once, invoke `main(x)` K
//! times on one executor and one store, fold each I64 result into the shared checksum, print
//! ONE checksum line at the end. Results are never printed per invocation in timing mode.
//!
//! Changes from v2:
//!
//! 1. There is no public prepare API (P2a ruling), so the native tier compiles at the first
//!    invocation. `first - ready` is therefore the literal FIRST INVOCATION: on `p2-on` it
//!    includes the full prepare (analysis, compile, audit, load, bookkeeping) and the first
//!    run; on `p2-off` the first run only. No phase is labelled "prepare" here.
//! 2. After the run (outside every timed phase), the harness counts the anonymous executable
//!    mappings of its own process from /proc/self/maps and prints them as a second stdout
//!    line `MAPS anon_exec_mappings=N anon_exec_kib=K`. Loaded regions are the only
//!    anonymous executable mappings this process creates, so this is the release binary's
//!    own evidence of whether native code was loaded (no API needed).
//! 3. `--transcript` (validation only, never timed) prints each invocation's result on its
//!    own line before the checksum line.
//! 4. PHASES `-` writes no phases file (validation runs record no timing).
//!
//! Phase timestamps (CLOCK_MONOTONIC via `Instant`, nanoseconds since `start`) go to the
//! phases file; stderr stays empty on success. Any stderr output, including the native
//! tier's invalid-exit line, makes the sample invalid.
//!
//! Build: one source, one Cargo.lock, one profile; `p2-on` with `--features native`
//! (rune's `fresh-core-native`), `p2-off` without. See ../build.sh.
use rune::experimental_core::execute::Executor;
use rune::experimental_core::frontend;
use rune::experimental_core::heap::{Limits, Store};
use rune::experimental_core::value::Scalar;
use rune::{Context, Diagnostics, Hash, Source, Sources};
use std::io::Write;
use std::time::Instant;

/// Large enough never to halt these kernels; fuel charging stays on.
const FUEL: usize = 1 << 40;

fn anon_exec() -> (usize, u64) {
	let maps = std::fs::read_to_string("/proc/self/maps").expect("/proc/self/maps");
	let mut count = 0;
	let mut kib = 0;
	for line in maps.lines() {
		let mut fields = line.split_whitespace();
		let range = fields.next().expect("maps range");
		let perms = fields.next().expect("maps perms");
		let path = fields.nth(3);
		if perms.as_bytes().get(2) == Some(&b'x') && path.is_none() {
			let (start, end) = range.split_once('-').expect("maps range");
			let start = u64::from_str_radix(start, 16).expect("maps start");
			let end = u64::from_str_radix(end, 16).expect("maps end");
			count += 1;
			kib += (end - start) / 1024;
		}
	}
	(count, kib)
}

fn main() {
	let start = Instant::now();
	let args: Vec<String> = std::env::args().skip(1).collect();
	let transcript = match args.len() {
		4 => false,
		5 if args[4] == "--transcript" => true,
		_ => {
			eprintln!("usage: p2-probe-harness KERNEL.rn K INPUTS.txt PHASES.txt|- [--transcript]");
			std::process::exit(2);
		}
	};
	let k: usize = args[1].parse().expect("K");
	assert!(k >= 1, "K >= 1");
	let xs: Vec<i64> = std::fs::read_to_string(&args[2])
		.expect("inputs readable")
		.lines()
		.take(k)
		.map(|l| l.parse().expect("integer input"))
		.collect();
	assert_eq!(xs.len(), k, "not enough inputs");

	let context = Context::with_default_modules().expect("context");
	let text = std::fs::read_to_string(&args[0]).expect("source readable");
	let mut sources = Sources::new();
	sources
		.insert(Source::new(&args[0], &text).expect("source"))
		.expect("source insertion");
	let mut diagnostics = Diagnostics::new();
	let program =
		frontend::compile(&mut sources, &context, &mut diagnostics, 1).expect("fresh-core compile");
	let main = program.function(Hash::type_hash(["main"])).expect("main");
	let mut store = Store::new(Limits {
		entries: 1 << 16,
		bytes: 1 << 24,
	})
	.expect("store");
	let mut executor = Executor::new().expect("executor");
	let ready = start.elapsed();

	let stdout = std::io::stdout();
	let mut out = std::io::BufWriter::new(stdout.lock());
	let mut checksum: i64 = 0;
	let mut first = None;
	for &x in &xs {
		let arg = store.root_i64(x);
		let result = match executor.invoke(&program, &mut store, main, &[&arg], FUEL) {
			Ok(root) => root,
			Err(failure) => {
				eprintln!("error:{:?}", failure.error);
				std::process::exit(7);
			}
		};
		let r = match store.root_scalar(&result).expect("result scalar") {
			Some(Scalar::I64(r)) => r,
			other => {
				eprintln!("error:non-integer result {other:?}");
				std::process::exit(7);
			}
		};
		store.root_release(result).expect("result release");
		store.root_release(arg).expect("argument release");
		checksum = (checksum * 31 + r) % 1_000_000_007;
		if transcript {
			writeln!(out, "{r}").expect("transcript write");
		}
		first.get_or_insert_with(|| start.elapsed());
	}
	let done = start.elapsed();
	assert!(executor.is_idle(), "executor idle after the run");
	assert_eq!(store.live_entries(), 0, "no live entries after the run");

	if args[3] != "-" {
		let mut p = std::fs::File::create(&args[3]).expect("phases file");
		writeln!(
			p,
			"start 0\nready {}\nfirst {}\ndone {}",
			ready.as_nanos(),
			first.expect("k >= 1").as_nanos(),
			done.as_nanos()
		)
		.expect("phases write");
	}
	let (mappings, kib) = anon_exec();
	writeln!(out, "CHECKSUM {checksum} invocations={k}").expect("stdout");
	writeln!(
		out,
		"MAPS anon_exec_mappings={mappings} anon_exec_kib={kib}"
	)
	.expect("stdout");
	out.flush().expect("stdout flush");
}
