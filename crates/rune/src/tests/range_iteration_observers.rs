//! Additional base contracts requested by the 0175 tests-first review.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc as StdArc;

// Each fixture has its own counter, so parallel tests cannot pollute it.
#[derive(Any)]
struct ResourceProbe(StdArc<AtomicUsize>);

impl Drop for ResourceProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn remaining_budget() -> usize {
    let mut guard = budget::acquire();
    let mut remaining = 0;
    while guard.take() {
        remaining += 1;
        assert!(remaining <= 1000, "only finite fixture budgets are sampled");
    }
    remaining
}

// These additional base goldens are immutable during candidate testing.
fn observer_golden(actual: &str, expected: &str) {
    assert_eq!(actual, expected);
}

#[test]
fn owning_old_output_preserves_drop_and_slot_boundaries() {
    crate::runtime::range_next_test_reset();
    use crate::runtime::inst::Kind;
    let kinds = [
        Kind::Allocate { size: 3 },
        Kind::CallAssociated {
            hash: Protocol::NEXT.hash,
            addr: Address::ZERO,
            args: 1,
            out: Output::keep(1),
        },
        Kind::IterNext {
            addr: Address::new(1),
            jump: 4,
            out: Output::keep(1),
        },
        Kind::ReturnUnit,
        Kind::ReturnUnit,
    ];
    let mut trace = String::new();
    for permits in [1, 2, 3, 10] {
        let drops = StdArc::new(AtomicUsize::new(0));
        let mut vm = raw_machine_with_args(&kinds, 2);
        let range = Value::new(crate::runtime::range::RangeIter::new(0i64..3)).unwrap();
        let alias = range.clone();
        let old = Value::new(ResourceProbe(drops.clone())).unwrap();
        drop(vm.execute(Hash::EMPTY, (range, old)).unwrap());
        writeln!(trace, "CASE resource-old-output budget={permits}").unwrap();
        for _ in 0..16 {
            let (halt, remaining) = budget::with(permits, || {
                let halt = vm.run(None).unwrap();
                (halt, remaining_budget())
            })
            .call();
            writeln!(
                trace,
                "{halt:?} remaining-budget={remaining} drops={} {}",
                drops.load(Ordering::SeqCst),
                snapshot(&vm)
            )
            .unwrap();
            if matches!(halt, VmHalt::Exited(_)) {
                break;
            }
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            crate::runtime::range::test_i64_iter_bounds(&alias.borrow_ref().unwrap()),
            (1, 3)
        );
    }
    observer_golden(&trace, include_str!("range_iteration_resource_output.txt"));
    assert_eq!(crate::runtime::range_next_test_counts().0, 0);
}

fn execution_trace(permits: usize, observed: bool) -> String {
    execution_trace_on(machine(SOURCE), permits, observed)
}

fn execution_trace_on(mut vm: Vm, permits: usize, observed: bool) -> String {
    drop(vm.execute(["main"], ()).unwrap());
    let mut diagnostics = Diagnostics::new();
    let mut trace = String::new();
    for _ in 0..512 {
        let (halt, remaining) = budget::with(permits, || {
            let observer = if observed {
                Some(&mut diagnostics as &mut dyn crate::runtime::VmDiagnostics)
            } else {
                None
            };
            let halt = vm.run(observer).unwrap();
            (halt, remaining_budget())
        })
        .call();
        writeln!(
            trace,
            "{halt:?} remaining-budget={remaining} {}",
            snapshot(&vm)
        )
        .unwrap();
        if let VmHalt::Exited(Some(address)) = halt {
            assert_eq!(vm.stack().at(address).as_integer::<i64>().unwrap(), 0);
            return trace;
        }
        assert!(matches!(halt, VmHalt::Limited));
    }
    panic!("execution failed to finish");
}

#[test]
fn diagnostics_preserve_every_observed_boundary() {
    crate::runtime::range_next_test_reset();
    let mut trace = String::new();
    for permits in [1, 2, 1000] {
        writeln!(trace, "CASE diagnostics budget={permits}").unwrap();
        trace.push_str(&execution_trace(permits, true));
    }
    observer_golden(&trace, include_str!("range_iteration_diagnostics.txt"));
    assert_eq!(crate::runtime::range_next_test_counts().0, 0);
}

#[test]
fn whole_execution_inside_explicit_memory_scopes() {
    crate::runtime::range_next_test_reset();
    let mut trace = String::new();
    for memory in [usize::MAX, 1 << 20] {
        for permits in [1, 2, 1000] {
            writeln!(trace, "CASE limit={memory} budget={permits}").unwrap();
            let vm = machine(SOURCE);
            let actual = limit::with(memory, || execution_trace_on(vm, permits, false)).call();
            trace.push_str(&actual);
        }
        let mut vm =
            machine("pub async fn main() { let total = 0; for n in -1..2 { total += n; } total }");
        let value = block_on(limit::with(
            memory,
            budget::with(1000, vm.async_call(["main"], ())),
        ))
        .unwrap();
        let result = from_value::<i64>(value).unwrap();
        assert_eq!(result, 0);
        writeln!(trace, "ASYNC limit={memory} result={result}").unwrap();
    }
    observer_golden(&trace, include_str!("range_iteration_explicit_scopes.txt"));
    assert_eq!(crate::runtime::range_next_test_counts().0, 0);
}

#[derive(Any)]
struct CustomIterator {
    position: i64,
    calls: StdArc<AtomicUsize>,
}

impl CustomIterator {
    fn next(&mut self) -> Option<i64> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.position == 3 {
            return None;
        }
        self.position += 1;
        Some(self.position)
    }
}

fn custom_machine() -> Vm {
    let mut context = Context::with_default_modules().unwrap();
    let mut module = Module::new();
    module.ty::<CustomIterator>().unwrap();
    module
        .associated_function(&Protocol::NEXT, CustomIterator::next)
        .unwrap();
    module
        .implement_trait::<CustomIterator>(rune::item!(::std::iter::Iterator))
        .unwrap();
    context.install(module).unwrap();
    crate::tests::vm(
        &context,
        &mut crate::tests::sources(
            "pub fn main(iter) { let total = 0; for n in iter { total += n; } total }",
        ),
        &mut Diagnostics::new(),
        false,
    )
    .unwrap()
}

#[test]
fn other_receiver_and_user_iterator_keep_their_protocols() {
    crate::runtime::range_next_test_reset();
    let mut trace = String::new();
    for permits in [1, 2, 1000] {
        let mut vm =
            machine("pub fn main() { let total = 0; for n in [1, 2, 3] { total += n; } total }");
        drop(vm.execute(["main"], ()).unwrap());
        writeln!(trace, "CASE vector budget={permits}").unwrap();
        for _ in 0..512 {
            let (halt, remaining) = budget::with(permits, || {
                let halt = vm.run(None).unwrap();
                (halt, remaining_budget())
            })
            .call();
            writeln!(
                trace,
                "{halt:?} remaining-budget={remaining} {}",
                snapshot(&vm)
            )
            .unwrap();
            if let VmHalt::Exited(Some(address)) = halt {
                assert_eq!(vm.stack().at(address).as_integer::<i64>().unwrap(), 6);
                break;
            }
        }
        let mut vm = custom_machine();
        let calls = StdArc::new(AtomicUsize::new(0));
        let iter = Value::new(CustomIterator {
            position: 0,
            calls: calls.clone(),
        })
        .unwrap();
        let alias = iter.clone();
        drop(vm.execute(["main"], (iter,)).unwrap());
        writeln!(trace, "CASE custom-Iterator budget={permits}").unwrap();
        for _ in 0..512 {
            let (halt, remaining) = budget::with(permits, || {
                let halt = vm.run(None).unwrap();
                (halt, remaining_budget())
            })
            .call();
            let position = alias.borrow_ref::<CustomIterator>().unwrap().position;
            writeln!(
                trace,
                "{halt:?} remaining-budget={remaining} position={position} calls={} {}",
                calls.load(Ordering::SeqCst),
                snapshot(&vm)
            )
            .unwrap();
            if let VmHalt::Exited(Some(address)) = halt {
                assert_eq!(vm.stack().at(address).as_integer::<i64>().unwrap(), 6);
                assert_eq!(calls.load(Ordering::SeqCst), 4);
                break;
            }
        }
    }
    observer_golden(&trace, include_str!("range_iteration_other_receivers.txt"));
    assert_eq!(crate::runtime::range_next_test_counts().0, 0);
}
