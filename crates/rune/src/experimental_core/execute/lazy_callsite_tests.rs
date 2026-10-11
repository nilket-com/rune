//! D14 tests-first: the call chain, error location, cleanup and reuse that a
//! lazily derived callsite must preserve exactly. These pass on D13 (eager
//! Location per frame) and are the oracle for D14. Every site is distinct, the
//! calls sit at nonzero pcs, and the expected chain is innermost-first.
use super::*;
use crate::experimental_core::{
    arithmetic::Binary,
    code::{Builder, Function},
    heap::Limits,
};

fn loc(n: u32) -> Location {
    Location {
        source: 14,
        revision: 1,
        start: n,
        end: n + 1,
    }
}
fn store() -> Store {
    Store::new(Limits {
        entries: 16,
        bytes: 4096,
    })
    .unwrap()
}
/// Push `ops` with their own locations.
fn function(slots: u32, parameters: u32, ops: &[(Op, u32)]) -> Function {
    let mut f = Function::new(slots, parameters).unwrap();
    for &(op, at) in ops {
        f.push(op, loc(at)).unwrap();
    }
    f
}
fn call(f: &mut Function, out: u32, callee: u32, args: &[u32], at: u32) {
    let (args_start, args_len) = f.arguments(args).unwrap();
    f.push(
        Op::Call {
            out,
            function: callee,
            args_start,
            args_len,
        },
        loc(at),
    )
    .unwrap();
}
fn lit(out: u32, value: Value) -> Op {
    Op::Literal { out, value }
}
fn code(functions: std::vec::Vec<Function>) -> Code {
    let mut b = Builder::default();
    for f in functions {
        b.push(f).unwrap();
    }
    b.finish().unwrap()
}
/// main(a, b) -> f1(a, b) -> f2(a, b) = a / b; calls at nonzero pcs.
fn three_levels() -> Code {
    let mut main = function(3, 2, &[(lit(2, Value::I64(0)), 100)]);
    call(&mut main, 2, 1, &[0, 1], 101);
    main.push(Op::Return { value: 2 }, loc(102)).unwrap();
    let mut f1 = function(
        3,
        2,
        &[(lit(2, Value::Unit), 200), (lit(2, Value::I64(1)), 201)],
    );
    call(&mut f1, 2, 2, &[0, 1], 202);
    f1.push(Op::Return { value: 2 }, loc(203)).unwrap();
    let f2 = function(
        3,
        2,
        &[
            (
                Op::Binary {
                    out: 2,
                    op: Binary::Div,
                    lhs: 0,
                    rhs: 1,
                },
                300,
            ),
            (Op::Return { value: 2 }, 301),
        ],
    );
    code(std::vec![main, f1, f2])
}
// `Failure`, the public error type of `invoke`, is large by design.
#[allow(clippy::result_large_err)]
fn run(
    e: &mut Executor,
    c: &Code,
    s: &mut Store,
    args: &[i64],
    fuel: usize,
) -> core::result::Result<Value, Failure> {
    let roots: std::vec::Vec<_> = args.iter().map(|&a| s.root_i64(a)).collect();
    let refs: std::vec::Vec<_> = roots.iter().collect();
    let result = e.invoke_code(c, s, 0, &refs, fuel).map(|root| {
        let v = root.value;
        s.root_release(root).unwrap();
        v
    });
    for r in roots {
        s.root_release(r).unwrap();
    }
    result
}

#[test]
fn d14_nested_failure_chain_is_innermost_first_with_caller_sites() {
    let c = three_levels();
    let mut s = store();
    let mut e = Executor::new().unwrap();
    for _ in 0..2 {
        let failure = run(&mut e, &c, &mut s, &[7, 0], 100).unwrap_err();
        assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
        assert_eq!(failure.error.location, Some(loc(300)));
        // f2 was called from f1's pc 2 (loc 202); f1 from main's pc 1 (loc 101).
        assert_eq!(&failure.call_chain[..], &[loc(202), loc(101)]);
        assert!(failure.cleanup_error.is_none() && e.is_idle());
        assert_eq!(
            run(&mut e, &c, &mut s, &[7, 2], 100).unwrap(),
            Value::I64(3)
        );
        assert!(e.is_idle());
    }
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

/// rec(n): if n == 0 { 1 / 0 } else { rec(n - 1) }; called from main.
fn recursion() -> Code {
    let mut main = function(2, 1, &[(lit(1, Value::Unit), 400)]);
    call(&mut main, 1, 1, &[0], 401);
    main.push(Op::Return { value: 1 }, loc(402)).unwrap();
    let mut rec = function(
        4,
        1,
        &[
            (lit(1, Value::I64(0)), 500),
            (
                Op::Binary {
                    out: 2,
                    op: Binary::Eq,
                    lhs: 0,
                    rhs: 1,
                },
                501,
            ),
            (
                Op::Branch {
                    condition: 2,
                    yes: 6,
                    no: 3,
                },
                502,
            ),
            (lit(1, Value::I64(1)), 503),
            (
                Op::Binary {
                    out: 3,
                    op: Binary::Sub,
                    lhs: 0,
                    rhs: 1,
                },
                504,
            ),
        ],
    );
    call(&mut rec, 3, 1, &[3], 505); // pc 5
    rec.push(
        Op::Binary {
            out: 3,
            op: Binary::Div,
            lhs: 1,
            rhs: 1,
        },
        loc(506),
    )
    .unwrap(); // pc 6: 0 / 0 (slot 1 holds 0 here)
    rec.push(Op::Return { value: 3 }, loc(507)).unwrap();
    code(std::vec![main, rec])
}

#[test]
fn d14_recursion_chain_repeats_the_recursive_site_then_the_entry_site() {
    let c = recursion();
    let mut s = store();
    let mut e = Executor::new().unwrap();
    for depth in [0usize, 1, 2, 5] {
        let failure = run(&mut e, &c, &mut s, &[depth as i64], 1_000).unwrap_err();
        assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
        assert_eq!(failure.error.location, Some(loc(506)));
        let mut want = std::vec![loc(505); depth];
        want.push(loc(401));
        assert_eq!(&failure.call_chain[..], &want[..], "depth {depth}");
        assert!(failure.cleanup_error.is_none() && e.is_idle());
    }
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d14_frame_capacity_chain_has_every_published_callsite_in_order() {
    // main calls rec at pc 1 (loc 601); rec calls itself at pc 1 (loc 701).
    let mut main = function(1, 0, &[(lit(0, Value::Unit), 600)]);
    call(&mut main, 0, 1, &[], 601);
    main.push(Op::Return { value: 0 }, loc(602)).unwrap();
    let mut rec = function(1, 0, &[(lit(0, Value::Unit), 700)]);
    call(&mut rec, 0, 1, &[], 701);
    rec.push(Op::Return { value: 0 }, loc(702)).unwrap();
    let c = code(std::vec![main, rec]);
    let mut s = store();
    let mut e = Executor::new().unwrap();
    let failure = e.invoke_code(&c, &mut s, 0, &[], 100_000).unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::FrameCapacity);
    assert_eq!(failure.error.location, Some(loc(701)));
    // 1024 published frames: main (no callsite), rec from main, 1022 recursive.
    let mut want = std::vec![loc(701); MAX_FRAMES - 2];
    want.push(loc(601));
    assert_eq!(&failure.call_chain[..], &want[..]);
    assert!(failure.cleanup_error.is_none() && e.is_idle());
}

/// Every function's pc 0 is charged (main at loc 400, each rec frame at loc
/// 500), so the k-th charge lands in the (k-1)-th nested rec frame.
fn charged_chain(k: usize) -> (Location, std::vec::Vec<Location>) {
    if k == 0 {
        return (loc(400), std::vec::Vec::new());
    }
    let mut chain = std::vec![loc(505); k - 1];
    chain.push(loc(401));
    (loc(500), chain)
}

#[test]
fn d14_fuel_exhaustion_inside_nested_callees_has_the_exact_chain() {
    let c = recursion();
    let mut s = store();
    let mut e = Executor::new().unwrap();
    // Depth 3 publishes main plus rec(3..0): charges 0..=4 exist; fuel k fails at charge k.
    for fuel in 0..=4 {
        let failure = run(&mut e, &c, &mut s, &[3], fuel).unwrap_err();
        let (location, chain) = charged_chain(fuel);
        assert_eq!(failure.error.kind, ErrorKind::FuelExhausted, "fuel {fuel}");
        assert_eq!(failure.error.location, Some(location), "fuel {fuel}");
        assert_eq!(&failure.call_chain[..], &chain[..], "fuel {fuel}");
        assert!(failure.cleanup_error.is_none() && e.is_idle());
    }
    // Enough fuel reaches the division by zero in the deepest frame.
    let failure = run(&mut e, &c, &mut s, &[3], 5).unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
    assert_eq!(failure.error.location, Some(loc(506)));
    assert_eq!(
        &failure.call_chain[..],
        &[loc(505), loc(505), loc(505), loc(401)]
    );
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d14_cancellation_inside_nested_callees_has_the_exact_chain() {
    // Test-only cancel_at_charge_for_test requests cancellation at the n-th
    // charged instruction (there is no other way to cancel mid-invocation).
    let c = recursion();
    let mut s = store();
    let mut e = Executor::new().unwrap();
    for n in 0..=4 {
        cancel_at_charge_for_test(Some(n));
        let failure = run(&mut e, &c, &mut s, &[3], 1_000).unwrap_err();
        cancel_at_charge_for_test(None);
        let (location, chain) = charged_chain(n);
        assert_eq!(failure.error.kind, ErrorKind::Cancelled, "charge {n}");
        assert_eq!(failure.error.location, Some(location), "charge {n}");
        assert_eq!(&failure.call_chain[..], &chain[..], "charge {n}");
        assert!(failure.cleanup_error.is_none() && e.is_idle());
        // The request is consumed: the next invocation is not cancelled.
        let next = run(&mut e, &c, &mut s, &[1], 1_000).unwrap_err();
        assert_eq!(next.error.kind, ErrorKind::DivisionByZero);
        assert_eq!(&next.call_chain[..], &[loc(505), loc(401)]);
    }
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d14_failed_unpublished_call_adds_no_chain_entry() {
    // main(a) -> f1(a) -> f2(a): f1's call retains `a` past u32::MAX.
    let mut main = function(2, 1, &[(lit(1, Value::Unit), 800)]);
    call(&mut main, 1, 1, &[0], 801);
    main.push(Op::Return { value: 1 }, loc(802)).unwrap();
    let mut f1 = function(2, 1, &[(lit(1, Value::Unit), 900)]);
    call(&mut f1, 1, 2, &[0], 901);
    f1.push(Op::Return { value: 1 }, loc(902)).unwrap();
    let f2 = function(1, 1, &[(Op::Return { value: 0 }, 1000)]);
    let c = code(std::vec![main, f1, f2]);
    let mut s = store();
    let text = s.root_string("held").unwrap();
    // Host root (1) + entry slot (+1) + main's call (+1) reach u32::MAX; f1's call overflows.
    s.test_set_strong(text.value, u32::MAX - 2);
    let mut e = Executor::new().unwrap();
    let failure = e.invoke_code(&c, &mut s, 0, &[&text], 100).unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::Overflow);
    assert_eq!(failure.error.location, Some(loc(901)));
    assert_eq!(&failure.call_chain[..], &[loc(801)]);
    assert!(failure.cleanup_error.is_none() && e.is_idle());
    assert_eq!(s.test_strong(text.value), u32::MAX - 2);
    s.test_set_strong(text.value, 1);
    s.root_release(text).unwrap();
    assert_eq!(s.live_entries(), 0);
}

#[test]
fn d14_failure_in_leave_after_return_popped_its_frame_adds_no_chain_entry() {
    // main -> f1 -> f2, with no puts before the calls (enter and Call write
    // slots directly), so f2's leave is the program's first slot release; it
    // drains a queued stale handle (test-only push_work_for_test) and fails
    // after f2's frame was popped. f2's callsite (f1's loc 1102) is absent.
    let mut main = function(2, 1, &[]);
    call(&mut main, 1, 1, &[0], 1101);
    main.push(Op::Return { value: 1 }, loc(1103)).unwrap();
    let mut f1 = function(2, 1, &[]);
    call(&mut f1, 1, 2, &[0], 1102);
    f1.push(Op::Return { value: 1 }, loc(1105)).unwrap();
    let f2 = function(1, 1, &[(Op::Return { value: 0 }, 1200)]);
    let c = code(std::vec![main, f1, f2]);
    let mut s = store();
    let stale = s.string("stale").unwrap();
    s.release(stale).unwrap();
    let arg = s.root_i64(5);
    s.push_work_for_test(stale);
    let mut e = Executor::new().unwrap();
    let failure = e.invoke_code(&c, &mut s, 0, &[&arg], 100).unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::InvalidHandle);
    assert_eq!(failure.error.location, Some(loc(1200)));
    assert_eq!(&failure.call_chain[..], &[loc(1101)]);
    assert!(failure.cleanup_error.is_none() && e.is_idle());
    assert_eq!(s.queued_work_for_test(), 0);
    // The queued stale edge was the only extra state; the store owns nothing else.
    s.root_release(arg).unwrap();
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d14_synthetic_put_failure_after_return_popped_its_frame_adds_no_chain_entry() {
    // SYNTHETIC (not production-reachable): Return's put fails only if releasing
    // the caller's old out-slot value fails after leave succeeded. With valid
    // code and refcounts that cannot happen: a closed store or queued drain would
    // already fail in leave, a slot cannot hold a stale handle, and a lease holds
    // its own strong reference, so a slot release is never the last one while it
    // is leased. Here test_set_strong makes put's release the last one while a
    // host iteration lease is live, so the release is refused at put.
    // entry(a): o = a; release a; o = f1(); return o.   f1(): r = 7; return r.
    let mut entry = function(
        2,
        1,
        &[
            (Op::Copy { out: 1, value: 0 }, 1300),
            (Op::Release { value: 0 }, 1301),
        ],
    );
    call(&mut entry, 1, 1, &[], 1302);
    entry.push(Op::Return { value: 1 }, loc(1303)).unwrap();
    let f1 = function(
        1,
        0,
        &[
            (lit(0, Value::I64(7)), 1400),
            (Op::Return { value: 0 }, 1401),
        ],
    );
    let c = code(std::vec![entry, f1]);
    let mut s = store();
    let list = s
        .list(crate::experimental_core::value::Type::Vec, &[Value::I64(1)])
        .unwrap();
    let root = s.export(list);
    let iter = s.begin_iter(list).unwrap();
    // True owners: the host root and the lease. Synthetic count 0: enter (+1),
    // Copy (+1) and Release (-1) leave 1, so put's release is the last one.
    s.test_set_strong(list, 0);
    let mut e = Executor::new().unwrap();
    let failure = e.invoke_code(&c, &mut s, 0, &[&root], 100).unwrap_err();
    // release_one refuses a last release of a leased entry (heap.rs, next == 0 && readers != 0).
    assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
    assert_eq!(failure.error.location, Some(loc(1401)));
    // f1's frame was popped before put; only the entry remains, which has no callsite.
    assert!(failure.call_chain.is_empty());
    assert!(failure.cleanup_error.is_none() && e.is_idle());
    // The refused last release left the entry untouched; restore the two real owners.
    assert_eq!(s.test_strong(list), 1);
    s.test_set_strong(list, 2);
    s.end_iter(iter).unwrap();
    s.root_release(root).unwrap();
    assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
// `Failure`, the public error type of `invoke`, is large by design.
#[allow(clippy::result_large_err)]
fn d14_diagnostics_exhaustion_breaks_after_the_last_pushed_site() {
    // Test-only mechanism: prefill the call-chain buffer to one below its
    // reserved capacity and invoke under a zero allocation budget, so the
    // second chain push must grow the buffer and is refused (the loop breaks).
    let c = three_levels();
    let mut s = store();
    let mut e = Executor::new().unwrap();
    let capacity = e.diagnostics.capacity();
    assert!(capacity >= MAX_FRAMES);
    while e.diagnostics.len() + 1 < capacity {
        e.diagnostics.try_push(loc(1)).unwrap();
    }
    let a = s.root_i64(7);
    let b = s.root_i64(0);
    let failure = rune_alloc::limit::with(0, || e.invoke_code(&c, &mut s, 0, &[&a, &b], 100))
        .call()
        .unwrap_err();
    assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
    assert_eq!(failure.error.location, Some(loc(300)));
    assert_eq!(failure.call_chain.len(), capacity);
    assert!(failure.call_chain[..capacity - 1]
        .iter()
        .all(|site| *site == loc(1)));
    // Only the innermost site fit; the next (loc 101) was refused.
    assert_eq!(failure.call_chain[capacity - 1], loc(202));
    assert!(failure.cleanup_error.is_none() && e.is_idle());
    s.root_release(a).unwrap();
    s.root_release(b).unwrap();
    // Reuse after the buffer was moved out: it is reserved again.
    assert_eq!(
        run(&mut e, &c, &mut s, &[9, 3], 100).unwrap(),
        Value::I64(3)
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
fn d14_frame_is_40_bytes_on_64_bit() {
    // Measured D13 layout: 72 B (callsite Option<Location> was 32 B). D14 packs a
    // u32 callsite pc beside the u32 function: 4 + 4 + 8 + 8 + 16.
    assert_eq!(core::mem::size_of::<Frame>(), 40);
}
