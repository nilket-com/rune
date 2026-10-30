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
	sync::{Arc, Mutex, OnceLock},
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
	builds: Arc<Mutex<BTreeMap<String, Arc<BuildSlot>>>>,
	aliases: Arc<Mutex<BTreeMap<String, String>>>,
	pub budget: Arc<crate::scheduler::Budget>,
	native_events: Arc<Mutex<Vec<serde_json::Value>>>,
	source: Arc<Source>,
}
struct BuildSlot {
	value: OnceLock<std::result::Result<Build, String>>,
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
			test_threads: test_threads.min(crate::hosts::available_cpus()?.min(jobs)),
			builds: Arc::new(Mutex::new(BTreeMap::new())),
			aliases: Arc::new(Mutex::new(BTreeMap::new())),
			budget: crate::scheduler::Budget::new(crate::hosts::available_cpus()?.min(jobs))?,
			native_events: Arc::new(Mutex::new(Vec::new())),
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
		let slots: Vec<_> = cache.values().cloned().collect();
		drop(cache);
		for slot in slots {
			if let Some(Ok(build)) = slot.value.get() {
				build.verify()?;
			}
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
		self.run_with_admission(label, command, timeout_secs, || None, None)
	}

	pub fn run_with_deadline(
		&self,
		label: &str,
		command: &mut Command,
		timeout_secs: f64,
		deadline: impl FnOnce() -> Instant,
	) -> Result<Run> {
		self.run_with_admission(label, command, timeout_secs, || Some(deadline()), None)
	}
	pub fn run_exclusive(
		&self,
		label: &str,
		command: &mut Command,
		timeout_secs: u64,
	) -> Result<Run> {
		self.run_with_admission(
			label,
			command,
			timeout_secs as f64,
			|| None,
			Some(self.budget.capacity()),
		)
	}
	fn run_with_admission(
		&self,
		label: &str,
		command: &mut Command,
		timeout_secs: f64,
		deadline: impl FnOnce() -> Option<Instant>,
		reservation: Option<usize>,
	) -> Result<Run> {
		valid_label(label)?;
		ensure!(
			timeout_secs.is_finite() && timeout_secs > 0.0,
			"timeout must be finite and positive"
		);
		let args_for_budget: Vec<_> = command.get_args().map(|x| x.to_string_lossy()).collect();
		let cargo = command.get_program().to_string_lossy().ends_with("cargo")
			|| args_for_budget
				.iter()
				.any(|x| x.ends_with("/cargo") || x == "cargo");
		let slots = reservation.unwrap_or_else(|| {
			if cargo {
				1
			} else {
				args_for_budget
					.iter()
					.find_map(|x| {
						x.strip_prefix("--test-threads=")
							.and_then(|n| n.parse::<usize>().ok())
					})
					.unwrap_or(1)
					.min(self.budget.capacity())
			}
		});
		let adaptive = reservation.is_none() && !cargo && slots > 1;
		let permit = if adaptive {
			self.budget.acquire_up_to(label, slots)?
		} else {
			self.budget.acquire(label, slots)?
		};
		let effective_slots = permit.slots();
		let timeout_secs = if let Some(deadline) = deadline() {
			timeout_secs.min(
				deadline
					.saturating_duration_since(Instant::now())
					.as_secs_f64(),
			)
		} else {
			timeout_secs
		};
		ensure!(
			timeout_secs > 0.0,
			"job deadline expired while waiting for CPU slots: {label}"
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
		let args: Vec<OsString> = command
			.get_args()
			.map(|arg| {
				if adaptive && arg.to_string_lossy().starts_with("--test-threads=") {
					OsString::from(format!("--test-threads={effective_slots}"))
				} else {
					arg.into()
				}
			})
			.collect();
		let environment: Vec<(OsString, Option<OsString>)> = command
			.get_envs()
			.map(|(k, v)| (k.to_os_string(), v.map(Into::into)))
			.collect();
		let cwd = command
			.get_current_dir()
			.unwrap_or(&self.repo)
			.to_path_buf();
		let resources = self.out.join(format!("{label}.resources"));
		ensure!(!resources.exists(), "resource receipt already exists");
		let native = program.to_string_lossy().ends_with("rune-tests")
			|| args
				.iter()
				.any(|arg| arg.to_string_lossy().ends_with("/rune-tests"));
		let trace_prefix = self.out.join(format!("{label}.native.trace"));
		// time collects kernel resource usage only after wait; timeout owns the process group.
		let mut child = Command::new("/usr/bin/time");
		child
			.args(["-q", "-f", "%M %U %S", "-o"])
			.arg(&resources)
			.arg("/usr/bin/timeout");
		child.env_clear();
		for key in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "TMPDIR"] {
			if let Some(value) = std::env::var_os(key) {
				child.env(key, value);
			}
		}
		child
			.args(["--signal=TERM", "--kill-after=5s"])
			.arg(format!("{timeout_secs}s"));
		if native {
			child
				.args([
					"/usr/bin/strace",
					"--seccomp-bpf",
					"-q",
					"-ff",
					"-ttt",
					"-e",
					"trace=execve,exit_group",
					"-o",
				])
				.arg(&trace_prefix);
		}
		child
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
		if cargo {
			child.env_remove("CARGO_BUILD_JOBS");
			self.budget.configure(&mut child);
		}
		let effective_environment: Vec<(OsString, OsString)> = child
			.get_envs()
			.filter_map(|(key, value)| {
				value.map(|value| (key.to_os_string(), value.to_os_string()))
			})
			.collect();
		let start = Instant::now();
		let epoch = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
		let (result, pid) = match child.spawn() {
			Ok(mut process) => {
				let pid = process.id();
				(process.wait(), Some(pid))
			}
			Err(error) => (Err(error), None),
		};
		let seconds = start.elapsed().as_secs_f64();
		let (code, spawn_error) = match result {
			Ok(status) => (status.code().unwrap_or(-1), None),
			Err(error) => (-1, Some(error.to_string())),
		};
		if cargo && code != 0 {
			self.budget.abort();
		}
		let resource_usage=fs::read_to_string(&resources).ok().and_then(|text|{
			let fields:Vec<_>=text.lines().last()?.split_whitespace().collect();
			if fields.len()!=3{return None;}
			Some(json!({"peak_individual_rss_kib":fields[0].parse::<u64>().ok()?,"user_seconds":fields[1].parse::<f64>().ok()?,"system_seconds":fields[2].parse::<f64>().ok()?}))
		});
		if code == 0 {
			ensure!(
				resource_usage.is_some(),
				"missing terminal resource observation: {label}"
			);
		}
		let native_intervals = if native {
			crate::miri::native_intervals(&self.out, label)?
		} else {
			Vec::new()
		};
		self.native_events
			.lock()
			.map_err(|_| anyhow::anyhow!("native event lock poisoned"))?
			.extend(native_intervals.iter().cloned());
		write_json(
			&receipt,
			&json!({"schema":1,"requested_cpu_slots":slots,"effective_cpu_slots":effective_slots,"pid":pid,"resources":resources,"resource_usage":resource_usage,"native_intervals":native_intervals,"program":program,"args":args,"cwd":cwd,
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
	pub fn native_intervals(&self) -> Result<Vec<serde_json::Value>> {
		Ok(self
			.native_events
			.lock()
			.map_err(|_| anyhow::anyhow!("native event lock poisoned"))?
			.clone())
	}
	pub fn binary_receipt(&self, profile: &str) -> PathBuf {
		let aliases = self.aliases.lock().expect("build alias lock poisoned");
		self.root_out
			.join("build")
			.join(aliases.get(profile).map(String::as_str).unwrap_or(profile))
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
		let key = hash_bytes(&serde_json::to_vec(&json!({
			"source":self.source.files,"repo":self.repo,"toolchain":self.toolchain,
			"features":features,"package":"rune","lib":true,"debug":2,"incremental":false,
			"target":std::env::consts::ARCH,"profile":"test"
		}))?);
		{
			let mut aliases = self
				.aliases
				.lock()
				.map_err(|_| anyhow::anyhow!("build alias lock poisoned"))?;
			if let Some(old) = aliases.get(profile) {
				ensure!(
					old == &key,
					"profile reused with different inputs: {profile}"
				);
			}
			aliases.insert(profile.to_owned(), key.clone());
		}
		let slot = {
			let mut cache = self
				.builds
				.lock()
				.map_err(|_| anyhow::anyhow!("build cache poisoned"))?;
			cache
				.entry(key.clone())
				.or_insert_with(|| {
					Arc::new(BuildSlot {
						value: OnceLock::new(),
					})
				})
				.clone()
		};
		let result = slot.value.get_or_init(|| {
			self.build_rune_once(profile, features, &key)
				.map_err(|e| format!("{e:#}"))
		});
		let build = result.as_ref().map_err(|e| anyhow::anyhow!(e.clone()))?;
		ensure!(build.features == features, "build feature identity changed");
		build.verify()?;
		Ok(build.binary.clone())
	}
	fn build_rune_once(&self, profile: &str, features: &str, key: &str) -> Result<Build> {
		self.source.verify()?;
		let build_dir = self.root_out.join("build").join(key);
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
			.env("CARGO_TARGET_DIR", self.root_out.join("targets").join(key));
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
		Ok(build)
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
		let slot = BuildSlot {
			value: OnceLock::new(),
		};
		assert!(slot.value.set(Ok(build)).is_ok());
		ctx.builds
			.lock()
			.unwrap()
			.insert("test".into(), Arc::new(slot));
		assert!(ctx.verify_binaries().is_ok());
		for path in [&binary, &receipt, &dwarf] {
			fs::write(path, b"changed").unwrap();
			assert!(ctx.verify_binaries().is_err());
			fs::write(path, b"original").unwrap();
		}
		assert!(ctx.verify_binaries().is_ok());
	}
	#[test]
	fn admitted_native_threads_match_reserved_capacity() {
		let tree = TestTree::new();
		let mut ctx = tree.context();
		ctx.budget = crate::scheduler::Budget::new(3).unwrap();
		let interpreter = ctx.budget.acquire("interpreter", 1).unwrap();
		let mut command = Command::new("/bin/echo");
		command.arg("--test-threads=3");
		let run = ctx.checked("adaptive-native", &mut command, 30).unwrap();
		assert_eq!(
			fs::read_to_string(run.stdout).unwrap(),
			"--test-threads=2\n"
		);
		let receipt: serde_json::Value = read_json(&run.receipt).unwrap();
		assert_eq!(receipt["requested_cpu_slots"], 3);
		assert_eq!(receipt["effective_cpu_slots"], 2);
		drop(interpreter);
		let all = ctx.budget.acquire("all", 3).unwrap();
		assert_eq!(all.slots(), 3);
	}

	#[test]
	fn cargo_inherits_jobserver_through_timeout_and_resource_wrapper() {
		use std::os::unix::fs::PermissionsExt;
		let tree = TestTree::new();
		let mut ctx = tree.context();
		ctx.budget = crate::scheduler::Budget::new(2).unwrap();
		let program = tree.0.join("cargo");
		fs::write(&program,"#!/bin/sh\nexec \"$V4_PROBE\" --exact scheduler::tests::jobserver_child --ignored --nocapture --test-threads=1\n").unwrap();
		fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
		let mut command = Command::new(program);
		command
			.env("V4_PROBE", std::env::current_exe().unwrap())
			.env("V4_JOBSERVER_CHILD", "1");
		let run = ctx.checked("shared-client", &mut command, 30).unwrap();
		assert!(fs::read_to_string(run.stdout)
			.unwrap()
			.contains("SHARED_JOBSERVER_OK"));
		let receipt: serde_json::Value = read_json(&run.receipt).unwrap();
		assert!(receipt["resource_usage"]["peak_individual_rss_kib"].is_u64());
		// A child that exits without RAII cleanup loses its borrowed token.
		// Failure must cancel blocked/new launches, rather than waiting forever.
		command.env("V4_EXIT_WITH_TOKEN", "1");
		let failed = ctx.run("lost-child-token", &mut command, 30).unwrap();
		assert_eq!(failed.code, 7);
		assert!(ctx.budget.acquire("after-failed-cargo", 1).is_err());
	}

	#[test]
	fn parent_deadline_starts_after_cpu_admission() {
		let tree = TestTree::new();
		let ctx = tree.context();
		let held = ctx.budget.acquire("existing", 1).unwrap();
		let (deadline_tx, deadline_rx) = std::sync::mpsc::channel();
		std::thread::scope(|scope| {
			let worker = scope.spawn(|| {
				ctx.run_with_deadline("admitted", &mut Command::new("/bin/true"), 5.0, || {
					deadline_tx.send(()).unwrap();
					Instant::now() + std::time::Duration::from_secs(5)
				})
			});
			assert!(matches!(
				deadline_rx.recv_timeout(std::time::Duration::from_millis(25)),
				Err(std::sync::mpsc::RecvTimeoutError::Timeout)
			));
			drop(held);
			assert_eq!(worker.join().unwrap().unwrap().code, 0);
			deadline_rx.recv().unwrap();
		});
	}

	#[test]
	fn actual_sandbox_wrappers_produce_one_native_exec_interval() {
		use std::os::unix::fs::PermissionsExt;
		let tree = TestTree::new();
		let ctx = tree.context();
		let binary = tree.0.join("rune-tests");
		fs::write(&binary, "#!/bin/sh\nprintf NATIVE_WRAPPER_OK\n").unwrap();
		fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
		let mut command = Command::new("/usr/bin/prlimit");
		command
			.args([
				"--core=0",
				"--",
				"/usr/bin/bwrap",
				"--ro-bind",
				"/",
				"/",
				"--",
			])
			.arg(&binary);
		let run = ctx.checked("sandbox-exec", &mut command, 10).unwrap();
		assert_eq!(
			fs::read_to_string(&run.stdout).unwrap(),
			"NATIVE_WRAPPER_OK"
		);
		let receipt: serde_json::Value = read_json(&run.receipt).unwrap();
		assert_eq!(receipt["native_intervals"].as_array().unwrap().len(), 1);
		assert_eq!(ctx.native_intervals().unwrap().len(), 1);
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
