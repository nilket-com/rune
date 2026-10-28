const C12_POLICIES: &str = include_str!("policies.json");
const C12_P1_COUNTS: &str = include_str!("p1_counts.json");
fn c12_data() -> (Vec<Json>, Vec<Json>, Json) {
	(
		serde_json::from_str(C12_CASES).unwrap(),
		serde_json::from_str(C12_POLICIES).unwrap(),
		serde_json::from_str(C12_P1_COUNTS).unwrap(),
	)
}
#[test]
fn policy_schema_and_source_identity() {
	use sha2::{Digest, Sha256};
	let (cases, policies, counts) = c12_data();
	assert_eq!(cases.len(), 111);
	assert_eq!(policies.len(), cases.len());
	let mut labels = std::collections::BTreeSet::new();
	let mut kinds = std::collections::BTreeMap::new();
	for (i, (case, policy)) in cases.iter().zip(&policies).enumerate() {
		assert_eq!(case["id"], format!("{i:03}"));
		for key in ["id", "label", "source_sha256", "fixture", "fuel"] {
			assert_eq!(case[key], policy[key]);
		}
		assert!(labels.insert(case["label"].as_str().unwrap()));
		assert_eq!(
			hex(&Sha256::digest(case["source"].as_str().unwrap().as_bytes())),
			case["source_sha256"]
		);
		*kinds
			.entry(policy["policy"].as_str().unwrap())
			.or_insert(0usize) += 1;
	}
	assert_eq!(
		kinds,
		std::collections::BTreeMap::from([
			("Exact", 107),
			("Refused-surface", 3),
			("Retained-fuel", 1)
		])
	);
	assert_eq!(counts.as_object().unwrap().len(), 10);
	for paths in counts.as_object().unwrap().values() {
		for (path, _) in paths.as_object().unwrap() {
			assert!(p1_path(path));
		}
	}
}

fn subject_stdout(bytes: &[u8]) -> Result<Vec<u8>, String> {
	let text = std::str::from_utf8(bytes).map_err(|_| "non-UTF8 test stdout")?;
	let rest = text
		.strip_prefix(
			"\nrunning 1 test\ntest experimental_core::tests::gap_hunt::small_forms::child ... ",
		)
		.ok_or("test harness prefix")?;
	let (subject, footer) = rest
		.rsplit_once("ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; ")
		.ok_or("test harness suffix")?;
	let (filtered, time) = footer
		.split_once(" filtered out; finished in ")
		.ok_or("test harness footer")?;
	filtered
		.parse::<usize>()
		.map_err(|_| "test harness count")?;
	let seconds = time
		.strip_suffix("s\n\n")
		.ok_or("test harness timing suffix")?
		.parse::<f64>()
		.map_err(|_| "test harness time")?;
	if !seconds.is_finite() || seconds < 0.0 {
		return Err("invalid test time".into());
	}
	Ok(subject.as_bytes().to_vec())
}
#[test]
fn stdout_framing_preserves_subject_bytes() {
	let prefix =
		"\nrunning 1 test\ntest experimental_core::tests::gap_hunt::small_forms::child ... ";
	let suffix="ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.02s\n\n";
	assert_eq!(
		subject_stdout(format!("{prefix}hello\n{suffix}").as_bytes()).unwrap(),
		b"hello\n"
	);
	assert!(subject_stdout(b"hello\n").is_err());
}
fn c12_drain(
	mut input: impl std::io::Read,
	exceeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<u8> {
	let mut out = Vec::new();
	let mut buffer = [0u8; 4096];
	loop {
		let n = input.read(&mut buffer).unwrap();
		if n == 0 {
			break;
		}
		let available = (128 * 1024usize).saturating_sub(out.len());
		out.extend_from_slice(&buffer[..n.min(available)]);
		if n > available {
			exceeded.store(true, std::sync::atomic::Ordering::SeqCst);
		}
	}
	out
}
