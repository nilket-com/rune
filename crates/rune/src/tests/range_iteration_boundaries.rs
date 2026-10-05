//! Base-engine contract for the exclusive i64 range-loop specialization.
//! The checked-in trace is generated before any production fast-path changes.

prelude!();

use crate::alloc::limit;
use crate::runtime::{budget, VmHalt};
use core::fmt::Write as _;

const SOURCE: &str = "pub fn main() { let total = 0; for n in -1..2 { total += n; } total }";

fn machine(source: &str) -> Vm {
    machine_with_memoization(source, true)
}

fn machine_with_memoization(source: &str, memoize: bool) -> Vm {
    let context = Context::with_default_modules().unwrap();
    let mut sources = crate::tests::sources(source);
    let mut options = Options::default();
    options.memoize_instance_fn(memoize);
    let unit = crate::prepare(&mut sources)
        .with_context(&context)
        .with_options(&options)
        .build()
        .unwrap();
    Vm::new(
        Arc::try_new(context.runtime().unwrap()).unwrap(),
        Arc::try_new(unit).unwrap(),
    )
}

// Preserve every slot, including empty slots and the Option in NEXT's output.
// Opaque native functions are represented by their type rather than an ASLR
// address. The range's exact remaining bounds are read without advancing it.
fn snapshot(vm: &Vm) -> String {
    let stack = vm.stack();
    let mut text = format!(
        "ip={} last={} top={} len={}",
        vm.ip(),
        vm.last_ip(),
        stack.top(),
        stack.len()
    );
    for index in 0..stack.len() - stack.top() {
        let value = stack.at(Address::new(index));
        let item = if let Ok(option) = value.borrow_ref::<Option<Value>>() {
            format!("{option:?}")
        } else if let Ok(iter) = value.borrow_ref::<crate::runtime::range::RangeIter<i64>>() {
            format!(
                "range-i64{:?}",
                crate::runtime::range::test_i64_iter_bounds(&iter)
            )
        } else if matches!(value.as_ref(), crate::runtime::Repr::Inline(_)) {
            format!("{value:?}")
        } else {
            format!("opaque:{}", value.type_info())
        };
        write!(text, " | {index}:{item}").unwrap();
    }
    text
}

fn trace_with_memoization(memoize: bool) -> String {
    let mut vm = machine_with_memoization(SOURCE, memoize);
    drop(vm.execute(["main"], ()).unwrap());
    let mut text = String::new();
    for step in 0..512 {
        let halt = budget::with(1, || vm.run(None)).call().unwrap();
        writeln!(text, "{step} {halt:?} {}", snapshot(&vm)).unwrap();
        match halt {
            VmHalt::Limited => {}
            VmHalt::Exited(Some(address)) => {
                assert_eq!(vm.stack().at(address).as_integer::<i64>().unwrap(), 0);
                return text;
            }
            other => panic!("unexpected halt: {other:?}"),
        }
    }
    panic!("one-permit execution did not terminate");
}

#[test]
fn every_one_permit_boundary_matches_base() {
    for (memoize, golden) in [
        (true, include_str!("range_iteration_boundaries.txt")),
        (
            false,
            include_str!("range_iteration_associated_boundaries.txt"),
        ),
    ] {
        assert_eq!(trace_with_memoization(memoize), golden);
    }
}

#[test]
fn every_budget_prefix_and_resume_matches_base_steps() {
    let golden = trace_with_memoization(true);
    let lines: Vec<_> = golden.lines().collect();
    for permits in 0..lines.len() - 1 {
        let mut vm = machine(SOURCE);
        drop(vm.execute(["main"], ()).unwrap());
        assert!(matches!(
            budget::with(permits, || vm.run(None)).call().unwrap(),
            VmHalt::Limited
        ));
        if permits > 0 {
            assert!(lines[permits - 1].ends_with(&snapshot(&vm)));
        }
        let halt = budget::with(1, || vm.run(None)).call().unwrap();
        assert!(matches!(halt, VmHalt::Limited));
        assert!(lines[permits].ends_with(&snapshot(&vm)));
    }
}

#[test]
fn nested_memory_limits_restore_on_return_and_unwind() {
    let outside = limit::get();
    limit::with(4096, || {
        assert_eq!(limit::get(), 4096);
        limit::with(64, || assert_eq!(limit::get(), 64)).call();
        assert_eq!(limit::get(), 4096);
        limit::with(usize::MAX, || assert_eq!(limit::get(), usize::MAX)).call();
        assert_eq!(limit::get(), 4096);
        let result = std::panic::catch_unwind(|| limit::with(32, || panic!("scope unwind")).call());
        assert!(result.is_err());
        assert_eq!(limit::get(), 4096);
    })
    .call();
    assert_eq!(limit::get(), outside);
}

#[test]
fn memory_future_restores_after_pending_and_ready() {
    use core::future::Future;
    use core::task::{Context as TaskContext, Poll};
    let outside = limit::get();
    let mut polls = 0;
    let inner = core::future::poll_fn(|_| {
        assert_eq!(limit::get(), 128);
        limit::with(16, || assert_eq!(limit::get(), 16)).call();
        assert_eq!(limit::get(), 128);
        polls += 1;
        if polls == 1 {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    });
    let mut future = std::boxed::Box::pin(limit::with(128, inner));
    let waker = futures_util::task::noop_waker();
    let mut context = TaskContext::from_waker(&waker);
    assert!(future.as_mut().poll(&mut context).is_pending());
    assert_eq!(limit::get(), outside);
    assert!(future.as_mut().poll(&mut context).is_ready());
    assert_eq!(limit::get(), outside);
    drop(future);
    assert_eq!(polls, 2);
}

#[test]
fn range_control_flow_and_fallback_results() {
    let cases = [
        ("let s = 0; for x in 0..0 { s += 1; } s", 0),
        ("let s = 0; for x in 3..1 { s += 1; } s", 0),
        ("let s = 0; for x in -2..-1 { s += x; } s", -2),
        ("let s = 0; for x in 9223372036854775806..9223372036854775807 { s += x; } s", i64::MAX - 1),
        ("let s = 0; for x in -9223372036854775808..-9223372036854775807 { s += x; } s", i64::MIN),
        ("let s = 0; for x in 0..5 { if x == 1 { continue; } if x == 4 { break; } s += x; } s", 5),
        ("let s = 0; for x in 0..3 { for y in 0..2 { s += x + y; } } s", 9),
        ("for x in 0..3 { if x == 1 { return 42; } } 0", 42),
        ("let i = (-2..2).iter(); let a = i.next().unwrap(); let b = i.next_back().unwrap(); a + b", -1),
        ("let s = 0; for x in 0..=2 { s += x; } s", 3),
        ("let s = 0; for x in (0..3).iter().map(|x| x + 1) { s += x; } s", 6),
    ];
    for (body, expected) in cases {
        assert_eq!(eval::<i64>(body), expected, "{body}");
    }
}

#[test]
fn both_dispatch_forms_finish_after_budget_resume() {
    for memoize in [false, true] {
        for permits in [1, 2, 3, 10, 1000, 1_000_000] {
            let mut vm = machine_with_memoization(SOURCE, memoize);
            drop(vm.execute(["main"], ()).unwrap());
            let mut finished = false;
            for _ in 0..512 {
                match budget::with(permits, || vm.run(None)).call().unwrap() {
                    VmHalt::Limited => {}
                    VmHalt::Exited(Some(address)) => {
                        assert_eq!(vm.stack().at(address).as_integer::<i64>().unwrap(), 0);
                        finished = true;
                        break;
                    }
                    other => panic!("unexpected halt: {other:?}"),
                }
            }
            assert!(finished);
        }
    }
}

#[test]
fn a_borrowed_range_is_not_advanced_by_a_failed_next() {
    // Locate the first native NEXT in the base's single-permit trace by its
    // newly materialized Some. Recreate the state immediately before it.
    let trace = trace_with_memoization(true);
    let next_step = trace
        .lines()
        .position(|line| line.contains("Some("))
        .unwrap();
    let mut vm = machine(SOURCE);
    drop(vm.execute(["main"], ()).unwrap());
    assert!(matches!(
        budget::with(next_step, || vm.run(None)).call().unwrap(),
        VmHalt::Limited
    ));
    let value = (0..vm.stack().len() - vm.stack().top())
        .map(|index| vm.stack().at(Address::new(index)))
        .find(|value| {
            value
                .borrow_ref::<crate::runtime::range::RangeIter<i64>>()
                .is_ok()
        })
        .unwrap()
        .clone();
    let borrowed = value
        .borrow_ref::<crate::runtime::range::RangeIter<i64>>()
        .unwrap();
    let before = crate::runtime::range::test_i64_iter_bounds(&borrowed);
    let error = budget::with(1, || vm.run(None))
        .call()
        .expect_err("NEXT needs exclusive access");
    assert!(
        format!("{error:?}").contains("NotAccessibleMut"),
        "{error:?}"
    );
    assert_eq!(
        crate::runtime::range::test_i64_iter_bounds(&borrowed),
        before
    );
}

fn check_golden(actual: &str, expected: &str) {
    assert_eq!(actual, expected);
}

#[test]
fn next_allocation_failure_keeps_the_base_boundary() {
    let trace = trace_with_memoization(true);
    let step = trace
        .lines()
        .position(|line| line.contains("Some("))
        .unwrap();
    let mut vm = machine(SOURCE);
    drop(vm.execute(["main"], ()).unwrap());
    assert!(matches!(
        budget::with(step, || vm.run(None)).call().unwrap(),
        VmHalt::Limited
    ));
    let error = budget::with(1, limit::with(0, || vm.run(None)))
        .call()
        .expect_err("Option allocation must fail");
    let actual = format!("{error:?}\n{}\n", snapshot(&vm));
    check_golden(
        &actual,
        include_str!("range_iteration_allocation_failure.txt"),
    );
}

fn stable_error(error: &crate::runtime::VmError) -> String {
    match error.error().kind() {
        VmErrorKind::UnsupportedIterNextOperand { actual } => {
            // TypeInfo's Debug prints its formatting function's ASLR address.
            // Bind this fixture to the exact empty type, not just its spelling.
            assert_eq!(*actual, crate::runtime::TypeInfo::empty());
            assert!(error.chain().is_empty());
            format!("UnsupportedIterNextOperand actual={actual}")
        }
        _ => format!("{error:?}"),
    }
}

fn raw_machine(kinds: &[crate::runtime::inst::Kind]) -> Vm {
    use crate::runtime::unit::{DefaultStorage, UnitEncoder, UnitFn};
    use crate::runtime::{Call, Inst, Unit};
    let mut storage = DefaultStorage::default();
    for &kind in kinds {
        storage.encode(Inst::new(kind)).unwrap();
    }
    let mut functions = crate::hash::Map::default();
    functions
        .try_insert(
            Hash::EMPTY,
            UnitFn::Offset {
                offset: 0,
                call: Call::Immediate,
                args: 1,
                captures: None,
            },
        )
        .unwrap();
    let unit = Unit::new(
        storage,
        functions,
        alloc::Vec::new(),
        alloc::Vec::new(),
        alloc::Vec::new(),
        alloc::Vec::new(),
        crate::hash::Map::default(),
        None,
        crate::hash::Map::default(),
        alloc::Vec::new(),
        crate::hash::Map::default(),
    );
    let context = Context::with_default_modules().unwrap();
    Vm::new(
        Arc::try_new(context.runtime().unwrap()).unwrap(),
        Arc::try_new(unit).unwrap(),
    )
}

#[test]
fn unusual_next_units_preserve_errors_and_raw_slots() {
    use crate::runtime::inst::Kind;
    let mut actual = String::new();
    for (name, call_out, iter_addr, iter_out, args, intervening) in [
        ("ordinary", Output::keep(1), 1, Output::keep(1), 1, false),
        (
            "different-out",
            Output::keep(1),
            1,
            Output::keep(2),
            1,
            false,
        ),
        (
            "receiver-alias",
            Output::keep(0),
            0,
            Output::keep(0),
            1,
            false,
        ),
        (
            "ignored-call-out",
            Output::discard(),
            1,
            Output::keep(1),
            1,
            false,
        ),
        (
            "ignored-iter-out",
            Output::keep(1),
            1,
            Output::discard(),
            1,
            false,
        ),
        (
            "bad-iter-addr",
            Output::keep(1),
            999,
            Output::keep(1),
            1,
            false,
        ),
        (
            "bad-call-out",
            Output::keep(999),
            1,
            Output::keep(1),
            1,
            false,
        ),
        ("bad-args", Output::keep(1), 1, Output::keep(1), 2, false),
        (
            "intervening-copy",
            Output::keep(1),
            2,
            Output::keep(2),
            1,
            true,
        ),
    ] {
        let mut kinds = vec![
            Kind::Allocate { size: 3 },
            Kind::CallAssociated {
                hash: Protocol::NEXT.hash,
                addr: Address::ZERO,
                args,
                out: call_out,
            },
        ];
        if intervening {
            kinds.push(Kind::Copy {
                addr: Address::new(1),
                out: Output::keep(2),
            });
        }
        kinds.push(Kind::IterNext {
            addr: Address::new(iter_addr),
            jump: kinds.len() + 2,
            out: iter_out,
        });
        kinds.push(Kind::ReturnUnit);
        kinds.push(Kind::ReturnUnit);
        for permits in [1, 2, 3, 10] {
            let mut vm = raw_machine(&kinds);
            let range = Value::new(crate::runtime::range::RangeIter::new(0i64..3)).unwrap();
            let alias = range.clone();
            drop(vm.execute(Hash::EMPTY, (range,)).unwrap());
            writeln!(actual, "CASE {name} budget={permits}").unwrap();
            for _ in 0..16 {
                let (halt, remaining_budget) = budget::with(permits, || {
                    let halt = vm.run(None);
                    let mut guard = budget::acquire();
                    let remaining = (0..=permits).take_while(|_| guard.take()).count();
                    (halt, remaining)
                })
                .call();
                match halt {
                    Ok(halt) => {
                        writeln!(
                            actual,
                            "{halt:?} remaining-budget={remaining_budget} {}",
                            snapshot(&vm)
                        )
                        .unwrap();
                        if matches!(halt, VmHalt::Exited(_)) {
                            break;
                        }
                    }
                    Err(error) => {
                        writeln!(
                            actual,
                            "ERROR {} remaining-budget={remaining_budget} {}",
                            stable_error(&error),
                            snapshot(&vm)
                        )
                        .unwrap();
                        break;
                    }
                }
            }
            let remaining = crate::runtime::range::test_i64_iter_bounds(
                &alias
                    .borrow_ref::<crate::runtime::range::RangeIter<i64>>()
                    .unwrap(),
            );
            writeln!(actual, "remaining={remaining:?}").unwrap();
        }
    }
    check_golden(&actual, include_str!("range_iteration_unusual_units.txt"));
}
