//! C17a closed Option and Result method policy. Registration is constant work;
//! no handler is invoked.
use crate::runtime as rt;
use crate::{Hash, TypeHash};

/// Private module markers: the std module that registers each provider.
pub(crate) mod marker {
	pub(crate) const OPTION: u8 = 1;
	pub(crate) const RESULT: u8 = 2;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Method {
	Unwrap,
	Expect,
	IsSome,
	IsNone,
	IsOk,
	IsErr,
}

/// The receiver family a method was called on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
	Option,
	Result,
}

impl Method {
	pub(crate) const ALL: [Self; 6] = [
		Self::Unwrap,
		Self::Expect,
		Self::IsSome,
		Self::IsNone,
		Self::IsOk,
		Self::IsErr,
	];
	const FIRST_ID: u32 = 0x8000_0090;

	pub(crate) fn from_name(name: &str) -> Option<Self> {
		Some(match name.as_bytes() {
			b"unwrap" => Self::Unwrap,
			b"expect" => Self::Expect,
			b"is_some" => Self::IsSome,
			b"is_none" => Self::IsNone,
			b"is_ok" => Self::IsOk,
			b"is_err" => Self::IsErr,
			_ => return None,
		})
	}

	/// `Hash::ident` of the method name, computed at compile time.
	pub(crate) const fn hash(self) -> Hash {
		let associated = match self {
			Self::Unwrap => crate::hash_in!(crate, ::std::option::Option.unwrap),
			Self::Expect => crate::hash_in!(crate, ::std::option::Option.expect),
			Self::IsSome => crate::hash_in!(crate, ::std::option::Option.is_some),
			Self::IsNone => crate::hash_in!(crate, ::std::option::Option.is_none),
			Self::IsOk => crate::hash_in!(crate, ::std::option::Option.is_ok),
			Self::IsErr => crate::hash_in!(crate, ::std::option::Option.is_err),
		};
		Hash::new(
			associated.into_inner()
				^ <Option<rt::Value> as TypeHash>::HASH.into_inner()
				^ 0x5ea7_7ffb_cdf5_f302,
		)
	}

	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|m| m.hash() == hash)
	}

	pub(crate) fn from_id(id: u32) -> Option<Self> {
		Self::ALL
			.get(id.checked_sub(Self::FIRST_ID)? as usize)
			.copied()
	}

	pub(crate) const fn id(self) -> u32 {
		Self::FIRST_ID + self as u32
	}

	pub(crate) const fn bit(self) -> u8 {
		1 << self as u8
	}

	/// Operand count, receiver included.
	pub(crate) const fn arity(self) -> u32 {
		match self {
			Self::Expect => 2,
			_ => 1,
		}
	}

	/// Whether `family` registers this method in stock.
	pub(crate) const fn provided_by(self, family: Family) -> bool {
		match self {
			Self::Unwrap | Self::Expect => true,
			Self::IsSome | Self::IsNone => matches!(family, Family::Option),
			Self::IsOk | Self::IsErr => matches!(family, Family::Result),
		}
	}

	/// The provider family of `receiver` for this method, if stock registers one.
	pub(crate) fn provider(self, receiver: Hash) -> Option<Family> {
		let family = if receiver == <Option<rt::Value> as TypeHash>::HASH {
			Family::Option
		} else if receiver == <Result<rt::Value, rt::Value> as TypeHash>::HASH {
			Family::Result
		} else {
			return None;
		};
		self.provided_by(family).then_some(family)
	}

	fn required(self) -> u8 {
		[Family::Option, Family::Result]
			.into_iter()
			.filter(|family| self.provided_by(*family))
			.fold(0, |bits, family| bits | family_bit(family))
	}
}

const fn family_bit(family: Family) -> u8 {
	match family {
		Family::Option => marker::OPTION,
		Family::Result => marker::RESULT,
	}
}

/// Certification state. A method is ready only after every stock provider was
/// installed by its std module as a plain instance function and no other
/// registration touched a provider receiver under the same name.
#[derive(Default)]
pub(crate) struct State {
	pending: u8,
	certified: [u8; 6],
	closed: u8,
}

impl State {
	pub(crate) fn begin_module(&mut self, marker: u8) -> u8 {
		core::mem::replace(&mut self.pending, marker)
	}

	pub(crate) fn finish_module(&mut self, marker: u8) {
		self.pending = marker;
	}

	/// Returns the provider bit to certify once the handler is installed.
	pub(crate) fn begin(
		&mut self,
		m: Method,
		receiver: Hash,
		plain: bool,
		trait_hash: Option<Hash>,
	) -> Option<u8> {
		let family = m.provider(receiver)?;
		let bit = family_bit(family);
		if plain && trait_hash.is_none() && self.pending & bit != 0 {
			Some(bit)
		} else {
			self.closed |= m.bit();
			None
		}
	}

	pub(crate) fn installed(&mut self, m: Method, bit: u8) {
		self.certified[m as usize] |= bit;
	}

	pub(crate) fn ready(&self, m: Method) -> bool {
		self.pending == 0
			&& self.closed & m.bit() == 0
			&& self.certified[m as usize] == m.required()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hashes_and_ids() {
		for (m, name) in [
			(Method::Unwrap, "unwrap"),
			(Method::Expect, "expect"),
			(Method::IsSome, "is_some"),
			(Method::IsNone, "is_none"),
			(Method::IsOk, "is_ok"),
			(Method::IsErr, "is_err"),
		] {
			assert_eq!(m.hash(), Hash::ident(name));
			assert_eq!(Method::from_name(name), Some(m));
			assert_eq!(Method::from_hash(Hash::ident(name)), Some(m));
			assert_eq!(Method::from_id(m.id()), Some(m));
		}
		assert_eq!(Method::from_id(Method::IsErr.id() + 1), None);
		assert_eq!(Method::from_id(Method::Unwrap.id() - 1), None);
		assert_eq!(Method::from_name("unwrap_or"), None);
	}

	#[test]
	fn providers() {
		let option = <Option<rt::Value> as TypeHash>::HASH;
		let result = <Result<rt::Value, rt::Value> as TypeHash>::HASH;
		assert_eq!(Method::Unwrap.provider(option), Some(Family::Option));
		assert_eq!(Method::Expect.provider(result), Some(Family::Result));
		assert_eq!(Method::IsSome.provider(result), None);
		assert_eq!(Method::IsOk.provider(option), None);
		assert_eq!(Method::Unwrap.provider(rt::Vec::HASH), None);
	}

	#[test]
	fn foreign_or_partial_install_is_not_certified() {
		let option = <Option<rt::Value> as TypeHash>::HASH;
		let result = <Result<rt::Value, rt::Value> as TypeHash>::HASH;
		let mut s = State::default();
		s.begin_module(marker::OPTION);
		let bit = s.begin(Method::Unwrap, option, true, None).unwrap();
		s.installed(Method::Unwrap, bit);
		let bit = s.begin(Method::IsSome, option, true, None).unwrap();
		s.installed(Method::IsSome, bit);
		s.finish_module(0);
		// Result has not installed its provider yet.
		assert!(!s.ready(Method::Unwrap));
		assert!(s.ready(Method::IsSome));
		s.begin_module(marker::RESULT);
		let bit = s.begin(Method::Unwrap, result, true, None).unwrap();
		assert!(!s.ready(Method::Unwrap));
		s.installed(Method::Unwrap, bit);
		s.finish_module(0);
		assert!(s.ready(Method::Unwrap));
		// A registration outside the std module closes the name.
		assert!(s.begin(Method::Unwrap, option, true, None).is_none());
		assert!(!s.ready(Method::Unwrap));
		assert!(s.ready(Method::IsSome));
		// A trait or parameterized registration closes it too.
		s.begin_module(marker::OPTION);
		assert!(s
			.begin(Method::IsSome, option, true, Some(Hash::new(1)))
			.is_none());
		s.finish_module(0);
		assert!(!s.ready(Method::IsSome));
		// Unrelated receivers are not providers.
		assert!(s.begin(Method::IsOk, rt::Vec::HASH, true, None).is_none());
		assert!(!s.ready(Method::IsOk));
	}
}
