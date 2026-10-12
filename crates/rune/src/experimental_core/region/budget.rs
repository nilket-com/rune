//! Resource accounting for the native tier (P3a). Safe Rust only, no Cranelift, no file
//! system: everything here runs under Miri ("safe glue only").
//!
//! - [`Budget`] and [`BVec`]: a bound on the requested bytes of the region ANALYSIS
//!   buffers alive at once. Every analysis buffer is a `BVec`, which charges the exact
//!   capacity it is about to request *before* it allocates and releases the charge only
//!   after the buffer is freed. During growth the old buffer is still charged when the new
//!   one is charged, so a reallocation counts both.
//!
//!   The pre-allocation guarantee rests on one pinned invariant of `rune_alloc`: an exact
//!   reservation (`Vec::try_reserve_exact`, `RawVec::grow_exact`) that succeeds sets the
//!   capacity to exactly `len + additional`, whatever block the allocator returned
//!   (`rune_alloc`'s `RawVec` growth path). `BVec` checks that invariant
//!   after every growth. Should it ever fail, the check is AFTER the allocation (it cannot
//!   be anything else): the oversized buffer is freed at once, every charge of this vector
//!   is released, the failure is counted ([`Budget::violations`]) and the growth returns
//!   [`Grow::Invariant`], an analysis error (interpreter fallback). It is never reconciled
//!   by a silent extra charge.
//!
//!   It counts requested bytes only: allocator headers, rounding and fragmentation are not
//!   included, and Cranelift's compilation heap and the compiler/audit temporaries are
//!   outside it (Cranelift allocates infallibly and is not bounded by this budget).
//!
//!   Element contract: a `BVec` holds only [`AnalysisPod`] types: `Copy` analysis data
//!   (no `Drop`, no owned heap memory, a bitwise `try_clone`), so the only allocation a
//!   `BVec` makes is its own buffer. This is a contract for the listed types, not a bound
//!   for an arbitrary `T`.
//! - [`Pool`] and [`Token`]: a process-wide reservation of native code mappings, by count
//!   and by requested mapping bytes, held as an RAII token by whoever owns the mapping.
//!   A POLICY bound on our own allocation count and requested page bytes, not a guarantee
//!   on physical footprint or the kernel's VMA count.
use crate::alloc::clone::TryClone;
use crate::alloc::Vec;
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Why a budgeted buffer could not grow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grow {
	/// The allocator refused (the bytes stay uncharged).
	Alloc,
	/// The budget would be exceeded, or the byte count overflows (nothing was allocated).
	Budget,
	/// The allocated capacity differs from the exact request (the pinned `rune_alloc`
	/// invariant failed). The buffer was freed and its charges released; an analysis error.
	Invariant,
}

/// Element types a [`BVec`] may hold: allocation-free analysis data. `Copy` rules out a
/// `Drop` impl and owned heap memory; every implementor's `try_clone` (where it has one) is
/// the bitwise copy. Implemented only for the types the region analysis uses.
pub(crate) trait AnalysisPod: Copy {}

macro_rules! analysis_pod {
	($($t:ty),* $(,)?) => {$(
		impl AnalysisPod for $t {}
	)*};
}

analysis_pod!(bool, u32, u64, usize, (u32, u32), Option<u32>);

/// A transient-bytes budget, checked before every charged allocation.
pub(crate) struct Budget {
	limit: usize,
	used: Cell<usize>,
	peak: Cell<usize>,
	/// Exact-capacity invariant failures seen (see [`Grow::Invariant`]).
	violations: Cell<u32>,
}

impl Budget {
	pub(crate) const fn new(limit: usize) -> Self {
		Self {
			limit,
			used: Cell::new(0),
			peak: Cell::new(0),
			violations: Cell::new(0),
		}
	}

	/// No limit (tests and the test-only listing path); still counts.
	pub(crate) const fn unlimited() -> Self {
		Self::new(usize::MAX)
	}

	pub(crate) fn used(&self) -> usize {
		self.used.get()
	}

	/// The most bytes charged at once since creation.
	pub(crate) fn peak(&self) -> usize {
		self.peak.get()
	}

	/// How many growths failed the exact-capacity invariant (each one an analysis error).
	pub(crate) fn violations(&self) -> u32 {
		self.violations.get()
	}

	/// Charges `bytes`, or refuses without charging anything.
	pub(crate) fn charge(&self, bytes: usize) -> Result<(), Grow> {
		let next = self.used.get().checked_add(bytes).ok_or(Grow::Budget)?;
		if next > self.limit {
			return Err(Grow::Budget);
		}
		self.used.set(next);
		if next > self.peak.get() {
			self.peak.set(next);
		}
		Ok(())
	}

	/// Releases bytes charged earlier.
	pub(crate) fn release(&self, bytes: usize) {
		let used = self.used.get();
		debug_assert!(bytes <= used, "budget release {bytes} > used {used}");
		self.used.set(used.saturating_sub(bytes));
	}
}

/// Bytes charged for as long as this value lives: the internal ownership handoff of
/// [`BVec::into_parts`]. Whoever holds it keeps the backing buffers it stands for alive no
/// longer than itself (drop the buffers first, then the `Held`); buffers that are retained
/// beyond it (a loaded region's slots, entry classes and exits) are charged separately, to
/// the program and thread metadata bounds.
pub(crate) struct Held<'b> {
	budget: &'b Budget,
	bytes: usize,
}

impl<'b> Held<'b> {
	/// Takes over `bytes` already charged to `budget`.
	pub(crate) fn adopt(budget: &'b Budget, bytes: usize) -> Self {
		Self { budget, bytes }
	}

	pub(crate) fn bytes(&self) -> usize {
		self.bytes
	}
}

impl Drop for Held<'_> {
	fn drop(&mut self) {
		self.budget.release(self.bytes);
	}
}

/// The capacity std and rune_alloc choose for an amortized push into a full vector:
/// `max(MIN_NON_ZERO_CAP, 2 * cap, cap + 1)`. Used so that buffers which outlive the
/// analysis keep exactly the capacities they had before P3a.
fn amortized<T>(cap: usize) -> Option<usize> {
	let min = if core::mem::size_of::<T>() == 1 {
		8
	} else if core::mem::size_of::<T>() <= 1024 {
		4
	} else {
		1
	};
	let doubled = cap.checked_mul(2)?;
	let plus = cap.checked_add(1)?;
	Some(min.max(doubled).max(plus))
}

/// A vector whose whole capacity is charged to a [`Budget`] before it is allocated (see
/// the module documentation for the invariant this relies on and the element contract).
pub(crate) struct BVec<'b, T: AnalysisPod> {
	vec: Vec<T>,
	budget: &'b Budget,
	/// Bytes charged for `vec`'s capacity: always `vec.capacity() * size_of::<T>()`.
	charged: usize,
}

impl<'b, T: AnalysisPod> BVec<'b, T> {
	pub(crate) fn new(budget: &'b Budget) -> Self {
		const {
			assert!(
				core::mem::size_of::<T>() != 0,
				"BVec elements are not zero-sized"
			);
		}

		Self {
			vec: Vec::new(),
			budget,
			charged: 0,
		}
	}

	/// An empty vector with exactly `capacity` charged and allocated.
	pub(crate) fn with_capacity(budget: &'b Budget, capacity: usize) -> Result<Self, Grow> {
		let mut v = Self::new(budget);
		if capacity > 0 {
			v.grow_to(capacity)?;
		}
		Ok(v)
	}

	/// Grows the capacity to exactly `capacity` (> current): the new size is charged while
	/// the old one is still charged (both are live during the reallocation).
	fn grow_to(&mut self, capacity: usize) -> Result<(), Grow> {
		debug_assert!(capacity > self.vec.capacity());
		let bytes = capacity
			.checked_mul(core::mem::size_of::<T>())
			.ok_or(Grow::Budget)?;
		self.budget.charge(bytes)?;
		let additional = capacity - self.vec.len();

		#[cfg(test)]
		let additional = additional + take_injected_extra_capacity();

		if self.vec.try_reserve_exact(additional).is_err() {
			self.budget.release(bytes);
			return Err(Grow::Alloc);
		}

		// The pinned rune_alloc invariant: a successful exact reservation leaves the capacity
		// at exactly the request. Checked in every build; a violation is an explicit error,
		// never a silent extra charge. The check is necessarily after the allocation, so the
		// oversized buffer is freed at once, then both charges are released.
		if self.vec.capacity() != capacity {
			self.vec = Vec::new();
			self.budget.release(bytes);
			self.budget
				.release(core::mem::replace(&mut self.charged, 0));
			self.budget
				.violations
				.set(self.budget.violations.get().saturating_add(1));
			return Err(Grow::Invariant);
		}

		self.budget.release(self.charged);
		self.charged = bytes;
		Ok(())
	}

	/// Appends, growing like an ordinary amortized push.
	pub(crate) fn push(&mut self, value: T) -> Result<(), Grow> {
		if self.vec.len() == self.vec.capacity() {
			let capacity = amortized::<T>(self.vec.capacity()).ok_or(Grow::Budget)?;
			self.grow_to(capacity)?;
		}
		self.vec.try_push(value).map_err(|_| Grow::Alloc)
	}

	/// Resizes to `len` elements, growing to exactly `len` if needed.
	pub(crate) fn resize(&mut self, len: usize, value: T) -> Result<(), Grow>
	where
		T: TryClone,
	{
		if len > self.vec.capacity() {
			self.grow_to(len)?;
		}
		self.vec.try_resize(len, value).map_err(|_| Grow::Alloc)
	}

	pub(crate) fn pop(&mut self) -> Option<T> {
		self.vec.pop()
	}

	/// The bytes charged for this vector.
	pub(crate) fn charged(&self) -> usize {
		self.charged
	}

	/// The vector's capacity in elements (tests).
	#[cfg(test)]
	pub(crate) fn capacity(&self) -> usize {
		self.vec.capacity()
	}

	/// The vector and its charge, which the caller now holds (see [`Held::adopt`]): the
	/// charge must stay held for as long as the returned buffer lives.
	pub(crate) fn into_parts(mut self) -> (Vec<T>, usize) {
		let charged = core::mem::replace(&mut self.charged, 0);
		(core::mem::take(&mut self.vec), charged)
	}
}

impl<T: AnalysisPod> core::ops::Deref for BVec<'_, T> {
	type Target = [T];

	fn deref(&self) -> &[T] {
		&self.vec
	}
}

impl<T: AnalysisPod> core::ops::DerefMut for BVec<'_, T> {
	fn deref_mut(&mut self) -> &mut [T] {
		&mut self.vec
	}
}

impl<T: AnalysisPod> Drop for BVec<'_, T> {
	fn drop(&mut self) {
		// Free the buffer first, then release its charge.
		drop(core::mem::take(&mut self.vec));
		self.budget.release(self.charged);
	}
}

#[cfg(test)]
std::thread_local! {
	/// Extra elements the next exact growth requests beyond its charge: simulates a
	/// violated exact-capacity invariant (tests only).
	static INJECT_EXTRA_CAPACITY: Cell<usize> = const { Cell::new(0) };
}

/// Makes the next exact growth on this thread allocate `extra` more elements than it
/// charged, as a violated exact-capacity invariant would (tests only).
#[cfg(test)]
pub(crate) fn inject_extra_capacity_for_test(extra: usize) {
	INJECT_EXTRA_CAPACITY.with(|c| c.set(extra));
}

#[cfg(test)]
fn take_injected_extra_capacity() -> usize {
	INJECT_EXTRA_CAPACITY.with(|c| c.replace(0))
}

// ------------------------------------------------------------ process reservations

/// A process-wide pool of native code mappings: how many are reserved and how many
/// requested mapping bytes they hold. Two independent atomics; a reservation takes both or
/// neither.
pub(crate) struct Pool {
	count: AtomicUsize,
	bytes: AtomicUsize,
}

impl Pool {
	pub(crate) const fn new() -> Self {
		Self {
			count: AtomicUsize::new(0),
			bytes: AtomicUsize::new(0),
		}
	}

	/// Reserves one mapping of `bytes`, or `None` when either bound would be exceeded (or
	/// a sum overflows). On a bytes refusal the count taken first is rolled back, so a
	/// refused reservation holds nothing. Another thread may observe the count briefly
	/// raised by a reservation that is then rolled back; it is then refused spuriously,
	/// never admitted past a bound.
	pub(crate) fn reserve(
		&self,
		bytes: usize,
		max_count: usize,
		max_bytes: usize,
	) -> Option<Token<'_>> {
		if !add_within(&self.count, 1, max_count) {
			return None;
		}

		if !add_within(&self.bytes, bytes, max_bytes) {
			sub(&self.count, 1);
			return None;
		}

		Some(Token { pool: self, bytes })
	}

	/// `(mappings, bytes)` currently reserved.
	pub(crate) fn current(&self) -> (usize, usize) {
		(
			self.count.load(Ordering::Acquire),
			self.bytes.load(Ordering::Acquire),
		)
	}
}

/// Adds `n` to `atomic` unless the sum overflows or exceeds `max` (a compare-exchange loop).
fn add_within(atomic: &AtomicUsize, n: usize, max: usize) -> bool {
	let mut current = atomic.load(Ordering::Acquire);
	loop {
		let Some(next) = current.checked_add(n).filter(|&next| next <= max) else {
			return false;
		};
		match atomic.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
			Ok(_) => return true,
			Err(actual) => current = actual,
		}
	}
}

/// Subtracts `n` from `atomic`, never below zero (an underflow is a bookkeeping bug).
fn sub(atomic: &AtomicUsize, n: usize) {
	let mut current = atomic.load(Ordering::Acquire);
	loop {
		debug_assert!(current >= n, "pool underflow: {current} - {n}");
		let next = current.saturating_sub(n);
		match atomic.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
			Ok(_) => return,
			Err(actual) => current = actual,
		}
	}
}

/// One reserved mapping. Released exactly once, when dropped. The owner keeps it for as
/// long as the mapping exists; a mapping that is leaked keeps its token leaked too
/// (`core::mem::forget`), so the pool never undercounts.
pub(crate) struct Token<'p> {
	pool: &'p Pool,
	bytes: usize,
}

impl Token<'_> {
	pub(crate) fn bytes(&self) -> usize {
		self.bytes
	}
}

impl Drop for Token<'_> {
	fn drop(&mut self) {
		sub(&self.pool.bytes, self.bytes);
		sub(&self.pool.count, 1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn budget_charges_before_allocating_and_counts_old_and_new_during_growth() {
		let budget = Budget::new(1024);
		{
			let mut v: BVec<'_, u64> = BVec::new(&budget);
			for i in 0..4 {
				v.push(i).unwrap();
			}
			assert_eq!((v.charged(), budget.used()), (32, 32));
			// Growth 4 -> 8: 32 old + 64 new were charged together.
			v.push(4).unwrap();
			assert_eq!((v.charged(), budget.used(), budget.peak()), (64, 64, 96));
		}
		assert_eq!(budget.used(), 0, "released on drop");
	}

	#[test]
	fn budget_refuses_without_allocating_and_keeps_contents() {
		// k - 1, k, k + 1 around a 64-byte limit with u64 elements.
		for (limit, ok) in [(63usize, false), (64, true), (65, true)] {
			let budget = Budget::new(limit);
			let r = BVec::<u64>::with_capacity(&budget, 8);
			assert_eq!(r.is_ok(), ok, "limit {limit}");
			drop(r);
			assert_eq!(budget.used(), 0);
		}

		// Growth that would exceed: refused, nothing allocated, old buffer intact.
		let budget = Budget::new(32 + 63);
		let mut v: BVec<'_, u64> = BVec::new(&budget);
		for i in 0..4 {
			v.push(i).unwrap();
		}
		assert_eq!(v.push(9), Err(Grow::Budget));
		assert_eq!(&v[..], &[0, 1, 2, 3]);
		assert_eq!(budget.used(), 32);
	}

	#[test]
	fn budget_overflow_and_zero_sizes() {
		let budget = Budget::unlimited();
		assert_eq!(
			BVec::<u64>::with_capacity(&budget, usize::MAX / 4).err(),
			Some(Grow::Budget),
			"byte count overflow refuses"
		);
		let v = BVec::<u64>::with_capacity(&budget, 0).unwrap();
		assert_eq!(v.charged(), 0);
		budget.charge(usize::MAX - 1).unwrap();
		assert_eq!(budget.charge(2), Err(Grow::Budget), "checked add");
		budget.release(usize::MAX - 1);
		assert_eq!(budget.used(), 0);
	}

	#[test]
	fn into_parts_hands_the_charge_over() {
		let budget = Budget::new(100);
		let mut v: BVec<'_, u32> = BVec::new(&budget);
		v.resize(5, 7).unwrap();
		let (inner, bytes) = v.into_parts();
		assert_eq!(
			(inner.len(), inner.capacity(), bytes, budget.used()),
			(5, 5, 20, 20)
		);
		let held = Held::adopt(&budget, bytes);
		assert_eq!(held.bytes(), 20);
		drop(held);
		assert_eq!(budget.used(), 0);
	}

	/// The accounting invariant: after every operation each vector's charge is exactly its
	/// capacity in bytes and the budget holds exactly the sum of the live charges; a growth
	/// whose real capacity differs from the request (injected) fails explicitly, frees the
	/// buffer and leaves nothing charged for it.
	#[test]
	fn accounting_invariant_exact_capacity_and_violation_fails_explicitly() {
		fn exact<T: AnalysisPod>(v: &BVec<'_, T>) -> bool {
			v.charged() == v.capacity() * core::mem::size_of::<T>()
		}

		let budget = Budget::new(4096);
		let mut a: BVec<'_, u64> = BVec::new(&budget);
		let mut b: BVec<'_, bool> = BVec::with_capacity(&budget, 3).unwrap();
		let mut c: BVec<'_, (u32, u32)> = BVec::new(&budget);
		for i in 0..40u64 {
			a.push(i).unwrap();
			b.push(i % 2 == 0).unwrap();
			c.resize(i as usize + 1, (1, 2)).unwrap();
			assert!(exact(&a) && exact(&b) && exact(&c), "step {i}");
			assert_eq!(
				budget.used(),
				a.charged() + b.charged() + c.charged(),
				"step {i}"
			);
		}

		// A refused growth changes nothing.
		let used = budget.used();
		let tight = Budget::new(used);
		let mut d: BVec<'_, u32> = BVec::new(&tight);
		assert_eq!(d.resize(used / 4 + 1, 0), Err(Grow::Budget));
		assert_eq!((d.charged(), d.capacity(), tight.used()), (0, 0, 0));

		// Injected violation on a growth of a non-empty vector: explicit error, buffer
		// freed, both the old and the new charge released, the violation counted.
		let before = budget.used() - a.charged();
		inject_extra_capacity_for_test(5);
		let capacity = a.capacity();
		while a.len() < capacity {
			a.push(0).unwrap();
		}
		assert_eq!(a.push(1), Err(Grow::Invariant));
		inject_extra_capacity_for_test(0);
		assert_eq!((a.len(), a.capacity(), a.charged()), (0, 0, 0));
		assert_eq!(budget.used(), before);
		assert_eq!(budget.violations(), 1);

		// The same through `with_capacity` (nothing charged before).
		inject_extra_capacity_for_test(1);
		assert_eq!(
			BVec::<u32>::with_capacity(&budget, 7).err(),
			Some(Grow::Invariant)
		);
		assert_eq!((budget.used(), budget.violations()), (before, 2));

		// The vector stays usable and exact afterwards.
		a.push(3).unwrap();
		assert!(exact(&a));
		drop((a, b, c));
		assert_eq!(budget.used(), 0);
	}

	#[test]
	fn amortized_matches_vec_growth() {
		let mut v: Vec<u64> = Vec::new();
		let mut cap = 0;
		for i in 0..100u64 {
			if v.len() == v.capacity() {
				cap = amortized::<u64>(cap).unwrap();
			}
			v.try_push(i).unwrap();
			assert_eq!(v.capacity(), cap);
		}
		let mut w: Vec<u8> = Vec::new();
		w.try_push(1).unwrap();
		assert_eq!((w.capacity(), amortized::<u8>(0)), (8, Some(8)));
	}

	#[test]
	fn pool_takes_both_or_neither() {
		let pool = Pool::new();
		let a = pool.reserve(4096, 2, 8192).unwrap();
		let b = pool.reserve(4096, 2, 8192).unwrap();
		assert_eq!(pool.current(), (2, 8192));
		assert!(pool.reserve(1, 2, usize::MAX).is_none(), "count bound");
		drop(b);
		assert!(
			pool.reserve(8192, 3, 8192).is_none(),
			"bytes bound rolls the count back"
		);
		assert_eq!(pool.current(), (1, 4096));
		assert!(
			pool.reserve(usize::MAX, 3, usize::MAX).is_none(),
			"checked add"
		);
		assert_eq!(pool.current(), (1, 4096));
		drop(a);
		assert_eq!(pool.current(), (0, 0));
	}

	#[test]
	fn pool_concurrent_reserve_and_drop_returns_to_zero() {
		static POOL: Pool = Pool::new();
		let threads = if cfg!(miri) { 4 } else { 8 };
		let rounds = if cfg!(miri) { 20 } else { 2_000 };
		let handles: std::vec::Vec<_> = (0..threads)
			.map(|t| {
				std::thread::spawn(move || {
					let mut held = std::vec::Vec::new();
					let mut admitted = 0usize;
					for i in 0..rounds {
						if let Some(token) = POOL.reserve(4096 * (1 + (t + i) % 3), 5, 4 * 4096) {
							admitted += 1;
							let (count, bytes) = POOL.current();
							assert!(count <= 5 && bytes <= 4 * 4096);
							held.push(token);
						}
						if held.len() > 1 || i % 3 == 0 {
							held.clear();
						}
					}
					admitted
				})
			})
			.collect();
		let admitted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
		assert!(admitted > 0);
		assert_eq!(POOL.current(), (0, 0));
	}
}
