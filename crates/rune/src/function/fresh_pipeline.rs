//! C14b exact native and trait-installed text pipeline certification.
use crate::runtime as rt;
use crate::{Hash, TypeHash};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Method {
	Chars,
	Unwrap,
	Skip,
	Take,
	Count,
	Next,
	CollectVec,
	CollectString,
}
impl Method {
	pub(crate) fn from_name(n: &str, parameters: Hash) -> Option<Self> {
		Some(match n.as_bytes() {
			b"chars" if parameters == Hash::EMPTY => Self::Chars,
			b"unwrap_or" if parameters == Hash::EMPTY => Self::Unwrap,
			b"skip" if parameters == Hash::EMPTY => Self::Skip,
			b"take" if parameters == Hash::EMPTY => Self::Take,
			b"count" if parameters == Hash::EMPTY => Self::Count,
			b"next" if parameters == Hash::EMPTY => Self::Next,
			b"collect" if parameters == Hash::parameters([rt::Vec::HASH]) => Self::CollectVec,
			b"collect" if parameters == Hash::parameters([crate::alloc::String::HASH]) => {
				Self::CollectString
			}
			_ => return None,
		})
	}
	pub(crate) const fn hash(self) -> Hash {
		let a = match self {
			Self::Chars => crate::hash_in!(crate, ::std::string::String.chars),
			Self::Unwrap => crate::hash_in!(crate, ::std::string::String.unwrap_or),
			Self::Skip => crate::hash_in!(crate, ::std::string::String.skip),
			Self::Take => crate::hash_in!(crate, ::std::string::String.take),
			Self::Count => crate::hash_in!(crate, ::std::string::String.count),
			Self::Next => crate::hash_in!(crate, ::std::string::String.next),
			Self::CollectVec | Self::CollectString => {
				crate::hash_in!(crate, ::std::string::String.collect)
			}
		};
		Hash::new(a.into_inner() ^ crate::alloc::String::HASH.into_inner() ^ 0x5ea7_7ffb_cdf5_f302)
	}
	pub(crate) const fn arity(self) -> usize {
		match self {
			Self::Unwrap | Self::Skip | Self::Take => 2,
			_ => 1,
		}
	}
	pub(crate) const fn id(self) -> u32 {
		0x8000_0070 + self as u32
	}
	pub(crate) fn provider(self, h: Hash) -> Option<(u8, u8)> {
		if self == Self::Chars {
			return (h == crate::alloc::String::HASH).then_some((1, 1));
		}
		if self == Self::Unwrap {
			return if h == Option::<rt::Value>::HASH {
				Some((1, 2))
			} else if h == Result::<rt::Value, rt::Value>::HASH {
				Some((2, 4))
			} else {
				None
			};
		}
		origins()
			.iter()
			.position(|x| *x == h)
			.map(|i| (1 << i, if i >= 3 { 8 } else { 1 }))
	}
	fn required(self) -> u8 {
		match self {
			Self::Chars => 1,
			Self::Unwrap => 3,
			_ => 31,
		}
	}
}
pub(crate) fn origins() -> [Hash; 5] {
	[
		super::fresh_text::split_hash(true),
		super::fresh_text::split_hash(false),
		crate::modules::string::Chars::HASH,
		crate::modules::iter::Skip::HASH,
		crate::modules::iter::Take::HASH,
	]
}
#[derive(Default)]
pub(crate) struct State {
	pending: u8,
	certified: [u8; 8],
	closed: u8,
}
impl State {
	pub(crate) fn begin_module(&mut self, n: u8) -> u8 {
		core::mem::replace(&mut self.pending, n)
	}
	pub(crate) fn finish_module(&mut self, n: u8) {
		self.pending = n;
	}
	pub(crate) fn begin(
		&mut self,
		m: Method,
		h: Hash,
		plain: bool,
		trait_hash: Option<Hash>,
		builtin_trait: bool,
	) -> Option<u8> {
		let p = m.provider(h);
		let valid = plain
			&& p.is_some_and(|(_, marker)| self.pending & marker != 0)
			&& if matches!(m, Method::Chars | Method::Unwrap) {
				trait_hash.is_none()
			} else {
				builtin_trait && trait_hash == Some(crate::hash_in!(crate, ::std::iter::Iterator))
			};
		if !valid && p.is_some() {
			self.closed |= 1 << m as u8;
		}
		if valid {
			p.map(|(bit, _)| bit)
		} else {
			None
		}
	}
	pub(crate) fn installed(&mut self, m: Method, bit: u8) {
		self.certified[m as usize] |= bit;
	}
	pub(crate) fn ready(&self, m: Method) -> bool {
		self.pending == 0
			&& self.closed & (1 << m as u8) == 0
			&& self.certified[m as usize] == m.required()
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn const_hashes() {
		for (m, n) in [
			(Method::Chars, "chars"),
			(Method::Unwrap, "unwrap_or"),
			(Method::Skip, "skip"),
			(Method::Take, "take"),
			(Method::Count, "count"),
			(Method::Next, "next"),
			(Method::CollectVec, "collect"),
		] {
			assert_eq!(m.hash(), Hash::ident(n));
		}
	}
	#[test]
	fn foreign_or_failed_install_is_not_certified() {
		let mut s = State::default();
		s.begin_module(1);
		let b = s
			.begin(Method::Chars, crate::alloc::String::HASH, true, None, false)
			.unwrap();
		assert!(!s.ready(Method::Chars));
		s.installed(Method::Chars, b);
		s.finish_module(0);
		assert!(s.ready(Method::Chars));
		s.begin(
			Method::Chars,
			crate::alloc::String::HASH,
			false,
			None,
			false,
		);
		assert!(!s.ready(Method::Chars));
	}
}
