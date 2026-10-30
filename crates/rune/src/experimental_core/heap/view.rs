//! Bounded, borrowed inspection. No script protocol or host handler is invoked.

use super::{err, Payload, Store};
use crate::experimental_core::{
	error::{Error, ErrorKind, Result, TypeName},
	nominal,
	value::{Root, Scalar, Type, Value},
};
use core::cell::Cell;

/// Limits shared by every child obtained from one [`RootView`].
#[derive(Clone, Copy, Debug)]
pub struct ViewLimits {
	/// Maximum successful root/child visits, including repeated aliases.
	pub nodes: usize,
	/// Maximum child depth. The root is at depth zero.
	pub depth: usize,
}

impl Default for ViewLimits {
	fn default() -> Self {
		Self {
			nodes: 4096,
			depth: 32,
		}
	}
}

/// Comparable identity for one heap value; contains no address or public handle.
/// Identities from different Stores or generations are different.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueIdentity {
	store: u64,
	slot: u32,
	generation: u32,
}

impl core::fmt::Debug for ValueIdentity {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		f.write_str("ValueIdentity { .. }")
	}
}

/// Declaration shape, independent of the physical heap layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NominalShape {
	/// Named fields, accessible by index and field name.
	Named,
	/// No fields and no tuple punctuation.
	Empty,
	/// Positional fields.
	Tuple(u32),
}

/// Fixed-size copy of immutable nominal metadata. No heap allocation is needed.
#[derive(Clone, Debug)]
pub struct NominalInfo {
	name: [u8; 256],
	len: usize,
	/// Declaration shape.
	pub shape: NominalShape,
	/// Canonical type hash (the parent type for an enum variant).
	pub hash: u64,
	/// Variant hash and declaration ordinal, or `None` for a struct.
	pub variant: Option<(u64, u32)>,
}

impl NominalInfo {
	/// Complete declared name, including the variant name when present.
	pub fn name(&self) -> &str {
		// Only the interner's validated UTF-8 is copied into this private buffer.
		core::str::from_utf8(&self.name[..self.len]).expect("nominal name is UTF-8")
	}
}

/// One value's presentation shape. Containers expose their children separately.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewKind<'a> {
	/// A moved payload: only identity and diagnostic type remain readable.
	/// Child, field and content access continue to return AccessConflict.
	Moved(Type),
	/// A scalar, including unit.
	Scalar(Scalar),
	/// Borrowed string bytes; no string is copied.
	String(&'a str),
	/// A vector or tuple, with its indexed child count.
	List {
		/// Vector or tuple tag.
		tag: Type,
		/// Indexed child count.
		len: usize,
	},
	/// An anonymous object, with indexed field names and children.
	Object {
		/// Indexed field count.
		len: usize,
	},
	/// A script nominal, with indexed fields and separate declaration metadata.
	Nominal {
		/// Indexed child count.
		len: usize,
	},
	/// `None` or `Some`; the latter has child zero.
	Option {
		/// Whether child zero is present.
		some: bool,
	},
	/// `Ok` or `Err`, each with child zero.
	Result {
		/// Whether the payload is the success variant.
		ok: bool,
	},
	/// A function token or closure. Captures are intentionally not exposed.
	Function,
	/// A range or iterator. Its diagnostic type name is available separately.
	Opaque(Type),
	/// A script type value.
	Type(Type),
}

/// The shared traversal budget and immutable Store borrow for a root.
///
/// Inspection allocates nothing. Each indexed access is constant time; repeated
/// aliases consume visits and cannot bypass the node/depth limits. Consumers
/// keep their own path of [`ValueIdentity`] values to recognize cycles. Holding
/// this borrow prevents mutation, closing the Store or releasing its roots.
///
/// ```compile_fail
/// use rune::experimental_core::{heap::{Store, ViewLimits}, value::Root};
/// fn close(store: &mut Store, root: &Root) {
///     let view = store.root_view(root, ViewLimits::default()).unwrap();
///     store.close().unwrap();
///     let _ = view.root();
/// }
/// ```
///
/// ```compile_fail
/// use rune::experimental_core::{heap::{Store, ViewLimits}, value::Root};
/// fn release(store: &mut Store, root: Root) {
///     let view = store.root_view(&root, ViewLimits::default()).unwrap();
///     store.root_release(root).unwrap();
///     let _ = view.root();
/// }
/// ```
pub struct RootView<'s> {
	store: &'s Store,
	root: Value,
	remaining: Cell<usize>,
	depth: usize,
}

impl Store {
	/// Validate the root and borrow its graph without retaining or executing it.
	pub fn root_view(&self, root: &Root, limits: ViewLimits) -> Result<RootView<'_>> {
		let value = self.import(root)?;
		if let Value::Heap(handle) = value {
			self.entry(handle)?;
		}
		Ok(RootView {
			store: self,
			root: value,
			remaining: Cell::new(limits.nodes),
			depth: limits.depth,
		})
	}
}

impl<'s> RootView<'s> {
	/// Borrow the root, consuming one visit from this view's shared budget.
	pub fn root(&self) -> Result<ValueView<'_, 's>> {
		self.visit(self.root, 0)
	}

	fn visit(&self, value: Value, depth: usize) -> Result<ValueView<'_, 's>> {
		if depth > self.depth || self.remaining.get() == 0 {
			return Err(Error::new(ErrorKind::FormatLimit, "value view nodes/depth"));
		}
		if let Value::Heap(handle) = value {
			self.store.entry(handle)?;
		}
		self.remaining.set(self.remaining.get() - 1);
		Ok(ValueView {
			view: self,
			value,
			depth,
		})
	}
}

/// A borrowed node. It neither owns a Root nor exposes an internal heap handle.
pub struct ValueView<'v, 's> {
	view: &'v RootView<'s>,
	value: Value,
	depth: usize,
}

impl<'v, 's> ValueView<'v, 's> {
	fn payload(&self) -> Result<&'s Payload> {
		let Value::Heap(handle) = self.value else {
			return Err(err(ErrorKind::Type));
		};
		self.view.store.payload(handle)
	}

	/// Inspect the node's shape without copying any children.
	pub fn kind(&self) -> Result<ViewKind<'s>> {
		if let Value::Heap(handle) = self.value {
			if let Some(Payload::Moved(tag)) = self.view.store.entry(handle)?.payload.as_ref() {
				return Ok(ViewKind::Moved(*tag));
			}
		}
		Ok(match self.value {
			Value::Unit => ViewKind::Scalar(Scalar::Unit),
			Value::Bool(v) => ViewKind::Scalar(Scalar::Bool(v)),
			Value::I64(v) => ViewKind::Scalar(Scalar::I64(v)),
			Value::U64(v) => ViewKind::Scalar(Scalar::U64(v)),
			Value::F64(v) => ViewKind::Scalar(Scalar::F64(v)),
			Value::Char(v) => ViewKind::Scalar(Scalar::Char(v)),
			Value::Ordering(v) => ViewKind::Scalar(Scalar::Ordering(v)),
			Value::Function { .. } => ViewKind::Function,
			Value::Type(tag) => ViewKind::Type(tag),
			Value::Heap(_) => match self.payload()? {
				Payload::String(text) => ViewKind::String(text.as_str()),
				Payload::List {
					tag: Type::Struct(_),
					values,
				} => ViewKind::Nominal { len: values.len() },
				Payload::List { tag, values } => ViewKind::List {
					tag: *tag,
					len: values.len(),
				},
				Payload::Record {
					tag: Type::Struct(_),
					fields,
				} => ViewKind::Nominal { len: fields.len() },
				Payload::Record { fields, .. } => ViewKind::Object { len: fields.len() },
				Payload::Variant {
					tag: Type::Option,
					success,
					..
				} => ViewKind::Option { some: *success },
				Payload::Variant {
					tag: Type::Result,
					success,
					..
				} => ViewKind::Result { ok: *success },
				Payload::Closure { .. } => ViewKind::Function,
				payload => ViewKind::Opaque(payload.tag()),
			},
		})
	}

	/// Stable graph identity, or `None` for immediate scalars/function/type tokens.
	pub fn identity(&self) -> Option<ValueIdentity> {
		let Value::Heap(handle) = self.value else {
			return None;
		};
		Some(ValueIdentity {
			store: self.view.store.id,
			slot: handle.slot,
			generation: handle.generation,
		})
	}

	/// Typed diagnostic name, including the particular iterator type.
	pub fn type_name(&self) -> Result<TypeName> {
		self.view
			.store
			.diagnostic_type(self.value)
			.ok_or_else(|| err(ErrorKind::Type))
	}

	/// Copy nominal declaration metadata, if this is a nominal value/type token.
	pub fn nominal(&self) -> Result<Option<NominalInfo>> {
		let tag = match self.value {
			Value::Type(tag) => tag,
			Value::Heap(_) => self.payload()?.tag(),
			_ => return Ok(None),
		};
		let Type::Struct(nominal) = tag else {
			return Ok(None);
		};
		let mut name = [0; 256];
		let invalid = || err(ErrorKind::InvalidCode);
		let len = nominal::name_copy(nominal, &mut name).ok_or_else(invalid)?;
		let shape = match nominal::shape_copy(nominal).ok_or_else(invalid)? {
			nominal::Shape::Named => NominalShape::Named,
			nominal::Shape::Empty => NominalShape::Empty,
			nominal::Shape::Tuple(n) => NominalShape::Tuple(n),
		};
		let (variant, ordinal) = nominal::variant_info(nominal).ok_or_else(invalid)?;
		Ok(Some(NominalInfo {
			name,
			len,
			shape,
			hash: nominal::hash_copy(nominal).ok_or_else(invalid)?,
			variant: (variant != 0).then_some((variant, ordinal)),
		}))
	}

	/// Indexed field name for an object or named nominal; no key search or copy.
	pub fn field_name(&self, index: usize) -> Result<&'s str> {
		let Payload::Record { fields, .. } = self.payload()? else {
			return Err(err(ErrorKind::Type));
		};
		fields
			.get(index)
			.map(|(name, _)| name.as_str())
			.ok_or_else(|| err(ErrorKind::IndexOutOfBounds))
	}

	/// Borrow a positional child or the value of an indexed field. Option/Result
	/// payloads use index zero. Invalid indices do not consume a visit.
	pub fn child(&self, index: usize) -> Result<ValueView<'v, 's>> {
		let child = match self.payload()? {
			Payload::List { values, .. } => values.get(index).copied(),
			Payload::Record { fields, .. } => fields.get(index).map(|(_, value)| *value),
			Payload::Variant { payload, .. } if index == 0 => *payload,
			Payload::Variant { .. } => None,
			_ => return Err(err(ErrorKind::Type)),
		}
		.ok_or_else(|| err(ErrorKind::IndexOutOfBounds))?;
		let depth = self
			.depth
			.checked_add(1)
			.ok_or_else(|| Error::new(ErrorKind::FormatLimit, "value view depth"))?;
		self.view.visit(child, depth)
	}
}
