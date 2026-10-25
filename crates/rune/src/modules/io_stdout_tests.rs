//! Real stdout, isolated descriptors, and bounded subprocess handshakes.
use std::borrow::ToOwned;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::string::{String, ToString};
use std::time::{Duration, Instant};
use std::vec::Vec;

use crate::experimental_core::{
	error::ErrorKind,
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::Scalar,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};

const CHILD: &str = "RUNE_B1_STDOUT_CHILD";
const DEADLINE: Duration = Duration::from_secs(30);
// Linux F_SETFD; this descriptor fixture is compiled only on Linux.
const F_SETFD: i32 = 2;
thread_local! {
	static BARRIER: RefCell<Option<(UnixStream, usize)>> = const { RefCell::new(None) };
}

/// Called only after a successful write, never after an I/O failure.
pub(crate) fn after_print() {
	BARRIER.with(|barrier| {
		if let Some((socket, count)) = barrier.borrow_mut().as_mut() {
			*count += 1;
			assert_eq!(*count, 1, "second line must fail before the hook");
			socket.write_all(b"1").unwrap();
			let mut go = [0];
			socket.read_exact(&mut go).unwrap();
			assert_eq!(&go, b"g");
		}
	});
}

// Only descriptors created and owned by this test are made inheritable. stdout is
// replaced inside the child test, after libtest's startup output (sent to /dev/null).
unsafe extern "C" {
	fn fcntl(fd: i32, cmd: i32, ...) -> i32;
	fn dup2(old: i32, new: i32) -> i32;
	fn close(fd: i32) -> i32;
}

fn inherit(fd: i32) -> std::io::Result<()> {
	// SAFETY: F_SETFD with zero flags changes only this test-owned descriptor.
	if unsafe { fcntl(fd, F_SETFD, 0) } == -1 {
		Err(std::io::Error::last_os_error())
	} else {
		Ok(())
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "child-process scenario; parent spawn tests are native-only"
)]
fn child_stdout_contract() {
	let Ok(mode) = std::env::var(CHILD) else {
		return;
	};
	let data: i32 = std::env::var("RUNE_B1_DATA_FD").unwrap().parse().unwrap();
	// SAFETY: this descriptor was transferred by our parent exactly once, survives
	// exec deliberately, and is not a libtest or other host descriptor.
	let data = unsafe { OwnedFd::from_raw_fd(data) };
	assert!(data.as_raw_fd() > 2);
	// SAFETY: replace only this subprocess's stdout with our owned data endpoint.
	assert_eq!(unsafe { dup2(data.as_raw_fd(), 1) }, 1);
	drop(data);
	if mode.ends_with("closed") {
		// SAFETY: stdout was just installed in this isolated child. Closing only
		// its descriptor deliberately exercises EBADF, never the parent's IO.
		assert_eq!(unsafe { close(1) }, 0);
	}
	let barrier = mode.ends_with("after");
	if barrier {
		let fd = std::env::var("RUNE_B1_CONTROL_FD")
			.unwrap()
			.parse()
			.unwrap();
		// SAFETY: one transferred, test-owned endpoint; no other owner in the child.
		let socket = unsafe { UnixStream::from_raw_fd(fd) };
		socket.set_read_timeout(Some(DEADLINE)).unwrap();
		socket.set_write_timeout(Some(DEADLINE)).unwrap();
		BARRIER.with(|b| *b.borrow_mut() = Some((socket, 0)));
	}
	let source = if mode.contains("c2-dbg") {
		r#"pub fn main(a) { dbg!("first", "second"); 42 }"#
	} else if mode.contains("c2-print") && barrier {
		r#"pub fn main(a) { println!("first"); print!("{}", a); 42 }"#
	} else if mode.contains("c2-print") {
		r#"pub fn main(a) { print!("{}", a); 42 }"#
	} else if mode.ends_with("volume") {
		r#"pub fn main(a) { let i = 0; while i < 256 { println!("{}", a); i += 1; } 42 }"#
	} else if mode.ends_with("partial") {
		r#"pub fn main(a) { print!("a"); 42 }"#
	} else {
		r#"pub fn main(a) { println!("first"); println!("second"); 42 }"#
	};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let (message, span) = if mode.starts_with("fresh") {
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let arg = store.root_string(&"a".repeat(4095)).unwrap();
		let mut executor = Executor::new().unwrap();
		if barrier {
			executor.test_after_print = Some(after_print);
		}
		let outcome = executor.invoke(&program, &mut store, main, &[&arg], 100_000);
		assert!(executor.is_idle());
		match outcome {
			Ok(root) => {
				assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(42)));
				store.root_release(root).unwrap();
				store.root_release(arg).unwrap();
				assert_eq!(store.live_entries(), 0);
				// Test-host teardown only; PRINT_RAW itself does not flush.
				std::io::stdout().flush().unwrap();
				("return:42".to_owned(), None)
			}
			Err(failure) => {
				assert_eq!(failure.error.kind, ErrorKind::Output);
				let at = failure.error.location.unwrap();
				let message = failure.error.message(&program).to_string();
				store.root_release(arg).unwrap();
				assert_eq!(store.live_entries(), 0);
				(message, Some(at.start as usize..at.end as usize))
			}
		}
	} else {
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		match vm.call(["main"], ("a".repeat(4095),)) {
			Ok(value) => {
				assert_eq!(crate::from_value::<i64>(value).unwrap(), 42);
				// Explicit host teardown flush; print! itself does not flush.
				std::io::stdout().flush().unwrap();
				("return:42".to_owned(), None)
			}
			Err(error) => {
				let at = error.first_location().unwrap();
				let span = at
					.unit
					.debug_info()
					.unwrap()
					.instruction_at(at.ip)
					.unwrap()
					.span
					.range();
				(error.to_string(), Some(span))
			}
		}
	};
	if let Some(span) = span {
		let expected = if mode.contains("c2-dbg") {
			"dbg!(\"first\", \"second\")"
		} else if mode.contains("c2-print") {
			"print!(\"{}\", a)"
		} else if barrier {
			"println!(\"second\")"
		} else {
			"println!(\"first\")"
		};
		assert_eq!(&source[span], expected);
	}
	eprintln!("{message}");
	// No libtest footer can write after the deliberately broken stdout.
	std::process::exit(0);
}

struct Running(Child);
impl Drop for Running {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn run(mode: &str) -> (Vec<u8>, String) {
	let (reader, writer) = UnixStream::pair().unwrap();
	reader.set_read_timeout(Some(DEADLINE)).unwrap();
	let mut reader = if mode.ends_with("first") {
		drop(reader);
		None
	} else {
		Some(reader)
	};
	let (mut control, child_control) = UnixStream::pair().unwrap();
	control.set_read_timeout(Some(DEADLINE)).unwrap();
	control.set_write_timeout(Some(DEADLINE)).unwrap();
	let mut command = Command::new(std::env::current_exe().unwrap());
	command
		.args([
			"--exact",
			"modules::io::stdout_tests::child_stdout_contract",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD, mode)
		.env("RUNE_B1_DATA_FD", writer.as_raw_fd().to_string())
		.env("RUNE_B1_CONTROL_FD", child_control.as_raw_fd().to_string())
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped());
	let data_fd = writer.as_raw_fd();
	let control_fd = child_control.as_raw_fd();
	// SAFETY: only async-signal-safe fcntl on owned descriptors before exec.
	unsafe {
		command.pre_exec(move || {
			inherit(data_fd)?;
			inherit(control_fd)
		});
	}
	let mut child = Running(command.spawn().unwrap());
	drop(writer);
	drop(child_control);
	let started = Instant::now();
	let mut out = Vec::new();
	if mode.ends_with("after") {
		let expected = if mode.contains("c2-dbg") {
			&b"\"first\"\n"[..]
		} else {
			&b"first\n"[..]
		};
		let mut first = std::vec![0; expected.len()];
		reader.as_mut().unwrap().read_exact(&mut first).unwrap();
		assert_eq!(&first, expected);
		out.extend(first);
		let mut ack = [0];
		control.read_exact(&mut ack).unwrap();
		assert_eq!(&ack, b"1");
		// A concurrently forked child may briefly retain this descriptor until exec.
		// Shutdown closes the socket endpoint for every copy before releasing the barrier.
		reader
			.as_ref()
			.unwrap()
			.shutdown(std::net::Shutdown::Both)
			.unwrap();
		drop(reader.take());
		control.write_all(b"g").unwrap();
	} else if let Some(reader) = reader.as_mut() {
		reader.read_to_end(&mut out).unwrap();
	}
	let status = loop {
		if let Some(status) = child.0.try_wait().unwrap() {
			break status;
		}
		assert!(started.elapsed() < DEADLINE, "child deadline: {mode}");
		std::thread::sleep(Duration::from_millis(10));
	};
	let mut stderr = String::new();
	child
		.0
		.stderr
		.take()
		.unwrap()
		.read_to_string(&mut stderr)
		.unwrap();
	assert!(status.success(), "{mode}: {stderr}");
	if mode.ends_with("after") {
		let mut rest = Vec::new();
		control.read_to_end(&mut rest).unwrap();
		assert!(rest.is_empty(), "no duplicate ack or print retry");
	}
	(out, stderr)
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot spawn subprocesses; native CI only")]
fn broken_stdout_preserves_error_and_call_location() {
	let expected = format!(
		"Panicked: failed printing to stdout: {}\n",
		std::io::Error::from_raw_os_error(32)
	);
	for engine in ["fresh", "stock"] {
		for phase in ["first", "after"] {
			let (out, error) = run(&format!("{engine}-{phase}"));
			assert_eq!(
				out,
				if phase == "after" {
					&b"first\n"[..]
				} else {
					&[][..]
				}
			);
			assert_eq!(error, expected);
		}
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot spawn subprocesses; native CI only")]
fn volume_matches_stock_and_stock_partial_line_is_preserved() {
	let expected = format!("{}\n", "a".repeat(4095)).repeat(256).into_bytes();
	for engine in ["fresh", "stock"] {
		let (out, error) = run(&format!("{engine}-volume"));
		assert_eq!(out, expected);
		assert_eq!(error, "return:42\n");
	}
	assert_eq!(
		run("stock-partial"),
		(b"a".to_vec(), "return:42\n".to_owned())
	);
}

#[test]
#[ignore = "C2 stock-only bounded descriptor oracle; explicit new GAP_OUT"]
fn legacy_c2_probe() {
	let out = std::path::PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	legacy_c2_at(&out);
}

pub(crate) fn legacy_c2_at(out: &std::path::Path) {
	std::fs::create_dir(out).unwrap();
	let mut table = String::from("case\tstdout_hex\tstderr_hex\n");
	let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
	for mode in [
		"stock-c2-dbg-first",
		"stock-c2-dbg-after",
		"stock-c2-print-first",
		"stock-c2-print-after",
		"stock-c2-dbg-closed",
		"stock-c2-print-closed",
		"stock-partial",
	] {
		let (stdout, stderr) = run(mode);
		std::fs::write(out.join(format!("{mode}.stdout")), &stdout).unwrap();
		std::fs::write(out.join(format!("{mode}.stderr")), &stderr).unwrap();
		table.push_str(&format!(
			"{mode}\t{}\t{}\n",
			hex(&stdout),
			hex(stderr.as_bytes())
		));
		std::fs::write(out.join("stock-output-descriptors.tsv"), &table).unwrap();
	}
}

#[test]
#[ignore = "C2 execution descriptor comparison, explicit new GAP_OUT"]
fn c2_contract() {
	let out = std::path::PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	c2_at(&out);
}

pub(crate) fn c2_at(out: &std::path::Path) {
	std::fs::create_dir(out).unwrap();
	let rows: Vec<_> = include_str!("../experimental_core/tests/output_descriptors_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(rows.len(), 7);
	let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
	for row in rows {
		let fields: Vec<_> = row.split('\t').collect();
		assert_eq!(fields.len(), 3);
		if fields[0].contains("c2-dbg") {
			continue;
		}
		let mode = fields[0].replacen("stock", "fresh", 1);
		let (stdout, stderr) = run(&mode);
		std::fs::write(out.join(format!("{mode}.stdout")), &stdout).unwrap();
		std::fs::write(out.join(format!("{mode}.stderr")), &stderr).unwrap();
		assert_eq!(hex(&stdout), fields[1], "{mode} stdout");
		assert_eq!(hex(stderr.as_bytes()), fields[2], "{mode} stderr");
	}
}
