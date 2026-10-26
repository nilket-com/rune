use super::*;

pub(super) fn cases(ctx: &Context, m: &Manifest) -> Result<Vec<Case>> {
	let rows: Vec<Case> = serde_json::from_value(read(&ctx.fixtures.join(&m.l1_records))?)?;
	require_count(rows.len(), m.counts["l1"], "L1")?;
	let mut seen = std::collections::BTreeSet::new();
	for c in &rows {
		ensure!(seen.insert(&c.name), "duplicate L1 row {}", c.name);
	}
	Ok(rows)
}
/// Frozen indices retain historical logical source names. Resolve only the
/// suffix after the named archive anchor beneath the explicit fixtures root;
/// never use the recorded absolute host path for filesystem access.
fn fixture_relative(logical: &str, anchor: &str) -> Result<PathBuf> {
	use std::path::Component;
	ensure!(
		!anchor.is_empty()
			&& Path::new(anchor).components().count() == 1
			&& matches!(
				Path::new(anchor).components().next(),
				Some(Component::Normal(_))
			),
		"invalid fixture archive anchor"
	);
	let components: Vec<_> = Path::new(logical).components().collect();
	let anchors: Vec<_> = components
		.iter()
		.enumerate()
		.filter(
			|(_, part)| matches!(part,Component::Normal(name) if *name==std::ffi::OsStr::new(anchor)),
		)
		.map(|(i, _)| i)
		.collect();
	ensure!(
		anchors.len() == 1,
		"historical logical source must contain exactly one fixture anchor"
	);
	let suffix = &components[anchors[0] + 1..];
	ensure!(
		!suffix.is_empty()
			&& suffix
				.iter()
				.all(|part| matches!(part, Component::Normal(_))),
		"invalid relative historical fixture path"
	);
	Ok(suffix.iter().collect())
}
pub(super) fn source(ctx: &Context, m: &Manifest, c: &Case) -> Result<PathBuf> {
	let relative = fixture_relative(&c.source, &m.historical_anchor_component)?;
	let path = ctx.fixtures.join(relative);
	ensure!(
		common::hash_file(&path)? == c.source_sha256,
		"input hash changed: {}",
		path.display()
	);
	Ok(path)
}

fn require_l1_stdout(output: &[u8]) -> Result<()> {
	ensure!(
		std::str::from_utf8(output)?.trim() == "running 1 test",
		"L1 observer stdout must contain only the single-test marker"
	);
	Ok(())
}
pub(super) fn l1(ctx: &Context, m: &Manifest, binary: &Path) -> Result<Value> {
	let rows = cases(ctx, m)?;
	let root = ctx.out.join("records");
	fs::create_dir(&root)?;
	let baseline = ctx
		.fixtures
		.join(&m.l1_records)
		.parent()
		.context("baseline parent")?
		.to_path_buf();
	let mut admitted = 0;
	for c in &rows {
		let input = source(ctx, m, c)?;
		let dir = root.join(&c.name);
		let dest = dir.join("emission.code");
		let old = baseline.join(format!("{}.code", c.name));
		ensure!(
			common::hash_file(&old)? == c.sha256,
			"L1 baseline hash changed: {}",
			c.name
		);
		child(
			ctx,
			binary,
			&dir,
			"experimental_core::tests::gap_hunt::emission_child",
			&[
				("GAP_CASE", input.display().to_string()),
				("GAP_ORIGIN", input.display().to_string()),
				("GAP_RESULT", dest.display().to_string()),
			],
			20,
			false,
		)?;
		require_l1_stdout(&fs::read(dir.join("stdout"))?)?;
		ensure!(fs::metadata(&dest)?.len() > 0, "empty emission: {}", c.name);
		if fs::read(&old)?.starts_with(b"functions=") {
			admitted += 1;
		}
		exact(&old, &dest)?;
	}
	ensure!(
		admitted == m.counts["l1_admitted"],
		"L1 admission count changed"
	);
	let inherited = read(&ctx.fixtures.join(&m.l1_classification))?;
	Ok(
		json!({"cases":rows.len(),"old_admissions":admitted,"old_admissions_identical":admitted,"all_records_identical":rows.len(),"new_refusal_progressions":[],"named_refusal_progressions":inherited["named_refusal_progressions"],"note":"Inherited historical progression labels; no new compiler delta."}),
	)
}
pub(super) fn units(ctx: &Context, m: &Manifest, binary: &Path) -> Result<Value> {
	let l1 = cases(ctx, m)?;
	let old = read(&ctx.fixtures.join(&m.unit_records))?;
	let old = array(&old)?;
	require_count(old.len(), m.counts["legacy_units"], "legacy Units")?;
	let baseline = ctx
		.fixtures
		.join(&m.unit_records)
		.parent()
		.context("Unit baseline parent")?
		.join("records");
	let root = ctx.out.join("records");
	fs::create_dir(&root)?;
	let mut compiled = 0;
	let awfy = [
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
	];
	for (i, row) in old.iter().enumerate() {
		let name = string(row, "case")?;
		let (expected, input) = if i < l1.len() {
			(format!("l1-{}", l1[i].name), Some(source(ctx, m, &l1[i])?))
		} else {
			let case = awfy.get(i - l1.len()).context("extra AWFY Unit row")?;
			(format!("awfy-{case}"), None)
		};
		ensure!(name == expected, "Unit row order/name changed at {i}");
		let dir = root.join(name);
		let dest = dir.join("record.json");
		let mut env = vec![("GAP_RESULT", dest.display().to_string())];
		if let Some(input) = input {
			env.push(("GAP_CASE", input.display().to_string()));
			env.push(("GAP_ORIGIN", l1[i].source.clone()));
			let relative = fixture_relative(&l1[i].source, &m.historical_anchor_component)?;
			let mut logical_root = PathBuf::from(&l1[i].source);
			for _ in relative.components() {
				ensure!(logical_root.pop(), "missing logical fixture prefix");
			}
			env.push(("GAP_LOGICAL_ROOT", logical_root.display().to_string()));
			env.push(("GAP_FIXTURE_ROOT", ctx.fixtures.display().to_string()));
		} else {
			env.push(("C18E_AWFY_CASE", name[5..].to_owned()));
		}
		child(
			ctx,
			binary,
			&dir,
			"experimental_core::tests::gap_hunt::legacy_unit_receipts::legacy_unit_child",
			&env,
			30,
			false,
		)?;
		let old_record = baseline.join(format!("{name}.json"));
		ensure!(
			common::hash_file(&old_record)? == string(row, "record_sha256")?,
			"Unit baseline metadata hash changed: {name}"
		);
		exact(&old_record, &dest)?;
		let observed = read(&dest)?;
		let status = string(&observed, "status")?;
		ensure!(
			matches!(status, "compiled" | "compile-error"),
			"invalid Unit status"
		);
		if name.starts_with("awfy-") {
			ensure!(status == "compiled", "AWFY Unit no longer compiles: {name}");
		}
		if status == "compiled" {
			let digest = string(&observed, "unit_sha256")?;
			let old_unit = baseline.join(format!("{name}.unit"));
			ensure!(
				common::hash_file(&old_unit)? == digest,
				"Unit baseline byte digest changed: {name}"
			);
			ensure!(
				common::hash_file(&dest.with_extension("unit"))? == digest,
				"Unit output byte digest changed: {name}"
			);
			exact(&old_unit, &dest.with_extension("unit"))?;
			compiled += 1;
		}
	}
	Ok(
		json!({"records":old.len(),"L1":l1.len(),"AWFY":awfy.len(),"compiled":compiled,"compile_errors":old.len()-compiled,"encoding":"musli storage binary; complete Unit including float bits"}),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn historical_logical_paths_resolve_without_host_prefix() {
		assert_eq!(
			fixture_relative("/old/host/.reviews/record/input.rn", ".reviews").unwrap(),
			Path::new("record/input.rn")
		);
		assert_eq!(
			fixture_relative("/different/host/.reviews/record/input.rn", ".reviews").unwrap(),
			Path::new("record/input.rn")
		);
		for invalid in [
			"/old/host/input.rn",
			"/old/.reviews/../escape",
			"/old/.reviews/a/.reviews/b",
			"/old/.reviews",
		] {
			assert!(fixture_relative(invalid, ".reviews").is_err());
		}
	}
	#[test]
	fn l1_requires_the_exact_historical_single_test_output() {
		for accepted in [
			b"running 1 test".as_slice(),
			b"\nrunning 1 test\n",
			b" \nrunning 1 test \n",
		] {
			assert!(require_l1_stdout(accepted).is_ok());
		}
		for rejected in [
			b"running 0 tests".as_slice(),
			b"running 1 test\nextra text",
			b"extra text\nrunning 1 test",
			b"running 1 test\nrunning 1 test",
			b"",
			b"running 1 test\xff",
		] {
			assert!(require_l1_stdout(rejected).is_err());
		}
	}
}
