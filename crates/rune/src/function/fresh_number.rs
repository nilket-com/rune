//! C17c numeric method policy: the closed method table, its stock providers and
//! their certification. Registration work is confined to the std numeric
//! modules; no handler is invoked.
use crate::{Hash, TypeHash};

/// XOR constant of `Hash::associated_function`, checked against it in tests.
const ASSOCIATED: u64 = 0x5ea7_7ffb_cdf5_f302;

/// Private module markers: the std module that registers each receiver.
pub(crate) mod marker {
	pub(crate) const I64: u8 = 1;
	pub(crate) const U64: u8 = 2;
	pub(crate) const F64: u8 = 4;
	pub(crate) const CHAR: u8 = 8;
}

/// A receiver type with stock numeric providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Receiver {
	I64,
	U64,
	F64,
	Char,
}

impl Receiver {
	pub(crate) const ALL: [Self; 4] = [Self::I64, Self::U64, Self::F64, Self::Char];

	pub(crate) const fn hash(self) -> Hash {
		match self {
			Self::I64 => i64::HASH,
			Self::U64 => u64::HASH,
			Self::F64 => f64::HASH,
			Self::Char => char::HASH,
		}
	}

	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|r| r.hash() == hash)
	}

	const fn marker(self) -> u8 {
		match self {
			Self::I64 => marker::I64,
			Self::U64 => marker::U64,
			Self::F64 => marker::F64,
			Self::Char => marker::CHAR,
		}
	}
}

macro_rules! methods {
	($($variant:ident $name:ident $provider:ident;)*) => {
		/// A numeric method name. `FromI64` is the free function `char::from_i64`.
		#[derive(Clone, Copy, Debug, PartialEq, Eq)]
		pub(crate) enum Method {
			$($variant,)*
		}

		impl Method {
			pub(crate) const ALL: [Self; [$(stringify!($name)),*].len()] = [$(Self::$variant),*];

			#[cfg(test)]
			pub(crate) const fn name(self) -> &'static str {
				match self {
					$(Self::$variant => stringify!($name),)*
				}
			}

			pub(crate) fn from_name(name: &str) -> Option<Self> {
				match name {
					$(stringify!($name) => Some(Self::$variant),)*
					_ => None,
				}
			}

			/// `Hash::ident` of the name, computed at compile time.
			pub(crate) const fn hash(self) -> Hash {
				let (associated, receiver) = match self {
					$(Self::$variant => (
						crate::hash_in!(crate, ::std::$provider.$name),
						<$provider as TypeHash>::HASH,
					),)*
				};
				Hash::new(associated.into_inner() ^ receiver.into_inner() ^ ASSOCIATED)
			}

			/// The item `::std::<receiver>::<name>`, computed at compile time.
			const fn item(self, receiver: Receiver) -> Hash {
				match self {
					$(Self::$variant => match receiver {
						Receiver::I64 => crate::hash_in!(crate, ::std::i64::$name),
						Receiver::U64 => crate::hash_in!(crate, ::std::u64::$name),
						Receiver::F64 => crate::hash_in!(crate, ::std::f64::$name),
						Receiver::Char => crate::hash_in!(crate, ::std::char::$name),
					},)*
				}
			}
		}
	};
}

methods! {
	Sqrt sqrt f64;
	Floor floor f64;
	Ceil ceil f64;
	Exp exp f64;
	Ln ln f64;
	Powi powi f64;
	Min min f64;
	Max max f64;
	PartialCmp partial_cmp f64;
	ToString to_string f64;
	Pow pow i64;
	CheckedPow checked_pow i64;
	WrappingPow wrapping_pow i64;
	SaturatingPow saturating_pow i64;
	CheckedAdd checked_add i64;
	CheckedSub checked_sub i64;
	CheckedMul checked_mul i64;
	CheckedDiv checked_div i64;
	CheckedRem checked_rem i64;
	WrappingAdd wrapping_add i64;
	WrappingSub wrapping_sub i64;
	WrappingMul wrapping_mul i64;
	WrappingDiv wrapping_div i64;
	WrappingRem wrapping_rem i64;
	SaturatingAdd saturating_add i64;
	SaturatingSub saturating_sub i64;
	SaturatingMul saturating_mul i64;
	CheckedAbs checked_abs i64;
	WrappingAbs wrapping_abs i64;
	SaturatingAbs saturating_abs i64;
	Signum signum i64;
	ToI64 to_i64 char;
	FromI64 from_i64 char;
}

/// First id of the method intrinsics for receivers only known at run time.
const METHOD_BASE: u32 = crate::experimental_core::code::FIRST_INTRINSIC_ID + 0xa8;
/// First id of the provider intrinsics, one per [`PAIRS`] entry.
const PAIR_BASE: u32 = crate::experimental_core::code::FIRST_INTRINSIC_ID + 0x04;

/// The type a provider converts an operand to, as its stock `FromValue` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operand {
	I64,
	U64,
	F64,
	Char,
	U32,
	I32,
}

impl Method {
	/// Whether the name is an instance method. `char::from_i64` is not.
	pub(crate) const fn instance(self) -> bool {
		!matches!(self, Self::FromI64)
	}

	/// Whether receivers outside the numeric types also provide the name in
	/// stock (`Ord`/`PartialOrd` trait methods), so a site needs a static
	/// proof of its receiver type.
	pub(crate) const fn shared_name(self) -> bool {
		matches!(self, Self::Min | Self::Max | Self::PartialCmp)
	}

	/// The stock trait installs these names as aliases of the receiver's own
	/// protocol function.
	const fn protocol(self) -> Option<Hash> {
		match self {
			Self::Min => Some(crate::runtime::Protocol::MIN.hash),
			Self::Max => Some(crate::runtime::Protocol::MAX.hash),
			Self::PartialCmp => Some(crate::runtime::Protocol::PARTIAL_CMP.hash),
			_ => None,
		}
	}

	/// Whether `receiver` provides this method in stock.
	pub(crate) const fn provided_by(self, receiver: Receiver) -> bool {
		use Receiver as R;
		match self {
			Self::Sqrt | Self::Floor | Self::Ceil | Self::Exp | Self::Ln | Self::Powi => {
				matches!(receiver, R::F64)
			}
			Self::PartialCmp => matches!(receiver, R::F64),
			Self::Min | Self::Max | Self::ToString => matches!(receiver, R::I64 | R::U64 | R::F64),
			Self::CheckedAbs | Self::WrappingAbs | Self::SaturatingAbs | Self::Signum => {
				matches!(receiver, R::I64)
			}
			Self::ToI64 | Self::FromI64 => matches!(receiver, R::Char),
			_ => matches!(receiver, R::I64 | R::U64),
		}
	}

	/// The id of the run-time dispatch intrinsic: instance methods whose
	/// receiver need not be proven.
	pub(crate) const fn dynamic_id(self) -> Option<u32> {
		if self.instance() && !self.shared_name() {
			Some(METHOD_BASE + self as u32)
		} else {
			None
		}
	}

	pub(crate) fn from_dynamic_id(id: u32) -> Option<Self> {
		let m = *Self::ALL.get(id.checked_sub(METHOD_BASE)? as usize)?;
		(m.dynamic_id() == Some(id)).then_some(m)
	}

	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		let at = METHOD_HASHES
			.binary_search_by_key(&hash.into_inner(), |(h, _)| *h)
			.ok()?;
		Some(Self::ALL[METHOD_HASHES[at].1 as usize])
	}

	/// Operand count, receiver included.
	pub(crate) const fn arity(self) -> u32 {
		match self {
			Self::Sqrt
			| Self::Floor
			| Self::Ceil
			| Self::Exp
			| Self::Ln
			| Self::ToString
			| Self::CheckedAbs
			| Self::WrappingAbs
			| Self::SaturatingAbs
			| Self::Signum
			| Self::ToI64
			| Self::FromI64 => 1,
			_ => 2,
		}
	}
}

/// A stock provider: one method on one receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pair {
	method: Method,
	receiver: Receiver,
}

const fn pair_count() -> usize {
	let mut n = 0;
	let mut m = 0;
	while m < Method::ALL.len() {
		let mut r = 0;
		while r < Receiver::ALL.len() {
			if Method::ALL[m].provided_by(Receiver::ALL[r]) {
				n += 1;
			}
			r += 1;
		}
		m += 1;
	}
	n
}

/// Every provider, in method then receiver order. The index is the id offset.
pub(crate) const PAIRS: [Pair; pair_count()] = {
	let mut out = [Pair {
		method: Method::Sqrt,
		receiver: Receiver::F64,
	}; pair_count()];
	let mut n = 0;
	let mut m = 0;
	while m < Method::ALL.len() {
		let mut r = 0;
		while r < Receiver::ALL.len() {
			if Method::ALL[m].provided_by(Receiver::ALL[r]) {
				out[n] = Pair {
					method: Method::ALL[m],
					receiver: Receiver::ALL[r],
				};
				n += 1;
			}
			r += 1;
		}
		m += 1;
	}
	out
};

/// `PAIRS` index by method and receiver; unused cells are never read because
/// a `Pair` exists only for a provider.
const PAIR_INDEX: [[u8; 4]; Method::ALL.len()] = {
	let mut out = [[u8::MAX; 4]; Method::ALL.len()];
	let mut i = 0;
	while i < PAIRS.len() {
		out[PAIRS[i].method as usize][PAIRS[i].receiver as usize] = i as u8;
		i += 1;
	}
	out
};

/// Sorts `(hash, index)` rows by hash at compile time.
const fn sorted<const N: usize>(mut rows: [(u64, u8); N]) -> [(u64, u8); N] {
	let mut i = 1;
	while i < N {
		let mut j = i;
		while j > 0 && rows[j - 1].0 > rows[j].0 {
			let t = rows[j - 1];
			rows[j - 1] = rows[j];
			rows[j] = t;
			j -= 1;
		}
		i += 1;
	}
	rows
}

const INSTANCE_METHODS: usize = Method::ALL.len() - 1;

/// Instance method name hashes to `Method::ALL` indexes, sorted by hash.
const METHOD_HASHES: [(u64, u8); INSTANCE_METHODS] = {
	let mut out = [(0, 0); INSTANCE_METHODS];
	let (mut i, mut n) = (0, 0);
	while i < Method::ALL.len() {
		if Method::ALL[i].instance() {
			out[n] = (Method::ALL[i].hash().into_inner(), i as u8);
			n += 1;
		}
		i += 1;
	}
	sorted(out)
};

const PATHS: usize = 2 * PAIRS.len() - 1;

/// Item and associated hashes a path call can name, to `PAIRS` indexes,
/// sorted by hash. `char::from_i64` has no associated form.
const PATH_HASHES: [(u64, u8); PATHS] = {
	let mut out = [(0, 0); PATHS];
	let (mut i, mut n) = (0, 0);
	while i < PAIRS.len() {
		out[n] = (PAIRS[i].item().into_inner(), i as u8);
		n += 1;
		if PAIRS[i].method.instance() {
			out[n] = (PAIRS[i].associated().into_inner(), i as u8);
			n += 1;
		}
		i += 1;
	}
	sorted(out)
};

impl Pair {
	pub(crate) fn new(method: Method, receiver: Receiver) -> Option<Self> {
		method
			.provided_by(receiver)
			.then_some(Self { method, receiver })
	}

	pub(crate) const fn method(self) -> Method {
		self.method
	}

	#[cfg(test)]
	pub(crate) const fn receiver(self) -> Receiver {
		self.receiver
	}

	fn index(self) -> usize {
		PAIR_INDEX[self.method as usize][self.receiver as usize] as usize
	}

	pub(crate) fn id(self) -> u32 {
		PAIR_BASE + self.index() as u32
	}

	pub(crate) fn from_id(id: u32) -> Option<Self> {
		PAIRS.get(id.checked_sub(PAIR_BASE)? as usize).copied()
	}

	fn bit(self) -> u64 {
		1 << self.index()
	}

	/// Operand conversions, receiver first.
	pub(crate) const fn operands(self) -> (Operand, Option<Operand>) {
		let own = match self.receiver {
			Receiver::I64 => Operand::I64,
			Receiver::U64 => Operand::U64,
			Receiver::F64 => Operand::F64,
			Receiver::Char => Operand::Char,
		};
		match self.method {
			Method::FromI64 => (Operand::I64, None),
			Method::Pow | Method::CheckedPow | Method::WrappingPow | Method::SaturatingPow => {
				(own, Some(Operand::U32))
			}
			Method::Powi => (own, Some(Operand::I32)),
			m if m.arity() == 2 => (own, Some(own)),
			_ => (own, None),
		}
	}

	/// The stock item a path call names, such as `::std::f64::sqrt`.
	pub(crate) const fn item(self) -> Hash {
		self.method.item(self.receiver)
	}

	/// The provider a path call names by its item or associated hash.
	pub(crate) fn from_item(hash: Hash) -> Option<Self> {
		let at = PATH_HASHES
			.binary_search_by_key(&hash.into_inner(), |(h, _)| *h)
			.ok()?;
		Some(PAIRS[PATH_HASHES[at].1 as usize])
	}

	/// The associated hash an instance call resolves to.
	pub(crate) const fn associated(self) -> Hash {
		Hash::new(self.receiver.hash().into_inner() ^ self.method.hash().into_inner() ^ ASSOCIATED)
	}
}

/// Std numeric constants, admitted as literals with these exact values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Constant {
	I64(i64),
	U64(u64),
	F64(u64),
}

/// `(item, value)` of every admitted std numeric constant.
pub(crate) const CONSTANTS: [(Hash, Constant); 10] = [
	(
		crate::hash_in!(crate, ::std::i64::MAX),
		Constant::I64(i64::MAX),
	),
	(
		crate::hash_in!(crate, ::std::i64::MIN),
		Constant::I64(i64::MIN),
	),
	(
		crate::hash_in!(crate, ::std::u64::MAX),
		Constant::U64(u64::MAX),
	),
	(
		crate::hash_in!(crate, ::std::f64::NAN),
		Constant::F64(f64::NAN.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::INFINITY),
		Constant::F64(f64::INFINITY.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::NEG_INFINITY),
		Constant::F64(f64::NEG_INFINITY.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::MAX),
		Constant::F64(f64::MAX.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::MIN),
		Constant::F64(f64::MIN.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::MIN_POSITIVE),
		Constant::F64(f64::MIN_POSITIVE.to_bits()),
	),
	(
		crate::hash_in!(crate, ::std::f64::EPSILON),
		Constant::F64(f64::EPSILON.to_bits()),
	),
];

pub(crate) fn constant(item: Hash) -> Option<Constant> {
	CONSTANTS
		.iter()
		.find(|(hash, _)| *hash == item)
		.map(|(_, c)| *c)
}

/// Certification state. A provider is ready only after its std module
/// registered it as a plain function, or, for the trait-installed names,
/// after the std trait forwarded the module's plain protocol function.
/// Native functions are insert-once, so a later registration cannot replace
/// a certified handler.
#[derive(Default)]
pub(crate) struct State {
	pending: u8,
	certified: u64,
	/// Pair bits whose alias the std trait forwarded during this module.
	forwarded: u64,
}

impl State {
	pub(crate) fn begin_module(&mut self, marker: u8) -> u8 {
		core::mem::replace(&mut self.pending, marker)
	}

	pub(crate) fn finish_module(&mut self, marker: u8) {
		self.pending = marker;
	}

	/// The pair whose alias a trait installs when it forwards `protocol` for
	/// `receiver` while that receiver's std module is installing.
	pub(crate) fn alias(&self, protocol: Hash, receiver: Hash) -> Option<Pair> {
		if self.pending == 0 {
			return None;
		}
		let receiver = Receiver::from_hash(receiver)?;
		if self.pending & receiver.marker() == 0 {
			return None;
		}
		let method = [Method::Min, Method::Max, Method::PartialCmp]
			.into_iter()
			.find(|m| m.protocol() == Some(protocol))?;
		Pair::new(method, receiver)
	}

	pub(crate) fn forwarded(&mut self, pair: Pair) {
		self.forwarded |= pair.bit();
	}

	/// Certifies the providers the std module with `marker` registered. Called
	/// once per std numeric module, after all of its entries were installed.
	pub(crate) fn certify_module(&mut self, module: &crate::module::Module) {
		use crate::compile::meta::AssociatedKind;
		use crate::module::{ModuleAssociatedKind, ModuleItemKind};
		let marker = self.pending;
		let mut protocols = 0u64;
		for assoc in &module.associated {
			let Some(receiver) = Receiver::from_hash(assoc.container) else {
				continue;
			};
			let plain = receiver.marker() & marker != 0
				&& assoc.name.function_parameters == Hash::EMPTY
				&& matches!(&assoc.kind, ModuleAssociatedKind::Function(f) if f.trait_hash.is_none());
			if !plain {
				continue;
			}
			match &assoc.name.kind {
				AssociatedKind::Instance(name) => {
					if let Some(pair) = Method::from_name(name)
						.filter(|m| m.protocol().is_none())
						.and_then(|m| Pair::new(m, receiver))
					{
						self.certified |= pair.bit();
					}
				}
				AssociatedKind::Protocol(p) => {
					for m in [Method::Min, Method::Max, Method::PartialCmp] {
						if m.protocol() == Some(p.hash) {
							if let Some(pair) = Pair::new(m, receiver) {
								protocols |= pair.bit();
							}
						}
					}
				}
				_ => {}
			}
		}
		self.certified |= protocols & self.forwarded;
		self.forwarded = 0;
		if marker & marker::CHAR != 0 {
			let pair = Pair {
				method: Method::FromI64,
				receiver: Receiver::Char,
			};
			for item in &module.items {
				if item.hash == pair.item() && matches!(item.kind, ModuleItemKind::Function(..)) {
					self.certified |= pair.bit();
				}
			}
		}
	}

	pub(crate) fn ready(&self, pair: Pair) -> bool {
		self.pending == 0 && self.certified & pair.bit() != 0
	}

	/// Every provider of `m` is certified.
	pub(crate) fn method_ready(&self, m: Method) -> bool {
		PAIRS
			.iter()
			.filter(|p| p.method == m)
			.all(|p| self.ready(*p))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn std_item(module: &str, name: &str) -> Hash {
		Hash::type_hash(&crate::ItemBuf::with_crate_item("std", [module, name]).unwrap())
	}

	#[test]
	fn hashes_ids_and_items() {
		assert_eq!(PAIRS.len(), 56);
		for m in Method::ALL {
			assert_eq!(m.hash(), Hash::ident(m.name()));
			assert_eq!(Method::from_name(m.name()), Some(m));
			assert_eq!(Method::from_hash(m.hash()), m.instance().then_some(m));
			if let Some(id) = m.dynamic_id() {
				assert_eq!(Method::from_dynamic_id(id), Some(m));
			}
		}
		assert_eq!(Method::from_name("abs"), None);
		for (i, p) in PAIRS.into_iter().enumerate() {
			assert_eq!(p.index(), i);
			assert_eq!(Pair::from_id(p.id()), Some(p));
			assert_eq!(Pair::new(p.method(), p.receiver()), Some(p));
			assert_eq!(
				p.associated(),
				Hash::associated_function(p.receiver().hash(), Hash::ident(p.method().name()))
			);
			let receiver = match p.receiver() {
				Receiver::I64 => "i64",
				Receiver::U64 => "u64",
				Receiver::F64 => "f64",
				Receiver::Char => "char",
			};
			assert_eq!(p.item(), std_item(receiver, p.method().name()));
			assert_eq!(Pair::from_item(p.item()), Some(p));
		}
		assert_eq!(Pair::from_id(PAIRS[55].id() + 1), None);
		for table in [&METHOD_HASHES[..], &PATH_HASHES[..]] {
			assert!(table.windows(2).all(|w| w[0].0 < w[1].0));
		}
		assert_eq!(Pair::from_item(Hash::EMPTY), None);
		assert_eq!(Method::from_hash(Hash::ident("abs")), None);
		assert!(PAIRS[55].id() < crate::experimental_core::code::FIRST_INTRINSIC_ID + 0x40);
		let last = Method::ALL.len() as u32 - 1;
		assert!(METHOD_BASE + last < crate::experimental_core::code::FIRST_INTRINSIC_ID + 0xff);
		assert_eq!(Method::Max.dynamic_id(), None);
		assert_eq!(Method::FromI64.dynamic_id(), None);
		assert_eq!(
			Method::from_dynamic_id(METHOD_BASE + Method::Max as u32),
			None
		);
		assert_eq!(Pair::new(Method::Sqrt, Receiver::I64), None);
		assert_eq!(Pair::new(Method::Signum, Receiver::U64), None);
	}

	#[test]
	fn constants_are_the_rust_values() {
		for (hash, c) in CONSTANTS {
			assert_eq!(constant(hash), Some(c));
		}
		assert_eq!(
			constant(std_item("f64", "NAN")),
			Some(Constant::F64(0x7ff8_0000_0000_0000))
		);
		assert_eq!(
			constant(std_item("i64", "MAX")),
			Some(Constant::I64(i64::MAX))
		);
		assert_eq!(constant(std_item("u64", "MIN")), None);
	}

	#[test]
	fn foreign_or_partial_install_is_not_certified() {
		let sqrt = Pair::new(Method::Sqrt, Receiver::F64).unwrap();
		let max = Pair::new(Method::Max, Receiver::F64).unwrap();
		let mut s = State::default();
		// A forward outside the receiver's std module is not an alias.
		assert_eq!(s.alias(crate::runtime::Protocol::MAX.hash, f64::HASH), None);
		s.begin_module(marker::F64);
		assert_eq!(
			s.alias(crate::runtime::Protocol::MAX.hash, f64::HASH),
			Some(max)
		);
		assert_eq!(s.alias(crate::runtime::Protocol::MAX.hash, i64::HASH), None);
		assert_eq!(s.alias(crate::runtime::Protocol::CMP.hash, f64::HASH), None);
		s.forwarded(max);
		// Still installing: nothing is ready.
		s.certified |= sqrt.bit();
		assert!(!s.ready(sqrt));
		s.finish_module(0);
		assert!(s.ready(sqrt));
		// Forwarded without the module's plain protocol function: not certified.
		assert!(!s.ready(max));
		assert!(!s.method_ready(Method::ToString));
	}
}
