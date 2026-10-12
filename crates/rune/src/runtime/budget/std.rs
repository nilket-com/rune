use core::cell::Cell;

std::thread_local!(static BUDGET: Cell<usize> = const { Cell::new(usize::MAX) });

pub(super) fn rune_budget_get() -> usize {
	BUDGET.with(|tls| tls.get())
}

pub(super) fn rune_budget_replace(value: usize) -> usize {
	BUDGET.with(|tls| tls.replace(value))
}

std::thread_local!(static YIELD_AT: Cell<usize> = const { Cell::new(0) });

pub(super) fn rune_yield_replace(value: usize) -> usize {
	YIELD_AT.with(|tls| tls.replace(value))
}
