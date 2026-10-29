//! Execute the checked-in L1 inputs in isolated legacy and fresh child processes.
//! Unknown differences fail closed after the complete corpus has been surveyed.
use super::inputs;
use crate::common::{self, Context, Result};
use anyhow::{ensure, Context as _};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

const CHILD: &str = "experimental_core::tests::gap_hunt::child";

fn phase(record: &str) -> Result<[&str; 5]> {
	let fields: Vec<_> = record.split('\t').collect();
	fields
		.try_into()
		.map_err(|_| anyhow::anyhow!("invalid L1 child observation"))
}

fn decode_hex(text: &str) -> Result<String> {
	ensure!(text.len().is_multiple_of(2), "invalid observation encoding");
	let bytes = text
		.as_bytes()
		.chunks_exact(2)
		.map(|part| u8::from_str_radix(std::str::from_utf8(part)?, 16).map_err(Into::into))
		.collect::<Result<Vec<_>>>()?;
	Ok(String::from_utf8(bytes)?)
}

fn valid_missing_method_evidence(value: &Value) -> bool {
	let site = &value["site"];
	let detail = &value["detail"];
	let hash = |v: &Value| {
		v.as_str()
			.and_then(|s| s.strip_prefix("0x"))
			.is_some_and(|digits| {
				!digits.is_empty()
					&& digits.len() <= 16
					&& digits.bytes().all(|b| b.is_ascii_hexdigit())
			})
	};
	value["schema"] == 1
		&& value["class"] == json!({"status":"class","name":"TypeMismatch"})
		&& matches!(
			value["origin"].as_str(),
			Some("Script" | "Host" | "Unknown")
		) && site.as_object().is_some_and(|o| o.len() == 3)
		&& site["source"].as_u64().is_some()
		&& matches!((site["start"].as_u64(), site["end"].as_u64()), (Some(start), Some(end)) if start <= end)
		&& detail.as_object().is_some_and(|o| o.len() == 3)
		&& detail["kind"] == "missing-instance-function"
		&& hash(&detail["hash"])
		&& hash(&detail["instance"])
}

// The sidecars serialize ErrorProjection. This is its same_class rule:
// only two reviewed Class projections with the same known class can match.
fn same_reviewed_class(a: &Value, b: &Value) -> bool {
	let class = &a["class"];
	class == &b["class"]
		&& class["status"] == "class"
		&& matches!(
			class["name"].as_str(),
			Some(
				"TypeMismatch"
					| "IndexOutOfBounds"
					| "Overflow" | "DivisionByZero"
					| "IllegalFloatComparison"
					| "ValueDepth" | "ArgumentCount"
					| "Panic" | "AccessConflict"
					| "IntegerCoercion"
					| "InvalidCast" | "BudgetExhausted"
					| "Allocation"
			)
		)
}

fn valid_origin_pair(a: &Value, b: &Value) -> bool {
	let a = a["origin"].as_str();
	let b = b["origin"].as_str();
	matches!(a, Some("Script" | "Host" | "Unknown"))
		&& matches!(b, Some("Script" | "Host" | "Unknown"))
		&& (a == Some("Unknown") || b == Some("Unknown") || a == b)
}

fn valid_site(site: &Value) -> bool {
	site.as_object().is_some_and(|o| o.len() == 3)
		&& site["source"].as_u64().is_some()
		&& matches!((site["start"].as_u64(), site["end"].as_u64()), (Some(start), Some(end)) if start <= end)
}

// S7a explicitly accepts the differing opaque function Debug display, only
// for the two cases whose value is the function object itself.
fn opaque_function_debug(id: &str, legacy: &str, fresh: &str) -> bool {
	if !matches!(id, "00620" | "02057") {
		return false;
	}
	let old = legacy
		.strip_prefix("immediate function (at: 0x")
		.and_then(|s| s.strip_suffix(')'));
	let new = fresh
		.strip_prefix("immediate function (function: ")
		.and_then(|s| s.strip_suffix(')'));
	old.is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_hexdigit()))
		&& new.is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
}

fn classify(
	id: &str,
	mode: &str,
	legacy: &str,
	fresh: &str,
	legacy_stdout: &[u8],
	fresh_stdout: &[u8],
	legacy_stderr: &[u8],
	fresh_stderr: &[u8],
	legacy_detail: Option<&Value>,
	fresh_detail: Option<&Value>,
) -> Result<String> {
	let s = phase(legacy)?;
	let f = phase(fresh)?;
	let streams_equal = legacy_stdout == fresh_stdout && legacy_stderr == fresh_stderr;
	if f[0] == "refused" {
		ensure!(
			f[1].starts_with("Some(") && f[1].ends_with(')'),
			"malformed fresh refusal class"
		);
		return Ok(format!("safe-refusal:{}", &f[1][5..f[1].len() - 1]));
	}
	if s[0] == "compile-error" {
		return Ok("unsafe-fresh-admission".into());
	}
	if mode == "compile" && s[0] == "compiled" && f[0] == "compiled" {
		return Ok("compile-only".into());
	}
	if s[0] == "observation-error"
		&& f[0] == "observation-error"
		&& s[1] == "Debug"
		&& f[1] == "Debug"
		&& s[3] == f[3]
		&& decode_hex(s[3])? == "Cannot read the value: it has been moved"
		&& s[4] == f[4]
		&& streams_equal
	{
		return Ok("equal-moved-observation-error".into());
	}
	if !matches!(s[0], "value" | "error") || !matches!(f[0], "value" | "error") {
		return Ok("non-executed-or-observation-failure".into());
	}
	if s[0] == "error" || f[0] == "error" {
		if let (Some(a), Some(b)) = (legacy_detail, fresh_detail) {
			let entry_arity = id == "00179"
				&& mode == "main"
				&& a["class"]["name"] == "ArgumentCount"
				&& a["site"].is_null()
				&& b["site"].is_null()
				&& decode_hex(s[3])? == "Wrong number of arguments 0, expected 1";
			if s[0] == "error"
				&& f[0] == "error"
				&& a["schema"] == 1
				&& b["schema"] == 1
				&& same_reviewed_class(a, b)
				&& valid_origin_pair(a, b)
				&& ((valid_site(&a["site"]) && a["site"] == b["site"]) || entry_arity)
				&& a["detail"].is_null()
				&& b["detail"].is_null()
				&& s[3] == f[3]
				&& s[4] == f[4]
				&& streams_equal
			{
				return Ok(if entry_arity {
					"entry-arity-detail-not-captured"
				} else {
					"equal-error-detail-not-captured"
				}
				.into());
			}
			let reviewed =
				s[0] == "error"
					&& f[0] == "error"
					&& s[1] == f[1] && valid_missing_method_evidence(a)
					&& valid_missing_method_evidence(b)
					&& a["class"] == b["class"]
					&& (a["origin"] == "Unknown"
						|| b["origin"] == "Unknown"
						|| a["origin"] == b["origin"])
					&& a["site"].is_object()
					&& a["site"] == b["site"]
					&& a["detail"]["kind"] == "missing-instance-function"
					&& a["detail"] == b["detail"]
					&& s[4] == f[4] && streams_equal;
			if reviewed {
				if s[3] == f[3] {
					return Ok("equal-reviewed-error".into());
				}
				if crate::d1_message::d1_missing_method_message(
					&decode_hex(s[3])?,
					&decode_hex(f[3])?,
				) {
					return Ok("d1-missing-method".into());
				}
			}
			if a["detail"].is_null() || b["detail"].is_null() {
				return Ok(format!("runtime-error-unverified:detail:{}", s[1]));
			}
		}
		// Missing reviewed class, site or supported typed detail is never equivalence.
		return Ok("runtime-error-unverified".into());
	}
	if s[0] == f[0] && s[1] == f[1] && s[3..] == f[3..] && streams_equal {
		return Ok("equal".into());
	}
	if s[0] == "value"
		&& f[0] == "value"
		&& s[1] == f[1]
		&& s[3] == f[3]
		&& streams_equal
		&& opaque_function_debug(id, &decode_hex(s[4])?, &decode_hex(f[4])?)
	{
		return Ok("function-debug-display".into());
	}
	Ok("divergence".into())
}

fn invoke(
	ctx: &Context,
	binary: &Path,
	corpus: &Path,
	case_dir: &Path,
	id: &str,
	source: &str,
	mode: &str,
	engine: &str,
) -> Result<(String, Vec<u8>, Vec<u8>, Option<Value>, Value)> {
	invoke_child(
		ctx, binary, corpus, case_dir, id, source, mode, engine, CHILD,
	)
}

/// The common isolated process boundary for semantic and work observations.
pub(super) fn invoke_child(
	ctx: &Context,
	binary: &Path,
	corpus: &Path,
	case_dir: &Path,
	id: &str,
	source: &str,
	mode: &str,
	engine: &str,
	child: &str,
) -> Result<(String, Vec<u8>, Vec<u8>, Option<Value>, Value)> {
	let output = case_dir.join(engine);
	fs::create_dir(&output)?;
	let sandbox_source = format!("/corpus/{source}");
	let mut cmd = Command::new("/usr/bin/prlimit");
	cmd.args([
		"--cpu=15",
		"--as=2147483648",
		"--fsize=2097152",
		"--core=0",
		"--",
		"/usr/bin/bwrap",
		"--die-with-parent",
		"--unshare-net",
		"--clearenv",
		"--ro-bind",
		"/usr",
		"/usr",
		"--ro-bind",
		"/lib",
		"/lib",
		"--ro-bind",
		"/lib64",
		"/lib64",
		"--proc",
		"/proc",
		"--dev",
		"/dev",
		"--tmpfs",
		"/tmp",
		"--ro-bind",
	])
	.arg(corpus)
	.arg("/corpus")
	.arg("--ro-bind")
	.arg(binary)
	.arg("/rune-tests")
	.arg("--bind")
	.arg(&output)
	.arg("/work")
	.args([
		"--chdir",
		"/work",
		"--setenv",
		"HOME",
		"/work",
		"--setenv",
		"PATH",
		"/usr/bin:/bin",
		"--setenv",
		"LANG",
		"C.UTF-8",
		"--setenv",
		"TZ",
		"UTC",
		"--setenv",
		"GAP_CASE",
	])
	.arg(&sandbox_source)
	.args(["--setenv", "GAP_ORIGIN"])
	.arg(&sandbox_source)
	.args([
		"--setenv",
		"GAP_MODE",
		mode,
		"--setenv",
		"GAP_ENGINE",
		engine,
		"--setenv",
		"GAP_RESULT",
		"/work/result",
		"--setenv",
		"GAP_L1_DETAIL",
		"/work/detail.json",
		"--setenv",
		"GAP_L1_WARNINGS",
		"/work/warnings.json",
		"--",
		"/rune-tests",
		"--exact",
		child,
		"--ignored",
		"--nocapture",
		"--quiet",
	]);
	let label = format!("{id}-{engine}");
	let run = ctx.run(&label, &mut cmd, 20)?;
	ensure!(run.code == 0, "L1 {label} child exited {}", run.code);
	let stdout = fs::read(&run.stdout)?;
	let stderr = fs::read(&run.stderr)?;
	ensure!(
		stdout.len() <= 2_097_152 && stderr.len() <= 2_097_152,
		"L1 {label} output exceeded cap"
	);
	ensure!(
		std::str::from_utf8(&stdout)?
			.lines()
			.filter(|line| line.trim() == "running 1 test")
			.count() == 1,
		"L1 {label} did not run exactly one test"
	);
	let record = fs::read_to_string(output.join("result"))
		.with_context(|| format!("missing L1 result {label}"))?;
	let detail = match fs::read(output.join("detail.json")) {
		Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
		Err(error) => return Err(error.into()),
	};
	let warnings: Value = serde_json::from_slice(&fs::read(output.join("warnings.json"))?)?;
	ensure!(warnings.is_array(), "missing L1 warning multiset");
	Ok((record, stdout, stderr, detail, warnings))
}

pub(super) fn run(ctx: &Context) -> Result<Value> {
	let binary = ctx.test_binary()?;
	run_with_binary(ctx, &binary)
}

pub(super) fn run_with_binary(ctx: &Context, binary: &Path) -> Result<Value> {
	let modes = inputs::verify(&ctx.repo)?;
	let corpus = ctx.repo.join("tools/xtask/corpus").canonicalize()?;
	let index = fs::read_to_string(corpus.join("l1-cases.tsv"))?;
	let cases = inputs::rows(&index)?;
	let binary_sha256 = common::hash_file(binary)?;
	let root = ctx.out.join("cases");
	fs::create_dir(&root)?;
	let next = std::sync::atomic::AtomicUsize::new(0);
	let outcomes = std::sync::Mutex::new(Vec::new());
	std::thread::scope(|scope| {
		for _ in 0..ctx.jobs {
			scope.spawn(|| loop {
				let position = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
				let Some(&[id, source, kind, entry, _, _]) = cases.get(position) else {
					break;
				};
				let mode = if kind == "test" {
					format!("test:{entry}")
				} else {
					kind.to_owned()
				};
				let case_dir = root.join(id);
				let result = (|| -> Result<_> {
					fs::create_dir(&case_dir)?;
					let s = invoke(ctx, binary, &corpus, &case_dir, id, source, &mode, "stock")?;
					let f = invoke(ctx, binary, &corpus, &case_dir, id, source, &mode, "fresh")?;
					if phase(&f.0)?[0] != "refused" && phase(&s.0)?[0] != "compile-error" {
						ensure!(
							s.4 == f.4,
							"L1 {id} compile warning multisets differ: legacy={} execution={}",
							s.4,
							f.4
						);
					}
					Ok(classify(
						id,
						kind,
						&s.0,
						&f.0,
						&s.1,
						&f.1,
						&s.2,
						&f.2,
						s.3.as_ref(),
						f.3.as_ref(),
					)?)
				})();
				let class = match result {
					Ok(class) => class,
					Err(error) => format!("child-error: {error:#}"),
				};
				outcomes.lock().unwrap().push((position, id, mode, class));
			});
		}
	});
	let mut outcomes = outcomes
		.into_inner()
		.map_err(|_| anyhow::anyhow!("L1 outcome lock poisoned"))?;
	ensure!(
		outcomes.len() == cases.len(),
		"incomplete L1 case inventory"
	);
	outcomes.sort_by_key(|row| row.0);
	let mut counts = BTreeMap::<String, usize>::new();
	let mut unexpected = Vec::new();
	let mut detail_not_captured = Vec::new();
	for (_, id, mode, class) in outcomes {
		*counts.entry(class.clone()).or_default() += 1;
		if matches!(
			class.as_str(),
			"equal-error-detail-not-captured" | "entry-arity-detail-not-captured"
		) {
			detail_not_captured.push(json!({"id": id, "detail": "not captured; exact message"}));
		}
		if !matches!(
			class.as_str(),
			"equal"
				| "compile-only"
				| "function-debug-display"
				| "equal-reviewed-error"
				| "equal-error-detail-not-captured"
				| "entry-arity-detail-not-captured"
				| "equal-moved-observation-error"
				| "d1-missing-method"
		) && !class.starts_with("safe-refusal:")
		{
			unexpected.push(json!({"id":id,"mode":mode,"class":class}));
		}
	}

	ctx.verify_source()?;
	ensure!(
		common::hash_file(binary)? == binary_sha256,
		"L1 test binary changed"
	);
	let value = json!({"cases":2941,"input_modes":modes,"classes":counts,"unreviewed":unexpected,"detail_not_captured":detail_not_captured,"binary_sha256":binary_sha256});
	common::write_json(&ctx.out.join("LIVE-COMPARISON.json"), &value)?;
	ensure!(
		value["unreviewed"].as_array().is_some_and(Vec::is_empty),
		"L1 has unreviewed divergences or coverage failures; see LIVE-COMPARISON.json"
	);
	Ok(value)
}

#[cfg(test)]
mod tests {
	use super::*;
	fn c(id: &str, mode: &str, legacy: &str, fresh: &str) -> String {
		classify(
			id, mode, legacy, fresh, b"out", b"out", b"", b"", None, None,
		)
		.unwrap()
	}
	#[test]
	fn narrow_policy_fails_closed() {
		let value = "value\t\t\t\t31";
		assert_eq!(c("x", "main", value, value), "equal");
		assert_eq!(
			c("x", "compile", "compiled\t\t\t\t", "compiled\t\t\t\t"),
			"compile-only"
		);
		assert_eq!(
			c("x", "main", value, "refused\tSome(Native)\t\t\t"),
			"safe-refusal:Native"
		);
		assert_eq!(
			c(
				"x",
				"main",
				"compile-error\tFrontendDiagnostic\t\t\t",
				value
			),
			"unsafe-fresh-admission"
		);
		assert_eq!(
			c("x", "main", "error\tType\t\t61\t", "error\tType\t\t62\t"),
			"runtime-error-unverified"
		);
		assert_eq!(
			c("x", "main", "error\tType\t\t61\t", "error\tOther\t\t62\t"),
			"runtime-error-unverified"
		);
		assert_eq!(
			c(
				"x",
				"main",
				"entry-gap\tno-main\t\t\t",
				"entry-gap\tno-main\t\t\t"
			),
			"non-executed-or-observation-failure"
		);
		assert!(classify(
			"x",
			"main",
			"malformed",
			value,
			b"",
			b"",
			b"",
			b"",
			None,
			None
		)
		.is_err());
		assert!(opaque_function_debug(
			"00620",
			"immediate function (at: 0x5)",
			"immediate function (function: 1)"
		));
		assert!(!opaque_function_debug(
			"other",
			"immediate function (at: 0x5)",
			"immediate function (function: 1)"
		));
	}

	#[test]
	fn uncaptured_detail_requires_exact_observables() {
		let evidence = json!({"schema":1,"class":{"status":"class","name":"TypeMismatch"},"origin":"Unknown","site":{"source":0,"start":2,"end":5},"detail":null});
		let run = |id: &str, legacy: &str, fresh: &str, a: &Value, b: &Value| {
			classify(
				id,
				"main",
				legacy,
				fresh,
				b"out",
				b"out",
				b"",
				b"",
				Some(a),
				Some(b),
			)
			.unwrap()
		};
		let error = "error\tType\t\t61\t";
		assert_eq!(
			run("x", error, error, &evidence, &evidence),
			"equal-error-detail-not-captured"
		);
		let mut changed = evidence.clone();
		changed["class"]["status"] = json!("unclassified");
		assert!(run("x", error, error, &changed, &changed).starts_with("runtime-error-unverified"));
		changed = evidence.clone();
		changed["site"]["end"] = json!(6);
		assert!(run("x", error, error, &evidence, &changed).starts_with("runtime-error-unverified"));
		changed = evidence.clone();
		changed["detail"] = json!({"kind":"other"});
		assert!(run("x", error, error, &evidence, &changed).starts_with("runtime-error-unverified"));
		assert!(run("x", error, "error\tType\t\t62\t", &evidence, &evidence)
			.starts_with("runtime-error-unverified"));
		let mut arity = evidence.clone();
		arity["class"]["name"] = json!("ArgumentCount");
		arity["site"] = Value::Null;
		let record = format!(
			"error\tArgumentCount\t\t{}\t",
			"Wrong number of arguments 0, expected 1"
				.bytes()
				.map(|b| format!("{b:02x}"))
				.collect::<String>()
		);
		assert_eq!(
			run("00179", &record, &record, &arity, &arity),
			"entry-arity-detail-not-captured"
		);
		assert!(
			run("other", &record, &record, &arity, &arity).starts_with("runtime-error-unverified")
		);
		let moved = format!(
			"observation-error\tDebug\t\t{}\t",
			"Cannot read the value: it has been moved"
				.bytes()
				.map(|b| format!("{b:02x}"))
				.collect::<String>()
		);
		assert_eq!(
			c("x", "main", &moved, &moved),
			"equal-moved-observation-error"
		);
	}

	#[test]
	fn d1_requires_matching_reviewed_evidence_and_named_wording() {
		let hex = |s: &str| {
			s.as_bytes()
				.iter()
				.map(|b| format!("{b:02x}"))
				.collect::<String>()
		};
		let legacy = format!(
			"error\tType\t\t{}\t",
			hex("Type `std::i64` has no matching method")
		);
		let fresh = format!(
			"error\tType\t\t{}\t",
			hex("Type `std::i64` has no method named `len`")
		);
		let detail = json!({"schema":1,"class":{"status":"class","name":"TypeMismatch"},"origin":"Script","site":{"source":0,"start":4,"end":12},"detail":{"kind":"missing-instance-function","hash":"0x0000000000000001","instance":"0x0000000000000002"}});
		let compare = |a: &Value, b: &Value, message: &str| {
			classify(
				"x",
				"main",
				&legacy,
				message,
				b"",
				b"",
				b"",
				b"",
				Some(a),
				Some(b),
			)
			.unwrap()
		};
		assert_eq!(compare(&detail, &detail, &fresh), "d1-missing-method");
		let mut unknown = detail.clone();
		unknown["origin"] = json!("Unknown");
		assert_eq!(compare(&unknown, &detail, &fresh), "d1-missing-method");
		assert_eq!(compare(&detail, &unknown, &fresh), "d1-missing-method");
		assert_eq!(compare(&unknown, &unknown, &fresh), "d1-missing-method");
		let mut host = detail.clone();
		host["origin"] = json!("Host");
		assert_eq!(compare(&detail, &host, &fresh), "runtime-error-unverified");
		let mut invalid_origin = detail.clone();
		invalid_origin["origin"] = json!("other");
		assert_eq!(
			compare(&unknown, &invalid_origin, &fresh),
			"runtime-error-unverified"
		);
		let mut short_hash = detail.clone();
		short_hash["detail"]["hash"] = json!("0xd70a3c86c27fa00");
		assert_eq!(
			compare(&short_hash, &short_hash, &fresh),
			"d1-missing-method"
		);
		for (key, value) in [
			("site", json!(null)),
			("detail", json!(null)),
			("class", json!({"status":"unclassified"})),
		] {
			let mut changed = detail.clone();
			changed[key] = value;
			assert!(
				compare(&changed, &changed, &fresh).starts_with("runtime-error-unverified"),
				"{key}"
			);
		}
		let mut changed = detail.clone();
		changed["detail"]["hash"] = json!("different");
		assert_eq!(
			compare(&detail, &changed, &fresh),
			"runtime-error-unverified"
		);
		changed = detail.clone();
		changed["site"]["start"] = json!(5);
		assert_eq!(
			compare(&detail, &changed, &fresh),
			"runtime-error-unverified"
		);
		let unrelated = format!(
			"error\tType\t\t{}\t",
			hex("Type `std::i64` has no method named `len!`")
		);
		assert_eq!(
			compare(&detail, &detail, &unrelated),
			"runtime-error-unverified"
		);
	}
}
