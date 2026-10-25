//! Exact identifier projection for frozen observer source hashes.
//! This does not normalize values, whitespace, or serialized observations.

use std::string::String;
use std::vec::Vec;

pub(crate) fn observer_source(source: &str) -> String {
	let mut out = String::with_capacity(source.len());
	let mut at = 0;
	while at < source.len() {
		let ch = source[at..].chars().next().unwrap();
		if ch.is_ascii_alphabetic() || ch == '_' {
			let start = at;
			at += 1;
			while at < source.len() {
				let byte = source.as_bytes()[at];
				if !byte.is_ascii_alphanumeric() && byte != b'_' {
					break;
				}
				at += 1;
			}
			out.push_str(match &source[start..at] {
				"RuntimeGraph" => "Graph",
				"runtime_graph" => "graph",
				"RuntimeValue" => "Value",
				token => token,
			});
		} else {
			out.push(ch);
			at += ch.len_utf8();
		}
	}
	out
}

pub(crate) fn observer_bytes(name: &str, source: &str) -> Vec<u8> {
	if name.ends_with(".rs") {
		observer_source(source).into_bytes()
	} else {
		source.as_bytes().to_vec()
	}
}
