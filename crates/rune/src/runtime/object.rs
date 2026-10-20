use core::borrow;
use core::cmp;
use core::fmt;
use core::hash;
use core::iter;
use core::ptr::NonNull;
use core::slice;

use crate as rune;
use crate::alloc::prelude::*;
use crate::alloc::{self, String};
use crate::runtime::object_map::ObjectMap;
use crate::runtime::{
	Dismantle, FromValue, Handover, MaybeTypeOf, ProtocolCaller, RawAnyGuard, Ref, Repr,
	RuntimeError, ToValue, Value, VmError, Worklist,
};
use crate::Any;

macro_rules! entry_iterator {
	($(#[$meta:meta])* $name:ident<$($life:lifetime)?>, $inner:ty, $item:ty, |$entry:pat_param| $map:expr) => {
		$(#[$meta])*
		pub struct $name<$($life)?> {
			inner: $inner,
		}

		impl<$($life)?> Iterator for $name<$($life)?> {
			type Item = $item;

			#[inline]
			fn next(&mut self) -> Option<Self::Item> {
				let $entry = self.inner.next()?;
				Some($map)
			}

			#[inline]
			fn size_hint(&self) -> (usize, Option<usize>) {
				self.inner.size_hint()
			}
		}

		impl<$($life)?> DoubleEndedIterator for $name<$($life)?> {
			#[inline]
			fn next_back(&mut self) -> Option<Self::Item> {
				let $entry = self.inner.next_back()?;
				Some($map)
			}
		}

		impl<$($life)?> ExactSizeIterator for $name<$($life)?> {}
		impl<$($life)?> iter::FusedIterator for $name<$($life)?> {}

		impl<$($life)?> fmt::Debug for $name<$($life)?> {
			fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				f.debug_struct(stringify!($name))
					.field("remaining", &self.inner.len())
					.finish()
			}
		}
	};
}

entry_iterator! {
	/// An owning iterator over the entries of an [`Object`], in insertion
	/// order, created by its `into_iter` method.
	IntoIter<>, alloc::vec::IntoIter<(String, Value)>, (String, Value), |entry| entry
}

entry_iterator! {
	/// A mutable iterator over the entries of an [`Object`], in insertion
	/// order, created by [`Object::iter_mut`].
	IterMut<'a>, crate::runtime::object_map::IterMut<'a, Value>, (&'a String, &'a mut Value), |entry| entry
}

entry_iterator! {
	/// An iterator over the entries of an [`Object`], in insertion order,
	/// created by [`Object::iter`].
	#[derive(Clone)]
	Iter<'a>, slice::Iter<'a, (String, Value)>, (&'a String, &'a Value), |(key, value)| (key, value)
}

entry_iterator! {
	/// An iterator over the keys of an [`Object`], in insertion order, created
	/// by [`Object::keys`].
	#[derive(Clone)]
	Keys<'a>, slice::Iter<'a, (String, Value)>, &'a String, |(key, _)| key
}

entry_iterator! {
	/// An iterator over the values of an [`Object`], in insertion order,
	/// created by [`Object::values`].
	#[derive(Clone)]
	Values<'a>, slice::Iter<'a, (String, Value)>, &'a Value, |(_, value)| value
}

/// Struct representing a dynamic anonymous object.
///
/// # Rust Examples
///
/// ```rust
/// use rune::alloc::String;
///
/// let mut object = rune::runtime::Object::new();
/// assert!(object.is_empty());
///
/// object.insert_value(String::try_from("foo")?, 42)?;
/// object.insert_value(String::try_from("bar")?, true)?;
/// assert_eq!(2, object.len());
///
/// assert_eq!(Some(42), object.get_value("foo")?);
/// assert_eq!(Some(true), object.get_value("bar")?);
/// assert_eq!(None::<bool>, object.get_value("baz")?);
/// # Ok::<_, rune::support::Error>(())
/// ```
#[derive(Any, Default)]
#[repr(transparent)]
#[rune(item = ::std::object, dismantle)]
pub struct Object {
	inner: ObjectMap<Value>,
}

/// An object is made of the values put into it, and a script can nest one
/// inside another without any bound, so it hands over what it is made of rather
/// than being dropped in place.
impl Dismantle for Object {
	fn dismantle(&mut self, out: &mut Handover<'_>) {
		// The map is emptied rather than walked in place, so that what it holds
		// is handed over in one pass over it.
		for (_, value) in self.inner.take_entries() {
			out.push(value);
		}
	}
}

impl Object {
	/// Construct a new object.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = Object::new();
	/// object.insert("Hello", "World");
	/// ```
	#[inline]
	#[rune::function(keep, path = Self::new)]
	pub fn new() -> Self {
		Self {
			inner: ObjectMap::new(),
		}
	}

	/// Construct a new object with the given capacity.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = Object::with_capacity(16);
	/// object.insert("Hello", "World");
	/// ```
	#[inline]
	#[rune::function(keep, path = Self::with_capacity)]
	pub fn with_capacity(capacity: usize) -> alloc::Result<Self> {
		Ok(Self {
			inner: ObjectMap::try_with_capacity(capacity)?,
		})
	}

	/// Returns the number of elements in the object.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = Object::with_capacity(16);
	/// object.insert("Hello", "World");
	/// assert_eq!(object.len(), 1);
	/// ```
	#[inline]
	#[rune::function(keep)]
	pub fn len(&self) -> usize {
		self.inner.len()
	}

	/// Returns `true` if the object is empty.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = Object::with_capacity(16);
	/// assert!(object.is_empty());
	/// object.insert("Hello", "World");
	/// assert!(!object.is_empty());
	/// ```
	#[inline]
	#[rune::function(keep)]
	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}

	/// Returns a reference to the value corresponding to the key.
	#[inline]
	pub fn get<Q>(&self, k: &Q) -> Option<&Value>
	where
		String: borrow::Borrow<Q>,
		Q: ?Sized + hash::Hash + cmp::Eq + cmp::Ord,
	{
		self.inner.get(k)
	}

	/// Get the given value at the given index.
	pub fn get_value<Q, T>(&self, k: &Q) -> Result<Option<T>, VmError>
	where
		String: borrow::Borrow<Q>,
		Q: ?Sized + hash::Hash + cmp::Eq + cmp::Ord,
		T: FromValue,
	{
		let value = match self.inner.get(k) {
			Some(value) => value.clone(),
			None => return Ok(None),
		};

		Ok(Some(T::from_value(value)?))
	}

	/// Returns a mutable reference to the value corresponding to the key.
	#[inline]
	pub fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut Value>
	where
		String: borrow::Borrow<Q>,
		Q: ?Sized + hash::Hash + cmp::Eq + cmp::Ord,
	{
		self.inner.get_mut(k)
	}

	/// Returns `true` if the map contains a value for the specified key.
	#[inline]
	pub fn contains_key<Q>(&self, k: &Q) -> bool
	where
		String: borrow::Borrow<Q>,
		Q: ?Sized + hash::Hash + cmp::Eq + cmp::Ord,
	{
		self.inner.contains_key(k)
	}

	/// Removes a key from the map, returning the value at the key if the key
	/// was previously in the map.
	#[inline]
	pub fn remove<Q>(&mut self, k: &Q) -> Option<Value>
	where
		String: borrow::Borrow<Q>,
		Q: ?Sized + hash::Hash + cmp::Eq + cmp::Ord,
	{
		self.inner.remove(k)
	}

	/// Inserts a key-value pair into the dynamic object, converting it as
	/// necessary through the [`ToValue`] trait.
	#[inline]
	pub fn insert_value<T>(&mut self, k: String, v: T) -> Result<(), VmError>
	where
		T: ToValue,
	{
		// Any value which was already there is taken apart rather than dropped
		// in place. A native function has no worklist of its own to do that
		// over, so it gets one which is empty - which costs nothing until it is
		// handed a value made of other values.
		if let Some(old) = self.inner.try_insert(k, v.to_value()?)? {
			Worklist::new().dismantle(old);
		}

		Ok(())
	}

	/// Inserts a key-value pair into the map.
	///
	/// If the map did not have this key present, `None` is returned.
	#[inline]
	pub fn insert(&mut self, k: String, v: Value) -> alloc::Result<Option<Value>> {
		self.inner.try_insert(k, v)
	}

	/// Take every value in the object apart rather than dropping them where
	/// they are - see [`Worklist::dismantle`].
	pub(crate) fn dismantle(&mut self, work: &mut Worklist) {
		work.dismantle_all(self.inner.iter_mut().map(|(_, value)| value));
		self.inner.clear();
	}

	/// Clears the object, removing all key-value pairs. Keeps the allocated
	/// memory for reuse.
	#[inline]
	pub fn clear(&mut self) {
		self.inner.clear();
	}

	/// Clears the object, removing all key-value pairs. Keeps the allocated
	/// memory for reuse.
	///
	/// The values are taken apart rather than dropped where they are - see
	/// [`Object::dismantle`].
	#[inline]
	#[rune::function(keep, path = Self::clear)]
	pub(crate) fn rune_clear(&mut self) {
		self.dismantle(&mut Worklist::new());
	}

	/// An iterator visiting all key-value pairs in insertion order.
	/// The iterator element type is `(&'a String, &'a Value)`.
	pub fn iter(&self) -> Iter<'_> {
		Iter {
			inner: self.inner.entries().iter(),
		}
	}

	/// An iterator visiting all keys in insertion order.
	/// The iterator element type is `&'a String`.
	pub fn keys(&self) -> Keys<'_> {
		Keys {
			inner: self.inner.entries().iter(),
		}
	}

	/// An iterator visiting all values in insertion order.
	/// The iterator element type is `&'a Value`.
	pub fn values(&self) -> Values<'_> {
		Values {
			inner: self.inner.entries().iter(),
		}
	}

	/// An iterator visiting all key-value pairs in insertion order, with
	/// mutable references to the values.
	///
	/// The iterator element type is `(&'a String, &'a mut Value)`.
	pub fn iter_mut(&mut self) -> IterMut<'_> {
		IterMut {
			inner: self.inner.iter_mut(),
		}
	}

	/// An iterator visiting all keys and values in insertion order.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = #{a: 1, b: 2, c: 3};
	/// let vec = [];
	///
	/// for key in object.iter() {
	///     vec.push(key);
	/// }
	///
	/// vec.sort_by(|a, b| a.0.cmp(b.0));
	/// assert_eq!(vec, [("a", 1), ("b", 2), ("c", 3)]);
	/// ```
	#[rune::function(keep, path = Self::iter)]
	pub fn rune_iter(this: Ref<Self>) -> RuneIter {
		// SAFETY: we're holding onto the related reference guard, and making
		// sure that it's dropped after the iterator.
		let (object, guard) = Ref::into_raw(this);
		RuneIter {
			cursor: Cursor { object, next: 0 },
			guard,
		}
	}

	/// An iterator visiting all keys in insertion order.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = #{a: 1, b: 2, c: 3};
	/// let vec = [];
	///
	/// for key in object.keys() {
	///     vec.push(key);
	/// }
	///
	/// vec.sort_by(|a, b| a.cmp(b));
	/// assert_eq!(vec, ["a", "b", "c"]);
	/// ```
	#[rune::function(keep, path = Self::keys)]
	pub fn rune_keys(this: Ref<Self>) -> RuneIterKeys {
		// SAFETY: we're holding onto the related reference guard, and making
		// sure that it's dropped after the iterator.
		let (object, guard) = Ref::into_raw(this);
		RuneIterKeys {
			cursor: Cursor { object, next: 0 },
			guard,
		}
	}

	/// An iterator visiting all values in insertion order.
	///
	/// # Examples
	///
	/// ```rune
	/// let object = #{a: 1, b: 2, c: 3};
	/// let vec = [];
	///
	/// for key in object.values() {
	///     vec.push(key);
	/// }
	///
	/// vec.sort_by(|a, b| a.cmp(b));
	/// assert_eq!(vec, [1, 2, 3]);
	/// ```
	#[rune::function(keep, path = Self::values)]
	pub fn rune_values(this: Ref<Self>) -> RuneValues {
		// SAFETY: we're holding onto the related reference guard, and making
		// sure that it's dropped after the iterator.
		let (object, guard) = Ref::into_raw(this);
		RuneValues {
			cursor: Cursor { object, next: 0 },
			guard,
		}
	}

	pub(crate) fn partial_eq_with(
		a: &Self,
		b: &Self,
		caller: &mut dyn ProtocolCaller,
	) -> Result<bool, VmError> {
		if a.len() != b.len() {
			return Ok(false);
		}

		for (k1, v1) in a.iter() {
			let Some(v2) = b.get(k1) else {
				return Ok(false);
			};

			if !Value::partial_eq_with(v1, v2, caller)? {
				return Ok(false);
			}
		}

		Ok(true)
	}

	pub(crate) fn eq_with(
		a: &Self,
		b: &Self,
		eq: fn(&Value, &Value, &mut dyn ProtocolCaller) -> Result<bool, VmError>,
		caller: &mut dyn ProtocolCaller,
	) -> Result<bool, VmError> {
		if a.inner.len() != b.inner.len() {
			return Ok(false);
		}

		for (key, a) in a.iter() {
			let Some(b) = b.inner.get(key) else {
				return Ok(false);
			};

			if !eq(a, b, caller)? {
				return Ok(false);
			}
		}

		Ok(true)
	}
}

impl TryClone for Object {
	fn try_clone(&self) -> alloc::Result<Self> {
		Ok(Self {
			inner: self.inner.try_clone()?,
		})
	}
}

impl<'a> IntoIterator for &'a Object {
	type Item = (&'a String, &'a Value);
	type IntoIter = Iter<'a>;

	fn into_iter(self) -> Self::IntoIter {
		self.iter()
	}
}

impl<'a> IntoIterator for &'a mut Object {
	type Item = (&'a String, &'a mut Value);
	type IntoIter = IterMut<'a>;

	fn into_iter(self) -> Self::IntoIter {
		self.iter_mut()
	}
}

impl IntoIterator for Object {
	type Item = (String, Value);
	type IntoIter = IntoIter;

	/// Creates a consuming iterator, that is, one that moves each key-value
	/// pair out of the object in insertion order. The object cannot be used
	/// after calling this.
	fn into_iter(self) -> Self::IntoIter {
		IntoIter {
			inner: self.inner.into_entries().into_iter(),
		}
	}
}

impl fmt::Debug for Object {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut entries = alloc::Vec::try_with_capacity(self.len()).map_err(|_| fmt::Error)?;
		for entry in self.iter() {
			entries.try_push(entry).map_err(|_| fmt::Error)?;
		}
		entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
		f.debug_map().entries(entries).finish()
	}
}

#[derive(Any)]
#[rune(item = ::std::object, name = Iter, dismantle)]
pub struct RuneIter {
	cursor: Cursor,
	guard: RawAnyGuard,
}

/// The object is kept alive through a guard rather than through a slot, so
/// taking the iterator apart releases the guard and hands the object over.
impl Dismantle for RuneIter {
	fn dismantle(&mut self, out: &mut Handover<'_>) {
		out.consume_ref(&mut self.guard);
	}
}

impl RuneIter {
	#[rune::function(instance, keep, protocol = NEXT)]
	pub fn next(&mut self) -> Result<Option<(String, Value)>, VmError> {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let Some((key, value)) = (unsafe { self.cursor.next() }) else {
			return Ok(None);
		};

		Ok(Some((key.try_clone()?, value.clone())))
	}

	#[rune::function(instance, keep, protocol = SIZE_HINT)]
	pub fn size_hint(&self) -> (usize, Option<usize>) {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let remaining = unsafe { self.cursor.remaining() };
		(remaining, Some(remaining))
	}

	#[rune::function(instance, keep, protocol = LEN)]
	pub fn len(&self) -> usize {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		unsafe { self.cursor.remaining() }
	}
}

impl iter::Iterator for RuneIter {
	type Item = Result<(String, Value), VmError>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		match RuneIter::next(self) {
			Ok(Some(value)) => Some(Ok(value)),
			Ok(None) => None,
			Err(err) => Some(Err(err)),
		}
	}
}

#[derive(Any)]
#[rune(item = ::std::object, name = Keys, dismantle)]
pub struct RuneIterKeys {
	cursor: Cursor,
	guard: RawAnyGuard,
}

/// The object is kept alive through a guard rather than through a slot, so
/// taking the iterator apart releases the guard and hands the object over.
impl Dismantle for RuneIterKeys {
	fn dismantle(&mut self, out: &mut Handover<'_>) {
		out.consume_ref(&mut self.guard);
	}
}

impl RuneIterKeys {
	#[rune::function(instance, keep, protocol = NEXT)]
	pub fn next(&mut self) -> Result<Option<String>, VmError> {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let Some((key, _)) = (unsafe { self.cursor.next() }) else {
			return Ok(None);
		};

		Ok(Some(key.try_clone()?))
	}

	#[rune::function(instance, keep, protocol = SIZE_HINT)]
	pub fn size_hint(&self) -> (usize, Option<usize>) {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let remaining = unsafe { self.cursor.remaining() };
		(remaining, Some(remaining))
	}

	#[rune::function(instance, keep, protocol = LEN)]
	pub fn len(&self) -> usize {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		unsafe { self.cursor.remaining() }
	}
}

impl iter::Iterator for RuneIterKeys {
	type Item = Result<String, VmError>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		match RuneIterKeys::next(self) {
			Ok(Some(value)) => Some(Ok(value)),
			Ok(None) => None,
			Err(err) => Some(Err(err)),
		}
	}
}

#[derive(Any)]
#[rune(item = ::std::object, name = Values, dismantle)]
pub struct RuneValues {
	cursor: Cursor,
	guard: RawAnyGuard,
}

/// The object is kept alive through a guard rather than through a slot, so
/// taking the iterator apart releases the guard and hands the object over.
impl Dismantle for RuneValues {
	fn dismantle(&mut self, out: &mut Handover<'_>) {
		out.consume_ref(&mut self.guard);
	}
}

impl RuneValues {
	#[rune::function(instance, keep, protocol = NEXT)]
	pub fn next(&mut self) -> Result<Option<Value>, VmError> {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let Some((_, value)) = (unsafe { self.cursor.next() }) else {
			return Ok(None);
		};

		Ok(Some(value.clone()))
	}

	#[rune::function(instance, keep, protocol = SIZE_HINT)]
	pub fn size_hint(&self) -> (usize, Option<usize>) {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		let remaining = unsafe { self.cursor.remaining() };
		(remaining, Some(remaining))
	}

	#[rune::function(instance, keep, protocol = LEN)]
	pub fn len(&self) -> usize {
		// SAFETY: the guard keeps the object alive and shared-borrowed.
		unsafe { self.cursor.remaining() }
	}
}

impl iter::Iterator for RuneValues {
	type Item = Result<Value, VmError>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		match RuneValues::next(self) {
			Ok(Some(value)) => Some(Ok(value)),
			Ok(None) => None,
			Err(err) => Some(Err(err)),
		}
	}
}

/// The position of a script iterator in an object it holds a shared guard on.
struct Cursor {
	object: NonNull<Object>,
	next: usize,
}

impl Cursor {
	/// # Safety
	///
	/// The object must be alive and not mutably borrowed, which the guard held
	/// next to the cursor ensures.
	unsafe fn next<'a>(&mut self) -> Option<&'a (String, Value)> {
		let entry = self.object.as_ref().inner.get_index(self.next)?;
		self.next += 1;
		Some(entry)
	}

	/// # Safety
	///
	/// As for [`Cursor::next`].
	unsafe fn remaining(&self) -> usize {
		self.object.as_ref().len() - self.next
	}
}

/// A script `String` argument used as an Object key: the key is borrowed and
/// copied, so every other handle to the String stays readable.
pub(crate) struct CopiedKey(pub(crate) String);

impl FromValue for CopiedKey {
	#[inline]
	fn from_value(value: Value) -> Result<Self, RuntimeError> {
		match value.as_ref() {
			Repr::Any(any) => Ok(Self(any.borrow_ref::<String>()?.try_clone()?)),
			actual => Err(RuntimeError::expected::<String>(actual.type_info())),
		}
	}
}

impl MaybeTypeOf for CopiedKey {
	#[inline]
	fn maybe_type_of() -> alloc::Result<crate::compile::meta::DocType> {
		String::maybe_type_of()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key(s: &str) -> String {
		String::try_from(s).unwrap()
	}

	fn object(keys: &[&str]) -> Object {
		let mut object = Object::new();

		for (i, k) in keys.iter().enumerate() {
			object.insert(key(k), Value::from(i as i64)).unwrap();
		}

		object
	}

	fn keys(object: &Object) -> std::vec::Vec<&str> {
		object.keys().map(|k| k.as_str()).collect()
	}

	#[test]
	fn rust_iterators_follow_insertion_order() {
		let mut object = object(&["c", "a", "b"]);
		assert_eq!(keys(&object), ["c", "a", "b"]);
		assert_eq!(object.iter().len(), 3);
		assert_eq!(object.keys().next_back().unwrap().as_str(), "b");
		assert!(object.remove("c").is_some());
		object.insert(key("c"), Value::from(9i64)).unwrap();
		assert_eq!(keys(&object), ["a", "b", "c"]);

		for (_, value) in object.iter_mut() {
			*value = Value::from(7i64);
		}

		let values: std::vec::Vec<i64> = object
			.values()
			.map(|v| v.as_integer::<i64>().unwrap())
			.collect();
		assert_eq!(values, [7, 7, 7]);
		let copy = object.try_clone().unwrap();
		assert_eq!(keys(&copy), ["a", "b", "c"]);
		let owned: std::vec::Vec<std::string::String> =
			copy.into_iter().map(|(k, _)| k.as_str().into()).collect();
		assert_eq!(owned, ["a", "b", "c"]);
	}

	#[test]
	fn script_iterators_follow_insertion_order_under_the_guard() {
		let value = Value::new(object(&["z", "y", "x"])).unwrap();
		let mut iter = Object::rune_iter(value.clone().into_ref::<Object>().unwrap());
		assert_eq!(iter.len(), 3);
		let (k, _) = iter.next().unwrap().unwrap();
		assert_eq!(k.as_str(), "z");
		assert_eq!(iter.size_hint(), (2, Some(2)));
		// The guard keeps the object shared while the iterator lives.
		assert!(value.borrow_mut::<Object>().is_err());
		let keys = Object::rune_keys(value.clone().into_ref::<Object>().unwrap())
			.map(|k| std::string::String::from(k.unwrap().as_str()))
			.collect::<std::vec::Vec<_>>();
		assert_eq!(keys, ["z", "y", "x"]);
		let values = Object::rune_values(value.clone().into_ref::<Object>().unwrap())
			.map(|v| v.unwrap().as_integer::<i64>().unwrap())
			.collect::<std::vec::Vec<_>>();
		assert_eq!(values, [0, 1, 2]);
		assert_eq!(
			iter.map(|e| std::string::String::from(e.unwrap().0.as_str()))
				.collect::<std::vec::Vec<_>>(),
			["y", "x"]
		);
		assert!(value.borrow_mut::<Object>().is_ok());
	}
}
