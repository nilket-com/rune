//! Direct iterator chain policy: the closed set of C17d terminals and adapters
//! and C17e closure adapters and consumers a direct chain may end in or pass
//! through, the iterator types that must provide each of them, the cold
//! intrinsics that implement them, and their certification, checked on first
//! use in a compile. Installation does no work for them; no handler is invoked.
use crate::runtime as rt;
use crate::{Hash, TypeHash};

/// XOR constant of `Hash::associated_function`, checked against it in tests.
const ASSOCIATED: u64 = 0x5ea7_7ffb_cdf5_f302;

/// The element type a `sum` or `product` turbofish names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Number {
	I64,
	U64,
	F64,
}

/// The container a `collect` turbofish names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Collection {
	Vec,
	Object,
	Tuple,
	String,
}

/// A certified Iterator trait function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Terminal {
	Count,
	Skip,
	Take,
	Collect(Collection),
	Sum(Number),
	Product(Number),
}

impl Number {
	const fn hash(self) -> Hash {
		match self {
			Self::I64 => i64::HASH,
			Self::U64 => u64::HASH,
			Self::F64 => f64::HASH,
		}
	}
}

impl Collection {
	const fn hash(self) -> Hash {
		match self {
			Self::Vec => rt::Vec::HASH,
			Self::Object => rt::Object::HASH,
			Self::Tuple => rt::OwnedTuple::HASH,
			Self::String => crate::alloc::String::HASH,
		}
	}
}

/// `Hash::ident` of a method name, computed at compile time from its
/// associated hash on `Vec`.
const fn ident(associated: Hash) -> Hash {
	Hash::new(associated.into_inner() ^ rt::Vec::HASH.into_inner() ^ ASSOCIATED)
}

impl Terminal {
	pub(crate) const ALL: [Self; 13] = [
		Self::Count,
		Self::Skip,
		Self::Take,
		Self::Collect(Collection::Vec),
		Self::Collect(Collection::Object),
		Self::Collect(Collection::Tuple),
		Self::Collect(Collection::String),
		Self::Sum(Number::I64),
		Self::Sum(Number::U64),
		Self::Sum(Number::F64),
		Self::Product(Number::I64),
		Self::Product(Number::U64),
		Self::Product(Number::F64),
	];

	const fn index(self) -> usize {
		match self {
			Self::Count => 0,
			Self::Skip => 1,
			Self::Take => 2,
			Self::Collect(c) => 3 + c as usize,
			Self::Sum(n) => 7 + n as usize,
			Self::Product(n) => 10 + n as usize,
		}
	}

	#[cfg(test)]
	const fn name(self) -> &'static str {
		match self {
			Self::Count => "count",
			Self::Skip => "skip",
			Self::Take => "take",
			Self::Collect(_) => "collect",
			Self::Sum(_) => "sum",
			Self::Product(_) => "product",
		}
	}

	/// `Hash::ident` of the name, computed at compile time.
	const fn name_hash(self) -> Hash {
		match self {
			Self::Count => ident(crate::hash_in!(crate, ::std::vec::Vec.count)),
			Self::Skip => ident(crate::hash_in!(crate, ::std::vec::Vec.skip)),
			Self::Take => ident(crate::hash_in!(crate, ::std::vec::Vec.take)),
			Self::Collect(_) => ident(crate::hash_in!(crate, ::std::vec::Vec.collect)),
			Self::Sum(_) => ident(crate::hash_in!(crate, ::std::vec::Vec.sum)),
			Self::Product(_) => ident(crate::hash_in!(crate, ::std::vec::Vec.product)),
		}
	}

	const fn parameters(self) -> Hash {
		match self {
			Self::Count | Self::Skip | Self::Take => Hash::EMPTY,
			Self::Collect(c) => Hash::parameters([c.hash()]),
			Self::Sum(n) | Self::Product(n) => Hash::parameters([n.hash()]),
		}
	}

	/// The hash a method call site resolves to, turbofish included.
	pub(crate) const fn hash(self) -> Hash {
		HASHES[self.index()]
	}

	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|t| t.hash() == hash)
	}

	/// The terminal's bit in `CompileTransaction::checked_terminal`; bit 13 is `range`.
	pub(crate) const fn bit(self) -> u16 {
		1 << self.index()
	}

	/// Operand count of the call, receiver included.
	pub(crate) const fn arity(self) -> usize {
		match self {
			Self::Skip | Self::Take => 2,
			_ => 1,
		}
	}
}

/// Turbofish parameters by [`Terminal::index`], computed at compile time.
const PARAMETERS: [Hash; 13] = {
	let mut out = [Hash::EMPTY; 13];
	let mut i = 0;
	while i < Terminal::ALL.len() {
		out[i] = Terminal::ALL[i].parameters();
		i += 1;
	}
	out
};

/// Call site hashes by [`Terminal::index`], computed at compile time.
const HASHES: [Hash; 13] = {
	let mut out = [Hash::EMPTY; 13];
	let mut i = 0;
	while i < Terminal::ALL.len() {
		out[i] = Terminal::ALL[i]
			.name_hash()
			.with_function_parameters(PARAMETERS[i]);
		i += 1;
	}
	out
};

/// The iterator types a direct chain can apply a terminal to: every iterator
/// `iter()` or `std::iter::range` returns for a new value, and the two
/// adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
	Slice,
	Object,
	Option,
	RangeI64,
	RangeU64,
	RangeChar,
	RangeInclusiveI64,
	RangeInclusiveU64,
	RangeInclusiveChar,
	RangeFromI64,
	RangeFromU64,
	RangeFromChar,
	Skip,
	Take,
}

impl Container {
	const ALL: [Self; 14] = [
		Self::Slice,
		Self::Object,
		Self::Option,
		Self::RangeI64,
		Self::RangeU64,
		Self::RangeChar,
		Self::RangeInclusiveI64,
		Self::RangeInclusiveU64,
		Self::RangeInclusiveChar,
		Self::RangeFromI64,
		Self::RangeFromU64,
		Self::RangeFromChar,
		Self::Skip,
		Self::Take,
	];

	const fn hash(self) -> Hash {
		use rt::{range, range_from, range_inclusive};
		match self {
			Self::Slice => rt::slice::Iter::HASH,
			Self::Object => rt::object::RuneIter::HASH,
			Self::Option => crate::modules::option::Iter::HASH,
			Self::RangeI64 => range::RangeIter::<i64>::HASH,
			Self::RangeU64 => range::RangeIter::<u64>::HASH,
			Self::RangeChar => range::RangeIter::<char>::HASH,
			Self::RangeInclusiveI64 => range_inclusive::RangeInclusiveIter::<i64>::HASH,
			Self::RangeInclusiveU64 => range_inclusive::RangeInclusiveIter::<u64>::HASH,
			Self::RangeInclusiveChar => range_inclusive::RangeInclusiveIter::<char>::HASH,
			Self::RangeFromI64 => range_from::RangeFromIter::<i64>::HASH,
			Self::RangeFromU64 => range_from::RangeFromIter::<u64>::HASH,
			Self::RangeFromChar => range_from::RangeFromIter::<char>::HASH,
			Self::Skip => crate::modules::iter::Skip::HASH,
			Self::Take => crate::modules::iter::Take::HASH,
		}
	}

	#[cfg(test)]
	fn from_hash(hash: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|c| c.hash() == hash)
	}
}

/// The item `::std::iter::range`.
pub(crate) const RANGE: Hash = crate::hash_in!(crate, ::std::iter::range);

/// A cold intrinsic of the expanded chains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intrinsic {
	/// `skip`/`take` count: the stock `usize` argument conversion.
	AdapterCount,
	/// `std::iter::range(start, end)`: the stock `i64` argument conversions.
	Range,
	/// The first `sum`/`product` element, converted to the turbofish type.
	Seed(Number),
	/// Adds the next element to the accumulator, in the accumulator's type.
	Sum,
	/// Multiplies the accumulator by the next element, in its type.
	Product,
	/// Inserts a `(key, value)` element into the collected Object.
	Insert,
	/// Turns the collected Vec into a Tuple.
	IntoTuple,
	/// Appends a String or char element to the collected String.
	Append,
	/// Whether a `filter_map` result is `Some`; anything but an Option is the
	/// stock conversion error.
	OptionSome,
	/// Whether a reversed source has no element left; the stock missing
	/// method error for a source without `rev`.
	RevEmpty,
	/// The offset of a reversed source's last element from its front.
	RevLast,
	/// The element at an offset from a reversed source's front.
	RevAt,
	/// A `chain` argument or `flat_map` result as an iterator, converted as
	/// stock's `protocol_into_iter`.
	IntoIter,
}

impl Intrinsic {
	const FIRST_ID: u32 = crate::experimental_core::code::FIRST_INTRINSIC_ID + 0xd0;

	pub(crate) const ALL: [Self; 15] = [
		Self::AdapterCount,
		Self::Range,
		Self::Seed(Number::I64),
		Self::Seed(Number::U64),
		Self::Seed(Number::F64),
		Self::Sum,
		Self::Product,
		Self::Insert,
		Self::IntoTuple,
		Self::Append,
		Self::OptionSome,
		Self::RevEmpty,
		Self::RevLast,
		Self::RevAt,
		Self::IntoIter,
	];

	const fn index(self) -> u32 {
		match self {
			Self::AdapterCount => 0,
			Self::Range => 1,
			Self::Seed(n) => 2 + n as u32,
			Self::Sum => 5,
			Self::Product => 6,
			Self::Insert => 7,
			Self::IntoTuple => 8,
			Self::Append => 9,
			Self::OptionSome => 10,
			Self::RevEmpty => 11,
			Self::RevLast => 12,
			Self::RevAt => 13,
			Self::IntoIter => 14,
		}
	}

	pub(crate) const fn id(self) -> u32 {
		Self::FIRST_ID + self.index()
	}

	pub(crate) fn from_id(id: u32) -> Option<Self> {
		Self::ALL
			.get(id.checked_sub(Self::FIRST_ID)? as usize)
			.copied()
	}

	/// Operand count.
	pub(crate) const fn arity(self) -> u32 {
		match self {
			Self::AdapterCount
			| Self::Seed(_)
			| Self::IntoTuple
			| Self::OptionSome
			| Self::RevEmpty
			| Self::RevLast
			| Self::IntoIter => 1,
			_ => 2,
		}
	}
}

/// Certifies `terminal` on first use in a compile. The std Iterator trait is
/// the builtin one and nothing is left pending when `next` is certified
/// (C12); then a function at a provider hash whose only metadata names the
/// Iterator trait was installed by the builtin trait handler: only a trait
/// context sets `trait_hash`, trait handlers are insert-once, and native
/// functions are insert-once, so a rival registration at a provider hash
/// either fails or makes the std install fail and leaves it pending.
pub(crate) fn provided(context: &crate::Context, terminal: Terminal) -> bool {
	settled(context)
		&& Container::ALL.into_iter().all(|container| {
			let hash = Hash::associated_function(container.hash(), terminal.hash());
			let mut meta = context.lookup_meta_by_hash(hash);
			context.lookup_function(hash).is_some()
				&& meta.len() == 1
				&& meta.all(|m| {
					matches!(m.kind, crate::compile::meta::Kind::Function { trait_hash: Some(t), .. } if t == ITERATOR)
				})
		})
}

/// `::std::iter::range` is the std iter module's function: that module
/// defines the builtin Iterator trait, and the item hash is insert-once.
pub(crate) fn range_provided(context: &crate::Context) -> bool {
	settled(context) && context.lookup_function(RANGE).is_some()
}

fn settled(context: &crate::Context) -> bool {
	context.iter_method(super::iter::Name::Next)
}

const ITERATOR: Hash = crate::hash_in!(crate, ::std::iter::Iterator);
const DOUBLE_ENDED: Hash = crate::hash_in!(crate, ::std::iter::DoubleEndedIterator);

/// A C17e Iterator trait function a direct chain may call besides the C17d
/// terminals: the adapters and consumers that take a function value, and
/// `enumerate`, `chain` and `rev`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Closure {
	Map,
	Filter,
	FilterMap,
	FlatMap,
	Enumerate,
	Chain,
	Rev,
	Fold,
	Reduce,
	Find,
	Any,
	All,
}

impl Closure {
	pub(crate) const ALL: [Self; 12] = [
		Self::Map,
		Self::Filter,
		Self::FilterMap,
		Self::FlatMap,
		Self::Enumerate,
		Self::Chain,
		Self::Rev,
		Self::Fold,
		Self::Reduce,
		Self::Find,
		Self::Any,
		Self::All,
	];

	#[cfg(test)]
	const fn name(self) -> &'static str {
		match self {
			Self::Map => "map",
			Self::Filter => "filter",
			Self::FilterMap => "filter_map",
			Self::FlatMap => "flat_map",
			Self::Enumerate => "enumerate",
			Self::Chain => "chain",
			Self::Rev => "rev",
			Self::Fold => "fold",
			Self::Reduce => "reduce",
			Self::Find => "find",
			Self::Any => "any",
			Self::All => "all",
		}
	}

	/// The hash a method call site without a turbofish resolves to.
	pub(crate) const fn hash(self) -> Hash {
		match self {
			Self::Map => ident(crate::hash_in!(crate, ::std::vec::Vec.map)),
			Self::Filter => ident(crate::hash_in!(crate, ::std::vec::Vec.filter)),
			Self::FilterMap => ident(crate::hash_in!(crate, ::std::vec::Vec.filter_map)),
			Self::FlatMap => ident(crate::hash_in!(crate, ::std::vec::Vec.flat_map)),
			Self::Enumerate => ident(crate::hash_in!(crate, ::std::vec::Vec.enumerate)),
			Self::Chain => ident(crate::hash_in!(crate, ::std::vec::Vec.chain)),
			Self::Rev => ident(crate::hash_in!(crate, ::std::vec::Vec.rev)),
			Self::Fold => ident(crate::hash_in!(crate, ::std::vec::Vec.fold)),
			Self::Reduce => ident(crate::hash_in!(crate, ::std::vec::Vec.reduce)),
			Self::Find => ident(crate::hash_in!(crate, ::std::vec::Vec.find)),
			Self::Any => ident(crate::hash_in!(crate, ::std::vec::Vec.any)),
			Self::All => ident(crate::hash_in!(crate, ::std::vec::Vec.all)),
		}
	}

	pub(crate) fn from_hash(hash: Hash) -> Option<Self> {
		Self::ALL.into_iter().find(|c| c.hash() == hash)
	}

	/// Operand count of the call, receiver included.
	pub(crate) const fn arity(self) -> usize {
		match self {
			Self::Enumerate | Self::Rev => 1,
			Self::Fold => 3,
			_ => 2,
		}
	}

	/// Whether the call returns an iterator rather than consuming the chain.
	pub(crate) const fn adapter(self) -> bool {
		matches!(
			self,
			Self::Map
				| Self::Filter
				| Self::FilterMap
				| Self::FlatMap
				| Self::Enumerate
				| Self::Chain
				| Self::Rev
		)
	}

	/// How many arguments the call passes to its function value, if it takes one.
	pub(crate) const fn passes(self) -> Option<u32> {
		match self {
			Self::Enumerate | Self::Chain | Self::Rev => None,
			Self::Fold | Self::Reduce => Some(2),
			_ => Some(1),
		}
	}

	/// The function's bit in `CompileTransaction::checked_closure`.
	pub(crate) const fn bit(self) -> u16 {
		1 << self as u16
	}
}

/// The iterator types a C17e chain adds to [`Container::ALL`]: the text
/// sources and the C17e adapters.
const EXTENDED: [Hash; 10] = [
	crate::modules::string::Chars::HASH,
	crate::modules::string::Split::<crate::alloc::String>::HASH,
	crate::modules::string::Split::<char>::HASH,
	crate::modules::iter::Map::HASH,
	crate::modules::iter::Filter::HASH,
	crate::modules::iter::FilterMap::HASH,
	crate::modules::iter::FlatMap::HASH,
	crate::modules::iter::Enumerate::HASH,
	crate::modules::iter::Chain::HASH,
	crate::modules::iter::Rev::HASH,
];

/// The receivers of an admitted `rev`, all of which stock makes double-ended.
const REVERSIBLE: [Hash; 13] = [
	rt::slice::Iter::HASH,
	crate::modules::option::Iter::HASH,
	rt::range::RangeIter::<i64>::HASH,
	rt::range::RangeIter::<u64>::HASH,
	rt::range::RangeIter::<char>::HASH,
	rt::range_inclusive::RangeInclusiveIter::<i64>::HASH,
	rt::range_inclusive::RangeInclusiveIter::<u64>::HASH,
	rt::range_inclusive::RangeInclusiveIter::<char>::HASH,
	crate::modules::iter::Skip::HASH,
	crate::modules::iter::Take::HASH,
	crate::modules::iter::Map::HASH,
	crate::modules::iter::Filter::HASH,
	crate::modules::iter::FilterMap::HASH,
];

/// Source iterators stock gives no `rev`; a direct `rev` on one raises the
/// stock missing-method error.
pub(crate) const IRREVERSIBLE: [Hash; 4] = [
	rt::object::RuneIter::HASH,
	rt::range_from::RangeFromIter::<i64>::HASH,
	rt::range_from::RangeFromIter::<u64>::HASH,
	rt::range_from::RangeFromIter::<char>::HASH,
];

/// Whether the only function at `hash` is one the std `trait_hash` handler installed.
fn trait_provided(context: &crate::Context, hash: Hash, trait_hash: Hash) -> bool {
	let mut meta = context.lookup_meta_by_hash(hash);
	context.lookup_function(hash).is_some()
		&& meta.len() == 1
		&& meta.all(|m| {
			matches!(m.kind, crate::compile::meta::Kind::Function { trait_hash: Some(t), .. } if t == trait_hash)
		})
}

/// Certifies `function` for every iterator a C17e chain can apply it to, on
/// first use in a compile, as [`provided`] does for the C17d terminals. `rev`
/// is the DoubleEndedIterator handler's; its absence on [`IRREVERSIBLE`] is
/// checked by the caller.
pub(crate) fn closure_provided(context: &crate::Context, function: Closure) -> bool {
	if !settled(context) {
		return false;
	}
	if function == Closure::Rev {
		return REVERSIBLE.into_iter().all(|container| {
			trait_provided(
				context,
				Hash::associated_function(container, function.hash()),
				DOUBLE_ENDED,
			)
		});
	}
	Container::ALL
		.into_iter()
		.map(Container::hash)
		.chain(EXTENDED)
		.all(|container| {
			trait_provided(
				context,
				Hash::associated_function(container, function.hash()),
				ITERATOR,
			)
		})
}

/// Certifies a C17d `terminal` on the iterators only a C17e chain reaches.
pub(crate) fn extended_provided(context: &crate::Context, terminal: Terminal) -> bool {
	provided(context, terminal)
		&& EXTENDED.into_iter().all(|container| {
			trait_provided(
				context,
				Hash::associated_function(container, terminal.hash()),
				ITERATOR,
			)
		})
}

/// Certifies `next` (and the for-loop NEXT protocol) on the C17e adapters:
/// each type's own std `next`, associated with the Iterator trait when the
/// std module implements it.
pub(crate) fn extended_next_provided(context: &crate::Context) -> bool {
	settled(context)
		&& EXTENDED[3..].iter().all(|&container| {
			trait_provided(
				context,
				Hash::associated_function(container, super::iter::Name::Next.hash()),
				ITERATOR,
			)
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hashes_ids_and_names() {
		for (i, t) in Terminal::ALL.into_iter().enumerate() {
			assert_eq!(t.index(), i);
			assert_eq!(t.name_hash(), Hash::ident(t.name()));
			assert_eq!(
				t.hash(),
				Hash::ident(t.name()).with_function_parameters(t.parameters())
			);
			assert_eq!(Terminal::from_hash(t.hash()), Some(t));
			// The provider hash is the one `install_associated` registers.
			for c in Container::ALL {
				assert_eq!(
					Hash::associated_function(c.hash(), t.hash()),
					Hash::associated_function(c.hash(), Hash::ident(t.name()))
						.with_function_parameters(t.parameters())
				);
			}
		}
		assert_eq!(
			Terminal::from_hash(
				Hash::ident("collect").with_function_parameters(Hash::parameters([
					crate::modules::collections::VecDeque::HASH
				]))
			),
			None
		);
		assert_eq!(Terminal::from_hash(Hash::ident("sum")), None);
		assert_eq!(
			RANGE,
			Hash::type_hash(&crate::ItemBuf::with_crate_item("std", ["iter", "range"]).unwrap())
		);
		for (i, intrinsic) in Intrinsic::ALL.into_iter().enumerate() {
			assert_eq!(intrinsic.index() as usize, i);
			assert_eq!(Intrinsic::from_id(intrinsic.id()), Some(intrinsic));
		}
		assert_eq!(Intrinsic::from_id(Intrinsic::FIRST_ID + 15), None);
		for c in Closure::ALL {
			assert_eq!(c.hash(), Hash::ident(c.name()));
			assert_eq!(Closure::from_hash(c.hash()), Some(c));
			assert!(Terminal::from_hash(c.hash()).is_none());
		}
		assert_eq!(Intrinsic::from_id(Intrinsic::FIRST_ID - 1), None);
		for (i, c) in Container::ALL.into_iter().enumerate() {
			assert_eq!(c as usize, i);
			assert_eq!(Container::from_hash(c.hash()), Some(c));
		}
	}
}
