//! Reviewed diagnostic wording relation; semantic identity is checked separately.

/// Recognize the D1 missing-method wording change for the same receiver name.
/// This does not establish semantic equality: compare class, origin, site,
/// missing-method hashes, arguments and effects independently.
pub fn d1_missing_method_message(stock: &str, fresh: &str) -> bool {
	let Some(receiver) = stock
		.strip_prefix("Type `")
		.and_then(|s| s.strip_suffix("` has no matching method"))
	else {
		return false;
	};
	let Some((other, method)) = fresh
		.strip_prefix("Type `")
		.and_then(|s| s.strip_suffix('`'))
		.and_then(|s| s.split_once("` has no method named `"))
	else {
		return false;
	};
	!receiver.is_empty()
		&& !receiver.contains('`')
		&& receiver == other
		&& !method.is_empty()
		&& method.chars().all(|c| c == '_' || c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_the_named_missing_method_rewrite_is_accepted() {
		let old = "Type `std::i64` has no matching method";
		assert!(d1_missing_method_message(
			old,
			"Type `std::i64` has no method named `len`"
		));
		for bad in [
			"Type `std::u64` has no method named `len`",
			"Type `std::i64` has no method named ``",
			"Type `std::i64` has no method named `len` extra",
			"Type `std::i64` has no method named `len!`",
			old,
		] {
			assert!(!d1_missing_method_message(old, bad), "{bad}");
		}
		assert!(!d1_missing_method_message(
			"not a missing method",
			"Type `std::i64` has no method named `len`"
		));
	}
}
