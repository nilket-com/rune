//! D1 tests-first on unchanged D2. Source-only until review.
use crate::experimental_core::{
	arithmetic::{Binary, Unary},
	code::{Builder, Function, Op},
	error::{ErrorKind, Location},
	value::{Type, Value},
};

fn location() -> Location {
	Location {
		source: 1,
		revision: 7,
		start: 10,
		end: 11,
	}
}

fn variant_index(op: Op) -> usize {
	match op {
		Op::Template { .. } => 0,
		Op::Print { .. } => 1,
		Op::TupleField { .. } => 2,
		Op::SetTupleField { .. } => 3,
		Op::Record { .. } => 4,
		Op::Field { .. } => 5,
		Op::SetField { .. } => 6,
		Op::Variant { .. } => 7,
		Op::Try { .. } => 8,
		Op::CallValue { .. } => 9,
		Op::Range { .. } => 10,
		Op::IterStart { .. } => 11,
		Op::IterNext { .. } => 12,
		Op::String { .. } => 13,
		Op::Clone { .. } => 14,
		Op::AssignBinary { .. } => 15,
		Op::Literal { .. } => 16,
		Op::Copy { .. } => 17,
		Op::Release { .. } => 18,
		Op::Unary { .. } => 19,
		Op::Binary { .. } => 20,
		Op::Cast { .. } => 21,
		Op::Is { .. } => 22,
		Op::Jump { .. } => 23,
		Op::Branch { .. } => 24,
		Op::Call { .. } => 25,
		Op::Return { .. } => 26,
		Op::List { .. } => 27,
		Op::Get { .. } => 28,
		Op::Set { .. } => 29,
		Op::Len { .. } => 30,
		Op::Push { .. } => 31,
		Op::Drop { .. } => 32,
	}
}

fn same(a: Op, b: Op) {
	match (a, b) {
		(
			Op::Template {
				out: a_out,
				args_start: a_args_start,
				args_len: a_args_len,
			},
			Op::Template {
				out: b_out,
				args_start: b_args_start,
				args_len: b_args_len,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_args_start, b_args_start);
			assert_eq!(a_args_len, b_args_len);
		}
		(Op::Print { value: a_value }, Op::Print { value: b_value }) => {
			assert_eq!(a_value, b_value);
		}
		(
			Op::TupleField {
				out: a_out,
				value: a_value,
				index: a_index,
			},
			Op::TupleField {
				out: b_out,
				value: b_value,
				index: b_index,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_index, b_index);
		}
		(
			Op::SetTupleField {
				value: a_value,
				index: a_index,
				new: a_new,
			},
			Op::SetTupleField {
				value: b_value,
				index: b_index,
				new: b_new,
			},
		) => {
			assert_eq!(a_value, b_value);
			assert_eq!(a_index, b_index);
			assert_eq!(a_new, b_new);
		}
		(
			Op::Record {
				out: a_out,
				tag: a_tag,
				args_start: a_args_start,
				args_len: a_args_len,
			},
			Op::Record {
				out: b_out,
				tag: b_tag,
				args_start: b_args_start,
				args_len: b_args_len,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_tag, b_tag);
			assert_eq!(a_args_start, b_args_start);
			assert_eq!(a_args_len, b_args_len);
		}
		(
			Op::Field {
				out: a_out,
				value: a_value,
				key: a_key,
			},
			Op::Field {
				out: b_out,
				value: b_value,
				key: b_key,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_key, b_key);
		}
		(
			Op::SetField {
				value: a_value,
				key: a_key,
				new: a_new,
			},
			Op::SetField {
				value: b_value,
				key: b_key,
				new: b_new,
			},
		) => {
			assert_eq!(a_value, b_value);
			assert_eq!(a_key, b_key);
			assert_eq!(a_new, b_new);
		}
		(
			Op::Variant {
				out: a_out,
				tag: a_tag,
				success: a_success,
				payload: a_payload,
			},
			Op::Variant {
				out: b_out,
				tag: b_tag,
				success: b_success,
				payload: b_payload,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_tag, b_tag);
			assert_eq!(a_success, b_success);
			assert_eq!(a_payload, b_payload);
		}
		(
			Op::Try {
				out: a_out,
				value: a_value,
			},
			Op::Try {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
		}
		(
			Op::CallValue {
				out: a_out,
				value: a_value,
				args_start: a_args_start,
				args_len: a_args_len,
			},
			Op::CallValue {
				out: b_out,
				value: b_value,
				args_start: b_args_start,
				args_len: b_args_len,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_args_start, b_args_start);
			assert_eq!(a_args_len, b_args_len);
		}
		(
			Op::Range {
				out: a_out,
				start: a_start,
				end: a_end,
			},
			Op::Range {
				out: b_out,
				start: b_start,
				end: b_end,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_start, b_start);
			assert_eq!(a_end, b_end);
		}
		(
			Op::IterStart {
				out: a_out,
				value: a_value,
			},
			Op::IterStart {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
		}
		(
			Op::IterNext {
				out: a_out,
				iterator: a_iterator,
				none: a_none,
			},
			Op::IterNext {
				out: b_out,
				iterator: b_iterator,
				none: b_none,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_iterator, b_iterator);
			assert_eq!(a_none, b_none);
		}
		(
			Op::String {
				out: a_out,
				text: a_text,
			},
			Op::String {
				out: b_out,
				text: b_text,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_text, b_text);
		}
		(
			Op::Clone {
				out: a_out,
				value: a_value,
			},
			Op::Clone {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
		}
		(
			Op::AssignBinary {
				out: a_out,
				op: a_op,
				lhs: a_lhs,
				rhs: a_rhs,
			},
			Op::AssignBinary {
				out: b_out,
				op: b_op,
				lhs: b_lhs,
				rhs: b_rhs,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_op, b_op);
			assert_eq!(a_lhs, b_lhs);
			assert_eq!(a_rhs, b_rhs);
		}
		(
			Op::Literal {
				out: a_out,
				value: a_value,
			},
			Op::Literal {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			same_value(a_value, b_value);
		}
		(
			Op::Copy {
				out: a_out,
				value: a_value,
			},
			Op::Copy {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
		}
		(Op::Release { value: a_value }, Op::Release { value: b_value }) => {
			assert_eq!(a_value, b_value);
		}
		(
			Op::Unary {
				out: a_out,
				op: a_op,
				value: a_value,
			},
			Op::Unary {
				out: b_out,
				op: b_op,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_op, b_op);
			assert_eq!(a_value, b_value);
		}
		(
			Op::Binary {
				out: a_out,
				op: a_op,
				lhs: a_lhs,
				rhs: a_rhs,
			},
			Op::Binary {
				out: b_out,
				op: b_op,
				lhs: b_lhs,
				rhs: b_rhs,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_op, b_op);
			assert_eq!(a_lhs, b_lhs);
			assert_eq!(a_rhs, b_rhs);
		}
		(
			Op::Cast {
				out: a_out,
				value: a_value,
				tag: a_tag,
			},
			Op::Cast {
				out: b_out,
				value: b_value,
				tag: b_tag,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_tag, b_tag);
		}
		(
			Op::Is {
				out: a_out,
				value: a_value,
				tag: a_tag,
				negate: a_negate,
			},
			Op::Is {
				out: b_out,
				value: b_value,
				tag: b_tag,
				negate: b_negate,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_tag, b_tag);
			assert_eq!(a_negate, b_negate);
		}
		(Op::Jump { target: a_target }, Op::Jump { target: b_target }) => {
			assert_eq!(a_target, b_target);
		}
		(
			Op::Branch {
				condition: a_condition,
				yes: a_yes,
				no: a_no,
			},
			Op::Branch {
				condition: b_condition,
				yes: b_yes,
				no: b_no,
			},
		) => {
			assert_eq!(a_condition, b_condition);
			assert_eq!(a_yes, b_yes);
			assert_eq!(a_no, b_no);
		}
		(
			Op::Call {
				out: a_out,
				function: a_function,
				args_start: a_args_start,
				args_len: a_args_len,
			},
			Op::Call {
				out: b_out,
				function: b_function,
				args_start: b_args_start,
				args_len: b_args_len,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_function, b_function);
			assert_eq!(a_args_start, b_args_start);
			assert_eq!(a_args_len, b_args_len);
		}
		(Op::Return { value: a_value }, Op::Return { value: b_value }) => {
			assert_eq!(a_value, b_value);
		}
		(
			Op::List {
				out: a_out,
				tag: a_tag,
				args_start: a_args_start,
				args_len: a_args_len,
			},
			Op::List {
				out: b_out,
				tag: b_tag,
				args_start: b_args_start,
				args_len: b_args_len,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_tag, b_tag);
			assert_eq!(a_args_start, b_args_start);
			assert_eq!(a_args_len, b_args_len);
		}
		(
			Op::Get {
				out: a_out,
				value: a_value,
				index: a_index,
			},
			Op::Get {
				out: b_out,
				value: b_value,
				index: b_index,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
			assert_eq!(a_index, b_index);
		}
		(
			Op::Set {
				value: a_value,
				index: a_index,
				new: a_new,
			},
			Op::Set {
				value: b_value,
				index: b_index,
				new: b_new,
			},
		) => {
			assert_eq!(a_value, b_value);
			assert_eq!(a_index, b_index);
			assert_eq!(a_new, b_new);
		}
		(
			Op::Len {
				out: a_out,
				value: a_value,
			},
			Op::Len {
				out: b_out,
				value: b_value,
			},
		) => {
			assert_eq!(a_out, b_out);
			assert_eq!(a_value, b_value);
		}
		(
			Op::Push {
				value: a_value,
				new: a_new,
			},
			Op::Push {
				value: b_value,
				new: b_new,
			},
		) => {
			assert_eq!(a_value, b_value);
			assert_eq!(a_new, b_new);
		}
		(Op::Drop { value: a_value }, Op::Drop { value: b_value }) => {
			assert_eq!(a_value, b_value);
		}
		_ => panic!("decoded variant changed"),
	}
}
fn same_value(a: Value, b: Value) {
	match (a, b) {
		(Value::F64(a), Value::F64(b)) => assert_eq!(a.to_bits(), b.to_bits()),
		(a, b) => assert_eq!(a, b),
	}
}

#[test]
fn every_encoded_variant_round_trips_fields() {
	let mut function = Function::new(8, 0).unwrap();
	function.string("key").unwrap();
	function.arguments(&[0, 1, 3, 4]).unwrap();
	let ops = [
		Op::Template {
			out: 2,
			args_start: 0,
			args_len: 2,
		},
		Op::Print { value: 2 },
		Op::TupleField {
			out: 2,
			value: 2,
			index: 2,
		},
		Op::SetTupleField {
			value: 2,
			index: 2,
			new: 2,
		},
		Op::Record {
			out: 2,
			tag: Type::Object,
			args_start: 0,
			args_len: 2,
		},
		Op::Field {
			out: 2,
			value: 2,
			key: 0,
		},
		Op::SetField {
			value: 2,
			key: 0,
			new: 2,
		},
		Op::Variant {
			out: 2,
			tag: Type::Option,
			success: true,
			payload: Some(2),
		},
		Op::Try { out: 2, value: 2 },
		Op::CallValue {
			out: 2,
			value: 2,
			args_start: 0,
			args_len: 2,
		},
		Op::Range {
			out: 2,
			start: 2,
			end: 2,
		},
		Op::IterStart { out: 2, value: 2 },
		Op::IterNext {
			out: 2,
			iterator: 2,
			none: 0,
		},
		Op::String { out: 2, text: 0 },
		Op::Clone { out: 2, value: 2 },
		Op::AssignBinary {
			out: 2,
			op: Binary::Add,
			lhs: 2,
			rhs: 2,
		},
		Op::Literal {
			out: 2,
			value: Value::I64(-17),
		},
		Op::Copy { out: 2, value: 2 },
		Op::Release { value: 2 },
		Op::Unary {
			out: 2,
			op: Unary::Neg,
			value: 2,
		},
		Op::Binary {
			out: 2,
			op: Binary::Add,
			lhs: 2,
			rhs: 2,
		},
		Op::Cast {
			out: 2,
			value: 2,
			tag: Type::Vec,
		},
		Op::Is {
			out: 2,
			value: 2,
			tag: Type::Vec,
			negate: true,
		},
		Op::Jump { target: 0 },
		Op::Branch {
			condition: 2,
			yes: 0,
			no: 0,
		},
		Op::Call {
			out: 2,
			function: 1,
			args_start: 0,
			args_len: 2,
		},
		Op::Return { value: 2 },
		Op::List {
			out: 2,
			tag: Type::Vec,
			args_start: 0,
			args_len: 2,
		},
		Op::Get {
			out: 2,
			value: 2,
			index: 2,
		},
		Op::Set {
			value: 2,
			index: 2,
			new: 2,
		},
		Op::Len { out: 2, value: 2 },
		Op::Push { value: 2, new: 2 },
		Op::Drop { value: 2 },
	];
	let mut coverage = [false; 33];
	for expected in ops {
		let index = variant_index(expected);
		assert!(!coverage[index], "duplicate variant {index}");
		coverage[index] = true;
		let pc = function.push(expected, location()).unwrap();
		same(expected, function.operation(pc as usize).unwrap());
	}
	assert!(coverage.into_iter().all(|seen| seen));
	function.push(Op::Return { value: 0 }, location()).unwrap();
	let mut callee = Function::new(2, 2).unwrap();
	callee.push(Op::Return { value: 0 }, location()).unwrap();
	let mut builder = Builder::default();
	builder.push(function).unwrap();
	builder.push(callee).unwrap();
	let code = builder.finish().unwrap();
	let f = code.function(0).unwrap();
	assert_eq!(f.cache_layout().unwrap().0, ops.len() + 1);
	assert_eq!(f.cache_layout().unwrap().0, f.cache_layout().unwrap().1);
	for (pc, expected) in ops.into_iter().enumerate() {
		same(expected, f.cached(pc).unwrap().op);
		f.audit_cached(pc, f.cached(pc).unwrap()).unwrap();
	}
}
#[test]
fn float_literals_keep_nan_payloads_and_signed_zero() {
	let mut function = Function::new(1, 0).unwrap();
	for bits in [0, 1u64 << 63, 0x7ff8000000000042, 0xfff8000000000123] {
		let op = Op::Literal {
			out: 0,
			value: Value::F64(f64::from_bits(bits)),
		};
		let pc = function.push(op, location()).unwrap();
		same(op, function.operation(pc as usize).unwrap());
	}
}
#[test]
fn local_function_literals_are_stamped_before_publication() {
	let mut function = Function::new(1, 0).unwrap();
	function
		.push(
			Op::Literal {
				out: 0,
				value: Value::Function { code: 0, id: 0 },
			},
			location(),
		)
		.unwrap();
	function.push(Op::Return { value: 0 }, location()).unwrap();
	let mut builder = Builder::default();
	builder.push(function).unwrap();
	let code = builder.finish().unwrap();
	assert_ne!(code.id(), 0);
	same(
		Op::Literal {
			out: 0,
			value: Value::Function {
				code: code.id(),
				id: 0,
			},
		},
		code.function(0).unwrap().operation(0).unwrap(),
	);
}
#[test]
fn completed_charge_map_preserves_gap_and_backward_target() {
	let mut function = Function::new(1, 0).unwrap();
	for _ in 0..600 {
		function
			.push(Op::Copy { out: 0, value: 0 }, location())
			.unwrap();
	}
	function.push(Op::Jump { target: 0 }, location()).unwrap();
	let mut builder = Builder::default();
	builder.push(function).unwrap();
	let code = builder.finish().unwrap();
	let f = code.function(0).unwrap();
	let charges: std::vec::Vec<_> = f
		.instructions()
		.iter()
		.enumerate()
		.filter_map(|(pc, i)| i.is_charged().then_some(pc))
		.collect();
	assert_eq!(charges, [0, 257, 514]);
}
#[test]
fn cache_preparation_refuses_the_old_charge_scratch_only_budget() {
	// Baseline code.rs:675-676: finish allocates one gap Vec<usize> per function,
	// exactly 128 entries here; it drops before the next function. Stamp,
	// validation, successors and charge placement allocate no other buffer.
	// Candidate cache construction follows that pass and exceeds this budget.
	// Builder and its pools were allocated before entering the limit.
	// Candidate expectation is prospectively Allocation: the cache uses this
	// same memory limit. Construction remains outside the constrained finish.
	let mut builder = Builder::default();
	for _ in 0..2 {
		let mut function = Function::new(1, 0).unwrap();
		for _ in 0..127 {
			function
				.push(Op::Copy { out: 0, value: 0 }, location())
				.unwrap();
		}
		function.push(Op::Return { value: 0 }, location()).unwrap();
		builder.push(function).unwrap();
	}
	let result = rune_alloc::limit::with(128 * core::mem::size_of::<usize>(), || {
		builder.finish().err().map(|e| e.kind)
	})
	.call();
	assert_eq!(result, Some(ErrorKind::Allocation));
}
#[test]
fn invalid_literal_and_argument_are_refused_before_execution() {
	for op in [
		Op::Literal {
			out: 0,
			value: Value::Function { code: 1, id: 0 },
		},
		Op::CallValue {
			out: 0,
			value: 0,
			args_start: 0,
			args_len: 1,
		},
	] {
		let mut function = Function::new(1, 0).unwrap();
		function.push(op, location()).unwrap();
		function.push(Op::Return { value: 0 }, location()).unwrap();
		let mut builder = Builder::default();
		builder.push(function).unwrap();
		assert_eq!(builder.finish().err().unwrap().kind, ErrorKind::InvalidCode);
	}
}

#[test]
fn all_non_heap_literal_kinds_survive_validation_and_pool_decode() {
	let nominal = crate::experimental_core::nominal::intern(0x184d1001, "D1Literal").unwrap();
	for value in [
		Value::Unit,
		Value::Bool(true),
		Value::I64(i64::MIN),
		Value::U64(u64::MAX),
		Value::Char('\u{1F600}'),
		Value::Type(Type::Struct(nominal)),
	] {
		let mut function = Function::new(1, 0).unwrap();
		function
			.push(Op::Literal { out: 0, value }, location())
			.unwrap();
		function.push(Op::Return { value: 0 }, location()).unwrap();
		let mut builder = Builder::default();
		builder.push(function).unwrap();
		let code = builder.finish().unwrap();
		same(
			Op::Literal { out: 0, value },
			code.function(0).unwrap().operation(0).unwrap(),
		);
	}
	let mut function = Function::new(1, 0).unwrap();
	function
		.push(
			Op::Literal {
				out: 0,
				value: Value::Heap(crate::experimental_core::value::Handle {
					slot: 0,
					generation: 1,
				}),
			},
			location(),
		)
		.unwrap();
	function.push(Op::Return { value: 0 }, location()).unwrap();
	let mut builder = Builder::default();
	builder.push(function).unwrap();
	assert_eq!(builder.finish().err().unwrap().kind, ErrorKind::InvalidCode);
}
