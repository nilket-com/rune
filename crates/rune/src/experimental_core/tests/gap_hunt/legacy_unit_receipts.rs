//! Operator-only, lossless stock Unit snapshots for receiver-proof invariants.
use super::*;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::io::Write;

fn digest(bytes: &[u8]) -> std::string::String {
	Sha256::digest(bytes)
		.iter()
		.map(|byte| std::format!("{byte:02x}"))
		.collect()
}

// This module is test-only and serde+musli gated. Historical source paths are
// observation data; filesystem reads must use the explicit relocated archive.
struct RelocatedSourceLoader {
	logical_root: PathBuf,
	actual_root: PathBuf,
}

impl RelocatedSourceLoader {
	fn actual(&self, logical: &Path) -> std::io::Result<PathBuf> {
		use std::path::Component;
		let relative = logical.strip_prefix(&self.logical_root).map_err(|_| {
			std::io::Error::new(
				std::io::ErrorKind::PermissionDenied,
				"module outside fixture mapping",
			)
		})?;
		if relative
			.components()
			.any(|c| !matches!(c, Component::Normal(_)))
		{
			return Err(std::io::Error::new(
				std::io::ErrorKind::PermissionDenied,
				"invalid fixture module path",
			));
		}
		let mut actual = self.actual_root.clone();
		for part in relative.components() {
			actual.push(part);
			match fs::symlink_metadata(&actual) {
				Ok(metadata) if metadata.file_type().is_symlink() => {
					return Err(std::io::Error::new(
						std::io::ErrorKind::PermissionDenied,
						"fixture module symlink forbidden",
					))
				}
				Ok(_) => (),
				// FileSourceLoader's is_file treats stat failures as a missing
				// candidate. Preserve that behavior, including NotADirectory.
				Err(_) => (),
			}
		}
		Ok(actual)
	}
}

impl crate::compile::SourceLoader for RelocatedSourceLoader {
	fn load(
		&mut self,
		sources: &Sources,
		id: crate::SourceId,
		item: &crate::Item,
		span: &dyn crate::ast::Spanned,
	) -> crate::compile::Result<Source> {
		use crate::alloc::prelude::*;
		use crate::compile::{Error, ErrorKind};
		use crate::item::ComponentRef;
		let Some(base) = sources.path(id) else {
			return Err(Error::new(span, ErrorKind::SourceWithoutPath));
		};
		let mut base = base.try_to_owned()?;
		if !base.pop() {
			return Err(Error::new(
				span,
				ErrorKind::UnsupportedModuleRoot {
					root: base.try_to_owned()?,
				},
			));
		}
		for component in item {
			if let ComponentRef::Str(name) = component {
				base.push(name);
			} else {
				return Err(Error::new(
					span,
					ErrorKind::UnsupportedModuleItem {
						item: item.try_to_owned()?,
					},
				));
			}
		}
		let candidates = [base.join("mod.rn"), base.with_extension("rn")];
		for logical in &candidates {
			let actual = self.actual(logical).map_err(|error| {
				Error::new(
					span,
					ErrorKind::SourceError {
						path: logical.clone(),
						error: error.into(),
					},
				)
			})?;
			if !actual.is_file() {
				continue;
			}
			let loaded = Source::from_path(&actual).map_err(|error| {
				Error::new(
					span,
					ErrorKind::SourceError {
						path: logical.clone(),
						error,
					},
				)
			})?;
			return Ok(Source::with_path(
				logical.to_string_lossy(),
				loaded.as_str(),
				logical,
			)?);
		}
		Err(Error::new(span, ErrorKind::ModNotFound { path: base }))
	}
}

fn relocated_loader() -> Option<RelocatedSourceLoader> {
	match (
		std::env::var_os("GAP_FIXTURE_ROOT"),
		std::env::var_os("GAP_LOGICAL_ROOT"),
	) {
		(None, None) => None,
		(Some(actual_root), Some(logical_root)) => Some(RelocatedSourceLoader {
			actual_root: actual_root.into(),
			logical_root: logical_root.into(),
		}),
		_ => panic!("both fixture mapping roots are required"),
	}
}

fn inputs() -> (Sources, Json) {
	if let Some(name) = std::env::var_os("C18E_AWFY_CASE") {
		let name = name.to_str().unwrap();
		assert!([
			"bounce",
			"cd",
			"havlak",
			"json",
			"list",
			"mandelbrot",
			"nbody",
			"permute",
			"queens",
			"sieve",
			"storage",
			"towers",
		]
		.contains(&name));
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benches/awfy");
		let mut files = std::vec![
			std::string::String::from("harness.rn"),
			std::string::String::from("benchmark.rn")
		];
		if matches!(name, "bounce" | "cd" | "havlak" | "json" | "storage") {
			files.push(std::string::String::from("som.rn"));
		}
		files.push(std::format!("{name}.rn"));
		let mut sources = Sources::new();
		let mut pins = std::vec::Vec::new();
		for file in files {
			let text = fs::read_to_string(root.join(&file)).unwrap();
			let label = std::format!("awfy/{file}");
			pins.push(json!({"name":label,"sha256":digest(text.as_bytes())}));
			sources
				.insert(crate::Source::new(label, &text).unwrap())
				.unwrap();
		}
		return (sources, json!({"awfy":name,"sources":pins}));
	}
	let path = std::env::var_os("GAP_CASE").expect("GAP_CASE required");
	let text = fs::read(&path).unwrap();
	(
		sources(Path::new(&path), "compile"),
		json!({"source_sha256":digest(&text)}),
	)
}

#[test]
#[ignore = "operator-only stock Unit capture; requires a new GAP_RESULT path"]
fn legacy_unit_child() {
	let result = std::env::var_os("GAP_RESULT").expect("GAP_RESULT required");
	let (observation, bytes) = std::thread::Builder::new()
		.stack_size(16 << 20)
		.spawn(|| {
			let context = Context::with_default_modules().unwrap();
			let (mut sources, input) = inputs();
			let mut diagnostics = Diagnostics::new();
			let mut loader = relocated_loader();
			let mut prepare = crate::prepare(&mut sources)
				.with_context(&context)
				.with_diagnostics(&mut diagnostics);
			if let Some(loader) = loader.as_mut() {
				prepare = prepare.with_source_loader(loader);
			}
			let unit = prepare.build();
			match unit {
				Ok(unit) => {
					// Binary encoding preserves all float bits; JSON is metadata only.
					let bytes = musli::storage::to_vec(&unit).unwrap();
					(json!({"input":input,"status":"compiled","unit_sha256":digest(&bytes),"unit_bytes":bytes.len(),"encoding":"musli-storage-binary"}), Some(bytes))
				}
				Err(error) => (json!({"input":input,"status":"compile-error","error":std::format!("{error:?}"),"diagnostics":std::format!("{:?}",diagnostics.diagnostics())}), None),
			}
		})
		.unwrap()
		.join()
		.unwrap();
	if let Some(bytes) = bytes {
		let path = Path::new(&result).with_extension("unit");
		std::fs::OpenOptions::new()
			.create_new(true)
			.write(true)
			.open(path)
			.unwrap()
			.write_all(&bytes)
			.unwrap();
	}
	std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(result)
		.unwrap()
		.write_all(
			serde_json::to_string_pretty(&observation)
				.unwrap()
				.as_bytes(),
		)
		.unwrap();
	std::process::exit(0);
}

#[test]
fn relocated_modules_preserve_logical_identity_and_never_fall_back() {
	use crate::compile::SourceLoader;
	let root =
		std::env::temp_dir().join(std::format!("rune-unit-relocation-{}", std::process::id()));
	fs::create_dir(&root).unwrap();
	let logical = root.join("old");
	let actual = root.join("archive");
	fs::create_dir(&logical).unwrap();
	fs::create_dir(&actual).unwrap();
	fs::write(logical.join("dep.rn"), "OLD HOST MUST NOT BE LOADED").unwrap();
	fs::write(actual.join("dep.rn"), "pub fn answer() {42}").unwrap();
	let mut sources = Sources::new();
	let id = sources
		.insert(Source::with_path("gap.rn", "mod dep;", logical.join("main.rn")).unwrap())
		.unwrap();
	let item = crate::ItemBuf::with_item(["dep"]).unwrap();
	let mut loader = RelocatedSourceLoader {
		logical_root: logical.clone(),
		actual_root: actual.clone(),
	};
	let loaded = loader
		.load(&sources, id, &item, &crate::ast::Span::empty())
		.unwrap();
	assert_eq!(loaded.as_str(), "pub fn answer() {42}");
	assert_eq!(loaded.path(), Some(logical.join("dep.rn").as_path()));
	assert_eq!(loaded.name(), logical.join("dep.rn").to_string_lossy());
	fs::create_dir(actual.join("dep")).unwrap();
	fs::write(actual.join("dep/mod.rn"), "pub fn directory_first() {7}").unwrap();
	let directory = loader
		.load(&sources, id, &item, &crate::ast::Span::empty())
		.unwrap();
	assert_eq!(directory.as_str(), "pub fn directory_first() {7}");
	assert_eq!(directory.path(), Some(logical.join("dep/mod.rn").as_path()));
	fs::remove_dir_all(actual.join("dep")).unwrap();
	fs::remove_file(actual.join("dep.rn")).unwrap();
	let error = match loader.load(&sources, id, &item, &crate::ast::Span::empty()) {
		Err(error) => error,
		Ok(_) => panic!("missing relocated module loaded from old root"),
	};
	let expected = crate::compile::Error::new(
		crate::ast::Span::empty(),
		crate::compile::ErrorKind::ModNotFound {
			path: logical.join("dep"),
		},
	);
	assert_eq!(std::format!("{error:?}"), std::format!("{expected:?}"));
	assert!(loader.actual(&logical.join("../escape.rn")).is_err());
	assert!(loader.actual(&root.join("unmapped.rn")).is_err());
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(logical.join("dep.rn"), actual.join("dep.rn")).unwrap();
		assert!(loader
			.load(&sources, id, &item, &crate::ast::Span::empty())
			.is_err());
	}
	fs::remove_dir_all(root).unwrap();
}
