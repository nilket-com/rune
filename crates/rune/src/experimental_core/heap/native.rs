//! B1: the native tier's read of a RangeIterator at region entry and validation, and its
//! one Store mutation, the commit of `next`. Safe Rust (the fresh core is
//! `forbid(unsafe_code)`).
//!
//! Every check of the setter runs before its single write, on the same exclusive borrow
//! of the store: a refusal mutates nothing.

/// The setter refused; nothing was mutated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RangeCommitRefused;

/// `new` is a value of `next` that `iterator_next` could reach from `(from, end)`.
/// Pure comparisons: no arithmetic, so no overflow at any i64 boundary.
pub(crate) fn range_next_reachable((from, end): (i64, i64), new: i64) -> bool {
	if from >= end {
		new == from
	} else {
		from <= new && new <= end
	}
}

#[cfg(test)]
mod tests;
