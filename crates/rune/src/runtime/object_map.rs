//! The insertion-ordered map behind [`Object`](super::Object).

use core::borrow::Borrow;
use core::convert::Infallible;
use core::hash::{BuildHasher, Hash};

use rune_alloc::hashbrown::raw::RawTable;
use rune_alloc::hashbrown::ErrorOrInsertSlot;

use crate::alloc::error::CustomError;
use crate::alloc::hash_map::DefaultHashBuilder;
use crate::alloc::prelude::*;
use crate::alloc::{self, String, Vec};

/// String keys mapped to values, iterated in insertion order.
///
/// Replacing the value of a present key keeps its position, removal shifts
/// the entries behind it forward, and a key inserted again after removal is
/// appended.
///
/// The index holds each position of `entries` exactly once, under the hash
/// of that entry's key; every method restores this before returning.
pub(crate) struct ObjectMap<V> {
	entries: Vec<(String, V)>,
	index: RawTable<usize>,
}

#[inline]
fn hash<Q: ?Sized + Hash>(key: &Q) -> u64 {
	DefaultHashBuilder::default().hash_one(key)
}

#[inline]
fn into_ok<T>(result: Result<T, Infallible>) -> T {
	match result {
		Ok(value) => value,
		Err(never) => match never {},
	}
}

impl<V> ObjectMap<V> {
	pub(crate) const fn new() -> Self {
		Self {
			entries: Vec::new(),
			index: RawTable::new(),
		}
	}

	pub(crate) fn try_with_capacity(capacity: usize) -> alloc::Result<Self> {
		Ok(Self {
			entries: Vec::try_with_capacity(capacity)?,
			index: RawTable::try_with_capacity(capacity)?,
		})
	}

	#[inline]
	pub(crate) fn len(&self) -> usize {
		self.entries.len()
	}

	#[inline]
	pub(crate) fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// The entry stored at a position taken from the index.
	#[inline]
	fn indexed(entries: &[(String, V)], at: usize) -> &(String, V) {
		debug_assert!(at < entries.len());
		// SAFETY: the index only holds positions of `entries`.
		unsafe { entries.get_unchecked(at) }
	}

	#[inline]
	fn position<Q>(&self, key: &Q) -> Option<usize>
	where
		String: Borrow<Q>,
		Q: ?Sized + Hash + Eq,
	{
		let entries = &self.entries;
		let eq = |_: &mut (), at: &usize| {
			Ok::<_, Infallible>(Self::indexed(entries, *at).0.borrow() == key)
		};
		let bucket = into_ok(self.index.find(&mut (), hash(key), eq))?;
		// SAFETY: the bucket was just found in this table and is occupied.
		Some(unsafe { *bucket.as_ref() })
	}

	#[inline]
	pub(crate) fn get<Q>(&self, key: &Q) -> Option<&V>
	where
		String: Borrow<Q>,
		Q: ?Sized + Hash + Eq,
	{
		let at = self.position(key)?;
		Some(&Self::indexed(&self.entries, at).1)
	}

	#[inline]
	pub(crate) fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
	where
		String: Borrow<Q>,
		Q: ?Sized + Hash + Eq,
	{
		let at = self.position(key)?;
		debug_assert!(at < self.entries.len());
		// SAFETY: the index only holds positions of `entries`.
		Some(unsafe { &mut self.entries.get_unchecked_mut(at).1 })
	}

	#[inline]
	pub(crate) fn contains_key<Q>(&self, key: &Q) -> bool
	where
		String: Borrow<Q>,
		Q: ?Sized + Hash + Eq,
	{
		self.position(key).is_some()
	}

	/// The entry at `at` in iteration order.
	#[inline]
	pub(crate) fn get_index(&self, at: usize) -> Option<&(String, V)> {
		self.entries.get(at)
	}

	/// Inserts `value` under `key` and returns the value it replaced. A new key
	/// is appended; a present key keeps its position.
	pub(crate) fn try_insert(&mut self, key: String, value: V) -> alloc::Result<Option<V>> {
		let key_hash = hash(key.as_str());
		let entries = &self.entries;
		let eq = |_: &mut (), at: &usize| Ok::<_, Infallible>(Self::indexed(entries, *at).0 == key);
		let rehash = |_: &mut (), at: &usize| {
			Ok::<_, Infallible>(hash(Self::indexed(entries, *at).0.as_str()))
		};

		let slot = match self
			.index
			.find_or_find_insert_slot(&mut (), key_hash, eq, rehash)
		{
			Ok(bucket) => {
				// SAFETY: the bucket was just found in this table and is occupied.
				let at = unsafe { *bucket.as_ref() };
				return Ok(Some(core::mem::replace(&mut self.entries[at].1, value)));
			}
			Err(ErrorOrInsertSlot::InsertSlot(slot)) => slot,
			Err(ErrorOrInsertSlot::Error(CustomError::Error(error))) => return Err(error),
			Err(ErrorOrInsertSlot::Error(CustomError::Custom(never))) => match never {},
		};

		let at = self.entries.len();
		self.entries.try_push((key, value))?;
		// SAFETY: the slot came from the probe above and the table has not been
		// touched since; pushing an entry only changes `entries`.
		unsafe {
			self.index.insert_in_slot(key_hash, slot, at);
		}

		Ok(None)
	}

	/// Removes `key`, shifting every later entry forward by one. This is linear
	/// in the number of entries, like `IndexMap::shift_remove`.
	pub(crate) fn remove<Q>(&mut self, key: &Q) -> Option<V>
	where
		String: Borrow<Q>,
		Q: ?Sized + Hash + Eq,
	{
		let entries = &self.entries;
		let eq = |_: &mut (), at: &usize| {
			Ok::<_, Infallible>(Self::indexed(entries, *at).0.borrow() == key)
		};
		let at = into_ok(self.index.remove_entry(&mut (), hash(key), eq))?;
		let (_, value) = self.entries.remove(at);

		// SAFETY: the table is not resized while its occupied buckets are walked.
		unsafe {
			for bucket in self.index.iter() {
				let position = bucket.as_mut();

				if *position > at {
					*position -= 1;
				}
			}
		}

		Some(value)
	}

	pub(crate) fn clear(&mut self) {
		self.index.clear();
		self.entries.clear();
	}

	/// Moves every entry out in iteration order, leaving the map empty.
	pub(crate) fn take_entries(&mut self) -> Vec<(String, V)> {
		self.index.clear();
		core::mem::take(&mut self.entries)
	}

	#[inline]
	pub(crate) fn entries(&self) -> &[(String, V)] {
		&self.entries
	}

	/// Entries in insertion order with mutable values; keys stay immutable so
	/// the index cannot go stale.
	#[inline]
	pub(crate) fn iter_mut(&mut self) -> IterMut<'_, V> {
		IterMut(self.entries.iter_mut())
	}

	pub(crate) fn into_entries(self) -> Vec<(String, V)> {
		self.entries
	}

	/// Asserts the index invariant. Test-only: it costs a lookup per entry.
	#[cfg(test)]
	pub(crate) fn check(&self) {
		assert_eq!(self.index.len(), self.entries.len());

		for (at, (key, _)) in self.entries.iter().enumerate() {
			assert_eq!(self.position(key.as_str()), Some(at));
		}
	}
}

pub(crate) struct IterMut<'a, V>(core::slice::IterMut<'a, (String, V)>);

impl<'a, V> Iterator for IterMut<'a, V> {
	type Item = (&'a String, &'a mut V);

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		let (key, value) = self.0.next()?;
		Some((key, value))
	}

	#[inline]
	fn size_hint(&self) -> (usize, Option<usize>) {
		self.0.size_hint()
	}
}

impl<V> DoubleEndedIterator for IterMut<'_, V> {
	#[inline]
	fn next_back(&mut self) -> Option<Self::Item> {
		let (key, value) = self.0.next_back()?;
		Some((key, value))
	}
}

impl<V> ExactSizeIterator for IterMut<'_, V> {}

impl<V> Default for ObjectMap<V> {
	#[inline]
	fn default() -> Self {
		Self::new()
	}
}

impl<V: TryClone> TryClone for ObjectMap<V> {
	fn try_clone(&self) -> alloc::Result<Self> {
		Ok(Self {
			entries: self.entries.try_clone()?,
			index: self.index.try_clone()?,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key(s: &str) -> String {
		String::try_from(s).unwrap()
	}

	fn keys(map: &ObjectMap<i64>) -> std::vec::Vec<(&str, i64)> {
		map.entries()
			.iter()
			.map(|(k, v)| (k.as_str(), *v))
			.collect()
	}

	#[test]
	fn insertion_order_replace_remove_reinsert() {
		let mut map = ObjectMap::new();

		for (i, k) in ["c", "a", "b", "d"].into_iter().enumerate() {
			assert_eq!(map.try_insert(key(k), i as i64).unwrap(), None);
			map.check();
		}

		assert_eq!(keys(&map), [("c", 0), ("a", 1), ("b", 2), ("d", 3)]);
		assert_eq!(map.try_insert(key("a"), 9).unwrap(), Some(1));
		assert_eq!(keys(&map), [("c", 0), ("a", 9), ("b", 2), ("d", 3)]);
		assert_eq!(map.remove("a"), Some(9));
		assert_eq!(map.remove("a"), None);
		map.check();
		assert_eq!(keys(&map), [("c", 0), ("b", 2), ("d", 3)]);
		assert_eq!(map.try_insert(key("a"), 5).unwrap(), None);
		map.check();
		assert_eq!(keys(&map), [("c", 0), ("b", 2), ("d", 3), ("a", 5)]);
		assert_eq!(map.get("d"), Some(&3));
		assert!(map.contains_key("b"));
		assert!(!map.contains_key("z"));
		*map.get_mut("b").unwrap() += 1;
		assert_eq!(map.get_index(1), Some(&(key("b"), 3)));
	}

	#[test]
	fn growth_shift_clone_and_clear() {
		let mut map = ObjectMap::try_with_capacity(2).unwrap();

		for i in 0..64i64 {
			map.try_insert(key(&std::format!("k{i}")), i).unwrap();
		}

		for i in (0..64i64).step_by(3) {
			assert_eq!(map.remove(std::format!("k{i}").as_str()), Some(i));
		}

		map.check();
		let order: std::vec::Vec<i64> = map.entries().iter().map(|(_, v)| *v).collect();
		let expected: std::vec::Vec<i64> = (0..64).filter(|i| i % 3 != 0).collect();
		assert_eq!(order, expected);

		let copy = map.try_clone().unwrap();
		copy.check();
		assert_eq!(keys(&copy), keys(&map));

		map.clear();
		map.check();
		assert!(map.is_empty());
		map.try_insert(key("z"), 1).unwrap();
		map.try_insert(key("y"), 2).unwrap();
		assert_eq!(keys(&map), [("z", 1), ("y", 2)]);

		let taken = map.take_entries();
		map.check();
		assert!(map.is_empty());
		assert_eq!(taken.len(), 2);
		assert_eq!(copy.into_entries().len(), 42);
	}
}
