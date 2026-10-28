//! Shared process receipts, exact test selection and source identity.
use anyhow::{ensure, Context as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
	collections::BTreeMap,
	ffi::OsString,
	fs::{self, File, OpenOptions},
	io::{Read, Write},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::{Arc, Mutex},
	time::{Instant, SystemTime, UNIX_EPOCH},
};
pub type Result<T> = anyhow::Result<T>;
pub const EXECUTION_FEATURES: &str = "fmt,workspace,execution,capture-io,disable-io,sha2,serde";

#[derive(Clone)]
pub struct Context {
	pub repo: PathBuf,
	pub fixtures: PathBuf,
	pub out: PathBuf,
	pub root_out: PathBuf,
	pub toolchain: String,
	pub jobs: usize,
	pub test_threads: usize,
	builds: Arc<Mutex<BTreeMap<String, Build>>>,
	source: Arc<Source>,
}
struct Build {
	features: String,
	binary: PathBuf,
	binary_sha256: String,
	receipt: PathBuf,
	receipt_sha256: String,
	dwarf: PathBuf,
	dwarf_sha256: String,
}
impl Build {
	fn verify(&self) -> Result<()> {
		for (path, expected) in [
			(&self.binary, &self.binary_sha256),
			(&self.receipt, &self.receipt_sha256),
			(&self.dwarf, &self.dwarf_sha256),
		] {
			ensure!(
				hash_file(path)? == *expected,
				"build artifact changed: {}",
				path.display()
			);
		}
		Ok(())
	}
}
#[derive(Serialize)]
pub struct Source {
	pub repo: PathBuf,
	pub head: String,
	pub dirty: bool,
	pub files: BTreeMap<String, String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Run {
	pub code: i32,
	pub stdout: PathBuf,
	pub stderr: PathBuf,
	pub receipt: PathBuf,
	pub seconds: f64,
}
#[derive(Debug, PartialEq, Eq)]
pub struct TestCounts {
	pub passed: usize,
	pub failed: usize,
	pub ignored: usize,
}

fn hex(bytes: &[u8]) -> String {
	const DIGITS: &[u8] = b"0123456789abcdef";
	let mut out = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		out.push(DIGITS[(byte >> 4) as usize] as char);
		out.push(DIGITS[(byte & 15) as usize] as char);
	}
	out
}
pub fn hash_bytes(bytes: &[u8]) -> String {
	hex(Sha256::digest(bytes).as_ref())
}
pub fn hash_file(path: &Path) -> Result<String> {
	let mut input = File::open(path).with_context(|| format!("open {}", path.display()))?;
	let mut hash = Sha256::new();
	let mut buffer = [0u8; 65536];
	loop {
		let n = input.read(&mut buffer)?;
		if n == 0 {
			break;
		}
		hash.update(&buffer[..n]);
	}
	Ok(hex(hash.finalize().as_ref()))
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
	serde_json::from_reader(File::open(path).with_context(|| format!("open {}", path.display()))?)
		.with_context(|| format!("decode {}", path.display()))
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
	let mut file = OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)
		.with_context(|| format!("create immutable receipt {}", path.display()))?;
	serde_json::to_writer_pretty(&mut file, value)?;
	file.write_all(b"\n")?;
	Ok(())
}
fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
	let output = Command::new("git").current_dir(repo).args(args).output()?;
	ensure!(
		output.status.success(),
		"git failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(output.stdout)
}
impl Source {
	fn capture(repo: &Path) -> Result<Self> {
		let head = String::from_utf8(git(repo, &["rev-parse", "HEAD"])?)?
			.trim()
			.to_owned();
		let dirty = !git(repo, &["status", "--porcelain"])?.is_empty();
		let mut files = BTreeMap::new();
		for name in git(
			repo,
			&[
				"ls-files",
				"--cached",
				"--others",
				"--exclude-standard",
				"-z",
			],
		)?
		.split(|b| *b == 0)
		.filter(|s| !s.is_empty())
		{
			let name = std::str::from_utf8(name)?;
			let path = repo.join(name);
			ensure!(path.is_file(), "missing source file {}", path.display());
			files.insert(name.to_owned(), hash_file(&path)?);
		}
		// Include the lockfile even when checking a historical checkout where it was ignored.
		if repo.join("Cargo.lock").is_file() {
			files.insert("Cargo.lock".into(), hash_file(&repo.join("Cargo.lock"))?);
		}
		Ok(Self {
			repo: repo.to_owned(),
			head,
			dirty,
			files,
		})
	}
	fn verify(&self) -> Result<()> {
		let current = Self::capture(&self.repo)?;
		ensure!(
			current.head == self.head && current.files == self.files,
			"source tree changed during verification"
		);
		Ok(())
	}
}
impl Context {
	pub fn new(
		repo: PathBuf,
		fixtures: PathBuf,
		out: PathBuf,
		toolchain: String,
		jobs: usize,
		test_threads: usize,
	) -> Result<Self> {
		ensure!(
			jobs > 0 && test_threads > 0,
			"jobs and test threads must be positive"
		);
		let repo = fs::canonicalize(repo)?;
		let fixtures = fs::canonicalize(fixtures).context("fixtures root must exist")?;
		ensure!(!out.exists(), "output already exists: {}", out.display());
		let absolute_out = if out.is_absolute() {
			out.clone()
		} else {
			std::env::current_dir()?.join(&out)
		};
		let absolute_out = resolved_destination(&absolute_out)?;
		if absolute_out.starts_with(&repo) {
			ensure!(
				absolute_out.starts_with(repo.join("target/xtask-verify")),
				"output inside the source tree must be under target/xtask-verify"
			);
			let relative = absolute_out.strip_prefix(&repo)?;
			ensure!(
				Command::new("git")
					.current_dir(&repo)
					.args(["check-ignore", "-q", "--"])
					.arg(relative)
					.status()?
					.success(),
				"in-tree scratch must be ignored by Git"
			);
		}
		if let Some(parent) = absolute_out.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::create_dir(&absolute_out)?;
		let out = fs::canonicalize(absolute_out)?;
		if out.starts_with(&repo) {
			ensure!(
				out.starts_with(repo.join("target/xtask-verify")),
				"resolved scratch output escaped target/xtask-verify"
			);
		}
		let source = Arc::new(Source::capture(&repo)?);
		write_json(&out.join("SOURCE.json"), source.as_ref())?;
		let executable = std::env::current_exe()?;
		write_json(
			&out.join("TOOL.json"),
			&json!({"schema":1,"executable":executable,
            "sha256":hash_file(&executable)?,"revision":env!("XTASK_REVISION"),
            "dirty":env!("XTASK_DIRTY"),"version":env!("CARGO_PKG_VERSION")}),
		)?;
		Ok(Self {
			repo,
			fixtures,
			root_out: out.clone(),
			out,
			toolchain,
			jobs,
			test_threads,
			builds: Arc::new(Mutex::new(BTreeMap::new())),
			source,
		})
	}
	pub fn suite(&self, name: &str) -> Result<Self> {
		valid_label(name)?;
		let mut next = self.clone();
		next.out = self.root_out.join(name);
		fs::create_dir(&next.out)?;
		Ok(next)
	}
	pub fn verify_source(&self) -> Result<()> {
		self.source.verify()?;
		self.verify_binaries()
	}
	pub fn verify_binaries(&self) -> Result<()> {
		let cache = self
			.builds
			.lock()
			.map_err(|_| anyhow::anyhow!("build cache poisoned"))?;
		for build in cache.values() {
			build.verify()?;
		}
		Ok(())
	}
	pub fn cargo(&self) -> Command {
		self.cargo_toolchain(&self.toolchain)
	}
	pub fn cargo_toolchain(&self, toolchain: &str) -> Command {
		let mut command = Command::new("cargo");
		command
			.arg(format!("+{toolchain}"))
			.current_dir(&self.repo)
			.env("CARGO_BUILD_JOBS", self.jobs.to_string())
			.env("CARGO_INCREMENTAL", "0")
			.env("CARGO_PROFILE_DEV_DEBUG", "2")
			.env("CARGO_PROFILE_TEST_DEBUG", "2")
			.env("RUST_MIN_STACK", (16usize << 20).to_string())
			.env("CARGO_TARGET_DIR", self.out.join("target-cargo"));
		command
	}
	pub fn run(&self, label: &str, command: &mut Command, timeout_secs: u64) -> Result<Run> {
		self.run_seconds(label, command, timeout_secs as f64)
	}
	pub fn run_seconds(
		&self,
		label: &str,
		command: &mut Command,
		timeout_secs: f64,
	) -> Result<Run> {
		valid_label(label)?;
		ensure!(
			timeout_secs.is_finite() && timeout_secs > 0.0,
			"timeout must be finite and positive"
		);
		let stdout = self.out.join(format!("{label}.stdout"));
		let stderr = self.out.join(format!("{label}.stderr"));
		let receipt = self.out.join(format!("{label}.json"));
		ensure!(
			!receipt.exists(),
			"receipt already exists: {}",
			receipt.display()
		);
		let output_file = OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&stdout)?;
		let error_file = OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&stderr)?;
		let program = command.get_program().to_os_string();
		let args: Vec<OsString> = command.get_args().map(Into::into).collect();
		let environment: Vec<(OsString, Option<OsString>)> = command
			.get_envs()
			.map(|(k, v)| (k.to_os_string(), v.map(Into::into)))
			.collect();
		let cwd = command
			.get_current_dir()
			.unwrap_or(&self.repo)
			.to_path_buf();
		// timeout owns a process group and blocks; no process-status polling loop.
		let mut child = Command::new("/usr/bin/timeout");
		child.env_clear();
		for key in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "TMPDIR"] {
			if let Some(value) = std::env::var_os(key) {
				child.env(key, value);
			}
		}
		child
			.args(["--signal=TERM", "--kill-after=5s"])
			.arg(format!("{timeout_secs}s"))
			.arg(&program)
			.args(&args)
			.current_dir(&cwd)
			.stdin(Stdio::null())
			.stdout(Stdio::from(output_file))
			.stderr(Stdio::from(error_file));
		for (key, value) in &environment {
			match value {
				Some(value) => {
					child.env(key, value);
				}
				None => {
					child.env_remove(key);
				}
			}
		}
		child.env("LANG", "C.UTF-8").env("TZ", "UTC");
		let effective_environment: Vec<(OsString, OsString)> = child
			.get_envs()
			.filter_map(|(key, value)| {
				value.map(|value| (key.to_os_string(), value.to_os_string()))
			})
			.collect();
		let start = Instant::now();
		let epoch = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
		let result = child.status();
		let seconds = start.elapsed().as_secs_f64();
		let (code, spawn_error) = match result {
			Ok(status) => (status.code().unwrap_or(-1), None),
			Err(error) => (-1, Some(error.to_string())),
		};
		write_json(
			&receipt,
			&json!({"schema":1,"program":program,"args":args,"cwd":cwd,
            "environment":effective_environment,"environment_overrides":environment,"start_epoch":epoch,"timeout_seconds":timeout_secs,
            "seconds":seconds,"code":code,"spawn_error":spawn_error,
            "stdout":stdout,"stdout_sha256":hash_file(&stdout)?,"stderr":stderr,"stderr_sha256":hash_file(&stderr)?}),
		)?;
		Ok(Run {
			code,
			stdout,
			stderr,
			receipt,
			seconds,
		})
	}
	pub fn checked(&self, label: &str, command: &mut Command, timeout_secs: u64) -> Result<Run> {
		let run = self.run(label, command, timeout_secs)?;
		ensure!(
			run.code == 0,
			"{label} failed with code {}; raw logs: {}, {}",
			run.code,
			run.stdout.display(),
			run.stderr.display()
		);
		Ok(run)
	}
	pub fn binary_receipt(&self, profile: &str) -> PathBuf {
		self.root_out
			.join("build")
			.join(profile)
			.join("BINARY.json")
	}
	pub fn test_binary_receipt(&self) -> Result<PathBuf> {
		self.test_binary()?;
		Ok(self.binary_receipt("execution-serde"))
	}
	pub fn test_binary(&self) -> Result<PathBuf> {
		self.build_rune("execution-serde", EXECUTION_FEATURES)
	}
	pub fn build_rune(&self, profile: &str, features: &str) -> Result<PathBuf> {
		valid_label(profile)?;
		let mut cache = self
			.builds
			.lock()
			.map_err(|_| anyhow::anyhow!("build cache poisoned"))?;
		if let Some(build) = cache.get(profile) {
			ensure!(
				build.features == features,
				"profile reused with different features: {profile}"
			);
			build.verify()?;
			return Ok(build.binary.clone());
		}
		self.source.verify()?;
		let build_dir = self.root_out.join("build").join(profile);
		fs::create_dir_all(build_dir.parent().unwrap())?;
		fs::create_dir(&build_dir)?;
		let mut ctx = self.clone();
		ctx.out = build_dir.clone();
		let mut command = self.cargo();
		command
			.args([
				"test",
				"--locked",
				"-p",
				"rune",
				"--lib",
				"--features",
				features,
				"--no-run",
				"--message-format=json",
			])
			.env(
				"CARGO_TARGET_DIR",
				self.root_out.join("targets").join(profile),
			);
		let run = ctx.checked("cargo", &mut command, 3600)?;
		let mut binaries = Vec::new();
		for line in fs::read_to_string(&run.stdout)?.lines() {
			let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
				continue;
			};
			if value["reason"] == "compiler-artifact"
				&& value["target"]["name"] == "rune"
				&& value["profile"]["test"] == true
			{
				if let Some(path) = value["executable"].as_str() {
					binaries.push(PathBuf::from(path));
				}
			}
		}
		ensure!(
			binaries.len() == 1,
			"expected one Rune test executable, got {}",
			binaries.len()
		);
		let binary = build_dir.join("rune-tests");
		fs::copy(&binaries[0], &binary)?;
		let mut dwarf = Command::new("/usr/bin/readelf");
		dwarf
			.args(["--debug-dump=info", "--dwarf-depth=1"])
			.arg(&binary);
		let dump = ctx.checked("dwarf", &mut dwarf, 180)?;
		let units = dwarf_units(&fs::read_to_string(&dump.stdout)?, &self.repo)?;
		write_json(
			&self.binary_receipt(profile),
			&json!({"status":"PASS","binary":binary,"path":binary,
            "binary_sha256":hash_file(&binary)?,"sha256":hash_file(&binary)?,"source":self.repo,
            "expected_repo":self.repo,"units":units,"features":features,"toolchain":self.toolchain,
            "dwarf_dump":dump.stdout,"dwarf_dump_sha256":hash_file(&dump.stdout)?,"source_manifest":self.root_out.join("SOURCE.json")}),
		)?;
		self.source.verify()?;
		let receipt = self.binary_receipt(profile);
		let build = Build {
			features: features.to_owned(),
			binary: binary.clone(),
			binary_sha256: hash_file(&binary)?,
			receipt_sha256: hash_file(&receipt)?,
			receipt,
			dwarf_sha256: hash_file(&dump.stdout)?,
			dwarf: dump.stdout,
		};
		build.verify()?;
		cache.insert(profile.to_owned(), build);
		Ok(binary)
	}
}
// Resolve existing symlinks before creating any directory. Missing suffixes are
// folded component by component so `link/../out` follows filesystem semantics.
fn resolved_destination(path: &Path) -> Result<PathBuf> {
	ensure!(path.is_absolute(), "destination must be absolute");
	let mut resolved = PathBuf::new();
	for component in path.components() {
		use std::path::Component;
		match component {
			Component::RootDir => resolved.push(component.as_os_str()),
			Component::CurDir => {}
			Component::ParentDir => {
				resolved.pop();
			}
			Component::Normal(name) => {
				resolved.push(name);
				if fs::symlink_metadata(&resolved).is_ok() {
					resolved = fs::canonicalize(&resolved)?;
				}
			}
			Component::Prefix(_) => anyhow::bail!("unsupported destination prefix"),
		}
	}
	Ok(resolved)
}
fn valid_label(label: &str) -> Result<()> {
	ensure!(
		!label.is_empty()
			&& label
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
		"invalid output label: {label}"
	);
	Ok(())
}
pub fn dwarf_units(text: &str, repo: &Path) -> Result<Vec<serde_json::Value>> {
	let mut units = Vec::new();
	for block in text.split("Compilation Unit @ offset") {
		let Some(name) = block.lines().find(|line| line.contains("DW_AT_name")) else {
			continue;
		};
		if !name.contains("crates/rune/src/lib.rs") {
			continue;
		}
		let dirs: Vec<_> = block
			.lines()
			.filter(|line| line.contains("DW_AT_comp_dir"))
			.collect();
		ensure!(
			dirs.len() == 1,
			"Rune compilation unit has missing or ambiguous source directory"
		);
		let cwd = dirs[0]
			.rsplit(": ")
			.next()
			.context("invalid DWARF directory")?
			.trim();
		ensure!(
			fs::canonicalize(cwd)? == repo,
			"test executable belongs to {cwd}, expected {}",
			repo.display()
		);
		units.push(json!({"name":name.rsplit(": ").next().unwrap().trim(),"recorded_cwd":cwd}));
	}
	ensure!(!units.is_empty(), "no Rune compilation units in executable");
	Ok(units)
}
pub fn test_counts(path: &Path) -> Result<TestCounts> {
	let text = fs::read_to_string(path)?;
	parse_counts(&text)
}
pub fn parse_counts(text: &str) -> Result<TestCounts> {
	let re =
		regex::Regex::new(r"(?m)^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;")?;
	let matches: Vec<_> = re.captures_iter(text).collect();
	ensure!(
		matches.len() == 1,
		"expected one successful test summary, got {}",
		matches.len()
	);
	let m = &matches[0];
	Ok(TestCounts {
		passed: m[1].parse()?,
		failed: m[2].parse()?,
		ignored: m[3].parse()?,
	})
}
pub fn require_one_test(path: &Path) -> Result<()> {
	ensure!(
		test_counts(path)?
			== (TestCounts {
				passed: 1,
				failed: 0,
				ignored: 0
			}),
		"exactly one passing test required"
	);
	Ok(())
}
pub fn require_one_selection(path: &Path, selector: &str) -> Result<()> {
	let text = fs::read_to_string(path)?;
	let names: Vec<_> = text
		.lines()
		.filter_map(|line| line.strip_suffix(": test"))
		.collect();
	ensure!(
		names == [selector] && text.lines().any(|line| line == "1 test, 0 benchmarks"),
		"missing, zero or multiple test selections for {selector}"
	);
	Ok(())
}
#[cfg(test)]
mod tests {
	use super::*;
	struct TestTree(PathBuf);
	impl TestTree {
		fn new() -> Self {
			static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
			let path = std::env::temp_dir().join(format!(
				"rune-xtask-{}-{}",
				std::process::id(),
				NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
			));
			fs::create_dir(&path).unwrap();
			let repo = path.join("repo");
			fs::create_dir(&repo).unwrap();
			assert!(Command::new("git")
				.args(["init", "--quiet"])
				.arg(&repo)
				.status()
				.unwrap()
				.success());
			fs::write(repo.join("subject"), b"original").unwrap();
			assert!(Command::new("git")
				.current_dir(&repo)
				.args(["add", "subject"])
				.status()
				.unwrap()
				.success());
			assert!(Command::new("git")
				.current_dir(&repo)
				.args([
					"-c",
					"user.name=Test",
					"-c",
					"user.email=test@example.invalid",
					"-c",
					"commit.gpgsign=false",
					"commit",
					"--quiet",
					"-m",
					"test"
				])
				.status()
				.unwrap()
				.success());
			Self(path)
		}
		fn context(&self) -> Context {
			Context::new(
				self.0.join("repo"),
				self.0.clone(),
				self.0.join("out"),
				"1.98.1".into(),
				1,
				1,
			)
			.unwrap()
		}
	}
	impl Drop for TestTree {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}
	#[test]
	fn copied_binary_and_receipt_mutation_fail() {
		let tree = TestTree::new();
		let ctx = tree.context();
		let binary = tree.0.join("binary");
		let receipt = tree.0.join("receipt");
		let dwarf = tree.0.join("dwarf");
		for path in [&binary, &receipt, &dwarf] {
			fs::write(path, b"original").unwrap();
		}
		let build = Build {
			features: "test".into(),
			binary_sha256: hash_file(&binary).unwrap(),
			binary: binary.clone(),
			receipt_sha256: hash_file(&receipt).unwrap(),
			receipt: receipt.clone(),
			dwarf_sha256: hash_file(&dwarf).unwrap(),
			dwarf: dwarf.clone(),
		};
		ctx.builds.lock().unwrap().insert("test".into(), build);
		assert!(ctx.verify_binaries().is_ok());
		for path in [&binary, &receipt, &dwarf] {
			fs::write(path, b"changed").unwrap();
			assert!(ctx.verify_binaries().is_err());
			fs::write(path, b"original").unwrap();
		}
		assert!(ctx.verify_binaries().is_ok());
	}
	#[test]
	fn dwarf_requires_expected_source_directory() {
		let tree = TestTree::new();
		let repo = fs::canonicalize(tree.0.join("repo")).unwrap();
		let unit = |cwd: &Path| {
			format!("Compilation Unit @ offset 0:\n DW_AT_name : crates/rune/src/lib.rs\n DW_AT_comp_dir : {}\n", cwd.display())
		};
		assert!(dwarf_units(&unit(&repo), &repo).is_ok());
		assert!(dwarf_units(&unit(&tree.0), &repo).is_err());
		assert!(dwarf_units("", &repo).is_err());
	}

	#[test]
	fn source_mutation_and_output_reuse_fail() {
		let tree = TestTree::new();
		let ctx = tree.context();
		assert!(ctx.verify_source().is_ok());
		fs::write(ctx.repo.join("subject"), b"changed").unwrap();
		assert!(ctx.verify_source().is_err());
		assert!(Context::new(
			ctx.repo.clone(),
			tree.0.clone(),
			ctx.out.clone(),
			"1.98.1".into(),
			1,
			1
		)
		.is_err());
		assert!(write_json(&ctx.out.join("SOURCE.json"), &json!({})).is_err());
	}
	#[cfg(unix)]
	#[test]
	fn symlink_output_cannot_create_inside_source() {
		let tree = TestTree::new();
		std::os::unix::fs::symlink(tree.0.join("repo"), tree.0.join("link")).unwrap();
		let result = Context::new(
			tree.0.join("repo"),
			tree.0.clone(),
			tree.0.join("link/new/out"),
			"1.98.1".into(),
			1,
			1,
		);
		assert!(result.is_err());
		assert!(!tree.0.join("repo/new").exists());
	}
	#[test]
	fn failure_timeout_and_receipt_immutability() {
		let tree = TestTree::new();
		let ctx = tree.context();
		let run = ctx
			.run(
				"failure",
				Command::new("/bin/sh").args(["-c", "printf evidence; exit 7"]),
				5,
			)
			.unwrap();
		assert_eq!(run.code, 7);
		assert_eq!(fs::read(&run.stdout).unwrap(), b"evidence");
		assert!(ctx
			.run("failure", &mut Command::new("/bin/true"), 5)
			.is_err());
		let run = ctx
			.run("timeout", Command::new("/bin/sleep").arg("10"), 1)
			.unwrap();
		assert_eq!(run.code, 124);
		let receipt: serde_json::Value = read_json(&run.receipt).unwrap();
		assert_eq!(receipt["code"], 124);
		assert!(receipt["environment"]
			.as_array()
			.unwrap()
			.iter()
			.any(|entry| entry[0] == json!(OsString::from("TZ"))
				&& entry[1] == json!(OsString::from("UTC"))));
	}

	#[test]
	fn zero_and_duplicate_summaries_cannot_pass() {
		assert_eq!(
			parse_counts("test result: ok. 0 passed; 0 failed; 0 ignored;\n")
				.unwrap()
				.passed,
			0
		);
		assert!(parse_counts("").is_err());
		assert!(parse_counts("test result: ok. 1 passed; 0 failed; 0 ignored;\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n").is_err());
		assert!(parse_counts("test result: FAILED. 0 passed; 1 failed; 0 ignored;\n").is_err());
	}
	#[test]
	fn labels_cannot_escape_output_directory() {
		for label in ["", "../x", "a/b", "/tmp/x", "x y"] {
			assert!(valid_label(label).is_err());
		}
		assert!(valid_label("case-0042").is_ok());
	}
	#[test]
	fn known_digest_is_stable() {
		assert_eq!(
			hash_bytes(b"abc"),
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
		);
	}
}
