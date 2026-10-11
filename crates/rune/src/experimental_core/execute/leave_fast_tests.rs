//! D13 tests-first: Executor::leave against the literal pre-D13 loop.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
	value::Type,
};

fn location(n: u32) -> Location {
	Location {
		source: 13,
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
fn frame(base: usize) -> Frame {
	Frame {
		function: 0,
		pc: 0,
		base,
		out: None,
		callsite_pc: NO_CALLSITE,
	}
}
fn scalars() -> [Value; 9] {
	[
		Value::Unit,
		Value::Bool(true),
		Value::I64(-42),
		Value::U64(u64::MAX),
		Value::F64(-0.0),
		Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
		Value::Char('μ'),
		Value::Type(Type::I64),
		Value::Function { code: 0, id: 0 },
	]
}
/// Exact slot identity: floats by bits, so -0.0 and NaN payloads are distinct.
fn key(value: &Value) -> std::string::String {
	match value {
		Value::F64(x) => std::format!("F64({:016x})", x.to_bits()),
		other => std::format!("{other:?}"),
	}
}

/// Literal pre-D13 `Executor::leave` (41044332 execute.rs:685-698). It is the
/// oracle and uses neither the new predicate nor any D13 helper.
fn old_leave(e: &mut Executor, store: &mut Store) -> Result<()> {
	let frame = e
		.frames
		.pop()
		.ok_or_else(|| error(ErrorKind::InvalidCode))?;
	while e.slots.len() > frame.base {
		store.release_slot(e.slots.pop().ok_or_else(|| error(ErrorKind::InvalidCode))?)?;
	}
	Ok(())
}

/// Everything observable after a leave: result, executor frames (callsite
/// included), slots and diagnostics, and the store's mutable state.
#[derive(Debug, PartialEq)]
struct Observed {
	result: std::string::String,
	frames: std::vec::Vec<(u32, usize, usize, Option<usize>, u32)>,
	slots: std::vec::Vec<std::string::String>,
	diagnostics: std::vec::Vec<Location>,
	store: std::string::String,
}
fn observe(result: Result<()>, e: &Executor, store: &Store) -> Observed {
	Observed {
		result: std::format!("{:?}", result.map_err(|error| (error.kind, error.location))),
		frames: e
			.frames
			.iter()
			.map(|f| (f.function, f.pc, f.base, f.out, f.callsite_pc))
			.collect(),
		slots: e.slots.iter().map(key).collect(),
		diagnostics: e.diagnostics.iter().copied().collect(),
		store: store.state_for_test(),
	}
}

/// Builds the same scenario twice and compares old and new leave. Returns
/// whether the new leave took the fast path, and what it observed.
fn compare_observed(setup: impl Fn(&mut Executor, &mut Store)) -> (bool, Observed) {
	let (mut e0, mut s0) = (Executor::new().unwrap(), store());
	setup(&mut e0, &mut s0);
	let r0 = old_leave(&mut e0, &mut s0);
	let expected = observe(r0, &e0, &s0);
	let (mut e1, mut s1) = (Executor::new().unwrap(), store());
	setup(&mut e1, &mut s1);
	let before = leave_fast_hits_for_test();
	let r1 = e1.leave(&mut s1);
	let taken = leave_fast_hits_for_test() - before;
	assert!(taken <= 1);
	assert_eq!(observe(r1, &e1, &s1), expected);
	(taken == 1, expected)
}
fn compare(setup: impl Fn(&mut Executor, &mut Store)) -> bool {
	compare_observed(setup).0
}

#[test]
fn d13_all_scalar_windows_take_the_fast_path_exactly() {
	for len in 0..=9 {
		for prefix in [false, true] {
			let (taken, seen) = compare_observed(|e, s| {
				let mut base = 0;
				if prefix {
					// The caller's prefix, heap included, must survive untouched.
					let h = s.string("caller").unwrap();
					e.slots.try_push(h).unwrap();
					e.slots.try_push(Value::F64(-0.0)).unwrap();
					e.frames
						.try_push(Frame {
							callsite_pc: 99,
							..frame(0)
						})
						.unwrap();
					e.diagnostics.try_push(location(98)).unwrap();
					base = 2;
				}
				for v in &scalars()[..len] {
					e.slots.try_push(*v).unwrap();
				}
				e.frames
					.try_push(Frame {
						callsite_pc: 7,
						..frame(base)
					})
					.unwrap();
			});
			assert!(taken, "len {len} prefix {prefix}");
			if prefix {
				// The surviving caller keeps its callsite and the diagnostics stay.
				assert_eq!(seen.frames, [(0, 0, 0, None, 99)]);
				assert_eq!(seen.diagnostics, [location(98)]);
				assert_eq!(seen.slots.len(), 2);
			} else {
				assert!(seen.frames.is_empty() && seen.slots.is_empty());
			}
		}
	}
}

#[test]
fn d13_base_beyond_len_and_missing_frame_keep_the_old_outcome() {
	assert!(!compare(|e, _| {
		e.slots.try_push(Value::I64(1)).unwrap();
		e.frames.try_push(frame(3)).unwrap();
	}));
	assert!(!compare(|e, _| {
		e.slots.try_push(Value::I64(1)).unwrap();
	}));
}

#[test]
fn d13_heap_anywhere_in_the_window_uses_the_old_loop() {
	for at in 0..3 {
		for kind in 0..4 {
			let taken = compare(|e, s| {
				let heap = match kind {
					0 => s.string("text").unwrap(),
					1 => {
						let child = s.string("child").unwrap();
						let list = s.list(Type::Vec, &[child, child]).unwrap();
						s.release(child).unwrap();
						list
					}
					2 => {
						// Stale: retired before the leave, so release fails early.
						let v = s.string("stale").unwrap();
						s.release(v).unwrap();
						v
					}
					_ => {
						// Leased: a live iteration token holds a reader.
						let list = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
						let _iter = s.begin_iter(list).unwrap();
						list
					}
				};
				let mut window = [Value::I64(7), Value::Unit, Value::Bool(false)];
				window[at] = heap;
				for v in window {
					e.slots.try_push(v).unwrap();
				}
				e.frames.try_push(frame(0)).unwrap();
			});
			assert!(!taken, "at {at} kind {kind}");
		}
	}
}

#[test]
fn d13_malformed_lease_release_fails_identically() {
	assert!(!compare(|e, s| {
		let list = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let _iter = s.begin_iter(list).unwrap();
		s.zero_readers_for_test(list);
		e.slots.try_push(Value::I64(3)).unwrap();
		e.slots.try_push(list).unwrap();
		e.slots.try_push(Value::I64(4)).unwrap();
		e.frames.try_push(frame(0)).unwrap();
	}));
}

#[test]
fn d13_closed_store_uses_the_old_loop() {
	for len in 0..3 {
		let taken = compare(|e, s| {
			for v in &scalars()[..len] {
				e.slots.try_push(*v).unwrap();
			}
			e.frames.try_push(frame(0)).unwrap();
			s.close().unwrap();
		});
		// Closed + empty window: the old loop succeeds without any release.
		assert!(!taken, "len {len}");
	}
}

#[test]
fn d13_queued_work_uses_the_old_loop() {
	for malformed in [false, true] {
		for len in 0..3 {
			let taken = compare(|e, s| {
				let v = s.string("queued").unwrap();
				if malformed {
					s.release(v).unwrap();
				}
				s.push_work_for_test(v);
				for x in &scalars()[..len] {
					e.slots.try_push(*x).unwrap();
				}
				e.frames.try_push(frame(0)).unwrap();
			});
			assert!(!taken, "malformed {malformed} len {len}");
		}
	}
}

#[test]
fn d13_scalar_calls_take_the_fast_path_through_invoke() {
	// main(a): r = callee(a, a); return r. callee(x, y): t = x + y; return t.
	use crate::experimental_core::arithmetic::Binary;
	let mut main = Function::new(3, 1).unwrap();
	let (args_start, args_len) = main.arguments(&[0, 0]).unwrap();
	main.push(
		Op::Call {
			out: 1,
			function: 1,
			args_start,
			args_len,
		},
		location(0),
	)
	.unwrap();
	main.push(Op::Return { value: 1 }, location(1)).unwrap();
	let mut callee = Function::new(3, 2).unwrap();
	callee
		.push(
			Op::Binary {
				out: 2,
				op: Binary::Add,
				lhs: 0,
				rhs: 1,
			},
			location(2),
		)
		.unwrap();
	callee.push(Op::Return { value: 2 }, location(3)).unwrap();
	let mut b = Builder::default();
	b.push(main).unwrap();
	b.push(callee).unwrap();
	let code = b.finish().unwrap();
	let mut s = store();
	let mut e = Executor::new().unwrap();
	let arg = s.root_i64(20);
	let before = leave_fast_hits_for_test();
	let root = e.invoke_code(&code, &mut s, 0, &[&arg], 100).unwrap();
	assert_eq!(root.value, Value::I64(40));
	s.root_release(root).unwrap();
	s.root_release(arg).unwrap();
	// The callee's and main's frames are both all-scalar.
	assert_eq!(leave_fast_hits_for_test() - before, 2);
	assert!(e.is_idle());
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d13_slot_scalar_classification_matches_release_slot() {
	// release_slot on an open, empty store mutates nothing exactly for the
	// values is_slot_scalar accepts; a heap value is freed.
	let mut s = store();
	let heap = s.string("heap").unwrap();
	for value in scalars().into_iter().chain([heap]) {
		let before = s.state_for_test();
		assert!(s.scalar_release_is_noop());
		s.release_slot(value).unwrap();
		assert_eq!(Store::is_slot_scalar(&value), s.state_for_test() == before);
	}
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}
