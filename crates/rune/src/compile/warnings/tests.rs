use crate::ast::Spanned;
use crate::diagnostics::Diagnostic;
use crate::experimental_core::frontend;
use crate::{Context, Diagnostics, Source, Sources};
use std::string::{String, ToString};
use std::vec::Vec;

fn warnings(diagnostics: &Diagnostics) -> Vec<(usize, usize, usize, String, String)> {
	let mut warnings: Vec<_> = diagnostics
		.diagnostics()
		.iter()
		.filter_map(|d| {
			let Diagnostic::Warning(w) = d else {
				return None;
			};
			let span = w.span();
			Some((
				w.source_id().into_index(),
				span.start.0 as usize,
				span.end.0 as usize,
				w.to_string(),
				format!("{w:?}"),
			))
		})
		.collect();
	warnings.sort();
	warnings
}

fn sources(texts: &[&str]) -> Sources {
	let mut sources = Sources::new();
	for (i, text) in texts.iter().enumerate() {
		sources
			.insert(Source::new(format!("warnings-{i}.rn"), *text).unwrap())
			.unwrap();
	}
	sources
}

fn compare(context: &Context, texts: &[&str]) {
	let mut legacy = Diagnostics::new();
	let mut execution = Diagnostics::new();
	let a = crate::prepare(&mut sources(texts))
		.with_context(context)
		.with_diagnostics(&mut legacy)
		.build();
	assert!(a.is_ok(), "legacy compile {texts:?}: {legacy:?}");
	let b = frontend::compile(&mut sources(texts), context, &mut execution, 1);
	assert!(
		b.is_ok(),
		"execution compile {texts:?}: {:?} {execution:?}",
		b.err()
	);
	assert_eq!(warnings(&execution), warnings(&legacy), "{texts:?}");
}

#[test]
#[cfg_attr(miri, ignore = "default-module frontend compilation")]
fn warning_families_match_live_legacy() {
	let context = Context::with_default_modules().unwrap();
	for text in [
		"pub fn main() { `hello` }",
		"pub fn main() { `hello`; 7 }",
		"pub fn main() { `hello {1}` }",
		"pub fn main() { `hello {\"world\"}` }",
		"pub fn main() { let [x] = [1]; x }",
		"pub fn main([x]) { x }",
		"pub fn main() { let _ = 1; 7 }",
		"pub fn main() { { let _ = 1; 7 } }",
		"pub fn main() { let (a,b) = (1,2); a+b }",
		"pub fn main() { for (a,b) in [(1,2)] { a+b; } }",
		"pub fn main() { return 1; 2 }",
		"pub fn main() { return 1; 2; 3; 4 }",
		"pub fn main() { { return 1; 2 }; 3 }",
		"pub fn main() { for _ in { return 1 } { 2 } 3 }",
		"pub fn main() { 123; 7 }",
		"const C = 1; pub fn main() { C; 7 }",
		"pub fn main() { || 1; 7 }",
		"pub fn main() { let f = || { 1; 2 }; f() }",
		"pub fn main() { let f = |[x]| { 1; x }; f([1]) }",
		"fn unused() { 1; 2 } pub fn main() { 7 }",
		"use std::collections::HashMap; pub fn main() { 7 }",
		"struct Empty(); pub fn main() { Empty() }",
		"pub fn main() { if true { 1; 2 } else { 3; 4 } }",
		"pub fn main() { if true { 1 } 2 }",
		"pub fn main() { if true { 1 } else { 2 }; 3 }",
		"pub fn main() { if true { return 1 } else { return 2 }; 3 }",
		"pub fn main() { match 1 { 0 => { 2; 3 }, _ => { 4; 5 } } }",
		"pub fn main() { match 1 { _ => 2, 0 => { 3; 4 } } }",
		"pub fn main() { match 1 { 0 => return 1, _ => return 2 }; 3 }",
		"pub fn main() { while true { 1; break; 2 } 3 }",
		"pub fn main() { loop { 1; break; 2 } 3 }",
		"pub fn main() { loop { break 1; 2 }; 3 }",
		"pub fn main() { 'b: { break 'b 1; 2 }; 3 }",
		"pub fn main() { true && { 1; true }; 3 }",
		"pub fn main() { true && { return 1 }; 3 }",
		"pub fn main() { while { return 1 } { 2; 3 } 4 }",
		"pub fn main() { if { return 1 } { 2; 3 } else { 4; 5 } }",
		"pub fn main() { if let _ = 1 { 2; 3 } else { 4; 5 } }",
		"pub fn main() { if let Some(x) = Some(1) { x; 2 } }",
		"pub fn main() { match 1 { _ if { return 1 } => { 2; 3 }, _ => { 4; 5 } } }",
		"pub fn main() { match 1 { 0 if { return 1 } => { 2; 3 }, _ => { 4; 5 } } }",
		"pub fn main() { let x = (1,2); x.0; x[0]; 3 }",
		"pub fn main() { let [a] = { 1; [2] }; a }",
		"pub fn main() { true && false; true || false; 3 }",
		"pub fn main() { (1,2); [1,2]; #{a:1,b:2}; 3 }",
		"pub fn main() { for _ in [1] { 1; 2 } 3 }",
		"pub fn main() { for _ in [1] { continue; 2 } 3 }",
		"pub fn main() { (return 1) + { 2; 3 }; 4 }",
		"pub fn main() { `hello {return 1} { { 2; 3 } }`; 4 }",
	] {
		compare(&context, &[text]);
	}
}

#[test]
#[cfg_attr(miri, ignore = "default-module frontend compilation")]
fn warnings_preserve_multiplicity_and_source_identity() {
	let context = Context::with_default_modules().unwrap();
	compare(
		&context,
		&[
			"pub fn main() { 1; 2; `x`; `y` }",
			"pub fn second() { let [x] = [1]; return x; 2 }",
		],
	);
}

#[test]
#[cfg_attr(miri, ignore = "default-module frontend compilation")]
fn approved_legacy_fatal_divergence_has_literal_warning_and_stays_admitted() {
	let context = Context::with_default_modules().unwrap();
	let text = "pub fn main() { let [a, b] = { return 7; [1, 2] }; a + b }";
	let mut diagnostics = Diagnostics::new();
	assert!(frontend::compile(&mut sources(&[text]), &context, &mut diagnostics, 1).is_ok());
	let actual = warnings(&diagnostics);
	assert_eq!(actual.len(), 1);
	assert_eq!((actual[0].0, actual[0].1, actual[0].2), (0, 41, 47));
	assert_eq!(actual[0].3, "Unreachable code");
	assert!(actual[0]
		.4
		.contains("Unreachable { span: 41:47, cause: 31:39 }"));
}

#[test]
#[cfg(feature = "doc")]
#[cfg_attr(miri, ignore = "default-module frontend compilation")]
fn deprecation_matches_live_legacy() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.host_function("old", || 1i64)
		.unwrap()
		.deprecated("W1 witness")
		.unwrap();
	context.install(module).unwrap();
	compare(&context, &["pub fn main() { old(); old() }"]);
}

#[test]
fn nested_warning_walk_uses_bounded_rust_stack_and_keeps_duplicate_diagnostics() {
	use crate::ast::Span;
	use crate::hir::{Expr, ExprKind, Exprs, Lit};
	let mut exprs = Exprs::new();
	let mut id = exprs
		.insert(Expr {
			span: Span::new(4, 5),
			kind: ExprKind::Lit(Lit::Signed(1)),
		})
		.unwrap();
	for _ in 0..if cfg!(miri) { 32 } else { 20_000 } {
		id = exprs
			.insert(Expr {
				span: Span::new(3, 6),
				kind: ExprKind::Group(id),
			})
			.unwrap();
	}
	let mut diagnostics = Diagnostics::new();
	for _ in 0..2 {
		super::walk(
			&exprs,
			super::Work::Expr(exprs.get(id), false, Span::new(0, 7)),
			crate::SourceId::empty(),
			&mut diagnostics,
		)
		.unwrap();
	}
	let actual = warnings(&diagnostics);
	assert_eq!(actual.len(), 2);
	assert_eq!(actual[0], actual[1]);
	assert_eq!((actual[0].1, actual[0].2), (4, 5));
	assert_eq!(actual[0].3, "Not used");
	assert!(actual[0].4.contains("context: Some(0:7)"));
}

#[test]
#[cfg(not(miri))]
fn policy_inputs_have_no_saved_compile_warning_entries() {
	fn check(value: &serde_json::Value, path: &std::path::Path) {
		match value {
			serde_json::Value::String(text) => {
				assert!(
					!text.contains("Warning(WarningDiagnostic"),
					"saved warning in {}",
					path.display()
				);
				// Some fixed Miri policies encode JSON as hexadecimal strings.
				if !text.is_empty()
					&& text.len().is_multiple_of(2)
					&& text.bytes().all(|b| b.is_ascii_hexdigit())
				{
					let bytes = text
						.as_bytes()
						.as_chunks::<2>()
						.0
						.iter()
						.map(|pair| {
							u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
						})
						.collect::<std::vec::Vec<_>>();
					if let Ok(value) = serde_json::from_slice(&bytes) {
						check(&value, path);
					}
				}
			}
			serde_json::Value::Array(values) => {
				for value in values {
					check(value, path);
				}
			}
			serde_json::Value::Object(values) => {
				if let Some(kind) = values.get("kind").and_then(serde_json::Value::as_str) {
					assert!(
						!matches!(
							kind,
							"NotUsed"
								| "Unreachable" | "LetPatternMightPanic"
								| "TemplateWithoutExpansions"
								| "RemoveTupleCallParams"
								| "UsedDeprecated"
						),
						"saved warning kind in {}",
						path.display()
					);
				}
				if let Some(warnings) = values.get("warnings") {
					assert!(
						!warnings.is_array() && !warnings.is_object(),
						"saved warning list or waiver in {}",
						path.display()
					);
				}
				for value in values.values() {
					check(value, path);
				}
			}
			_ => {}
		}
	}
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("src/experimental_core/tests/gap_hunt");
	let mut folders = vec![root];
	let mut checked = 0;
	while let Some(folder) = folders.pop() {
		for entry in std::fs::read_dir(folder).unwrap() {
			let path = entry.unwrap().path();
			if path.is_dir() {
				folders.push(path);
			} else if path
				.extension()
				.is_some_and(|extension| extension == "json")
			{
				check(
					&serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(),
					&path,
				);
				checked += 1;
			}
		}
	}
	assert!(checked > 0);
	let fixture_path = std::path::Path::new("src/experimental_core/tests/miri_fixtures.rs");
	for literal in include_str!("../../experimental_core/tests/miri_fixtures.rs")
		.split("r########\"")
		.skip(1)
	{
		let text = literal.split_once("\"########").unwrap().0;
		check(&serde_json::from_str(text).unwrap(), fixture_path);
	}
}
