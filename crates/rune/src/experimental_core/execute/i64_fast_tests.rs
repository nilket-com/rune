//! D5 candidate uses the same fast_i64 helper as both production arms.
use super::*;
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function},
	heap::Limits,
	value::Type,
};
const OPS: [Binary; 16] = [
	Binary::Add,
	Binary::Sub,
	Binary::Mul,
	Binary::Div,
	Binary::Rem,
	Binary::Eq,
	Binary::Ne,
	Binary::Lt,
	Binary::Le,
	Binary::Gt,
	Binary::Ge,
	Binary::BitAnd,
	Binary::BitOr,
	Binary::BitXor,
	Binary::Shl,
	Binary::Shr,
];
const NUMBERS: [i64; 9] = [
	i64::MIN,
	i64::MIN + 1,
	-2,
	-1,
	0,
	1,
	2,
	i64::MAX - 1,
	i64::MAX,
];
fn eligible(op: Binary, a: Value, b: Value) -> bool {
	let operation = match op {
		Binary::Add
		| Binary::Sub
		| Binary::Mul
		| Binary::Div
		| Binary::Rem
		| Binary::Eq
		| Binary::Ne
		| Binary::Lt
		| Binary::Le
		| Binary::Gt
		| Binary::Ge => true,
		Binary::BitAnd | Binary::BitOr | Binary::BitXor | Binary::Shl | Binary::Shr => false,
	};
	operation && matches!((a, b), (Value::I64(_), Value::I64(_)))
}
fn selected(store: &mut Store, assign: bool, op: Binary, a: Value, b: Value) -> Result<Value> {
	let result = fast_i64(op, a, b);
	assert_eq!(result.is_some(), eligible(op, a, b));
	if let Some(result) = result {
		return result;
	}
	if assign && op == Binary::Add && store.tag(a)? == Type::String {
		store.append_string(a, b)?;
		store.retain(a)?;
		Ok(a)
	} else {
		binary(store, op, a, b)
	}
}
fn loc() -> Location {
	Location {
		source: 3,
		revision: 7,
		start: 100,
		end: 103,
	}
}
fn store() -> Store {
	Store::new(Limits {
		entries: 32,
		bytes: 8192,
	})
	.unwrap()
}
fn program(assign: bool, op: Binary, out: u32) -> Code {
	let mut f = Function::new(3, 2).unwrap();
	let instruction = if assign {
		Op::AssignBinary {
			out,
			op,
			lhs: 0,
			rhs: 1,
		}
	} else {
		Op::Binary {
			out,
			op,
			lhs: 0,
			rhs: 1,
		}
	};
	f.push(instruction, loc()).unwrap();
	f.push(Op::Return { value: out }, loc()).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}
/// Compares arithmetic results. Rust does not guarantee the payload or sign of a NaN
/// produced by float arithmetic (Miri, for one, picks them non-deterministically), so two
/// NaNs are one class here. Every other float, including the signed zeros, must match
/// bit for bit, and a NaN never matches a non-NaN. This is for arithmetic results only;
/// store, copy and serialization tests keep exact bit preservation.
fn same(a: Value, b: Value) {
	match (a, b) {
		(Value::F64(a), Value::F64(b)) if a.is_nan() && b.is_nan() => {}
		(Value::F64(a), Value::F64(b)) => assert_eq!(a.to_bits(), b.to_bits()),
		_ => assert_eq!(a, b),
	}
}

#[test]
fn same_treats_nans_as_one_class_and_nothing_else() {
	let quiet = f64::from_bits(0x7ff8_0000_0000_0042);
	let negative = f64::from_bits(0xfff8_0000_0000_0000);
	same(Value::F64(quiet), Value::F64(negative));
	same(Value::F64(-0.0), Value::F64(-0.0));
	same(Value::F64(1.5), Value::F64(1.5));
	for (a, b) in [
		(quiet, 1.0),
		(1.0, quiet),
		(quiet, f64::INFINITY),
		(f64::NEG_INFINITY, negative),
		(0.0, -0.0),
		(-0.0, 0.0),
	] {
		let mismatch = std::panic::catch_unwind(|| same(Value::F64(a), Value::F64(b)));
		assert!(mismatch.is_err(), "{a:?} vs {b:?} must not match");
	}
}
fn run_case(assign: bool, op: Binary, a: Value, b: Value, out: u32, expected: Result<Value>) {
	let code = program(assign, op, out);
	let mut store = store();
	let mut e = Executor::new().unwrap();
	let ar = store.export(a);
	let br = store.export(b);
	let before = (store.live_entries(), store.payload_bytes());
	let result = e.invoke_code(&code, &mut store, 0, &[&ar, &br], 100);
	match (result, expected) {
		(Ok(root), Ok(want)) => {
			same(root.value, want);
			store.root_release(root).unwrap();
		}
		(Err(failure), Err(want)) => {
			assert_eq!(failure.error, want.at(loc()));
			assert!(failure.cleanup_error.is_none());
			assert!(failure.call_chain.is_empty());
		}
		_ => panic!("result kind differs: {assign} {op:?} {a:?} {b:?}"),
	}
	assert!(e.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	same(ar.value, a);
	same(br.value, b);
	store.root_release(ar).unwrap();
	store.root_release(br).unwrap();
	// The same executor can run a succeeding operation after every failure.
	let next = program(false, Binary::Add, 2);
	let a = store.root_i64(20);
	let b = store.root_i64(22);
	let root = e.invoke_code(&next, &mut store, 0, &[&a, &b], 100).unwrap();
	same(root.value, Value::I64(42));
	store.root_release(root).unwrap();
	store.root_release(a).unwrap();
	store.root_release(b).unwrap();
	assert!(e.is_idle());
}
#[test]
fn d5_all_ops_i64_boundary_matrix_runs_both_instruction_kinds() {
	let mut count = 0;
	for assign in [false, true] {
		for op in OPS {
			for a in NUMBERS {
				for b in NUMBERS {
					run_case(
						assign,
						op,
						Value::I64(a),
						Value::I64(b),
						2,
						arithmetic::binary(op, Value::I64(a), Value::I64(b)),
					);
					count += 1;
				}
			}
		}
	}
	assert_eq!(count, 2592);
}
#[test]
fn d5_output_aliases_preserve_operands_before_store() {
	for assign in [false, true] {
		for out in [0, 1] {
			for op in OPS {
				for (a, b) in [(i64::MAX, 1), (7, 2), (i64::MIN, -1)] {
					run_case(
						assign,
						op,
						Value::I64(a),
						Value::I64(b),
						out,
						arithmetic::binary(op, Value::I64(a), Value::I64(b)),
					);
				}
			}
		}
	}
}
#[test]
fn d5_error_selector_does_not_touch_scalar_or_heap_destination() {
	for assign in [false, true] {
		for op in [Binary::Add, Binary::Sub, Binary::Mul] {
			let (a, b) = match op {
				Binary::Sub => (i64::MIN, 1),
				_ => (i64::MAX, 2),
			};
			let mut store = store();
			let mut e = Executor::new().unwrap();
			let heap = store.string("destination").unwrap();
			e.slots.try_push(Value::I64(17)).unwrap();
			e.slots.try_push(heap).unwrap();
			let before = (store.live_entries(), store.payload_bytes());
			let result = selected(&mut store, assign, op, Value::I64(a), Value::I64(b));
			assert_eq!(
				result.unwrap_err(),
				arithmetic::binary(op, Value::I64(a), Value::I64(b)).unwrap_err()
			);
			assert_eq!(e.slots[0], Value::I64(17));
			assert_eq!(e.slots[1], heap);
			assert_eq!(store.text(heap).unwrap(), "destination");
			assert_eq!((store.live_entries(), store.payload_bytes()), before);
			store.release(heap).unwrap();
			e.slots.clear();
		}
	}
}
#[test]
fn d5_non_i64_scalar_classes_keep_fallback_results() {
	let pairs = [
		(Value::U64(7), Value::U64(2)),
		(Value::F64(-0.0), Value::F64(0.0)),
		(
			Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
			Value::F64(1.0),
		),
		(Value::I64(1), Value::U64(u64::MAX)),
		(Value::U64(1), Value::I64(-1)),
		(Value::I64(7), Value::U64(2)),
		(Value::U64(7), Value::I64(2)),
		(Value::Bool(true), Value::Bool(false)),
		(Value::Char('a'), Value::Char('b')),
		(Value::Unit, Value::Unit),
		(Value::Type(Type::I64), Value::Type(Type::I64)),
		(
			Value::Function { code: 1, id: 0 },
			Value::Function { code: 1, id: 0 },
		),
	];
	for assign in [false, true] {
		for op in OPS {
			for (a, b) in pairs {
				assert!(!eligible(op, a, b));
				let mut store = store();
				let expected = if assign && op == Binary::Add {
					match store.tag(a) {
						Err(e) => Err(e),
						Ok(_) => arithmetic::binary(op, a, b),
					}
				} else {
					arithmetic::binary(op, a, b)
				};
				match (selected(&mut store, assign, op, a, b), expected) {
					(Ok(a), Ok(b)) => same(a, b),
					(Err(a), Err(b)) => assert_eq!(a, b),
					_ => panic!("fallback mismatch"),
				}
				run_case(assign, op, a, b, 2, expected);
			}
		}
	}
}
#[test]
fn d5_heap_strings_keep_binary_and_compound_alias_effects() {
	for assign in [false, true] {
		let mut store = store();
		let a = store.string("a").unwrap();
		let b = store.string("b").unwrap();
		assert!(!eligible(Binary::Add, a, b));
		let code = program(assign, Binary::Add, 0);
		let ar = store.export(a);
		let br = store.export(b);
		let mut e = Executor::new().unwrap();
		let root = e
			.invoke_code(&code, &mut store, 0, &[&ar, &br], 100)
			.unwrap();
		assert_eq!(store.text(root.value).unwrap(), "ab");
		assert_eq!(
			store.text(ar.value).unwrap(),
			if assign { "ab" } else { "a" }
		);
		assert_eq!(store.text(br.value).unwrap(), "b");
		assert!(e.is_idle());
		store.root_release(root).unwrap();
		store.root_release(ar).unwrap();
		store.root_release(br).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
	for op in [
		Binary::Eq,
		Binary::Ne,
		Binary::Lt,
		Binary::Le,
		Binary::Gt,
		Binary::Ge,
	] {
		let mut store = store();
		let a = store.string("a").unwrap();
		let b = store.string("b").unwrap();
		same(
			selected(&mut store, false, op, a, b).unwrap(),
			arithmetic::compare(op, false, true, false).unwrap(),
		);
		store.release(a).unwrap();
		store.release(b).unwrap();
	}
}
#[test]
fn d5_invalid_heap_fallback_errors_keep_their_precedence() {
	for assign in [false, true] {
		let mut store = store();
		let stale = store.string("stale").unwrap();
		store.release(stale).unwrap();
		let live = store.string("live").unwrap();
		assert!(!eligible(Binary::Add, stale, live));
		assert_eq!(
			selected(&mut store, assign, Binary::Add, stale, live)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidHandle
		);
		store.invalidate(live).unwrap();
		assert_eq!(
			selected(&mut store, assign, Binary::Add, live, live)
				.unwrap_err()
				.kind,
			ErrorKind::AccessConflict
		);
		store.release(live).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}
#[test]
fn d7_admission_set_is_exactly_eleven_ops_on_two_i64_values() {
	let mut admitted = 0;
	for op in OPS {
		assert_eq!(
			fast_i64(op, Value::I64(1), Value::I64(2)).is_some(),
			eligible(op, Value::I64(1), Value::I64(2))
		);
		assert!(fast_i64(op, Value::I64(1), Value::U64(2)).is_none());
		assert!(fast_i64(op, Value::U64(1), Value::I64(2)).is_none());
		if eligible(op, Value::I64(1), Value::I64(2)) {
			admitted += 1;
		}
		assert!(!eligible(op, Value::I64(1), Value::U64(2)));
		assert!(!eligible(op, Value::U64(1), Value::I64(2)));
	}
	assert_eq!(admitted, 11);
}

// D7 expectations were added on unchanged D6. The 2592-row matrix
// expectations remain unchanged; D7 expands only admission to eleven.
fn d7_expected(n: core::result::Result<i64, ErrorKind>) -> Result<Value> {
	n.map(Value::I64)
		.map_err(|kind| Error::new(kind, "primitive operation"))
}
fn d7_program(assign: bool, op: Binary, out: u32, lhs: u32, rhs: u32, parameters: u32) -> Code {
	let mut f = Function::new(3, parameters).unwrap();
	let instruction = if assign {
		Op::AssignBinary { out, op, lhs, rhs }
	} else {
		Op::Binary { out, op, lhs, rhs }
	};
	f.push(instruction, loc()).unwrap();
	f.push(Op::Return { value: out }, loc()).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}
#[test]
fn d7_explicit_signed_divrem_values_errors_and_output_aliases() {
	use ErrorKind::{DivisionByZero as Zero, Overflow};
	// Explicit oracle: quotient truncates toward zero, remainder follows lhs.
	let cases = [
		(7, 3, Ok(2), Ok(1)),
		(-7, 3, Ok(-2), Ok(-1)),
		(7, -3, Ok(-2), Ok(1)),
		(-7, -3, Ok(2), Ok(-1)),
		(0, 3, Ok(0), Ok(0)),
		(i64::MIN, 1, Ok(i64::MIN), Ok(0)),
		(i64::MAX, 1, Ok(i64::MAX), Ok(0)),
		(i64::MIN, -1, Err(Overflow), Err(Overflow)),
		(i64::MIN, 0, Err(Zero), Err(Zero)),
		(0, 0, Err(Zero), Err(Zero)),
		(7, 0, Err(Zero), Err(Zero)),
	];
	for assign in [false, true] {
		for out in [0, 1, 2] {
			for (a, b, div, rem) in cases {
				for (op, expected) in [(Binary::Div, div), (Binary::Rem, rem)] {
					run_case(
						assign,
						op,
						Value::I64(a),
						Value::I64(b),
						out,
						d7_expected(expected),
					);
				}
			}
		}
	}
}
#[test]
fn d7_identical_operand_slot_and_output_aliases() {
	for assign in [false, true] {
		for out in [0, 1, 2] {
			for n in [i64::MIN, -7, 0, 7, i64::MAX] {
				for op in [Binary::Div, Binary::Rem] {
					let code = d7_program(assign, op, out, 0, 0, 2);
					let mut store = store();
					let a = store.root_i64(n);
					let b = store.root_i64(99);
					let mut executor = Executor::new().unwrap();
					let result = executor.invoke_code(&code, &mut store, 0, &[&a, &b], 100);
					if n == 0 {
						let failure = result.unwrap_err();
						assert_eq!(
							failure.error,
							Error::new(ErrorKind::DivisionByZero, "primitive operation").at(loc())
						);
						assert!(failure.cleanup_error.is_none() && failure.call_chain.is_empty());
					} else {
						let root = result.unwrap();
						same(
							root.value,
							Value::I64(if op == Binary::Div { 1 } else { 0 }),
						);
						store.root_release(root).unwrap();
					}
					assert!(executor.is_idle());
					same(a.value, Value::I64(n));
					same(b.value, Value::I64(99));
					store.root_release(a).unwrap();
					store.root_release(b).unwrap();
					assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
				}
			}
		}
	}
}
#[test]
fn d7_divrem_non_i64_fallback_and_ieee_zero_classes() {
	let pairs = [
		(Value::I64(-7), Value::U64(3)),
		(Value::U64(7), Value::I64(3)),
		(Value::I64(7), Value::U64(u64::MAX)),
		(Value::U64(7), Value::I64(-3)),
		(Value::I64(7), Value::U64(0)),
		(Value::U64(7), Value::I64(0)),
		(Value::U64(7), Value::U64(0)),
		(Value::F64(7.0), Value::F64(0.0)),
		(Value::F64(7.0), Value::F64(-0.0)),
		(Value::F64(0.0), Value::F64(0.0)),
		(Value::F64(f64::INFINITY), Value::F64(3.0)),
		(Value::F64(7.0), Value::F64(f64::INFINITY)),
		(
			Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
			Value::F64(3.0),
		),
		(Value::Bool(true), Value::I64(3)),
		(Value::I64(7), Value::Unit),
		(Value::Type(Type::I64), Value::I64(3)),
	];
	for assign in [false, true] {
		for op in [Binary::Div, Binary::Rem] {
			for (a, b) in pairs {
				assert!(fast_i64(op, a, b).is_none());
				let expected = arithmetic::binary(op, a, b);
				// Exact float bits, except that any two NaNs are one class (see `same`).
				run_case(assign, op, a, b, 2, expected);
			}
		}
	}
	for (divisor, sign) in [(0.0, false), (-0.0, true)] {
		let mut store = store();
		let Value::F64(div) = selected(
			&mut store,
			false,
			Binary::Div,
			Value::F64(7.0),
			Value::F64(divisor),
		)
		.unwrap() else {
			panic!("float result");
		};
		assert!(div.is_infinite() && div.is_sign_negative() == sign);
		let Value::F64(rem) = selected(
			&mut store,
			false,
			Binary::Rem,
			Value::F64(7.0),
			Value::F64(divisor),
		)
		.unwrap() else {
			panic!("float result");
		};
		assert!(rem.is_nan());
	}
}
#[test]
fn d7_production_heap_type_errors_preserve_roots_and_release_frame() {
	for assign in [false, true] {
		for op in [Binary::Div, Binary::Rem] {
			for heap_rhs in [false, true] {
				let mut store = store();
				let av = store.string("lhs").unwrap();
				let bv = if heap_rhs {
					store.string("rhs").unwrap()
				} else {
					Value::I64(0)
				};
				let dv = store.string("old destination").unwrap();
				let a = store.export(av);
				let b = store.export(bv);
				let destination = store.export(dv);
				let before = (store.live_entries(), store.payload_bytes());
				let code = d7_program(assign, op, 2, 0, 1, 3);
				let mut executor = Executor::new().unwrap();
				let failure = executor
					.invoke_code(&code, &mut store, 0, &[&a, &b, &destination], 100)
					.unwrap_err();
				assert_eq!(
					failure.error,
					Error::new(ErrorKind::Type, "primitive operation").at(loc())
				);
				assert!(failure.cleanup_error.is_none() && failure.call_chain.is_empty());
				assert!(executor.is_idle());
				assert_eq!((store.live_entries(), store.payload_bytes()), before);
				assert_eq!(store.text(destination.value).unwrap(), "old destination");
				assert_eq!(store.text(a.value).unwrap(), "lhs");
				if heap_rhs {
					assert_eq!(store.text(b.value).unwrap(), "rhs");
				}
				// This proves observable root preservation and production frame cleanup;
				// it does not observe the discarded destination slot mid-failure.
				let good = program(false, Binary::Add, 2);
				let x = store.root_i64(20);
				let y = store.root_i64(22);
				let result = executor
					.invoke_code(&good, &mut store, 0, &[&x, &y], 100)
					.unwrap();
				same(result.value, Value::I64(42));
				for root in [result, x, y, a, b, destination] {
					store.root_release(root).unwrap();
				}
				assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
			}
		}
	}
}
#[test]
fn d7_nested_failure_locations_call_chain_cleanup_and_reuse() {
	for assign in [false, true] {
		for op in [Binary::Div, Binary::Rem] {
			for (a, b, kind) in [
				(i64::MIN, -1, ErrorKind::Overflow),
				(i64::MIN, 0, ErrorKind::DivisionByZero),
			] {
				let callsite = Location {
					start: 10,
					end: 13,
					..loc()
				};
				let mut caller = Function::new(3, 2).unwrap();
				let (args_start, args_len) = caller.arguments(&[0, 1]).unwrap();
				caller
					.push(
						Op::Call {
							out: 2,
							function: 1,
							args_start,
							args_len,
						},
						callsite,
					)
					.unwrap();
				caller.push(Op::Return { value: 2 }, callsite).unwrap();
				let mut callee = Function::new(3, 2).unwrap();
				callee
					.push(
						if assign {
							Op::AssignBinary {
								out: 2,
								op,
								lhs: 0,
								rhs: 1,
							}
						} else {
							Op::Binary {
								out: 2,
								op,
								lhs: 0,
								rhs: 1,
							}
						},
						loc(),
					)
					.unwrap();
				callee.push(Op::Return { value: 2 }, loc()).unwrap();
				let mut builder = Builder::default();
				builder.push(caller).unwrap();
				builder.push(callee).unwrap();
				let code = builder.finish().unwrap();
				let mut store = store();
				let ar = store.root_i64(a);
				let br = store.root_i64(b);
				let mut executor = Executor::new().unwrap();
				let failure = executor
					.invoke_code(&code, &mut store, 0, &[&ar, &br], 100)
					.unwrap_err();
				assert_eq!(
					failure.error,
					Error::new(kind, "primitive operation").at(loc())
				);
				assert_eq!(&failure.call_chain[..], &[callsite]);
				assert!(failure.cleanup_error.is_none() && executor.is_idle());
				store.root_release(ar).unwrap();
				store.root_release(br).unwrap();
				let x = store.root_i64(7);
				let y = store.root_i64(3);
				let result = executor
					.invoke_code(&code, &mut store, 0, &[&x, &y], 100)
					.unwrap();
				same(
					result.value,
					Value::I64(if op == Binary::Div { 2 } else { 1 }),
				);
				for root in [result, x, y] {
					store.root_release(root).unwrap();
				}
				assert!(executor.is_idle());
				assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
			}
		}
	}
}
#[test]
fn d7_invalid_operand_or_output_is_refused_before_execution() {
	for assign in [false, true] {
		for op in [Binary::Div, Binary::Rem] {
			for (out, lhs, rhs) in [(3, 0, 1), (2, 3, 1), (2, 0, 3)] {
				let mut f = Function::new(3, 2).unwrap();
				f.push(
					if assign {
						Op::AssignBinary { out, op, lhs, rhs }
					} else {
						Op::Binary { out, op, lhs, rhs }
					},
					loc(),
				)
				.unwrap();
				f.push(Op::Return { value: 2 }, loc()).unwrap();
				let mut builder = Builder::default();
				builder.push(f).unwrap();
				assert_eq!(builder.finish().err().unwrap().kind, ErrorKind::InvalidCode);
			}
		}
	}
	// Validated Code cannot express invalid lhs/rhs/out at execution. No new
	// corruption hook is added; runtime error-order claims are not made here.
}
