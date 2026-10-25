//! C14 closed provider policy. Registration is constant work; no handler is invoked.
use crate::runtime as rt;
use crate::{Hash, TypeHash};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Method {
	Split,
	Replace,
	StartsWith,
	EndsWith,
	Contains,
	Trim,
}
impl Method {
	pub(crate) const ALL: [Self; 6] = [
		Self::Split,
		Self::Replace,
		Self::StartsWith,
		Self::EndsWith,
		Self::Contains,
		Self::Trim,
	];
	#[cfg(test)]
	pub(crate) const fn name(self) -> &'static str {
		match self {
			Self::Split => "split",
			Self::Replace => "replace",
			Self::StartsWith => "starts_with",
			Self::EndsWith => "ends_with",
			Self::Contains => "contains",
			Self::Trim => "trim",
		}
	}
	#[inline]
	pub(crate) fn from_name(s: &str) -> Option<Self> {
		Some(match s.as_bytes() {
			b"split" => Self::Split,
			b"replace" => Self::Replace,
			b"starts_with" => Self::StartsWith,
			b"ends_with" => Self::EndsWith,
			b"contains" => Self::Contains,
			b"trim" => Self::Trim,
			_ => return None,
		})
	}
	pub(crate) const fn hash(self) -> Hash {
		let a = match self {
			Self::Split => crate::hash_in!(crate, ::std::string::String.split),
			Self::Replace => crate::hash_in!(crate, ::std::string::String.replace),
			Self::StartsWith => crate::hash_in!(crate, ::std::string::String.starts_with),
			Self::EndsWith => crate::hash_in!(crate, ::std::string::String.ends_with),
			Self::Contains => crate::hash_in!(crate, ::std::string::String.contains),
			Self::Trim => crate::hash_in!(crate, ::std::string::String.trim),
		};
		Hash::new(a.into_inner() ^ crate::alloc::String::HASH.into_inner() ^ 0x5ea7_7ffb_cdf5_f302)
	}
	pub(crate) fn from_hash(h: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|m| m.hash() == h)
	}
	pub(crate) const fn bit(self) -> u8 {
		1 << self as u8
	}
	pub(crate) const fn arity(self) -> u32 {
		match self {
			Self::Trim => 1,
			Self::Replace => 3,
			_ => 2,
		}
	}
	pub(crate) const fn id(self) -> u32 {
		0x8000_0060 + self as u32
	}
}
pub(crate) const fn split_hash(string: bool) -> Hash {
	if string {
		crate::modules::string::Split::<crate::alloc::String>::HASH
	} else {
		crate::modules::string::Split::<char>::HASH
	}
}
pub(crate) fn range(h: Hash) -> bool {
	const R: [Hash; 6] = [
		rt::Range::HASH,
		rt::RangeFrom::HASH,
		rt::RangeFull::HASH,
		rt::RangeInclusive::HASH,
		rt::RangeTo::HASH,
		rt::RangeToInclusive::HASH,
	];
	R.contains(&h)
}
pub(crate) fn collect_hash() -> Hash {
	Hash::ident("collect").with_function_parameters(Hash::parameters([rt::Vec::HASH]))
}
#[derive(Default)]
pub(crate) struct State {
	pending: bool,
	certified: u8,
	closed: u8,
	collect: u8,
}
impl State {
	pub(crate) fn begin_module(&mut self, p: bool) -> bool {
		core::mem::replace(&mut self.pending, p)
	}
	pub(crate) fn finish_module(&mut self, p: bool) {
		self.pending = p;
	}
	pub(crate) fn begin(&mut self, m: Method, receiver: Hash, plain: bool) -> bool {
		let builtin = self.pending && plain && receiver == crate::alloc::String::HASH;
		// The six stock range providers are outside contains' proven call domain.
		if !builtin
			&& crate::function::iter::representable(receiver)
			&& !(m == Method::Contains && range(receiver))
		{
			self.closed |= m.bit();
		}
		builtin
	}
	pub(crate) fn installed(&mut self, m: Method) {
		self.certified |= m.bit();
	}
	pub(crate) fn ready(&self, m: Method) -> bool {
		!self.pending && self.closed & m.bit() == 0 && self.certified & m.bit() != 0
	}
	pub(crate) fn collect_installed(&mut self, receiver: Hash, plain_trait: bool) {
		if self.pending && plain_trait {
			if receiver == split_hash(true) {
				self.collect |= 1;
			}
			if receiver == split_hash(false) {
				self.collect |= 2;
			}
		}
	}
	pub(crate) fn collect_ready(&self) -> bool {
		!self.pending && self.collect == 3
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn hashes() {
		for m in Method::ALL {
			assert_eq!(m.hash(), Hash::ident(m.name()));
		}
		assert_ne!(split_hash(true), split_hash(false));
	}
}
