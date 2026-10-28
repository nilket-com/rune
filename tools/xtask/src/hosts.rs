//! Explicit local host description. No machine identities are built into xtask.
use crate::common::{hash_bytes, Result};
use anyhow::{ensure, Context as _};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
	collections::BTreeSet,
	fs,
	path::{Path, PathBuf},
};

/// Linux exposes the effective affinity directly; the standard library may also
/// apply a cgroup quota, which is a different resource from a CPU set.
#[cfg(target_os = "linux")]
pub fn available_cpus() -> Result<usize> {
	unsafe extern "C" {
		fn sched_getaffinity(pid: i32, size: usize, mask: *mut usize) -> i32;
	}
	let mut words = 16usize;
	loop {
		let mut mask = vec![0usize; words];
		// The buffer is aligned, writable and valid for the supplied byte count.
		let rc = unsafe {
			sched_getaffinity(
				0,
				mask.len() * std::mem::size_of::<usize>(),
				mask.as_mut_ptr(),
			)
		};
		if rc == 0 {
			let count = mask.iter().map(|word| word.count_ones() as usize).sum();
			ensure!(count > 0, "process has an empty CPU affinity");
			return Ok(count);
		}
		let error = std::io::Error::last_os_error();
		if error.raw_os_error() != Some(22) || words >= 16384 {
			return Err(error).context("sched_getaffinity failed");
		}
		words *= 2;
	}
}
#[cfg(not(target_os = "linux"))]
pub fn available_cpus() -> Result<usize> {
	Ok(std::thread::available_parallelism()?.get())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hosts {
	pub schema: u32,
	pub correctness: Option<Host>,
	pub measurement: Option<Host>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
	pub endpoint: Option<String>,
	pub cpus: Option<Vec<usize>>,
	pub memory_bytes: Option<u64>,
	pub measurement_cpu: Option<usize>,
	pub measurement_lock: Option<PathBuf>,
	pub measurement_max_load: Option<f64>,
	pub measurement_min_available_bytes: Option<u64>,
	pub replay_root: Option<PathBuf>,
	pub rustc_identity: Option<String>,
	pub linker_identity: Option<String>,
	pub valgrind_path: Option<PathBuf>,
	pub valgrind_root: Option<PathBuf>,
	pub valgrind_identity: Option<String>,
}

impl Hosts {
	fn validate(&self) -> Result<()> {
		ensure!(self.schema == 1, "unsupported host configuration schema");
		for host in [&self.correctness, &self.measurement].into_iter().flatten() {
			if let Some(cpus) = &host.cpus {
				let unique: BTreeSet<_> = cpus.iter().copied().collect();
				ensure!(
					!cpus.is_empty() && unique.len() == cpus.len(),
					"CPU list must be nonempty and unique"
				);
				if let Some(cpu) = host.measurement_cpu {
					ensure!(
						unique.contains(&cpu),
						"measurement CPU is outside host CPU list"
					);
				}
			}
			ensure!(
				host.memory_bytes != Some(0),
				"memory budget must be positive"
			);
			if let Some(limit) = host.measurement_max_load {
				ensure!(
					limit.is_finite() && limit > 0.0,
					"measurement load limit must be positive"
				);
			}
			ensure!(
				host.measurement_min_available_bytes != Some(0),
				"measurement available memory limit must be positive"
			);
			for path in [
				&host.measurement_lock,
				&host.replay_root,
				&host.valgrind_path,
				&host.valgrind_root,
			]
			.into_iter()
			.flatten()
			{
				ensure!(path.is_absolute(), "host paths must be absolute");
			}
			for identity in [
				&host.rustc_identity,
				&host.linker_identity,
				&host.valgrind_identity,
			]
			.into_iter()
			.flatten()
			{
				ensure!(!identity.is_empty(), "tool identity must not be empty");
			}
			if let Some(endpoint) = &host.endpoint {
				ensure!(
					!endpoint.is_empty()
						&& !endpoint.starts_with('-')
						&& !endpoint.chars().any(char::is_whitespace),
					"invalid SSH endpoint"
				);
			}
		}
		Ok(())
	}
}

pub struct Loaded {
	pub receipt: Value,
	pub config: Hosts,
}

fn default_path() -> Option<PathBuf> {
	let base = std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.filter(|p| p.is_absolute())
		.or_else(|| {
			std::env::var_os("HOME")
				.map(PathBuf::from)
				.filter(|p| p.is_absolute())
				.map(|p| p.join(".config"))
		})?;
	Some(base.join("rune-verify/hosts.json"))
}

pub fn load(explicit: Option<&Path>) -> Result<Option<Loaded>> {
	let path = match explicit {
		Some(p) => p.to_owned(),
		None => match default_path() {
			Some(p) if p.exists() => p,
			_ => return Ok(None),
		},
	};
	let path =
		fs::canonicalize(&path).with_context(|| format!("host config {}", path.display()))?;
	let bytes = fs::read(&path)?;
	let contents = std::str::from_utf8(&bytes).context("host configuration must be UTF-8")?;
	let config: Hosts = serde_json::from_str(contents).context("invalid host configuration")?;
	config.validate()?;
	Ok(Some(Loaded {
		receipt: json!({"path": path, "sha256": hash_bytes(&bytes), "contents": contents}),
		config,
	}))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn closed_schema_and_resource_validation() {
		let good = r#"{"schema":1,"measurement":{"cpus":[2,3],"measurement_cpu":3,"measurement_lock":"/tmp/lock"}}"#;
		let config: Hosts = serde_json::from_str(good).unwrap();
		assert!(config.validate().is_ok());
		let bad = good.replace("\"measurement_cpu\":3", "\"measurement_cpu\":4");
		assert!(serde_json::from_str::<Hosts>(&bad)
			.unwrap()
			.validate()
			.is_err());
		let extra = good.replace("\"schema\":1", "\"schema\":1,\"unexpected\":true");
		assert!(serde_json::from_str::<Hosts>(&extra).is_err());
		let duplicate = good.replace("[2,3]", "[2,2]");
		assert!(serde_json::from_str::<Hosts>(&duplicate)
			.unwrap()
			.validate()
			.is_err());
		let limits = good.replace("\"measurement_cpu\":3", "\"measurement_cpu\":3,\"measurement_max_load\":1.5,\"measurement_min_available_bytes\":4096");
		assert!(serde_json::from_str::<Hosts>(&limits)
			.unwrap()
			.validate()
			.is_ok());
		let bad_limit =
			limits.replace("\"measurement_max_load\":1.5", "\"measurement_max_load\":0");
		assert!(serde_json::from_str::<Hosts>(&bad_limit)
			.unwrap()
			.validate()
			.is_err());
	}
}
