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
			let unit = crate::prepare(&mut sources)
				.with_context(&context)
				.with_diagnostics(&mut diagnostics)
				.build();
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
