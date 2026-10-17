//! Compile-time closure of unsupported method names. Never invokes a handler.
use crate::runtime as rt;
use crate::{hash, Hash, TypeHash};

const ASSOCIATED: u64 = 0x5ea7_7ffb_cdf5_f302;

macro_rules! receivers {
	($($name:ident: $ty:ty),* $(,)?) => {
		$(const $name: u64 = <$ty>::HASH.into_inner();)*
		pub(crate) const RECEIVERS: &[Hash] = &[$(<$ty>::HASH,)*];
		fn represented(receiver: Hash) -> bool {
			matches!(receiver.into_inner(), $($name)|*)
		}
	};
}
receivers! {
	TUPLE: rt::OwnedTuple, BOOL: bool, I64: i64, U64: u64, F64: f64, CHAR: char,
	ORDERING: core::cmp::Ordering, STRING: crate::alloc::String, VEC: rt::Vec,
	OBJECT: rt::Object, OPTION: Option<rt::Value>, RESULT: Result<rt::Value, rt::Value>,
	FUNCTION: rt::Function, TYPE: rt::Type, RANGE: rt::Range, FROM: rt::RangeFrom,
	FULL: rt::RangeFull, INCLUSIVE: rt::RangeInclusive, TO_INCLUSIVE: rt::RangeToInclusive,
	TO: rt::RangeTo, VEC_ITER: rt::slice::Iter, OPTION_ITER: crate::modules::option::Iter,
	SIGNED_ITER: rt::range::RangeIter<i64>, UNSIGNED_ITER: rt::range::RangeIter<u64>,
	CHAR_ITER: rt::range::RangeIter<char>,
	SIGNED_INCLUSIVE_ITER: rt::range_inclusive::RangeInclusiveIter<i64>,
	UNSIGNED_INCLUSIVE_ITER: rt::range_inclusive::RangeInclusiveIter<u64>,
	CHAR_INCLUSIVE_ITER: rt::range_inclusive::RangeInclusiveIter<char>,
	SIGNED_FROM_ITER: rt::range_from::RangeFromIter<i64>,
	UNSIGNED_FROM_ITER: rt::range_from::RangeFromIter<u64>,
	CHAR_FROM_ITER: rt::range_from::RangeFromIter<char>,
 SPLIT_STRING: crate::modules::string::Split<crate::alloc::String>,
 SPLIT_CHAR: crate::modules::string::Split<char>,
}

#[derive(Default)]
pub(crate) struct State {
	names: hash::Map<()>,
	all: bool,
}
impl State {
	pub(crate) fn close(&mut self, name: Hash) {
		if !self.all && self.names.try_insert(name, ()).is_err() {
			self.all = true;
		}
	}
	pub(crate) fn close_all(&mut self) {
		self.all = true;
	}
	pub(crate) fn closed(&self, name: Hash) -> bool {
		self.all || self.names.contains_key(&name)
	}
	/// `base` is the associated hash already computed by stock, before parameters.
	pub(crate) fn associated(&mut self, receiver: Hash, base: Hash) {
		if represented(receiver) {
			self.close(Hash::new(
				base.into_inner() ^ receiver.into_inner() ^ ASSOCIATED,
			));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn identity_and_fail_closed() {
		assert_eq!(RECEIVERS.len(), 33);
		for receiver in RECEIVERS {
			assert!(represented(*receiver));
			for name in ["unrecognized", "parse", "next_back"] {
				let hash = Hash::ident(name);
				let mut state = State::default();
				state.associated(*receiver, Hash::associated_function(*receiver, hash));
				assert!(state.closed(hash));
				assert!(!state.closed(Hash::ident("another")));
				state.close_all();
				assert!(state.closed(Hash::ident("another")));
			}
		}
	}
}
