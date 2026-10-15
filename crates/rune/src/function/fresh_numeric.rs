//! Private numeric-handler identity policy. Never registers or invokes a handler.
use crate::Hash;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
	Abs,
	Round,
	Powf,
	Cmp,
}
impl Method {
	pub(crate) fn name(self) -> &'static str {
		match self {
			Self::Abs => "abs",
			Self::Round => "round",
			Self::Powf => "powf",
			Self::Cmp => "cmp",
		}
	}
	pub(crate) fn from_name(name: &str) -> Option<Self> {
		match name {
			"abs" => Some(Self::Abs),
			"round" => Some(Self::Round),
			"powf" => Some(Self::Powf),
			"cmp" => Some(Self::Cmp),
			_ => None,
		}
	}
	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		[Self::Abs, Self::Round, Self::Powf, Self::Cmp]
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
			Self::Cmp => {
				Pair::I64Cmp.bit() | Pair::U64Cmp.bit() | Pair::F64Cmp.bit() | Pair::StringCmp.bit()
			}
		}
	}
	/// Only receiver types for which this intrinsic promises semantics can close it.
	/// The fixed CMP set is the four admitted pairs plus the six missing-method types.
	/// Refused/unrepresentable receivers cannot change those promises.
	pub(crate) const fn closes_for(self, receiver: Hash) -> bool {
		match self {
			Self::Cmp => {
				const I64: u64 = crate::hash_in!(crate, ::std::i64).into_inner();
				const U64: u64 = crate::hash_in!(crate, ::std::u64).into_inner();
				const F64: u64 = crate::hash_in!(crate, ::std::f64).into_inner();
				const STRING: u64 = crate::hash_in!(crate, ::std::string::String).into_inner();
				const BOOL: u64 = crate::hash_in!(crate, ::std::bool).into_inner();
				const CHAR: u64 = crate::hash_in!(crate, ::std::char).into_inner();
				const ORDERING: u64 = crate::hash_in!(crate, ::std::cmp::Ordering).into_inner();
				const OBJECT: u64 = crate::hash_in!(crate, ::std::object::Object).into_inner();
				const FUNCTION: u64 = crate::hash_in!(crate, ::std::ops::Function).into_inner();
				const TYPE: u64 = crate::hash_in!(crate, ::std::any::Type).into_inner();
				matches!(
					receiver.into_inner(),
					I64 | U64 | F64 | STRING | BOOL | CHAR | ORDERING | OBJECT | FUNCTION | TYPE
				)
			}
			_ => true,
		}
	}
	pub(crate) fn arity(self) -> u32 {
		if matches!(self, Self::Powf | Self::Cmp) {
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
	I64Cmp,
	U64Cmp,
	F64Cmp,
	StringCmp,
}
impl Pair {
	pub(crate) const DIRECT: [Self; 4] =
		[Self::F64Abs, Self::F64Round, Self::F64Powf, Self::I64Abs];
	pub(crate) const ALL: [Self; 8] = [
		Self::F64Abs,
		Self::F64Round,
		Self::F64Powf,
		Self::I64Abs,
		Self::I64Cmp,
		Self::U64Cmp,
		Self::F64Cmp,
		Self::StringCmp,
	];
	pub(crate) fn bit(self) -> u8 {
		1 << self as u8
	}
	pub(crate) fn method(self) -> Method {
		match self {
			Self::F64Abs | Self::I64Abs => Method::Abs,
			Self::F64Round => Method::Round,
			Self::F64Powf => Method::Powf,
			Self::I64Cmp | Self::U64Cmp | Self::F64Cmp | Self::StringCmp => Method::Cmp,
		}
	}
	pub(crate) const fn receiver(self) -> Hash {
		match self {
			Self::I64Abs | Self::I64Cmp => crate::hash_in!(crate, ::std::i64),
			Self::U64Cmp => crate::hash_in!(crate, ::std::u64),
			Self::StringCmp => crate::hash_in!(crate, ::std::string::String),
			_ => crate::hash_in!(crate, ::std::f64),
		}
	}
	pub(crate) const fn item(self) -> Hash {
		match self {
			Self::F64Abs => crate::hash_in!(crate, ::std::f64::abs),
			Self::F64Round => crate::hash_in!(crate, ::std::f64::round),
			Self::F64Powf => crate::hash_in!(crate, ::std::f64::powf),
			Self::I64Abs => crate::hash_in!(crate, ::std::i64::abs),
			Self::I64Cmp => crate::hash_in!(crate, ::std::i64::cmp),
			Self::U64Cmp => crate::hash_in!(crate, ::std::u64::cmp),
			Self::F64Cmp => crate::hash_in!(crate, ::std::f64::cmp),
			Self::StringCmp => crate::hash_in!(crate, ::std::string::String::cmp),
		}
	}
	pub(crate) const fn associated(self) -> Hash {
		match self {
			Self::F64Abs => crate::hash_in!(crate, ::std::f64.abs),
			Self::F64Round => crate::hash_in!(crate, ::std::f64.round),
			Self::F64Powf => crate::hash_in!(crate, ::std::f64.powf),
			Self::I64Abs => crate::hash_in!(crate, ::std::i64.abs),
			Self::I64Cmp => crate::hash_in!(crate, ::std::i64.cmp),
			Self::U64Cmp => crate::hash_in!(crate, ::std::u64.cmp),
			Self::F64Cmp => crate::hash_in!(crate, ::std::f64.cmp),
			Self::StringCmp => crate::hash_in!(crate, ::std::string::String.cmp),
		}
	}
	pub(crate) const fn cmp_protocol(self) -> Option<Hash> {
		match self {
			Self::I64Cmp | Self::U64Cmp | Self::F64Cmp | Self::StringCmp => {}
			_ => return None,
		}
		// Hash::associated_function's fixed XOR encoding, checked independently below.
		Some(Hash::new(
			0x5ea7_7ffb_cdf5_f302
				^ self.receiver().into_inner()
				^ crate::runtime::Protocol::CMP.hash.into_inner(),
		))
	}
	pub(crate) fn from_item(hash: Hash) -> Option<Self> {
		Self::DIRECT
			.into_iter()
			.find(|p| p.item() == hash || p.associated() == hash)
	}
}
pub(crate) struct State {
	// Four infallible, allocation-free name hashes computed once per Context.
	name_hashes: [Hash; 4],
	seen: u8,
	certified: u8,
	closed: u8,
	pending_methods: u8,
	pending_pairs: u8,
	// Stock CMP protocol providers; named aliases are certified only after forwarding.
	cmp_protocols: u8,
}
impl Default for State {
	fn default() -> Self {
		Self {
			name_hashes: [
				Hash::ident("abs"),
				Hash::ident("round"),
				Hash::ident("powf"),
				Hash::ident("cmp"),
			],
			seen: 0,
			certified: 0,
			closed: 0,
			pending_methods: 0,
			pending_pairs: 0,
			cmp_protocols: 0,
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
	pub(crate) fn begin(&mut self, method: Method, receiver: Hash) -> bool {
		let old = self.closed & method.bit() != 0;
		if method.closes_for(receiver) {
			self.closed |= method.bit();
		}
		old
	}
	pub(crate) fn installed(&mut self, pair: Pair, was_closed: bool) {
		self.seen |= pair.bit();
		if !was_closed {
			self.closed &= !pair.method().bit();
		}
	}
	pub(crate) fn certify_cmp_protocol(&mut self, pair: Pair) {
		self.cmp_protocols |= pair.bit();
	}
	pub(crate) fn cmp_alias(&self, receiver: Hash) -> Option<Pair> {
		Pair::ALL.into_iter().find(|p| {
			p.method() == Method::Cmp
				&& p.receiver() == receiver
				&& self.cmp_protocols & p.bit() != 0
		})
	}
	pub(crate) fn certify(&mut self, pair: Pair) {
		self.certified |= pair.bit();
	}
	pub(crate) fn invalidate_hash(&mut self, hash: Hash) {
		for p in Pair::ALL {
			if p.cmp_protocol() == Some(hash) {
				self.cmp_protocols &= !p.bit();
				self.certified &= !p.bit();
			}
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
	MethodCmp,
}
impl Call {
	pub(crate) const ALL: [Self; 8] = [
		Self::MethodAbs,
		Self::MethodRound,
		Self::MethodPowf,
		Self::F64Abs,
		Self::F64Round,
		Self::F64Powf,
		Self::I64Abs,
		Self::MethodCmp,
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
			Method::Cmp => Self::MethodCmp,
		}
	}
	pub(crate) fn direct(p: Pair) -> Self {
		match p {
			Pair::F64Abs => Self::F64Abs,
			Pair::F64Round => Self::F64Round,
			Pair::F64Powf => Self::F64Powf,
			Pair::I64Abs => Self::I64Abs,
			Pair::I64Cmp | Pair::U64Cmp | Pair::F64Cmp | Pair::StringCmp => Self::MethodCmp,
		}
	}
	pub(crate) fn name(self) -> Method {
		match self {
			Self::MethodAbs | Self::F64Abs | Self::I64Abs => Method::Abs,
			Self::MethodRound | Self::F64Round => Method::Round,
			Self::MethodPowf | Self::F64Powf => Method::Powf,
			Self::MethodCmp => Method::Cmp,
		}
	}
	pub(crate) fn is_method(self) -> bool {
		matches!(
			self,
			Self::MethodAbs | Self::MethodRound | Self::MethodPowf | Self::MethodCmp
		)
	}
}

#[cfg(test)]
mod canonical_hash_tests {
	use super::*;
	use crate as rune;
	#[test]
	fn cached_hashes_equal_the_original_runtime_computations() {
		let state = State::default();
		for method in [Method::Abs, Method::Round, Method::Powf, Method::Cmp] {
			assert_eq!(state.name_hash(method), Hash::ident(method.name()));
		}
		for pair in Pair::ALL {
			let receiver = if matches!(pair, Pair::I64Abs | Pair::I64Cmp) {
				Hash::type_hash(crate::item!(::std::i64))
			} else if pair == Pair::U64Cmp {
				Hash::type_hash(crate::item!(::std::u64))
			} else if pair == Pair::StringCmp {
				Hash::type_hash(crate::item!(::std::string::String))
			} else {
				Hash::type_hash(crate::item!(::std::f64))
			};
			let item = match pair {
				Pair::F64Abs => Hash::type_hash(crate::item!(::std::f64::abs)),
				Pair::F64Round => Hash::type_hash(crate::item!(::std::f64::round)),
				Pair::F64Powf => Hash::type_hash(crate::item!(::std::f64::powf)),
				Pair::I64Abs => Hash::type_hash(crate::item!(::std::i64::abs)),
				Pair::I64Cmp => Hash::type_hash(crate::item!(::std::i64::cmp)),
				Pair::U64Cmp => Hash::type_hash(crate::item!(::std::u64::cmp)),
				Pair::F64Cmp => Hash::type_hash(crate::item!(::std::f64::cmp)),
				Pair::StringCmp => Hash::type_hash(crate::item!(::std::string::String::cmp)),
			};
			if pair.method() == Method::Cmp {
				assert_eq!(
					pair.cmp_protocol(),
					Some(Hash::associated_function(
						receiver,
						&crate::runtime::Protocol::CMP
					))
				);
			}
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
		for pair in Pair::DIRECT {
			assert_eq!(Pair::from_item(pair.item()), Some(pair));
			assert_eq!(Pair::from_item(pair.associated()), Some(pair));
		}
		assert_eq!(Pair::from_item(Hash::EMPTY), None);
	}
}

#[cfg(test)]
mod cmp_direct_refusal {
	use super::*;
	#[test]
	fn cmp_provider_pairs_never_admit_free_function_calls() {
		for pair in [Pair::I64Cmp, Pair::U64Cmp, Pair::F64Cmp, Pair::StringCmp] {
			assert_eq!(Pair::from_item(pair.item()), None);
			assert_eq!(Pair::from_item(pair.associated()), None);
		}
	}
}

#[cfg(test)]
mod cmp_receiver_controls {
	use super::*;
	use crate::TypeHash;
	fn trusted() -> State {
		let mut state = State::default();
		for pair in [Pair::I64Cmp, Pair::U64Cmp, Pair::F64Cmp, Pair::StringCmp] {
			state.installed(pair, false);
			state.certify(pair);
		}
		assert!(state.method(Method::Cmp));
		state
	}
	#[test]
	fn relevant_receiver_hashes_close_only_the_cmp_promise() {
		let hashes = [
			i64::HASH,
			u64::HASH,
			f64::HASH,
			crate::alloc::String::HASH,
			bool::HASH,
			char::HASH,
			core::cmp::Ordering::HASH,
			crate::runtime::Object::HASH,
			crate::runtime::Function::HASH,
			crate::runtime::Type::HASH,
		];
		for receiver in hashes {
			assert!(Method::Cmp.closes_for(receiver));
			let mut state = trusted();
			assert!(!state.begin(Method::Cmp, receiver));
			assert!(!state.method(Method::Cmp));
		}
		for receiver in [
			crate::runtime::OwnedTuple::HASH,
			crate::runtime::Vec::HASH,
			Option::<crate::runtime::Value>::HASH,
			Result::<crate::runtime::Value, crate::runtime::Value>::HASH,
			crate::runtime::Range::HASH,
			crate::runtime::RangeInclusive::HASH,
			crate::runtime::RangeFrom::HASH,
			crate::runtime::RangeFull::HASH,
			crate::runtime::RangeTo::HASH,
			crate::runtime::RangeToInclusive::HASH,
			crate::runtime::Bytes::HASH,
			Hash::new(0xC9FF_0001),
		] {
			assert!(!Method::Cmp.closes_for(receiver));
			let mut state = trusted();
			assert!(!state.begin(Method::Cmp, receiver));
			assert!(state.method(Method::Cmp));
			state.begin(Method::Cmp, bool::HASH);
			assert!(state.begin(Method::Cmp, receiver));
			assert!(!state.method(Method::Cmp)); // irrelevant registration never reopens a closed method.
			assert!(Method::Abs.closes_for(receiver)); // C6 global rules unchanged.
		}
	}
	#[test]
	fn pending_modules_and_protocol_invalidation_still_block_cmp() {
		let mut state = trusted();
		let outer = state.begin_module(Method::Cmp.bit(), 0);
		assert!(!state.method(Method::Cmp));
		let inner = state.begin_module(0, 0);
		state.finish_module(inner);
		assert!(!state.method(Method::Cmp));
		state.finish_module(outer);
		assert!(state.method(Method::Cmp));
		for pair in [Pair::I64Cmp, Pair::U64Cmp, Pair::F64Cmp, Pair::StringCmp] {
			let mut state = trusted();
			let _failed = state.begin_module(Method::Cmp.bit(), pair.bit());
			assert!(!state.method(Method::Cmp));
			assert!(!state.direct(pair)); // failed whole-module install leaves pending certificate blocked.
			let mut state = trusted();
			state.invalidate_hash(pair.cmp_protocol().unwrap());
			assert!(!state.method(Method::Cmp));
			// A successful protocol install may restore its temporary close, but cannot certify a named alias.
			let prior = state.begin(Method::Cmp, pair.receiver());
			state.installed(pair, prior);
			assert!(!state.method(Method::Cmp));
		}
	}
}
