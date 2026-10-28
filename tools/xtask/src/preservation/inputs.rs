//! Input-only L1 case index; no historical outputs are required here.
use crate::common::{self, Result};
use anyhow::{ensure, Context as _};
use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Component, Path},
};

const HEADER: &str = "id\tsource\tmode\tentry\targs\tsha256";

pub(super) fn rows(text: &str) -> Result<Vec<[&str; 6]>> {
	let mut lines = text.lines();
	ensure!(
		lines.next() == Some(HEADER),
		"invalid L1 input index header"
	);
	let mut ids = BTreeSet::new();
	let mut result = Vec::new();
	for line in lines {
		let columns: Vec<_> = line.split('\t').collect();
		let row: [&str; 6] = columns
			.try_into()
			.map_err(|_| anyhow::anyhow!("invalid L1 input row"))?;
		let [id, source, mode, entry, args, digest] = row;
		ensure!(
			!id.is_empty()
				&& id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
				&& ids.insert(id),
			"invalid or duplicate L1 id"
		);
		ensure!(
			!source.is_empty()
				&& Path::new(source)
					.components()
					.all(|c| matches!(c, Component::Normal(_))),
			"invalid L1 source path"
		);
		ensure!(args.is_empty(), "L1 argument encoding is not yet supported");
		match mode {
			"compile" => ensure!(entry.is_empty(), "compile-only case has an entry"),
			"main" | "script" => ensure!(entry == "main", "invalid main entry"),
			"test" => ensure!(
				!entry.is_empty()
					&& entry.split("::").all(|part| !part.is_empty()
						&& part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
				"invalid test entry"
			),
			_ => anyhow::bail!("invalid L1 mode"),
		}
		ensure!(
			digest.len() == 64
				&& digest
					.bytes()
					.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
			"invalid L1 source digest"
		);
		result.push(row);
	}
	Ok(result)
}

pub(super) fn verify(repo: &Path) -> Result<BTreeMap<String, usize>> {
	let root = repo.join("tools/xtask/corpus").canonicalize()?;
	let index = std::fs::read_to_string(root.join("l1-cases.tsv"))?;
	let rows = rows(&index)?;
	ensure!(rows.len() == 2941, "unexpected L1 input case count");
	// These two programs previously fed AWFY diagnostic snapshots. Their
	// behavior is now covered by the live L1 comparison, so retain both IDs.
	for id in ["00063", "00357"] {
		ensure!(
			rows.iter().any(|row| row[0] == id),
			"missing former AWFY diagnostic input: {id}"
		);
	}
	let mut counts = BTreeMap::new();
	for [id, source, mode, _, _, expected] in rows {
		let path = root.join(source);
		let resolved = path
			.canonicalize()
			.with_context(|| format!("missing input for {id}"))?;
		ensure!(
			resolved.starts_with(&root) && resolved.is_file(),
			"L1 source escapes corpus"
		);
		ensure!(
			resolved.metadata()?.len() < 1_000_000,
			"L1 source exceeds size limit"
		);
		ensure!(
			common::hash_file(&resolved)? == expected,
			"L1 source digest differs: {id}"
		);
		*counts.entry(mode.to_owned()).or_insert(0) += 1;
	}
	ensure!(
		counts.get("compile") == Some(&164)
			&& counts.get("main") == Some(&2554)
			&& counts.get("script") == Some(&34)
			&& counts.get("test") == Some(&189),
		"L1 input modes differ from the declared corpus"
	);
	Ok(counts)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn checked_in_corpus_matches_its_index() {
		let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let counts = verify(&repo).unwrap();
		assert_eq!(counts.values().sum::<usize>(), 2941);
	}

	#[test]
	fn rejects_bad_paths_modes_and_duplicate_ids() {
		let hash = "a".repeat(64);
		let good = format!("{HEADER}\n00001\tdir/a.rn\ttest\tmodule::test\t\t{hash}\n");
		assert_eq!(rows(&good).unwrap().len(), 1);
		assert!(rows(&good.replace("dir/a.rn", "../a.rn")).is_err());
		assert!(rows(&good.replace("test\tmodule::test", "compile\tmodule::test")).is_err());
		assert!(rows(&format!("{good}00001\tb.rn\tcompile\t\t\t{hash}\n")).is_err());
	}
}
