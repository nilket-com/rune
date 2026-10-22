//! Receiver identities that no admitted fresh value can carry.
//!
//! These exact stock types have no builtin payload, constructor or host import
//! representation. Nominal admission also reserves their hashes, including enum
//! parent hashes, so a source declaration in another Context cannot mint one.
//! Roots preserve this invariant across programs: hosts cannot forge a Nominal.
//! Adding a representation for any of these types must first remove its exclusion
//! and implement or refuse every affected dynamic method provider.
use super::error::{Error, Refusal, Result};
use crate::{Hash, TypeHash};

pub(super) const UNREPRESENTED: [Hash; 3] = [
	crate::runtime::Bytes::HASH,
	crate::modules::collections::HashMap::HASH,
	crate::modules::collections::HashSet::HASH,
];

pub(super) fn unreachable(receiver: Hash) -> bool {
	UNREPRESENTED.contains(&receiver)
}

/// Reject before looking up or publishing a nominal, even if an earlier
/// descriptor already has the hash. A variant dispatches by its enum's hash.
pub(super) fn check_nominal(hash: u64, enum_hash: u64) -> Result<()> {
	if unreachable(Hash::new(hash)) || unreachable(Hash::new(enum_hash)) {
		return Err(Error::refused(Refusal::TypeExpr));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn excluded_receivers_are_disjoint_from_represented_builtins() {
		for receiver in UNREPRESENTED {
			assert!(!crate::function::fresh_methods::RECEIVERS.contains(&receiver));
			assert!(unreachable(receiver));
			assert_eq!(
				check_nominal(receiver.into_inner(), 0).unwrap_err().refusal,
				Some(Refusal::TypeExpr)
			);
			assert_eq!(
				check_nominal(1, receiver.into_inner()).unwrap_err().refusal,
				Some(Refusal::TypeExpr)
			);
		}
		assert!(!unreachable(Hash::new(1)));
		check_nominal(1, 0).unwrap();
	}
}
