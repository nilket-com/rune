//! Values of the fresh core, as a host sees them: [`Root`]s, [`Scalar`]s and [`Type`]s.
//!
//! Internally values are plain tagged slots, with explicit store-mediated retain and
//! release for handles. Inside the core a value belongs to exactly one store by construction, so a
//! handle carries no owner: the owner check happens once, at the sealed host
//! boundary ([`Root`]), never on the executor's path. `Value` is 16 bytes.

/// An entry of one store. Crate-private fields: nothing outside the crate can
/// construct or inspect one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Handle {
	pub(crate) slot: u32,
	pub(crate) generation: u32,
}

/// A nominal type identity: an index into the crate-private, process-wide
/// interner. Same hash, same identity, in every program and store.
/// Private field: a host cannot fabricate one.
///
/// The table is global to the process and append-only: an identity is never reclaimed,
/// even after every program and store that used it is gone. It holds at most 4,096
/// identities, with names of at most 256 bytes and at most 256 KiB of names in total for
/// the life of the process. A compile that needs a new identity once the table is full is
/// refused with [`Refusal::TypeTable`](crate::experimental_core::error::Refusal::TypeTable).
///
/// ```compile_fail
/// let _ = rune::experimental_core::value::Nominal(0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nominal(pub(crate) u32);

/// The type of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
	/// `()`.
	Unit,
	/// `bool`.
	Bool,
	/// `i64`.
	I64,
	/// `u64`.
	U64,
	/// `f64`.
	F64,
	/// `char`.
	Char,
	/// `std::cmp::Ordering`.
	Ordering,
	/// `String`.
	String,
	/// `Vec`.
	Vec,
	/// A tuple.
	Tuple,
	/// An anonymous object (`#{...}`).
	Object,
	/// A struct declared by a script, identified by its interned [`Nominal`].
	Struct(Nominal),
	/// `Option`.
	Option,
	/// `Result`.
	Result,
	/// A range.
	Range,
	/// An iterator.
	Iterator,
}

/// The executor's slot value. A `Heap` variant can only be produced by a
/// `Store`, because no `Handle` can be constructed or read outside the crate
/// (see [`Handle`]); `Function` carries the identity of the program it
/// indexes. A `Value` never crosses the host boundary: `Executor::invoke`
/// and every `Store` method a host can call take and return [`Root`]s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Value {
	Unit,
	Bool(bool),
	I64(i64),
	U64(u64),
	F64(f64),
	Char(char),
	/// A `std::cmp::Ordering`.
	Ordering(core::cmp::Ordering),
	Heap(Handle),
	Function {
		code: u32,
		id: u32,
	},
	Type(Type),
}

impl Value {
	/// Heap tags require generation validation by the store, not a blind tag read.
	pub(crate) fn primitive_type(self) -> Option<Type> {
		Some(match self {
			Self::Unit => Type::Unit,
			Self::Bool(_) => Type::Bool,
			Self::I64(_) => Type::I64,
			Self::U64(_) => Type::U64,
			Self::F64(_) => Type::F64,
			Self::Char(_) => Type::Char,
			Self::Ordering(_) => Type::Ordering,
			_ => return None,
		})
	}
}

/// A scalar read back through the host boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
	/// `()`.
	Unit,
	/// A `bool`.
	Bool(bool),
	/// An `i64`.
	I64(i64),
	/// A `u64`.
	U64(u64),
	/// An `f64`.
	F64(f64),
	/// A `char`.
	Char(char),
	/// A `std::cmp::Ordering`.
	Ordering(core::cmp::Ordering),
}

/// The host's owned reference to a value of one store: the only way a value
/// crosses the boundary. Affine: not `Copy`, not `Clone`; `Store::root_release`
/// consumes it and `Store::root_retain` makes an independently counted owner.
/// Private fields: a host can neither read the brand nor forge one.
///
/// Release is manual. Dropping a `Root` without
/// [`Store::root_release`](crate::experimental_core::heap::Store::root_release) does not
/// release its reference: a heap entry it owns stays live and charged against the store's
/// [`Limits`](crate::experimental_core::heap::Limits) until the store is closed or dropped,
/// which frees every entry. Scalar roots own no entry.
///
/// ```compile_fail
/// fn f(r: rune::experimental_core::value::Root) -> u64 { r.store }
/// ```
///
/// ```compile_fail
/// fn f(r: &rune::experimental_core::value::Root) -> rune::experimental_core::value::Root { r.clone() }
/// ```
///
/// ```compile_fail
/// fn f(r: rune::experimental_core::value::Root) -> (rune::experimental_core::value::Root, rune::experimental_core::value::Root) { (r, r) }
/// ```
///
/// ```compile_fail
/// fn f(s: &mut rune::experimental_core::heap::Store, r: rune::experimental_core::value::Root) {
///     s.root_release(r).unwrap();
///     s.root_len(&r).unwrap();
/// }
/// ```
///
/// ```compile_fail
/// fn f(s: &rune::experimental_core::heap::Store, r: &rune::experimental_core::value::Root) {
///     s.import(r).unwrap();
/// }
/// ```
///
/// ```compile_fail
/// fn f(s: &rune::experimental_core::heap::Store, v: rune::experimental_core::value::Value) -> rune::experimental_core::value::Root {
///     s.export(v)
/// }
/// ```
#[derive(Debug)]
pub struct Root {
	pub(crate) store: u64,
	pub(crate) value: Value,
}

/// Why `root_release` refused before consuming the reference: the unchanged
/// owned root comes back so the only owner is not lost.
#[derive(Debug)]
pub enum ReleaseError {
	/// Refused before the root's edge was released (closed, foreign or retired
	/// store entry, or a last release rejected by its retirement checks): the
	/// unchanged root is returned.
	Refused {
		/// The unchanged root, still owned by the caller.
		root: Root,
		/// Why the release was refused.
		error: super::error::Error,
	},
	/// The edge was released and a later drainage step failed: the root is consumed.
	Failed(super::error::Error),
}

impl Type {
	pub(crate) fn nominal(self) -> Option<Nominal> {
		match self {
			Self::Struct(n) => Some(n),
			_ => None,
		}
	}
}
