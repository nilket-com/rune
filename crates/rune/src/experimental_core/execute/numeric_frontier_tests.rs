//! C6 source-free loop/native/fuel controls. Small integer fixtures are Miri eligible.
use super::*;
use crate::experimental_core::{arithmetic::Binary, code::Builder, heap::Limits};
use crate::function::fresh_numeric::Call;

fn at(n: u32) -> Location {
	Location {
		source: 0,
		revision: 1,
		start: n,
		end: n + 1,
	}
}
fn loop_program(call: Call) -> Program {
	let mut f = super::super::code::Function::new(8, 1).unwrap();
	let integer = call == Call::I64Abs;
	let (args_start, args_len) = f
		.arguments(if call.name().arity() == 2 {
			&[5, 6]
		} else {
			&[5]
		})
		.unwrap();
	let ops = [
		Op::Literal {
			out: 1,
			value: Value::I64(0),
		},
		Op::Literal {
			out: 2,
			value: if integer {
				Value::I64(0)
			} else {
				Value::F64(0.0)
			},
		},
		Op::Literal {
			out: 4,
			value: Value::I64(1),
		},
		Op::Literal {
			out: 5,
			value: if integer {
				Value::I64(-7)
			} else {
				Value::F64(2.0)
			},
		},
		Op::Literal {
			out: 6,
			value: Value::F64(3.0),
		},
		Op::Binary {
			out: 3,
			op: Binary::Lt,
			lhs: 1,
			rhs: 0,
		},
		Op::Branch {
			condition: 3,
			yes: 7,
			no: 11,
		},
		Op::Call {
			out: 7,
			function: call.id(),
			args_start,
			args_len,
		},
		Op::Binary {
			out: 2,
			op: Binary::Add,
			lhs: 2,
			rhs: 7,
		},
		Op::Binary {
			out: 1,
			op: Binary::Add,
			lhs: 1,
			rhs: 4,
		},
		Op::Jump { target: 5 },
		Op::Return { value: 2 },
	];
	for (pc, op) in ops.into_iter().enumerate() {
		f.push(op, at(pc as u32)).unwrap();
	}
	let mut b = Builder::default();
	b.push(f).unwrap();
	Program::for_diagnostics_test(b.finish().unwrap())
}
#[test]
fn explicit_receiver_copy_preserves_old_value_across_later_write() {
	let mut f = super::super::code::Function::new(4, 0).unwrap();
	let (args_start, args_len) = f.arguments(&[1, 2]).unwrap();
	for (pc, op) in [
		Op::Literal {
			out: 0,
			value: Value::F64(2.0),
		},
		Op::Copy { out: 1, value: 0 },
		Op::Literal {
			out: 0,
			value: Value::F64(3.0),
		},
		Op::Literal {
			out: 2,
			value: Value::F64(2.0),
		},
		Op::Call {
			out: 3,
			function: Call::F64Powf.id(),
			args_start,
			args_len,
		},
		Op::Return { value: 3 },
	]
	.into_iter()
	.enumerate()
	{
		f.push(op, at(pc as u32)).unwrap();
	}
	let mut b = Builder::default();
	b.push(f).unwrap();
	let p = Program::for_diagnostics_test(b.finish().unwrap());
	let mut store = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let root = e.invoke(&p, &mut store, 0, &[], 100).unwrap();
	assert_eq!(root.value, Value::F64(4.0));
	store.root_release(root).unwrap();
	assert!(e.is_idle());
	assert_eq!(store.live_entries(), 0);
}
#[test]
fn integer_numeric_loop_every_fuel_and_midloop_cancellation_are_balanced() {
	let p = loop_program(Call::I64Abs);
	let f = p.code.function(0).unwrap();
	let charges: std::vec::Vec<_> = f
		.instructions()
		.iter()
		.enumerate()
		.filter(|(_, i)| i.is_charged())
		.map(|(pc, _)| pc)
		.collect();
	assert_eq!(charges, [0, 5]);
	let mut store = Store::new(Limits::default()).unwrap();
	let limit = store.root_i64(32);
	let mut e = Executor::new().unwrap();
	// Entry once and loop condition 33 times, including its final false branch.
	for fuel in 0..=40 {
		let result = e.invoke(&p, &mut store, 0, &[&limit], fuel);
		if fuel < 34 {
			let fail = result.unwrap_err();
			assert_eq!(fail.error.kind, ErrorKind::FuelExhausted);
			assert_eq!(fail.error.location, Some(at(if fuel == 0 { 0 } else { 5 })));
			assert!(fail.cleanup_error.is_none());
		} else {
			let result = result.unwrap();
			assert_eq!(result.value, Value::I64(224));
			store.root_release(result).unwrap();
		}
		assert!(e.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
	cancel_at_charge_for_test(Some(2));
	let result = e.invoke(&p, &mut store, 0, &[&limit], 100);
	cancel_at_charge_for_test(None);
	let fail = result.unwrap_err();
	assert_eq!(fail.error.kind, ErrorKind::Cancelled);
	assert_eq!(fail.error.location, Some(at(5)));
	assert!(fail.cleanup_error.is_none());
	assert!(e.is_idle());
	let result = e.invoke(&p, &mut store, 0, &[&limit], 100).unwrap();
	assert_eq!(result.value, Value::I64(224));
	store.root_release(result).unwrap();
	store.root_release(limit).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}
#[test]
#[cfg(feature = "fresh-core-native")]
#[cfg_attr(
	miri,
	ignore = "native analysis proof; integer frontier runs separately under Miri"
)]
fn every_numeric_reserved_call_refuses_its_enclosing_native_region() {
	use super::super::region::{analyze, InlineReason, RegionRefusal};
	for call in [
		Call::MethodAbs,
		Call::MethodRound,
		Call::MethodPowf,
		Call::F64Abs,
		Call::F64Round,
		Call::F64Powf,
		Call::I64Abs,
	] {
		let p = loop_program(call);
		let regions = analyze(&p.code, 0).unwrap();
		let row = regions.iter().find(|(pc, _)| *pc == 5).unwrap();
		assert!(matches!(
			row.1,
			Err(RegionRefusal::Inline {
				pc: 7,
				reason: InlineReason::IntrinsicCall
			})
		));
		// No claim about other functions/regions: this pins the selected loop.
	}
}
#[test]
#[cfg(feature = "fresh-core-native")]
#[cfg_attr(
	miri,
	ignore = "native cache/analysis proof; integer frontier runs separately under Miri"
)]
fn hot_powf_loop_native_on_falls_back_with_identical_bits_and_no_mapping() {
	use super::super::region::cache::{cache_state_for_test, clear_for_test, enable_for_test};
	let p = loop_program(Call::F64Powf);
	let mut store = Store::new(Limits::default()).unwrap();
	let limit = store.root_i64(1000);
	let mut e = Executor::new().unwrap();
	clear_for_test();
	enable_for_test(false);
	let root = e.invoke(&p, &mut store, 0, &[&limit], 5000).unwrap();
	let expected = root.value;
	store.root_release(root).unwrap();
	assert_eq!(expected, Value::F64(8000.0));
	clear_for_test();
	enable_for_test(true);
	let result = e.invoke(&p, &mut store, 0, &[&limit], 5000);
	enable_for_test(false);
	let root = result.unwrap();
	let (Value::F64(expected), Value::F64(observed)) = (expected, root.value) else {
		panic!("float result")
	};
	assert_eq!(expected.to_bits(), observed.to_bits());
	store.root_release(root).unwrap();
	store.root_release(limit).unwrap();
	assert_eq!(
		cache_state_for_test().1,
		0,
		"selected fixture maps no native region"
	);
	assert_eq!(cache_state_for_test().3, 0);
	clear_for_test();
	assert!(e.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}
