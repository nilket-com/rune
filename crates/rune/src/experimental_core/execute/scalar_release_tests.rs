//! D4 private-site tests: unchanged D1 put/leave/Release behavior.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
	value::Type,
};

fn store() -> Store {
	Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap()
}
fn location(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}
fn scalar(kind: usize) -> Value {
	match kind {
		0 => Value::Unit,
		1 => Value::Bool(true),
		2 => Value::I64(-42),
		3 => Value::U64(u64::MAX),
		4 => Value::F64(-0.0),
		5 => Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
		6 => Value::Char('μ'),
		7 => Value::Type(Type::I64),
		8 => Value::Function { code: 0, id: 0 },
		_ => unreachable!(),
	}
}
fn code_for(value: Value) -> Code {
	let mut f = Function::new(2, 0).unwrap();
	f.push(Op::Literal { out: 0, value }, location(1)).unwrap();
	f.push(Op::Release { value: 0 }, location(2)).unwrap();
	f.push(
		Op::Literal {
			out: 1,
			value: Value::I64(42),
		},
		location(3),
	)
	.unwrap();
	f.push(Op::Return { value: 1 }, location(4)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}
#[test]
fn d4_all_scalars_pass_through_put_leave_and_explicit_release() {
	for kind in 0..9 {
		let mut store = store();
		let mut e = Executor::new().unwrap();
		let mut value = scalar(kind);
		// Builder stamps validated Function tokens with this code's identity.
		let code = code_for(value);
		if let Value::Function { id, .. } = value {
			value = Value::Function {
				code: code.id(),
				id,
			};
		}
		e.slots.try_push(value).unwrap();
		e.put(&mut store, 0, Value::Unit).unwrap();
		assert_eq!(e.slots[0], Value::Unit);
		e.put(&mut store, 0, value).unwrap();
		e.frames
			.try_push(Frame {
				function: 0,
				pc: 0,
				base: 0,
				out: None,
				callsite_pc: NO_CALLSITE,
			})
			.unwrap();
		e.leave(&mut store).unwrap();
		assert!(e.is_idle());
		for _ in 0..2 {
			let root = e.invoke_code(&code, &mut store, 0, &[], 100).unwrap();
			assert_eq!(root.value, Value::I64(42));
			store.root_release(root).unwrap();
			assert!(e.is_idle());
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		}
	}
}
#[test]
fn d4_put_and_leave_release_heap_mixtures_in_slot_order() {
	let mut store = store();
	let mut e = Executor::new().unwrap();
	let first = store.string("first").unwrap();
	let last = store.string("last").unwrap();
	e.slots.try_push(first).unwrap();
	e.slots.try_push(Value::I64(1)).unwrap();
	e.slots.try_push(last).unwrap();
	e.put(&mut store, 0, Value::Bool(false)).unwrap();
	assert_eq!(
		store.text(first).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 1);
	e.frames
		.try_push(Frame {
			function: 0,
			pc: 0,
			base: 0,
			out: None,
			callsite_pc: NO_CALLSITE,
		})
		.unwrap();
	e.leave(&mut store).unwrap();
	assert_eq!(store.text(last).unwrap_err().kind, ErrorKind::InvalidHandle);
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	assert!(e.is_idle());
}
#[test]
fn d4_closed_private_sites_keep_mutation_before_error_and_public_refusal() {
	let mut store = store();
	let mut e = Executor::new().unwrap();
	e.slots.try_push(Value::I64(1)).unwrap();
	store.close().unwrap();
	assert_eq!(
		e.put(&mut store, 0, Value::Bool(true)).unwrap_err().kind,
		ErrorKind::ClosedStore
	);
	assert_eq!(e.slots[0], Value::Bool(true));
	e.slots.try_push(Value::I64(2)).unwrap();
	e.frames
		.try_push(Frame {
			function: 0,
			pc: 0,
			base: 0,
			out: None,
			callsite_pc: NO_CALLSITE,
		})
		.unwrap();
	assert_eq!(
		e.leave(&mut store).unwrap_err().kind,
		ErrorKind::ClosedStore
	);
	assert!(e.frames.is_empty());
	assert_eq!(e.slots.as_slice(), &[Value::Bool(true)]);
	e.slots.clear();
	let code = code_for(Value::I64(9));
	let failure = e.invoke_code(&code, &mut store, 0, &[], 100).unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::ClosedStore);
	assert!(failure.error.location.is_none());
	assert!(failure.cleanup_error.is_none());
	assert!(e.is_idle());
}

/// M1: the D21 `Trace`/`Mark` acceptance effects are re-expressed as `Copy` (the same
/// retain into a slot) followed by `Print` of a label through the per-executor
/// `test_output` seam, so the ordered-effect, failure-location, release and reuse
/// semantics are kept without the removed acceptance opcodes.
#[test]
fn d4_mixed_explicit_release_trace_order_error_location_and_reuse() {
	use crate::experimental_core::arithmetic::Binary;
	for fail in [false, true] {
		let mut f = Function::new(7, 1).unwrap();
		let one = f.string("1").unwrap();
		let mark = f.string("m").unwrap();
		let mut ops = std::vec![
			Op::Literal {
				out: 1,
				value: Value::I64(1)
			},
			// was Trace { out: 2, label: 1, value: 0 }
			Op::Copy { out: 2, value: 0 },
			Op::String { out: 6, text: one },
			Op::Print { value: 6 },
			// was Mark { out: 3, value: 2 }
			Op::Copy { out: 3, value: 2 },
			Op::String { out: 6, text: mark },
			Op::Print { value: 6 },
			Op::Release { value: 2 },
			Op::Literal {
				out: 3,
				value: Value::I64(7)
			},
			// was Trace { out: 4, label: 1, value: 0 }
			Op::Copy { out: 4, value: 0 },
			Op::String { out: 6, text: one },
			Op::Print { value: 6 },
			Op::Release { value: 0 },
			Op::Literal {
				out: 5,
				value: Value::I64(if fail { 0 } else { 42 })
			},
		];
		if fail {
			ops.push(Op::Binary {
				out: 5,
				op: Binary::Div,
				lhs: 3,
				rhs: 5,
			});
		}
		ops.push(Op::Return { value: 5 });
		for (i, op) in ops.into_iter().enumerate() {
			f.push(op, location(i as u32 + 10)).unwrap();
		}
		let mut b = Builder::default();
		b.push(f).unwrap();
		let code = b.finish().unwrap();
		let mut store = store();
		let mut e = Executor::new().unwrap();
		e.test_output = Some(std::vec::Vec::new());
		for heap in [false, true] {
			let input = if heap {
				let v = store.string("input").unwrap();
				store.export(v)
			} else {
				store.root_i64(9)
			};
			let before = (store.live_entries(), store.payload_bytes());
			for _ in 0..2 {
				match e.invoke_code(&code, &mut store, 0, &[&input], 100) {
					Ok(root) => {
						assert!(!fail);
						assert_eq!(root.value, Value::I64(42));
						store.root_release(root).unwrap();
					}
					Err(failure) => {
						assert!(fail);
						assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
						assert_eq!(failure.error.location, Some(location(24)));
						assert!(failure.cleanup_error.is_none());
					}
				}
				// Every effect before the failure is kept, in program order.
				assert_eq!(
					e.test_output.replace(std::vec::Vec::new()).as_deref(),
					Some(&b"1\nm\n1\n"[..])
				);
				assert!(e.is_idle());
				assert_eq!((store.live_entries(), store.payload_bytes()), before);
			}
			store.root_release(input).unwrap();
			assert_eq!(store.live_entries(), 0);
		}
		store.close().unwrap();
		assert_eq!(
			e.invoke_code(&code, &mut store, 0, &[], 100)
				.unwrap_err()
				.error
				.kind,
			ErrorKind::ClosedStore
		);
		assert!(e.is_idle());
		assert_eq!(e.test_output.as_deref(), Some(&b""[..]));
	}
}
