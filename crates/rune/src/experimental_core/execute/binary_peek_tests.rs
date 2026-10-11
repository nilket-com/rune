//! D21 tests-first: production Binary/AssignBinary selection on mixed operand
//! classes. Every case runs a real Builder program through `invoke` (no test-side
//! selector), for all 16 operations, both instruction kinds, both operand
//! directions and output aliases. The expectation comes from an independent twin
//! store: `arithmetic::binary` for mixed pairs, and `append_string` + `retain` for
//! AssignBinary Add on a string target. These pass on D20 and are the oracle for
//! D21's borrowed i64 operands.
use super::*;
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function},
	heap::Limits,
	value::Type,
};
use std::{borrow::ToOwned, format, string::String};
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
/// One side of a mixed pair: never two I64 values.
#[derive(Clone, Copy, Debug)]
enum Class {
	I64(i64),
	U64(u64),
	F64(f64),
	Bool,
	Char,
	Unit,
	Text,
}
const OTHERS: [Class; 9] = [
	Class::U64(5),
	Class::U64(70),
	Class::U64(u64::MAX),
	Class::F64(2.5),
	Class::Bool,
	Class::Char,
	Class::Unit,
	Class::Text,
	Class::F64(0.0),
];
const INTS: [i64; 3] = [3, -1, 0];
fn loc() -> Location {
	Location {
		source: 21,
		revision: 1,
		start: 210,
		end: 211,
	}
}
fn store() -> Store {
	Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap()
}
fn value(store: &mut Store, class: Class) -> Value {
	match class {
		Class::I64(n) => Value::I64(n),
		Class::U64(n) => Value::U64(n),
		Class::F64(n) => Value::F64(n),
		Class::Bool => Value::Bool(true),
		Class::Char => Value::Char('x'),
		Class::Unit => Value::Unit,
		Class::Text => store.string("ab").unwrap(),
	}
}
/// Slots 0 and 1 are the operands; `out` is 0, 1 or the fresh slot 2.
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
/// What a value reads as after the run: text for a heap string, else the scalar.
#[derive(Debug, PartialEq)]
enum Seen {
	Scalar(Value),
	F64(u64),
	Text(String),
}
fn seen(store: &Store, v: Value) -> Seen {
	match v {
		Value::F64(n) => Seen::F64(n.to_bits()),
		Value::Heap(_) => Seen::Text(store.text(v).unwrap().to_owned()),
		_ => Seen::Scalar(v),
	}
}
/// The independent semantics on a twin store: the result (or error) and what
/// the two operands read as afterwards.
fn oracle(
	assign: bool,
	op: Binary,
	l: Class,
	r: Class,
) -> (core::result::Result<Seen, Error>, Seen, Seen) {
	let mut twin = store();
	let a = value(&mut twin, l);
	let b = value(&mut twin, r);
	let result = if assign
		&& op == Binary::Add
		&& matches!(a, Value::Heap(_))
		&& twin.tag(a).unwrap() == Type::String
	{
		twin.append_string(a, b).map(|()| {
			twin.retain(a).unwrap();
			a
		})
	} else {
		arithmetic::binary(op, a, b)
	};
	let result = result.map(|v| seen(&twin, v));
	(result, seen(&twin, a), seen(&twin, b))
}
fn run(assign: bool, op: Binary, l: Class, r: Class, out: u32) {
	let (want, want_l, want_r) = oracle(assign, op, l, r);
	let code = program(assign, op, out);
	let mut store = store();
	let a = value(&mut store, l);
	let b = value(&mut store, r);
	let ar = store.export(a);
	let br = store.export(b);
	let before = (store.live_entries(), store.payload_bytes());
	let mut e = Executor::new().unwrap();
	let case = format!("{assign} {op:?} {l:?} {r:?} out={out}");
	match (e.invoke_code(&code, &mut store, 0, &[&ar, &br], 100), want) {
		(Ok(root), Ok(want)) => {
			assert_eq!(seen(&store, root.value), want, "{case}");
			store.root_release(root).unwrap();
		}
		(Err(failure), Err(want)) => {
			assert_eq!(failure.error, want.at(loc()), "{case}");
			assert!(failure.cleanup_error.is_none(), "{case}");
			assert!(failure.call_chain.is_empty(), "{case}");
		}
		(got, want) => panic!(
			"{case}: result kind differs: {:?} vs {want:?}",
			got.map(|r| r.value)
		),
	}
	assert!(e.is_idle(), "{case}");
	// The operands read exactly as on the twin (an AssignBinary append mutates
	// the target in both); every executor root and slot reference is gone.
	assert_eq!(seen(&store, ar.value), want_l, "{case}");
	assert_eq!(seen(&store, br.value), want_r, "{case}");
	if !(assign && op == Binary::Add && matches!(l, Class::Text)) {
		assert_eq!(
			(store.live_entries(), store.payload_bytes()),
			before,
			"{case}"
		);
	}
	store.root_release(ar).unwrap();
	store.root_release(br).unwrap();
	assert_eq!(store.live_entries(), 0, "{case}");
	// The executor and store still run a valid i64 operation afterwards.
	let next = program(false, Binary::Add, 2);
	let x = store.root_i64(20);
	let y = store.root_i64(22);
	let root = e.invoke_code(&next, &mut store, 0, &[&x, &y], 100).unwrap();
	assert_eq!(root.value, Value::I64(42), "{case}");
	store.root_release(root).unwrap();
	assert!(e.is_idle(), "{case}");
}
#[test]
fn d21_mixed_classes_select_the_fallback_in_both_directions_and_kinds() {
	let mut count = 0;
	for assign in [false, true] {
		for op in OPS {
			for n in INTS {
				for other in OTHERS {
					run(assign, op, Class::I64(n), other, 2);
					run(assign, op, other, Class::I64(n), 2);
					count += 2;
				}
			}
		}
	}
	assert_eq!(count, 2 * 16 * 3 * 9 * 2);
}
#[test]
fn d21_mixed_classes_keep_output_alias_semantics() {
	for assign in [false, true] {
		for op in OPS {
			for other in [Class::U64(5), Class::F64(2.5), Class::Text, Class::Unit] {
				for out in [0, 1] {
					run(assign, op, Class::I64(3), other, out);
					run(assign, op, other, Class::I64(3), out);
				}
			}
		}
	}
}
#[test]
fn d21_excluded_operations_on_two_i64_values_fall_back_through_the_handler() {
	// BitAnd/BitOr/BitXor/Shl/Shr are not admitted by fast_i64 even for two I64
	// values; production must still reach the fallback with identical results and
	// errors (InvalidShift included), for both kinds and output aliases.
	for assign in [false, true] {
		for op in [
			Binary::BitAnd,
			Binary::BitOr,
			Binary::BitXor,
			Binary::Shl,
			Binary::Shr,
		] {
			for (a, b) in [(6, 3), (-1, 63), (1, 64), (5, -1), (i64::MIN, 1)] {
				for out in [2, 0, 1] {
					run(assign, op, Class::I64(a), Class::I64(b), out);
				}
			}
		}
	}
}
#[test]
fn d21_assign_add_string_target_with_an_i64_operand_in_both_directions() {
	// String target + I64: the append path's own result or error, the target
	// text and the entry count match the twin; I64 target + String: no append.
	for out in [2, 0, 1] {
		run(true, Binary::Add, Class::Text, Class::I64(7), out);
		run(true, Binary::Add, Class::I64(7), Class::Text, out);
		run(false, Binary::Add, Class::Text, Class::I64(7), out);
		run(false, Binary::Add, Class::I64(7), Class::Text, out);
	}
}
