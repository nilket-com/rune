//! The state a budget halt leaves behind in a store-heavy loop: the instruction
//! position, the stack and a native side effect, pinned against a recorded
//! golden so that a change to the store path can be shown not to move a halt.
//!
//! Run with `RUNE_UPDATE_STORE_HALT_GOLDEN=1` to record the golden from the
//! code being tested; otherwise the state is compared with the recorded one.

prelude!();

use core::cell::Cell;

use crate::runtime::{budget, VmOutcome};

std::thread_local! {
    static TICKS: Cell<i64> = const { Cell::new(0) };
}

fn tick() {
    TICKS.with(|t| t.set(t.get() + 1));
}

const SOURCE: &str = r#"
pub fn main() {
    let a = 0;
    let v = [];
    let o = #{};
    let i = 0;
    while i < 1000000 {
        a = i;
        v = [i, a];
        o = #{n: i};
        a = v;
        tick();
        i += 1;
    }
    a
}
"#;

const BUDGETS: [usize; 6] = [1, 2, 3, 10, 1000, 1000000];

/// Run `main` under `budget` and describe where it stopped.
fn halt_state(budget_permits: usize) -> String {
    let mut module = Module::new();
    module.function("tick", tick).build().unwrap();
    let mut context = Context::with_default_modules().unwrap();
    context.install(module).unwrap();

    let mut sources = crate::tests::sources(SOURCE);
    let mut diagnostics = Diagnostics::new();
    let mut vm = crate::tests::vm(&context, &mut sources, &mut diagnostics, false).unwrap();

    TICKS.with(|t| t.set(0));

    // Executing rather than calling keeps the suspended frame, so the halted
    // stack can be read as well as the instruction position.
    let mut execution = vm.execute(["main"], ()).unwrap();
    let outcome = budget::with(budget_permits, || execution.resume().complete()).call();

    let outcome = match outcome {
        Ok(VmOutcome::Limited) => std::string::String::from("limited"),
        Ok(VmOutcome::Complete(value)) => std::format!("complete {value:?}"),
        Ok(..) => std::string::String::from("other"),
        Err(..) => std::string::String::from("error"),
    };

    let vm = execution.vm();

    let stack: std::vec::Vec<std::string::String> = vm
        .stack()
        .get(..)
        .unwrap_or(&[])
        .iter()
        .map(describe)
        .collect();

    std::format!(
        "budget={budget_permits} outcome={outcome} ip={} last_ip={} ticks={} stack={:?}\n",
        vm.ip(),
        vm.last_ip(),
        TICKS.with(|t| t.get()),
        stack,
    )
}

/// A value's contents without any heap address, so that the description is
/// the same on every run.
fn describe(value: &Value) -> std::string::String {
    if let Ok(vec) = value.borrow_ref::<crate::runtime::Vec>() {
        let items: std::vec::Vec<_> = vec.iter().map(describe).collect();
        return std::format!("vec{items:?}");
    }

    if let Ok(object) = value.borrow_ref::<crate::runtime::Object>() {
        let mut items: std::vec::Vec<_> = object
            .iter()
            .map(|(k, v)| std::format!("{}={}", k.as_str(), describe(v)))
            .collect();
        items.sort();
        return std::format!("object{items:?}");
    }

    std::format!("{value:?}")
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/store_halt_state.golden")
}

#[test]
fn budget_halts_leave_the_recorded_state() {
    let state: std::string::String = BUDGETS.iter().map(|b| halt_state(*b)).collect();

    if std::env::var_os("RUNE_UPDATE_STORE_HALT_GOLDEN").is_some() {
        std::fs::write(golden_path(), &state).unwrap();
        return;
    }

    let golden = std::fs::read_to_string(golden_path()).unwrap();
    assert_eq!(state, golden);
}

/// The comparison is sensitive: a halt one permit later is a different state.
#[test]
fn shifted_halt_state_is_detected() {
    for b in [2usize, 3, 10, 1000] {
        assert_ne!(halt_state(b), halt_state(b + 1).replace(&std::format!("budget={}", b + 1), &std::format!("budget={b}")));
    }
}
