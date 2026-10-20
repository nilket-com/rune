//! Private iterator-method identity policy (C12). Never registers or invokes a handler.
//!
//! Fresh admits `iter`, `next` and `any` by name only when every stock provider it can meet at
//! runtime is the builtin std registration, and no other registration gives one of those names
//! to a receiver fresh can represent. Native functions are insert-once (`ConflictingFunction`),
//! so a provider recorded after its insertion is the handler that stays installed.
use crate::runtime as rt;
use crate::{Hash, TypeHash};

/// A certified method name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Name {
	Iter,
	Next,
	Any,
}

impl Name {
	pub(crate) const ALL: [Self; 3] = [Self::Iter, Self::Next, Self::Any];

	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::Iter => "iter",
			Self::Next => "next",
			Self::Any => "any",
		}
	}

	/// Compile-time associated hashes recover the identifier hash without a
	/// runtime SipHasher during Context installation. The XOR encoding is
	/// independently checked against the public Hash constructors below.
	pub(crate) const fn hash(self) -> Hash {
		const SALT: u64 = 0x5ea7_7ffb_cdf5_f302;
		let (associated, receiver) = match self {
			Self::Iter => (crate::hash_in!(crate, ::std::vec::Vec.iter), rt::Vec::HASH),
			Self::Next => (
				crate::hash_in!(crate, ::std::slice::Iter.next),
				rt::slice::Iter::HASH,
			),
			Self::Any => (
				crate::hash_in!(crate, ::std::slice::Iter.any),
				rt::slice::Iter::HASH,
			),
		};
		Hash::new(associated.into_inner() ^ receiver.into_inner() ^ SALT)
	}
	pub(crate) const fn associated(self, receiver: Hash) -> Hash {
		Hash::new(0x5ea7_7ffb_cdf5_f302 ^ receiver.into_inner() ^ self.hash().into_inner())
	}
	pub(crate) fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|n| n.as_str() == name)
	}

	/// The name's bit in `QueryInner::fresh_iter_script` and in `State::closed`.
	pub(crate) fn bit(self) -> u8 {
		1 << self as u8
	}

	/// The providers that must all be certified before the name is admitted.
	fn required(self) -> u32 {
		Provider::ALL
			.into_iter()
			.filter(|p| p.name() == self)
			.fold(0, |acc, p| acc | p.bit())
	}
}

/// Private module markers. Set only by the std modules that register the providers; user
/// registration APIs never set them.
pub(crate) mod marker {
	/// Registers the builtin `iter` of a representable receiver.
	pub(crate) const ITER: u8 = 1 << 0;
	/// Implements `::std::iter::Iterator` for a supported iterator type (the trait handler then
	/// installs its `next` and `any` instance functions).
	pub(crate) const ITERATOR: u8 = 1 << 1;
	/// Defines the `::std::iter::Iterator` trait whose handler provides `any`.
	pub(crate) const ITERATOR_TRAIT: u8 = 1 << 2;
}

const ITERATOR_TRAIT: Hash = crate::hash_in!(crate, ::std::iter::Iterator);

/// A stock provider fresh relies on: one name on one container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Provider {
	VecIter,
	OptionIter,
	TupleIter,
	ObjectIter,
	RangeIter,
	RangeInclusiveIter,
	RangeFromIter,
	SliceIterNext,
	ObjectIterNext,
	OptionIterNext,
	RangeIterNext,
	RangeInclusiveIterNext,
	RangeFromIterNext,
	SliceIterAny,
	ObjectIterAny,
	OptionIterAny,
	RangeIterAny,
	RangeInclusiveIterAny,
	RangeFromIterAny,
	SplitStringNext,
	SplitCharNext,
	SplitStringAny,
	SplitCharAny,
}

impl Provider {
	pub(crate) const ALL: [Self; 23] = [
		Self::VecIter,
		Self::OptionIter,
		Self::TupleIter,
		Self::ObjectIter,
		Self::RangeIter,
		Self::RangeInclusiveIter,
		Self::RangeFromIter,
		Self::SliceIterNext,
		Self::ObjectIterNext,
		Self::OptionIterNext,
		Self::RangeIterNext,
		Self::RangeInclusiveIterNext,
		Self::RangeFromIterNext,
		Self::SliceIterAny,
		Self::ObjectIterAny,
		Self::OptionIterAny,
		Self::RangeIterAny,
		Self::RangeInclusiveIterAny,
		Self::RangeFromIterAny,
		Self::SplitStringNext,
		Self::SplitCharNext,
		Self::SplitStringAny,
		Self::SplitCharAny,
	];

	fn bit(self) -> u32 {
		1 << self as u32
	}

	pub(crate) fn name(self) -> Name {
		match self {
			Self::SplitStringNext | Self::SplitCharNext => Name::Next,
			Self::SplitStringAny | Self::SplitCharAny => Name::Any,
			Self::VecIter
			| Self::OptionIter
			| Self::TupleIter
			| Self::ObjectIter
			| Self::RangeIter
			| Self::RangeInclusiveIter
			| Self::RangeFromIter => Name::Iter,
			Self::SliceIterNext
			| Self::ObjectIterNext
			| Self::OptionIterNext
			| Self::RangeIterNext
			| Self::RangeInclusiveIterNext
			| Self::RangeFromIterNext => Name::Next,
			_ => Name::Any,
		}
	}

	/// The receiver type hash, including type parameters.
	pub(crate) const fn container(self) -> Hash {
		match self {
			Self::SplitStringNext | Self::SplitStringAny => {
				crate::function::fresh_text::split_hash(true)
			}
			Self::SplitCharNext | Self::SplitCharAny => {
				crate::function::fresh_text::split_hash(false)
			}
			Self::VecIter => rt::Vec::HASH,
			Self::OptionIter => Option::<rt::Value>::HASH,
			Self::TupleIter => rt::OwnedTuple::HASH,
			Self::ObjectIter => rt::Object::HASH,
			Self::RangeIter => rt::Range::HASH,
			Self::RangeInclusiveIter => rt::RangeInclusive::HASH,
			Self::RangeFromIter => rt::RangeFrom::HASH,
			Self::SliceIterNext | Self::SliceIterAny => rt::slice::Iter::HASH,
			Self::ObjectIterNext | Self::ObjectIterAny => rt::object::RuneIter::HASH,
			Self::OptionIterNext | Self::OptionIterAny => crate::modules::option::Iter::HASH,
			Self::RangeIterNext | Self::RangeIterAny => rt::range::RangeIter::<i64>::HASH,
			Self::RangeInclusiveIterNext | Self::RangeInclusiveIterAny => {
				rt::range_inclusive::RangeInclusiveIter::<i64>::HASH
			}
			Self::RangeFromIterNext | Self::RangeFromIterAny => {
				rt::range_from::RangeFromIter::<i64>::HASH
			}
		}
	}

	/// The module marker that must be pending when this provider is installed.
	fn marker(self) -> u8 {
		match self.name() {
			Name::Iter => marker::ITER,
			Name::Next | Name::Any => marker::ITERATOR,
		}
	}

	fn find(name: Name, container: Hash) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|p| p.name() == name && p.container() == container)
	}
}

/// Builtin iterator types whose trait-installed `next`/`any` are not providers (fresh's
/// intrinsic decides their payloads) and do not close the names.
const AUXILIARY: [Hash; 9] = [
	crate::modules::string::Chars::HASH,
	crate::modules::iter::Skip::HASH,
	crate::modules::iter::Take::HASH,
	rt::range::RangeIter::<u64>::HASH,
	rt::range::RangeIter::<char>::HASH,
	rt::range_inclusive::RangeInclusiveIter::<u64>::HASH,
	rt::range_inclusive::RangeInclusiveIter::<char>::HASH,
	rt::range_from::RangeFromIter::<u64>::HASH,
	rt::range_from::RangeFromIter::<char>::HASH,
];

/// Receivers a fresh value can have at runtime. A foreign registration of a certified name on one
/// of these would change what fresh owes, so it closes that name. Unrepresentable receivers
/// (`HashMap`, generators, external types) never reach fresh and cannot close it.
/// The list mirrors `experimental_core::detail::TypeName::builtin_hash` plus the C12 range
/// identities; a new fresh value kind must be added here.
pub(crate) fn representable(container: Hash) -> bool {
	const OTHERS: [Hash; 22] = [
		crate::modules::string::Chars::HASH,
		crate::modules::iter::Skip::HASH,
		crate::modules::iter::Take::HASH,
		bool::HASH,
		i64::HASH,
		u64::HASH,
		f64::HASH,
		char::HASH,
		core::cmp::Ordering::HASH,
		crate::alloc::String::HASH,
		core::result::Result::<rt::Value, rt::Value>::HASH,
		rt::Function::HASH,
		rt::Type::HASH,
		rt::RangeTo::HASH,
		rt::RangeToInclusive::HASH,
		rt::RangeFull::HASH,
		rt::range::RangeIter::<u64>::HASH,
		rt::range::RangeIter::<char>::HASH,
		rt::range_inclusive::RangeInclusiveIter::<u64>::HASH,
		rt::range_inclusive::RangeInclusiveIter::<char>::HASH,
		rt::range_from::RangeFromIter::<u64>::HASH,
		rt::range_from::RangeFromIter::<char>::HASH,
	];
	Provider::ALL
		.into_iter()
		.any(|p| p.container() == container)
		|| OTHERS.contains(&container)
}

/// Installation fast path, run for every associated registration: the certified name this
/// registration touches, if any. An instance name is matched by its bytes; a protocol or other
/// association by its effective hash against the three named-function hashes. Every other
/// registration skips [`State::begin_associated`] entirely.
#[inline]
pub(crate) fn touched_name(
	instance: Option<&str>,
	effective: Hash,
	container: Hash,
) -> Option<Name> {
	match instance {
		Some(name) => match name.as_bytes() {
			b"iter" => Some(Name::Iter),
			b"next" => Some(Name::Next),
			b"any" => Some(Name::Any),
			_ => None,
		},
		None => Name::ALL
			.into_iter()
			.find(|n| effective == n.associated(container)),
	}
}

/// What an associated registration means for the certificate.
pub(crate) struct Touch {
	name: Name,
	provider: Option<Provider>,
}

#[derive(Default)]
pub(crate) struct State {
	/// Providers whose builtin handler is installed.
	certified: u32,
	/// Names closed by a foreign registration on a representable receiver.
	closed: u8,
	/// Markers of the module being installed.
	pending: u8,
	/// `::std::iter::Iterator` was defined by the marked std module.
	iterator_trait: bool,
}

impl State {
	pub(crate) fn builtin_trait(&self) -> bool {
		self.iterator_trait
	}
	pub(crate) fn begin_module(&mut self, markers: u8) -> u8 {
		core::mem::replace(&mut self.pending, markers)
	}

	pub(crate) fn finish_module(&mut self, old: u8) {
		self.pending = old;
	}

	/// After `install_trait` stored a trait handler (insert-once, `ConflictingTrait`).
	pub(crate) fn trait_installed(&mut self, hash: Hash) {
		if hash == ITERATOR_TRAIT {
			self.iterator_trait = self.pending & marker::ITERATOR_TRAIT != 0;
		}
	}

	/// Before `install_associated` inserts anything. `instance` is the instance name, or `None`
	/// for protocol associations, which are matched only by the named-function hash. The NEXT
	/// protocol hash differs from `next`; the Iterator trait handler's `find` copies the
	/// protocol handler into a separate named `next` instance function, which is the provider. A foreign registration
	/// closes its name immediately, so a later failure cannot leave it open.
	#[cold]
	#[inline(never)]
	pub(crate) fn begin_associated(
		&mut self,
		name: Name,
		container: Hash,
		plain: bool,
		trait_hash: Option<Hash>,
	) -> Option<Touch> {
		let provider = Provider::find(name, container).filter(|p| {
			// Both `next` (the trait handler's named copy of NEXT) and `any` are trait-installed.
			plain
				&& self.pending & p.marker() != 0
				&& match name {
					// The Iterator trait handler installs the `next` and `any` instance
					// functions (from the NEXT protocol and its provided method).
					Name::Next | Name::Any => {
						trait_hash == Some(ITERATOR_TRAIT) && self.iterator_trait
					}
					Name::Iter => trait_hash.is_none(),
				}
		});

		// The only non-provider builtin registrations on representable receivers are the
		// Iterator trait's `next`/`any` on the u64/char range iterators, installed by the marked
		// ops module; fresh's intrinsic decides those payloads. Anything else on a representable
		// receiver is foreign, even when it rides in a marked std module value (embedders can add
		// functions to the public `Module` before installing it).
		let auxiliary = matches!(name, Name::Next | Name::Any)
			&& AUXILIARY.contains(&container)
			&& plain && self.pending & marker::ITERATOR != 0
			&& trait_hash == Some(ITERATOR_TRAIT)
			&& self.iterator_trait;
		if provider.is_none() && !auxiliary && representable(container) {
			self.closed |= name.bit();
			self.certified &= !name.required();
		}

		Some(Touch { name, provider })
	}

	/// After `install_associated` succeeded. Native functions are insert-once
	/// (`insert_native_fn` refuses an occupied hash with `ConflictingFunction`), so the handler
	/// certified here stays installed: an earlier foreign insert at a provider hash makes this
	/// install fail before this point (leaving the module pending, which keeps every name
	/// uncertified), and a later one fails itself. No per-insert scan is needed.
	pub(crate) fn associated_installed(&mut self, touch: Touch) {
		if let Some(p) = touch.provider {
			if self.closed & touch.name.bit() == 0 {
				self.certified |= p.bit();
			}
		}
	}

	/// The name is admitted: all its providers are the installed builtins and nothing closed it.
	pub(crate) fn certified(&self, name: Name) -> bool {
		let required = name.required();
		self.pending == 0
			&& self.closed & name.bit() == 0
			&& self.certified & required == required
			&& (name == Name::Iter || self.iterator_trait)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn const_hashes_match_public_hash_constructors() {
		for name in Name::ALL {
			assert_eq!(name.hash(), Hash::ident(name.as_str()));
			for p in Provider::ALL {
				assert_eq!(
					name.associated(p.container()),
					Hash::associated_function(p.container(), Hash::ident(name.as_str()))
				);
			}
		}
	}
	#[test]
	fn provider_hashes_are_distinct_and_names_partition() {
		for (i, a) in Provider::ALL.into_iter().enumerate() {
			for b in &Provider::ALL[i + 1..] {
				assert!(
					a.container() != b.container() || a.name() != b.name(),
					"{a:?} {b:?}"
				);
			}
		}
		let all = Name::ALL.into_iter().fold(0, |acc, n| acc | n.required());
		assert_eq!(all.count_ones() as usize, Provider::ALL.len());
	}

	#[test]
	fn foreign_registration_on_representable_receiver_closes() {
		let mut s = State::default();
		let old = s.begin_module(marker::ITER);
		for p in Provider::ALL.into_iter().filter(|p| p.name() == Name::Iter) {
			let t = s
				.begin_associated(Name::Iter, p.container(), true, None)
				.unwrap();
			s.associated_installed(t);
		}
		s.finish_module(old);
		assert!(s.certified(Name::Iter));
		// An unmarked module gives `iter` to String: fresh's missing-method promise breaks.
		let t = s
			.begin_associated(Name::Iter, crate::alloc::String::HASH, true, None)
			.unwrap();
		s.associated_installed(t);
		assert!(!s.certified(Name::Iter));
	}

	#[test]
	fn unmarked_or_parameterized_provider_never_certifies() {
		let mut s = State::default();
		let p = Provider::VecIter;
		let t = s
			.begin_associated(Name::Iter, p.container(), true, None)
			.unwrap();
		s.associated_installed(t);
		assert!(!s.certified(Name::Iter));
		let mut s = State::default();
		let old = s.begin_module(marker::ITER);
		let t = s
			.begin_associated(Name::Iter, p.container(), false, None)
			.unwrap();
		s.associated_installed(t);
		s.finish_module(old);
		assert!(!s.certified(Name::Iter));
	}

	#[test]
	fn any_requires_the_marked_iterator_trait() {
		let mut s = State::default();
		let old = s.begin_module(marker::ITERATOR);
		for p in Provider::ALL.into_iter().filter(|p| p.name() == Name::Any) {
			let t = s
				.begin_associated(Name::Any, p.container(), true, Some(ITERATOR_TRAIT))
				.unwrap();
			s.associated_installed(t);
		}
		s.finish_module(old);
		// The trait was never defined by the marked module: `any` stays uncertified.
		assert!(!s.certified(Name::Any));
	}

	#[test]
	fn foreign_next_and_any_on_char_range_from_iter_close() {
		for (name, s) in [("next", Name::Next), ("any", Name::Any)] {
			let mut st = State::default();
			let t = st
				.begin_associated(
					Name::from_name(name).unwrap(),
					rt::range_from::RangeFromIter::<char>::HASH,
					true,
					None,
				)
				.unwrap();
			st.associated_installed(t);
			assert_ne!(st.closed & s.bit(), 0, "{name}");
		}
	}

	#[test]
	fn failed_module_install_keeps_every_name_uncertified() {
		let mut s = State::default();
		let old = s.begin_module(marker::ITER);
		for p in Provider::ALL.into_iter().filter(|p| p.name() == Name::Iter) {
			let t = s
				.begin_associated(Name::Iter, p.container(), true, None)
				.unwrap();
			s.associated_installed(t);
		}
		// `Context::install` returned early: `finish_module` never ran.
		assert!(!s.certified(Name::Iter));
		s.finish_module(old);
		assert!(s.certified(Name::Iter));
	}

	#[test]
	fn marked_module_extra_registration_on_representable_receiver_closes() {
		// An embedder adds `String.iter` / `Vec.any` to a std module value before installing it:
		// the module's private marker must not make those foreign registrations look builtin.
		let mut s = State::default();
		let old = s.begin_module(marker::ITER | marker::ITERATOR);
		let t = s
			.begin_associated(Name::Iter, crate::alloc::String::HASH, true, None)
			.unwrap();
		s.associated_installed(t);
		let t = s
			.begin_associated(Name::Any, rt::Vec::HASH, true, Some(ITERATOR_TRAIT))
			.unwrap();
		s.associated_installed(t);
		s.finish_module(old);
		assert_ne!(s.closed & Name::Iter.bit(), 0);
		assert_ne!(s.closed & Name::Any.bit(), 0);
	}

	#[test]
	fn marked_auxiliary_range_iterators_do_not_close() {
		let mut s = State::default();
		let old = s.begin_module(marker::ITERATOR_TRAIT);
		s.trait_installed(ITERATOR_TRAIT);
		s.finish_module(old);
		let old = s.begin_module(marker::ITERATOR);
		for container in AUXILIARY {
			for name in ["next", "any"] {
				let t = s
					.begin_associated(
						Name::from_name(name).unwrap(),
						container,
						true,
						Some(ITERATOR_TRAIT),
					)
					.unwrap();
				s.associated_installed(t);
			}
		}
		s.finish_module(old);
		assert_eq!(s.closed, 0);
	}

	#[test]
	fn touched_name_matches_instance_bytes_and_named_hashes_only() {
		assert_eq!(
			touched_name(Some("iter"), Hash::EMPTY, rt::Vec::HASH),
			Some(Name::Iter)
		);
		assert_eq!(
			touched_name(Some("next"), Hash::EMPTY, rt::Vec::HASH),
			Some(Name::Next)
		);
		assert_eq!(
			touched_name(Some("any"), Hash::EMPTY, rt::Vec::HASH),
			Some(Name::Any)
		);
		for other in ["it", "iters", "nex", "anyx", "len", "into_iter", ""] {
			assert_eq!(
				touched_name(Some(other), Hash::EMPTY, rt::Vec::HASH),
				None,
				"{other}"
			);
		}
		for name in Name::ALL {
			let h = Hash::associated_function(rt::slice::Iter::HASH, Hash::ident(name.as_str()));
			assert_eq!(touched_name(None, h, rt::slice::Iter::HASH), Some(name));
		}
		let next_protocol =
			Hash::associated_function(rt::slice::Iter::HASH, crate::runtime::Protocol::NEXT.hash);
		assert_eq!(
			touched_name(None, next_protocol, rt::slice::Iter::HASH),
			None
		);
	}
}
