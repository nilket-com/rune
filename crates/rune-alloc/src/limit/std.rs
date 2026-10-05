use core::cell::Cell;

std::thread_local!(static MEMORY: Cell<usize> = const { Cell::new(usize::MAX) });

pub(super) fn rune_memory_take(amount: usize) -> bool {
    MEMORY.with(|tls| {
        let v = tls.get();

        if v >= amount {
            tls.set(v.wrapping_sub(amount));
            true
        } else {
            false
        }
    })
}

pub(super) fn rune_memory_release(amount: usize) {
    MEMORY.with(|tls| {
        let v = tls.get();
        tls.set(v.saturating_add(amount));
    })
}

pub(super) fn rune_memory_get() -> usize {
    MEMORY.with(|tls| tls.get())
}

pub(super) fn rune_memory_replace(value: usize) -> usize {
    MEMORY.with(|tls| tls.replace(value))
}

std::thread_local!(static EXPLICIT_SCOPE: Cell<bool> = const { Cell::new(false) });

pub(super) fn explicit_scope_active() -> bool {
    EXPLICIT_SCOPE.with(Cell::get)
}

pub(super) fn replace_explicit_scope(active: bool) -> bool {
    EXPLICIT_SCOPE.with(|tls| tls.replace(active))
}
