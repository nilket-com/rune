//! C6 cold-call controls: no source compilation or host Context construction.
use super::*;
use crate::experimental_core::code::{Builder, FIRST_INTRINSIC_ID};
use crate::experimental_core::heap::Limits;
use crate::function::fresh_numeric::Call;

fn loc() -> Location {
	Location {
		source: 0,
		revision: 1,
		start: 10,
		end: 20,
	}
}
fn program(call: Call, out: u32) -> Program {
	let n = call.name().arity();
	let mut f = super::super::code::Function::new(n + 1, n).unwrap();
	let slots: std::vec::Vec<_> = (0..n).collect();
	let (args_start, args_len) = f.arguments(&slots).unwrap();
	f.push(
		Op::Call {
			out,
			function: call.id(),
			args_start,
			args_len,
		},
		loc(),
	)
	.unwrap();
	f.push(Op::Return { value: out }, loc()).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	Program::for_diagnostics_test(b.finish().unwrap())
}
#[test]
fn numeric_intrinsic_invalid_metadata_is_fail_closed_without_mutation() {
	let p = program(Call::I64Abs, 1);
	let c = &p.code;
	let f = c.function(0).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	e.slots.try_push(Value::I64(-7)).unwrap();
	e.slots.try_push(Value::I64(42)).unwrap();
	for (base, out, id, start, len) in [
		(0, 1, FIRST_INTRINSIC_ID + 0xff, 0, 1),
		(0, 1, Call::I64Abs.id(), 0, 0),
		(0, 1, Call::I64Abs.id(), 0, 2),
		(0, 1, Call::I64Abs.id(), u32::MAX, 1),
		(usize::MAX, 1, Call::I64Abs.id(), 0, 1),
		(0, u32::MAX, Call::I64Abs.id(), 0, 1),
	] {
		let before = store.state_for_test();
		assert_eq!(
			e.intrinsic(c, &mut store, f, base, out, id, start, len)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidCode
		);
		assert_eq!(e.slots.as_slice(), &[Value::I64(-7), Value::I64(42)]);
		assert_eq!(store.state_for_test(), before);
	}
	let mut malformed = super::super::code::Function::new(2, 1).unwrap();
	malformed.arguments(&[99]).unwrap();
	assert_eq!(
		e.intrinsic(c, &mut store, &malformed, 0, 1, Call::I64Abs.id(), 0, 1)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
	e.slots.clear();
	assert!(e.is_idle());
	assert_eq!(store.live_entries(), 0);
}
// C6 Miri ruling: powf exactness is native-only; Miri perturbs transcendental results.
#[cfg(not(miri))]
#[test]
fn numeric_calls_read_inputs_before_writing_aliasing_destinations() {
	for out in [0, 1, 2] {
		let p = program(Call::F64Powf, out);
		let mut store = Store::new(Limits::default()).unwrap();
		let x = store.root_f64(2.0);
		let y = store.root_f64(3.0);
		let mut e = Executor::new().unwrap();
		let root = e.invoke(&p, &mut store, 0, &[&x, &y], 100).unwrap();
		assert_eq!(
			store.root_scalar(&root).unwrap(),
			Some(super::super::value::Scalar::F64(8.0))
		);
		assert_eq!(
			store.root_scalar(&x).unwrap(),
			Some(super::super::value::Scalar::F64(2.0))
		);
		assert_eq!(
			store.root_scalar(&y).unwrap(),
			Some(super::super::value::Scalar::F64(3.0))
		);
		store.root_release(root).unwrap();
		store.root_release(x).unwrap();
		store.root_release(y).unwrap();
		assert!(e.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}

// C6 Miri ruling: use exact abs operations to check the same three destinations.
#[cfg(miri)]
#[test]
fn numeric_calls_read_inputs_before_writing_aliasing_destinations() {
	for out in [0, 1, 2] {
		for call in [Call::F64Abs, Call::I64Abs] {
			let mut f = super::super::code::Function::new(3, 1).unwrap();
			let (args_start, args_len) = f.arguments(&[0]).unwrap();
			f.push(
				Op::Call {
					out,
					function: call.id(),
					args_start,
					args_len,
				},
				loc(),
			)
			.unwrap();
			f.push(Op::Return { value: out }, loc()).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			let p = Program::for_diagnostics_test(b.finish().unwrap());
			let mut store = Store::new(Limits::default()).unwrap();
			let (x, before, expected) = if call == Call::F64Abs {
				(
					store.root_f64(-2.0),
					super::super::value::Scalar::F64(-2.0),
					super::super::value::Scalar::F64(2.0),
				)
			} else {
				(
					store.root_i64(-7),
					super::super::value::Scalar::I64(-7),
					super::super::value::Scalar::I64(7),
				)
			};
			let mut e = Executor::new().unwrap();
			let root = e.invoke(&p, &mut store, 0, &[&x], 100).unwrap();
			assert_eq!(store.root_scalar(&root).unwrap(), Some(expected));
			assert_eq!(store.root_scalar(&x).unwrap(), Some(before));
			store.root_release(root).unwrap();
			store.root_release(x).unwrap();
			assert!(e.is_idle());
			assert_eq!(store.live_entries(), 0);
			assert_eq!(store.payload_bytes(), 0);
		}
	}
}

#[test]
fn numeric_failures_release_executor_ownership_and_allow_reuse() {
	let mut store = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	for (call, value, kind, arg) in [
		(Call::F64Abs, Value::Bool(true), ErrorKind::Type, Some(0)),
		(Call::I64Abs, Value::Bool(true), ErrorKind::Type, Some(0)),
		(
			Call::I64Abs,
			Value::U64(u64::MAX),
			ErrorKind::IntegerCoercion,
			Some(0),
		),
		(
			Call::I64Abs,
			Value::I64(i64::MIN),
			ErrorKind::Overflow,
			None,
		),
	] {
		let p = program(call, 1);
		// Scalar inputs carry no heap ownership.
		let root = match value {
			Value::Bool(v) => store.root_bool(v),
			Value::U64(v) => store.root_u64(v),
			Value::I64(v) => store.root_i64(v),
			_ => panic!("fixture scalar"),
		};
		let fail = e.invoke(&p, &mut store, 0, &[&root], 100).unwrap_err();
		assert_eq!(fail.error.kind, kind);
		assert_eq!(fail.error.detail.numeric_argument().map(|(_, a)| a), arg);
		assert!(fail.cleanup_error.is_none());
		assert!(e.is_idle());
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		reuse(&mut e, &mut store);
	}
	for call in [Call::MethodAbs, Call::F64Abs] {
		let p = program(call, 1);
		let root = store.root_string("wrong kind").unwrap();
		let before = store.state_for_test();
		let fail = e.invoke(&p, &mut store, 0, &[&root], 100).unwrap_err();
		assert_eq!(fail.error.kind, ErrorKind::Type);
		assert!(fail.cleanup_error.is_none());
		assert!(e.is_idle());
		assert_eq!(store.state_for_test(), before);
		if call == Call::MethodAbs {
			assert!(fail.missing_instance_function().is_some());
		} else {
			assert_eq!(
				fail.error.detail.numeric_argument().map(|(_, a)| a),
				Some(0)
			);
		}
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		reuse(&mut e, &mut store);
	}
}
fn reuse(e: &mut Executor, store: &mut Store) {
	let p = program(Call::I64Abs, 1);
	let root = store.root_i64(-7);
	let result = e.invoke(&p, store, 0, &[&root], 100).unwrap();
	assert_eq!(result.value, Value::I64(7));
	store.root_release(result).unwrap();
	store.root_release(root).unwrap();
	assert!(e.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}
#[test]
fn numeric_fuel_and_precall_cancellation_are_atomic_and_reusable() {
	let p = program(Call::I64Abs, 1);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let root = store.root_i64(-7);
	let mut succeeded = false;
	for fuel in 0..=8 {
		match e.invoke(&p, &mut store, 0, &[&root], fuel) {
			Ok(result) => {
				succeeded = true;
				assert_eq!(result.value, Value::I64(7));
				store.root_release(result).unwrap();
			}
			Err(fail) => {
				assert!(!succeeded);
				assert_eq!(fail.error.kind, ErrorKind::FuelExhausted);
				assert!(fail.cleanup_error.is_none());
			}
		}
		assert!(e.is_idle());
		assert_eq!(store.live_entries(), 0);
	}
	assert!(succeeded);
	e.request_cancel();
	let fail = e.invoke(&p, &mut store, 0, &[&root], 100).unwrap_err();
	assert_eq!(fail.error.kind, ErrorKind::Cancelled);
	assert_eq!(fail.error.location, Some(loc()));
	assert!(fail.cleanup_error.is_none());
	store.root_release(root).unwrap();
	reuse(&mut e, &mut store);
}
