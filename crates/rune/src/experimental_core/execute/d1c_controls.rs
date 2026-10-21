//! D1c malformed-call controls; these run first on unchanged production.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};
const FIRST: u32 = 0x8000_0000;
const PROBE: u32 = 0x8000_0d1c;
fn site() -> Location {
	Location {
		start: 42,
		end: 43,
		..Location::default()
	}
}
fn callee() -> Function {
	let mut f = Function::new(1, 0).unwrap();
	f.push(Op::Return { value: 0 }, site()).unwrap();
	f
}
#[test]
fn uncertified_ids_bad_ranges_and_outputs_never_seal() {
	for (id, out, start, len) in [
		(2, 0, 0, 0),
		(FIRST, 0, 0, 0),
		(PROBE, 0, 0, 0),
		(u32::MAX, 0, 0, 0),
		(1, 1, 0, 0),
		(1, 0, u32::MAX, 0),
		(1, 0, 0, 1),
		(1, 0, u32::MAX, 1),
	] {
		let mut f = Function::new(1, 0).unwrap();
		f.push(
			Op::Call {
				out,
				function: id,
				args_start: start,
				args_len: len,
			},
			site(),
		)
		.unwrap();
		f.push(Op::Return { value: 0 }, site()).unwrap();
		let mut target = Function::new(1, len).unwrap();
		target.push(Op::Return { value: 0 }, site()).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		b.push(target).unwrap();
		let error = b.finish().err().expect("invalid call refused");
		assert_eq!(error.kind, ErrorKind::InvalidCode);
	}
}
#[test]
fn reserved_function_literals_are_not_capabilities() {
	for id in [
		FIRST,
		super::super::code::PRINT_RAW,
		super::super::code::VARIANT_SUCCESS,
		PROBE,
		u32::MAX,
	] {
		let mut f = Function::new(1, 0).unwrap();
		f.push(
			Op::Literal {
				out: 0,
				value: Value::Function { code: 0, id },
			},
			site(),
		)
		.unwrap();
		f.push(Op::Return { value: 0 }, site()).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		assert_eq!(
			b.finish()
				.err()
				.expect("reserved function literal refused")
				.kind,
			ErrorKind::InvalidCode
		);
	}
}
#[test]
fn call_value_never_routes_invalid_tokens_into_intrinsics() {
	let mut f = Function::new(2, 1).unwrap();
	f.push(
		Op::CallValue {
			out: 1,
			value: 0,
			args_start: 0,
			args_len: 0,
		},
		site(),
	)
	.unwrap();
	f.push(Op::Return { value: 1 }, site()).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.push(callee()).unwrap();
	let code = b.finish().unwrap();
	let mut b = Builder::default();
	b.push(callee()).unwrap();
	let foreign = b.finish().unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	for (token, kind) in [
		(
			Value::Function {
				code: code.id(),
				id: super::super::code::VARIANT_SUCCESS,
			},
			ErrorKind::InvalidCode,
		),
		(
			Value::Function {
				code: code.id(),
				id: super::super::code::PRINT_RAW,
			},
			ErrorKind::InvalidCode,
		),
		(
			Value::Function {
				code: code.id(),
				id: FIRST,
			},
			ErrorKind::InvalidCode,
		),
		(
			Value::Function {
				code: code.id(),
				id: PROBE,
			},
			ErrorKind::InvalidCode,
		),
		(
			Value::Function {
				code: code.id(),
				id: 999,
			},
			ErrorKind::InvalidCode,
		),
		(
			Value::Function {
				code: foreign.id(),
				id: 0,
			},
			ErrorKind::Type,
		),
	] {
		let failure = e
			.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[token]), 100)
			.unwrap_err();
		assert_eq!(failure.error.kind, kind);
		assert_eq!(failure.error.location, Some(site()));
		assert!(failure.cleanup_error.is_none() && failure.call_chain.is_empty());
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	let failure = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[Value::I64(1)]), 0)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::FuelExhausted);
	assert_eq!(failure.error.location, Some(site()));
	e.request_cancel();
	let failure = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[Value::I64(1)]), 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert_eq!(failure.error.location, Some(site()));
	let failure = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[Value::I64(1)]), 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	assert!(e.is_idle());
}

#[test]
fn ordinary_call_bad_argument_slot_is_refused() {
	let mut f = Function::new(1, 0).unwrap();
	let (start, len) = f.arguments(&[99]).unwrap();
	f.push(
		Op::Call {
			out: 0,
			function: 1,
			args_start: start,
			args_len: len,
		},
		site(),
	)
	.unwrap();
	f.push(Op::Return { value: 0 }, site()).unwrap();
	let mut target = Function::new(1, 1).unwrap();
	target.push(Op::Return { value: 0 }, site()).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.push(target).unwrap();
	assert_eq!(
		b.finish().err().expect("bad argument slot refused").kind,
		ErrorKind::InvalidCode
	);
}

#[test]
fn existing_codec_layout_stays_fixed() {
	assert_eq!(
		core::mem::size_of::<crate::experimental_core::code::Instruction>(),
		16
	);
	if cfg!(target_pointer_width = "64") {
		// H1 host descriptors, C13 absence proofs and the C18b1 script method
		// table each add a 24-byte Vec; opcode layouts stay fixed.
		assert_eq!(core::mem::size_of::<Code>(), 104);
		assert_eq!(core::mem::size_of::<Op>(), 24);
		assert_eq!(
			core::mem::size_of::<crate::experimental_core::code::CachedInstruction>(),
			32
		);
	}
	std::println!("D1C CODE HEADER {}", core::mem::size_of::<Code>());
}

// Appended only with the foundation, after the baseline-compatible tests pass.
#[test]
fn unknown_intrinsic_has_no_operand_or_owner_side_effects() {
	let mut b = Builder::default();
	b.push(callee()).unwrap();
	let code = b.finish().unwrap();
	let caller = Function::new(1, 0).unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	for id in [0, FIRST, PROBE, u32::MAX] {
		let kind = rune_alloc::limit::with(0, || {
			e.intrinsic(
				&code,
				&mut s,
				&caller,
				usize::MAX,
				u32::MAX,
				id,
				u32::MAX,
				u32::MAX,
			)
			.map_err(|e| e.kind)
		})
		.call();
		assert_eq!(kind, Err(ErrorKind::InvalidCode));
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
		assert!(code.intrinsic(id).is_none());
	}
}
