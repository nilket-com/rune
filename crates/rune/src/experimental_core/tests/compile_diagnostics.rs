//! Shared compile-diagnostic observations for native and Miri test modules.

use crate::Diagnostics;
use std::{
	format,
	string::{String, ToString},
	vec::Vec,
};

pub(super) fn nonwarning_diagnostics(diagnostics: &Diagnostics) -> String {
	let diagnostics: Vec<_> = diagnostics
		.diagnostics()
		.iter()
		.filter(|d| !matches!(d, crate::diagnostics::Diagnostic::Warning(_)))
		.collect();
	format!("{diagnostics:?}")
}

pub(super) fn warning_multiset(diagnostics: &Diagnostics) -> Vec<serde_json::Value> {
	let mut warnings: Vec<_> = diagnostics.diagnostics().iter().filter_map(|d| {
		let crate::diagnostics::Diagnostic::Warning(warning) = d else { return None; };
		let span = crate::ast::Spanned::span(warning);
		Some(serde_json::json!({"source":warning.source_id().into_index(),"start":span.start.0,"end":span.end.0,
			"message":warning.to_string(),"kind_and_auxiliary":format!("{warning:?}")}))
	}).collect();
	warnings.sort_unstable_by_key(serde_json::Value::to_string);
	warnings
}
