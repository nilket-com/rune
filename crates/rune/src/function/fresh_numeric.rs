//! Private numeric-handler identity policy. Never registers or invokes a handler.
use crate::Hash;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
	Abs,
	Round,
	Powf,
}
impl Method {
	pub(crate) fn name(self) -> &'static str {
		match self {
			Self::Abs => "abs",
			Self::Round => "round",
			Self::Powf => "powf",
		}
	}
	pub(crate) fn from_name(name: &str) -> Option<Self> {
		match name {
			"abs" => Some(Self::Abs),
			"round" => Some(Self::Round),
			"powf" => Some(Self::Powf),
			_ => None,
		}
	}
	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		[Self::Abs, Self::Round, Self::Powf]
			.into_iter()
			.find(|m| Hash::ident(m.name()) == hash)
	}
	pub(crate) fn bit(self) -> u8 {
		1 << self as u8
	}
	pub(crate) fn pairs(self) -> u8 {
		match self {
			Self::Abs => Pair::F64Abs.bit() | Pair::I64Abs.bit(),
			Self::Round => Pair::F64Round.bit(),
			Self::Powf => Pair::F64Powf.bit(),
		}
	}
	pub(crate) fn arity(self) -> u32 {
		if self == Self::Powf {
			2
		} else {
			1
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pair {
	F64Abs,
	F64Round,
	F64Powf,
	I64Abs,
}
impl Pair {
	pub(crate) const ALL: [Self; 4] = [Self::F64Abs, Self::F64Round, Self::F64Powf, Self::I64Abs];
	pub(crate) fn bit(self) -> u8 {
		1 << self as u8
	}
	pub(crate) fn method(self) -> Method {
		match self {
			Self::F64Abs | Self::I64Abs => Method::Abs,
			Self::F64Round => Method::Round,
			Self::F64Powf => Method::Powf,
		}
	}
	pub(crate) const fn receiver(self) -> Hash {
		match self {
			Self::I64Abs => crate::hash_in!(crate, ::std::i64),
			_ => crate::hash_in!(crate, ::std::f64),
		}
	}
	pub(crate) const fn item(self) -> Hash {
		match self {
			Self::F64Abs => crate::hash_in!(crate, ::std::f64::abs),
			Self::F64Round => crate::hash_in!(crate, ::std::f64::round),
			Self::F64Powf => crate::hash_in!(crate, ::std::f64::powf),
			Self::I64Abs => crate::hash_in!(crate, ::std::i64::abs),
		}
	}
	pub(crate) const fn associated(self) -> Hash {
		match self {
			Self::F64Abs => crate::hash_in!(crate, ::std::f64.abs),
			Self::F64Round => crate::hash_in!(crate, ::std::f64.round),
			Self::F64Powf => crate::hash_in!(crate, ::std::f64.powf),
			Self::I64Abs => crate::hash_in!(crate, ::std::i64.abs),
		}
	}
	pub(crate) fn from_item(hash: Hash) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|p| p.item() == hash || p.associated() == hash)
	}
}
pub(crate) struct State {
	// Three infallible, allocation-free name hashes computed once per Context.
	name_hashes: [Hash; 3],
	seen: u8,
	certified: u8,
	closed: u8,
	pending_methods: u8,
	pending_pairs: u8,
}
impl Default for State {
	fn default() -> Self {
		Self {
			name_hashes: [
				Hash::ident("abs"),
				Hash::ident("round"),
				Hash::ident("powf"),
			],
			seen: 0,
			certified: 0,
			closed: 0,
			pending_methods: 0,
			pending_pairs: 0,
		}
	}
}
impl State {
	pub(crate) fn name_hash(&self, method: Method) -> Hash {
		self.name_hashes[method as usize]
	}
	pub(crate) fn begin_module(&mut self, methods: u8, pairs: u8) -> (u8, u8) {
		let old = (self.pending_methods, self.pending_pairs);
		self.pending_methods |= methods;
		self.pending_pairs |= pairs;
		old
	}
	pub(crate) fn finish_module(&mut self, old: (u8, u8)) {
		self.pending_methods = old.0;
		self.pending_pairs = old.1;
	}
	pub(crate) fn begin(&mut self, method: Method) -> bool {
		let old = self.closed & method.bit() != 0;
		self.closed |= method.bit();
		old
	}
	pub(crate) fn installed(&mut self, pair: Pair, was_closed: bool) {
		self.seen |= pair.bit();
		if !was_closed {
			self.closed &= !pair.method().bit();
		}
	}
	pub(crate) fn certify(&mut self, pair: Pair) {
		self.certified |= pair.bit();
	}
	pub(crate) fn invalidate_hash(&mut self, hash: Hash) {
		for p in Pair::ALL {
			if p.item() == hash || p.associated() == hash {
				self.certified &= !p.bit();
			}
		}
	}
	pub(crate) fn method(&self, m: Method) -> bool {
		(self.closed | self.pending_methods) & m.bit() == 0
			&& self.seen & m.pairs() == m.pairs()
			&& self.certified & m.pairs() == m.pairs()
	}
	pub(crate) fn direct(&self, p: Pair) -> bool {
		self.pending_pairs & p.bit() == 0 && self.certified & p.bit() != 0
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
	MethodAbs,
	MethodRound,
	MethodPowf,
	F64Abs,
	F64Round,
	F64Powf,
	I64Abs,
}
impl Call {
	pub(crate) const ALL: [Self; 7] = [
		Self::MethodAbs,
		Self::MethodRound,
		Self::MethodPowf,
		Self::F64Abs,
		Self::F64Round,
		Self::F64Powf,
		Self::I64Abs,
	];
	pub(crate) const fn id(self) -> u32 {
		crate::experimental_core::code::FIRST_INTRINSIC_ID + 0x80 + self as u32
	}
	pub(crate) fn from_id(id: u32) -> Option<Self> {
		Self::ALL.into_iter().find(|c| c.id() == id)
	}
	pub(crate) fn method(m: Method) -> Self {
		match m {
			Method::Abs => Self::MethodAbs,
			Method::Round => Self::MethodRound,
			Method::Powf => Self::MethodPowf,
		}
	}
	pub(crate) fn direct(p: Pair) -> Self {
		match p {
			Pair::F64Abs => Self::F64Abs,
			Pair::F64Round => Self::F64Round,
			Pair::F64Powf => Self::F64Powf,
			Pair::I64Abs => Self::I64Abs,
		}
	}
	pub(crate) fn name(self) -> Method {
		match self {
			Self::MethodAbs | Self::F64Abs | Self::I64Abs => Method::Abs,
			Self::MethodRound | Self::F64Round => Method::Round,
			Self::MethodPowf | Self::F64Powf => Method::Powf,
		}
	}
	pub(crate) fn is_method(self) -> bool {
		matches!(self, Self::MethodAbs | Self::MethodRound | Self::MethodPowf)
	}
}

#[cfg(test)]
mod canonical_hash_tests {
	use super::*;
	use crate as rune;
	#[test]
	fn cached_hashes_equal_the_original_runtime_computations() {
		let state = State::default();
		for method in [Method::Abs, Method::Round, Method::Powf] {
			assert_eq!(state.name_hash(method), Hash::ident(method.name()));
		}
		for pair in Pair::ALL {
			let receiver = if pair == Pair::I64Abs {
				Hash::type_hash(crate::item!(::std::i64))
			} else {
				Hash::type_hash(crate::item!(::std::f64))
			};
			let item = match pair {
				Pair::F64Abs => Hash::type_hash(crate::item!(::std::f64::abs)),
				Pair::F64Round => Hash::type_hash(crate::item!(::std::f64::round)),
				Pair::F64Powf => Hash::type_hash(crate::item!(::std::f64::powf)),
				Pair::I64Abs => Hash::type_hash(crate::item!(::std::i64::abs)),
			};
			assert_eq!(pair.receiver(), receiver);
			assert_eq!(pair.item(), item);
			assert_eq!(
				pair.associated(),
				Hash::associated_function(receiver, Hash::ident(pair.method().name()))
			);
		}
	}

	#[test]
	fn every_direct_pair_accepts_both_canonical_hashes() {
		for pair in Pair::ALL {
			assert_eq!(Pair::from_item(pair.item()), Some(pair));
			assert_eq!(Pair::from_item(pair.associated()), Some(pair));
		}
		assert_eq!(Pair::from_item(Hash::EMPTY), None);
	}
}
