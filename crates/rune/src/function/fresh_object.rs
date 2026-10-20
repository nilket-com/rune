//! C17b closed Object method and `is_empty` policy. Registration is constant
//! work; no handler is invoked.
use crate::runtime as rt;
use crate::{Hash, TypeHash};

/// Private module markers: the std module that registers each provider.
pub(crate) mod marker {
	pub(crate) const OBJECT: u8 = 1;
	pub(crate) const VEC: u8 = 2;
	pub(crate) const TUPLE: u8 = 4;
	pub(crate) const STRING: u8 = 8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Method {
	Get,
	ContainsKey,
	Insert,
	Remove,
	IsEmpty,
}

/// A receiver family with a certified stock provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
	Object,
	Vec,
	Tuple,
	String,
}

impl Method {
	pub(crate) const ALL: [Self; 5] = [
		Self::Get,
		Self::ContainsKey,
		Self::Insert,
		Self::Remove,
		Self::IsEmpty,
	];
	const FIRST_ID: u32 = 0x8000_00a0;

	pub(crate) fn from_name(name: &str) -> Option<Self> {
		Some(match name.as_bytes() {
			b"get" => Self::Get,
			b"contains_key" => Self::ContainsKey,
			b"insert" => Self::Insert,
			b"remove" => Self::Remove,
			b"is_empty" => Self::IsEmpty,
			_ => return None,
		})
	}

	/// `Hash::ident` of the method name, computed at compile time.
	pub(crate) const fn hash(self) -> Hash {
		let associated = match self {
			Self::Get => crate::hash_in!(crate, ::std::object::Object.get),
			Self::ContainsKey => crate::hash_in!(crate, ::std::object::Object.contains_key),
			Self::Insert => crate::hash_in!(crate, ::std::object::Object.insert),
			Self::Remove => crate::hash_in!(crate, ::std::object::Object.remove),
			Self::IsEmpty => crate::hash_in!(crate, ::std::object::Object.is_empty),
		};
		Hash::new(associated.into_inner() ^ rt::Object::HASH.into_inner() ^ 0x5ea7_7ffb_cdf5_f302)
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
			Self::IsEmpty => 1,
			Self::Insert => 3,
			Self::Get | Self::ContainsKey | Self::Remove => 2,
		}
	}

	/// Whether a site needs a static proof that its receiver is an Object. Vec,
	/// Tuple and String provide these names too, with other semantics.
	pub(crate) const fn shared_name(self) -> bool {
		matches!(self, Self::Get | Self::Insert | Self::Remove)
	}

	/// Whether `family` is a certified provider of this method.
	pub(crate) const fn provided_by(self, family: Family) -> bool {
		match self {
			Self::IsEmpty => true,
			_ => matches!(family, Family::Object),
		}
	}

	/// The provider family of `receiver` for this method, if one is certified.
	pub(crate) fn provider(self, receiver: Hash) -> Option<Family> {
		let family = if receiver == rt::Object::HASH {
			Family::Object
		} else if receiver == rt::Vec::HASH {
			Family::Vec
		} else if receiver == rt::OwnedTuple::HASH {
			Family::Tuple
		} else if receiver == crate::alloc::String::HASH {
			Family::String
		} else {
			return None;
		};
		self.provided_by(family).then_some(family)
	}

	fn required(self) -> u8 {
		[Family::Object, Family::Vec, Family::Tuple, Family::String]
			.into_iter()
			.filter(|family| self.provided_by(*family))
			.fold(0, |bits, family| bits | family_bit(family))
	}
}

const fn family_bit(family: Family) -> u8 {
	match family {
		Family::Object => marker::OBJECT,
		Family::Vec => marker::VEC,
		Family::Tuple => marker::TUPLE,
		Family::String => marker::STRING,
	}
}

/// Certification state. A method is ready only after every certified provider
/// was installed by its std module as a plain instance function and no other
/// registration touched a provider receiver under the same name.
#[derive(Default)]
pub(crate) struct State {
	pending: u8,
	certified: [u8; 5],
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
			(Method::Get, "get"),
			(Method::ContainsKey, "contains_key"),
			(Method::Insert, "insert"),
			(Method::Remove, "remove"),
			(Method::IsEmpty, "is_empty"),
		] {
			assert_eq!(m.hash(), Hash::ident(name));
			assert_eq!(Method::from_name(name), Some(m));
			assert_eq!(Method::from_hash(Hash::ident(name)), Some(m));
			assert_eq!(Method::from_id(m.id()), Some(m));
		}
		assert_eq!(Method::from_id(Method::IsEmpty.id() + 1), None);
		assert_eq!(Method::from_id(Method::Get.id() - 1), None);
		assert_eq!(Method::from_name("len"), None);
	}

	#[test]
	fn providers() {
		assert_eq!(Method::Get.provider(rt::Object::HASH), Some(Family::Object));
		assert_eq!(Method::Get.provider(rt::Vec::HASH), None);
		assert_eq!(Method::ContainsKey.provider(rt::OwnedTuple::HASH), None);
		assert_eq!(
			Method::IsEmpty.provider(crate::alloc::String::HASH),
			Some(Family::String)
		);
		assert_eq!(
			Method::IsEmpty.provider(rt::OwnedTuple::HASH),
			Some(Family::Tuple)
		);
		assert_eq!(Method::IsEmpty.provider(rt::Range::HASH), None);
	}

	#[test]
	fn foreign_or_partial_install_is_not_certified() {
		let mut s = State::default();
		s.begin_module(marker::OBJECT);
		for m in Method::ALL {
			let bit = s.begin(m, rt::Object::HASH, true, None).unwrap();
			s.installed(m, bit);
		}
		s.finish_module(0);
		assert!(s.ready(Method::Get));
		// Vec, Tuple and String have not installed `is_empty` yet.
		assert!(!s.ready(Method::IsEmpty));
		for (marker, receiver) in [
			(marker::VEC, rt::Vec::HASH),
			(marker::TUPLE, rt::OwnedTuple::HASH),
			(marker::STRING, crate::alloc::String::HASH),
		] {
			s.begin_module(marker);
			// Their own `get` is not a provider, so it neither certifies nor closes.
			assert!(s.begin(Method::Get, receiver, true, None).is_none());
			let bit = s.begin(Method::IsEmpty, receiver, true, None).unwrap();
			s.installed(Method::IsEmpty, bit);
			s.finish_module(0);
		}
		assert!(s.ready(Method::IsEmpty));
		assert!(s.ready(Method::Get));
		// A registration outside the std module closes the name.
		assert!(s
			.begin(Method::Insert, rt::Object::HASH, true, None)
			.is_none());
		assert!(!s.ready(Method::Insert));
		assert!(s.ready(Method::Remove));
		// A trait or parameterized registration closes it too.
		s.begin_module(marker::VEC);
		assert!(s
			.begin(Method::IsEmpty, rt::Vec::HASH, true, Some(Hash::new(1)))
			.is_none());
		s.finish_module(0);
		assert!(!s.ready(Method::IsEmpty));
		assert!(s
			.begin(Method::Remove, rt::Object::HASH, false, None)
			.is_none());
		assert!(!s.ready(Method::Remove));
	}
}
