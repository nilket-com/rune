//! Budgeting module for Runestick.
//!
//! This module contains methods which allows for limiting the execution of the
//! virtual machine to abide by the specified budget.
//!
//! By default the budget is disabled, but can be enabled by wrapping your
//! function call in [with].

#[cfg_attr(feature = "std", path = "budget/std.rs")]
mod no_std;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use pin_project::pin_project;
use rune_alloc::callable::Callable;

/// Wrapper for something being [budgeted].
///
/// See [with].
///
/// [budgeted]: self
#[pin_project]
pub struct Budget<T> {
	/// Instruction budget.
	budget: usize,
	/// The thing being budgeted.
	#[pin]
	value: T,
}

/// Wrap the given value with a budget.
///
/// Budgeting is only performed on a per-instruction basis in the virtual
/// machine. What exactly constitutes an instruction might be a bit vague. But
/// important to note is that without explicit co-operation from native
/// functions the budget cannot be enforced. So care must be taken with the
/// native functions that you provide to Rune to ensure that the limits you
/// impose cannot be circumvented.
///
/// The following things can be wrapped:
/// * A [`FnOnce`] closure, like `with(|| println!("Hello World")).call()`.
/// * A [`Future`], like `with(async { /* async work */ }).await`;
///
/// It's also possible to wrap other wrappers which implement [`Callable`].
///
/// # Examples
///
/// ```no_run
/// use rune::runtime::budget;
/// use rune::Vm;
///
/// let mut vm: Vm = todo!();
/// // The virtual machine and any tasks associated with it is only allowed to execute 100 budget.
/// budget::with(100, || vm.call(&["main"], ())).call()?;
/// # Ok::<(), rune::support::Error>(())
/// ```
///
/// This budget can be conveniently combined with the memory [`limit`] module
/// due to both wrappers implementing [`Callable`].
///
/// [`limit`]: crate::alloc::limit
///
/// ```
/// use rune::runtime::budget;
/// use rune::alloc::{limit, Vec};
///
/// #[derive(Debug, PartialEq)]
/// struct Marker;
///
/// // Limit the given closure to run one instruction and allocate 1024 bytes.
/// let f = budget::with(1, limit::with(1024, || {
///     let mut budget = budget::acquire();
///     assert!(budget.take());
///     assert!(!budget.take());
///     assert!(Vec::<u8>::try_with_capacity(1).is_ok());
///     assert!(Vec::<u8>::try_with_capacity(1024).is_ok());
///     assert!(Vec::<u8>::try_with_capacity(1025).is_err());
///     Marker
/// }));
///
/// assert_eq!(f.call(), Marker);
/// ```
pub fn with<T>(budget: usize, value: T) -> Budget<T> {
	tracing::trace!(?budget);
	Budget { budget, value }
}

/// Read the remaining ambient permits, or `None` when budgeting is disabled.
#[inline]
pub fn remaining() -> Option<usize> {
	match self::no_std::rune_budget_get() {
		usize::MAX => None,
		n => Some(n),
	}
}

/// Run a synchronous callable under a hard total and a soft VM slice.
///
/// Only the outer execution yields at an instruction boundary after `slice`
/// permits have been spent. Native calls and nested executions continue drawing
/// from `total` until they return or exhaust it; they cannot be suspended or
/// retried. A native call may therefore delay a yield for the remaining total,
/// and this wrapper does not promise a wall-clock cancellation bound.
/// Read [`remaining`] inside the callable before it returns to carry the hard
/// budget into the next slice. Both ambient settings are restored on unwind.
/// This wrapper deliberately does not implement [`Future`].
///
/// # Panics
/// Panics if `slice` is zero or `total` is the unlimited `usize::MAX` sentinel.
#[cfg(feature = "std")]
pub fn with_slice<T: Callable>(total: usize, slice: usize, value: T) -> Slice<T> {
	assert!(slice > 0, "a VM slice must make progress");
	assert!(total < usize::MAX, "a sliced total must be finite");
	Slice {
		total,
		slice,
		value,
	}
}

/// A synchronous hard budget with a soft yield point. See [`with_slice`].
#[cfg(feature = "std")]
pub struct Slice<T> {
	total: usize,
	slice: usize,
	value: T,
}

#[cfg(feature = "std")]
impl<T: Callable> Slice<T> {
	/// Call the budgeted function.
	pub fn call(self) -> T::Output {
		Callable::call(self)
	}
}

#[cfg(feature = "std")]
impl<T: Callable> Callable for Slice<T> {
	type Output = T::Output;

	fn call(self) -> Self::Output {
		let _budget = replace(self.total);
		let _yield = YieldPoint(self::no_std::rune_yield_replace(
			self.total.saturating_sub(self.slice),
		));
		self.value.call()
	}
}

/// Hide the outer execution's soft threshold from nested native frames.
/// The no-std path keeps its existing host ABI and never installs a threshold.
pub(crate) struct YieldPoint(usize);

impl YieldPoint {
	#[inline]
	pub(crate) fn take() -> Self {
		#[cfg(feature = "std")]
		return Self(self::no_std::rune_yield_replace(0));
		#[cfg(not(feature = "std"))]
		Self(0)
	}

	#[inline]
	pub(crate) fn threshold(&self) -> usize {
		self.0
	}
}

impl Drop for YieldPoint {
	fn drop(&mut self) {
		#[cfg(feature = "std")]
		self::no_std::rune_yield_replace(self.0);
	}
}

/// Replace the current budget returning a guard that will restore the one which
/// was in effect when it is dropped.
#[inline(never)]
pub fn replace(budget: usize) -> BudgetGuard {
	BudgetGuard(self::no_std::rune_budget_replace(budget))
}

/// Acquire the current budget, leaving no budget in effect for the duration of
/// the returned guard.
///
/// Use [`BudgetGuard::take`] to take permits from the returned budget.
///
/// Note that while the guard is alive nothing else can see the budget, so
/// anything called in the meantime - including a nested execution of the
/// virtual machine - runs unbudgeted. Prefer [`take`] unless that is what is
/// wanted.
#[inline(never)]
pub fn acquire() -> BudgetGuard {
	BudgetGuard(self::no_std::rune_budget_replace(usize::MAX))
}

/// Take a single permit from the ambient budget, if one is set.
///
/// Returns `false` if the budget has been exhausted.
///
/// This is the primitive every budgeted loop uses, the machine's own included.
/// The permit is taken out of the budget where it lives rather than out of a
/// local copy of it, so anything which is called while a loop is running - a
/// native function, or a nested execution of the machine - draws from the same
/// budget and cannot escape it.
///
/// # Examples
///
/// ```
/// use rune::runtime::budget;
///
/// budget::with(2, || {
///     assert!(budget::take());
///     assert!(budget::take());
///     assert!(!budget::take());
/// })
/// .call();
/// ```
#[inline]
pub fn take() -> bool {
	let budget = self::no_std::rune_budget_get();

	// No budget is in effect.
	if budget == usize::MAX {
		return true;
	}

	if budget == 0 {
		return false;
	}

	self::no_std::rune_budget_replace(budget - 1);
	true
}

/// Take a permit from the ambient budget on behalf of a natively driven loop.
///
/// The machine only takes a permit for the instructions it executes itself, so
/// a native loop which walks a value has to take one per step or a host which
/// set a budget has no way to interrupt it. Without this, a script can spend an
/// unbounded amount of time inside a single call such as
/// `(0..).iter().count()`.
///
/// The error raised is the same one the machine's own callers raise when the
/// budget runs out, so a host sees one shape either way.
#[inline]
pub(crate) fn permit() -> Result<(), crate::runtime::VmError> {
	use crate::runtime::{VmError, VmErrorKind, VmHaltInfo};

	if !take() {
		return Err(VmError::new(VmErrorKind::Halted {
			halt: VmHaltInfo::Limited,
		}));
	}

	Ok(())
}

/// A locally acquired budget.
///
/// This guard is acquired by calling [`take`] and can be used to take permits.
///
/// [`take`]: BudgetGuard::take
#[repr(transparent)]
pub struct BudgetGuard(usize);

impl BudgetGuard {
	/// Take a ticker from the budget.
	#[inline]
	pub fn take(&mut self) -> bool {
		if self.0 == usize::MAX {
			return true;
		}

		if self.0 == 0 {
			return false;
		}

		self.0 -= 1;
		true
	}
}

impl Drop for BudgetGuard {
	#[inline]
	fn drop(&mut self) {
		let _ = self::no_std::rune_budget_replace(self.0);
	}
}

impl<T> Budget<T>
where
	T: Callable,
{
	/// Call the budgeted function.
	#[inline]
	pub fn call(self) -> T::Output {
		Callable::call(self)
	}
}

impl<T> Callable for Budget<T>
where
	T: Callable,
{
	type Output = T::Output;

	#[inline]
	fn call(self) -> Self::Output {
		let _yield = YieldPoint::take();
		let _guard = BudgetGuard(self::no_std::rune_budget_replace(self.budget));
		self.value.call()
	}
}

impl<T> Future for Budget<T>
where
	T: Future,
{
	type Output = T::Output;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let this = self.project();

		let _yield = YieldPoint::take();
		let _guard = BudgetGuard(self::no_std::rune_budget_replace(*this.budget));
		let poll = this.value.poll(cx);
		*this.budget = self::no_std::rune_budget_get();
		poll
	}
}
