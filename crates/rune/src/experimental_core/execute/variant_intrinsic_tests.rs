//! C4 flag-only intrinsic ownership, admission and frontier controls.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function, PRINT_RAW, VARIANT_SUCCESS},
	heap::Limits,
	value::Type,
};
fn loc(n: u32) -> Location {
	Location {
		source: 0,
		revision: 1,
		start: n,
		end: n + 1,
	}
}
fn program(out: u32) -> Program {
	let mut f = Function::new(2, 1).unwrap();
	let (args_start, args_len) = f.arguments(&[0]).unwrap();
	f.push(
		Op::Call {
			out,
			function: VARIANT_SUCCESS,
			args_start,
			args_len,
		},
		loc(10),
	)
	.unwrap();
	f.push(Op::Return { value: out }, loc(11)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	Program::for_diagnostics_test(b.finish().unwrap())
}
#[test]
fn all_variant_flags_ignore_moved_children_and_shared_readers_without_state_changes() {
	for tag in [Type::Option, Type::Result] {
		for success in [false, true] {
			let mut s = Store::new(Limits::default()).unwrap();
			let child = s.string("child").unwrap();
			let payload = if tag == Type::Option && !success {
				None
			} else {
				Some(child)
			};
			let parent = s.variant(tag, success, payload).unwrap();
			s.invalidate(child).unwrap();
			s.set_readers_for_test(parent, 1);
			let before = s.state_for_test();
			assert_eq!(s.variant_success(parent).unwrap(), success);
			assert_eq!(s.state_for_test(), before);
			s.zero_readers_for_test(parent);
			s.release(parent).unwrap();
			s.release(child).unwrap();
			assert_eq!(s.live_entries(), 0);
		}
	}
}
#[test]
fn accessor_preserves_parent_error_domains_without_mutation() {
	let mut s = Store::new(Limits::default()).unwrap();
	let other = s.string("not variant").unwrap();
	for v in [Value::Unit, Value::I64(7), other] {
		let before = s.state_for_test();
		assert_eq!(
			s.variant_success(v).unwrap_err().kind,
			ErrorKind::InvalidCode
		);
		assert_eq!(s.state_for_test(), before);
	}
	s.release(other).unwrap();
	let parent = s.variant(Type::Result, true, Some(Value::Unit)).unwrap();
	s.invalidate(parent).unwrap();
	let before = s.state_for_test();
	let err = s.variant_success(parent).unwrap_err();
	assert_eq!(err.kind, ErrorKind::AccessConflict);
	assert_eq!(
		err.detail,
		Detail::Moved {
			access: crate::experimental_core::error::MovedAccess::Read
		}
	);
	assert_eq!(s.state_for_test(), before);
	s.release(parent).unwrap();
	let before = s.state_for_test();
	assert_eq!(
		s.variant_success(parent).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(s.state_for_test(), before);
	s.close().unwrap();
	assert_eq!(
		s.variant_success(parent).unwrap_err().kind,
		ErrorKind::ClosedStore
	);
}
#[test]
fn bool_put_releases_old_output_without_allocation_including_argument_alias() {
	for tag in [Type::Option, Type::Result] {
		for success in [false, true] {
			for out in [0, 1] {
				let p = program(out);
				let c = &p.code;
				let f = c.function(0).unwrap();
				let mut s = Store::new(Limits::default()).unwrap();
				let mut e = Executor::new().unwrap();
				let payload = if tag == Type::Option && !success {
					None
				} else {
					Some(Value::Unit)
				};
				let input = s.variant(tag, success, payload).unwrap();
				let old = s.string("old").unwrap();
				e.slots.try_push(input).unwrap();
				e.slots.try_push(old).unwrap();
				e.test_output = Some(std::vec::Vec::new());
				rune_alloc::limit::with(0, || {
					e.intrinsic(c, &mut s, f, 0, out, VARIANT_SUCCESS, 0, 1)
						.map_err(|e| e.kind)
				})
				.call()
				.unwrap();
				assert_eq!(e.slots[out as usize], Value::Bool(success));
				assert!(e.test_output.as_ref().unwrap().is_empty());
				assert_eq!(s.live_entries(), 1);
				let survivor = e.slots[1 - out as usize];
				e.slots.clear();
				s.release(survivor).unwrap();
				assert_eq!(s.live_entries(), 0);
			}
		}
	}
}
#[test]
fn bool_is_published_even_when_existing_put_drains_a_bad_pending_release() {
	let p = program(1);
	let c = &p.code;
	let f = c.function(0).unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let stale = s.string("gone").unwrap();
	s.release(stale).unwrap();
	let input = s.variant(Type::Result, true, Some(Value::Unit)).unwrap();
	e.slots.try_push(input).unwrap();
	e.slots.try_push(Value::Unit).unwrap();
	s.push_work_for_test(stale);
	assert_eq!(
		e.intrinsic(c, &mut s, f, 0, 1, VARIANT_SUCCESS, 0, 1)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(e.slots[1], Value::Bool(true));
	assert_eq!(s.queued_work_for_test(), 0);
	e.slots.clear();
	s.release(input).unwrap();
	assert_eq!(s.live_entries(), 0);
}
#[test]
fn both_certificates_validate_arity_range_argument_and_destination() {
	for id in [PRINT_RAW, VARIANT_SUCCESS] {
		for (out, start, len, arg, valid) in [
			(1, 0, 1, 0, true),
			(1, 0, 0, 0, false),
			(1, 0, 2, 0, false),
			(1, u32::MAX, 1, 0, false),
			(1, 0, 1, 99, false),
			(2, 0, 1, 0, false),
		] {
			let mut f = Function::new(2, 1).unwrap();
			f.arguments(&[arg]).unwrap();
			f.push(
				Op::Call {
					out,
					function: id,
					args_start: start,
					args_len: len,
				},
				loc(0),
			)
			.unwrap();
			f.push(Op::Return { value: 0 }, loc(1)).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			assert_eq!(b.finish().is_ok(), valid);
		}
	}
}
#[test]
fn cold_helper_rejects_bad_inputs_before_store_read_or_publication() {
	let p = program(1);
	let c = &p.code;
	let f = c.function(0).unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let input = s.variant(Type::Result, true, Some(Value::Unit)).unwrap();
	s.invalidate(input).unwrap();
	e.slots.try_push(input).unwrap();
	e.slots.try_push(Value::I64(42)).unwrap();
	for (base, out, id, start, len) in [
		(0, 1, VARIANT_SUCCESS, 0, 0),
		(0, 1, VARIANT_SUCCESS, 0, 2),
		(0, 1, VARIANT_SUCCESS, u32::MAX, 1),
		(usize::MAX, 1, VARIANT_SUCCESS, 0, 1),
		(0, u32::MAX, VARIANT_SUCCESS, 0, 1),
		(0, 1, VARIANT_SUCCESS + 1, 0, 1),
	] {
		let before = s.state_for_test();
		assert_eq!(
			e.intrinsic(c, &mut s, f, base, out, id, start, len)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidCode
		);
		assert_eq!(e.slots[1], Value::I64(42));
		assert_eq!(s.state_for_test(), before);
	}
	let mut malformed = Function::new(2, 1).unwrap();
	malformed.arguments(&[99]).unwrap();
	assert_eq!(
		e.intrinsic(c, &mut s, &malformed, 0, 1, VARIANT_SUCCESS, 0, 1)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
	let before = s.state_for_test();
	assert_eq!(
		e.intrinsic(c, &mut s, f, 0, 1, VARIANT_SUCCESS, 0, 1)
			.unwrap_err()
			.kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(e.slots[1], Value::I64(42));
	assert_eq!(s.state_for_test(), before);
	e.slots.clear();
	s.release(input).unwrap();
}
#[test]
fn intrinsic_fuel_and_cancel_frontiers_preserve_sites_and_reuse() {
	let p = program(1);
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let arg = s.root_i64(7);
	for fuel in 0..2 {
		let failure = e.invoke(&p, &mut s, 0, &[&arg], fuel).unwrap_err();
		assert_eq!(
			failure.error.kind,
			if fuel == 0 {
				ErrorKind::FuelExhausted
			} else {
				ErrorKind::InvalidCode
			}
		);
		assert_eq!(failure.error.location, Some(loc(10)));
		assert!(failure.cleanup_error.is_none());
		assert!(e.is_idle());
	}
	e.request_cancel();
	let arg = s.root_i64(7);
	let failure = e.invoke(&p, &mut s, 0, &[&arg], 100).unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert_eq!(failure.error.location, Some(loc(10)));
	let failure = e.invoke(&p, &mut s, 0, &[&arg], 100).unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(e.is_idle());
	assert_eq!(s.live_entries(), 0);
}
#[test]
fn nested_failure_retains_intrinsic_site_and_ordinary_call_chain() {
	let mut main = Function::new(2, 0).unwrap();
	main.push(
		Op::Literal {
			out: 0,
			value: Value::I64(7),
		},
		loc(1),
	)
	.unwrap();
	let (args_start, args_len) = main.arguments(&[0]).unwrap();
	main.push(
		Op::Call {
			out: 1,
			function: 1,
			args_start,
			args_len,
		},
		loc(2),
	)
	.unwrap();
	main.push(Op::Return { value: 1 }, loc(3)).unwrap();
	let mut b = Builder::default();
	b.push(main).unwrap();
	let mut child = Function::new(2, 1).unwrap();
	let (args_start, args_len) = child.arguments(&[0]).unwrap();
	child
		.push(
			Op::Call {
				out: 1,
				function: VARIANT_SUCCESS,
				args_start,
				args_len,
			},
			loc(10),
		)
		.unwrap();
	child.push(Op::Return { value: 1 }, loc(11)).unwrap();
	b.push(child).unwrap();
	let code = b.finish().unwrap();
	let mut s = Store::new(Limits::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let failure = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[]), 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert_eq!(failure.error.location, Some(loc(10)));
	assert_eq!(failure.call_chain.as_slice(), &[loc(2)]);
	assert!(failure.cleanup_error.is_none());
	assert!(e.is_idle());
}

#[test]
fn successful_flag_call_preserves_block_entry_charge_and_reuses() {
	let p = program(1);
	let charged: std::vec::Vec<_> = p
		.code
		.function(0)
		.unwrap()
		.instructions()
		.iter()
		.map(|i| i.is_charged())
		.collect();
	assert_eq!(charged, [true, false]);
	let mut s = Store::new(Limits::default()).unwrap();
	let input = s.variant(Type::Result, true, Some(Value::Unit)).unwrap();
	let mut e = Executor::new().unwrap();
	for fuel in 0..=2 {
		let before = s.state_for_test();
		let result = e.invoke_args_for_test(&p.code, &mut s, 0, Args::Values(&[input]), fuel);
		if fuel == 0 {
			let failure = result.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::FuelExhausted);
			assert_eq!(failure.error.location, Some(loc(10 + fuel as u32)));
			assert!(failure.cleanup_error.is_none());
		} else {
			assert_eq!(result.unwrap(), Value::Bool(true));
		}
		assert!(e.is_idle());
		assert_eq!(s.state_for_test(), before);
	}
	{
		let charge = 1;
		cancel_at_charge_for_test(Some(charge - 1));
		let failure = e
			.invoke_args_for_test(&p.code, &mut s, 0, Args::Values(&[input]), 100)
			.unwrap_err();
		cancel_at_charge_for_test(None);
		assert_eq!(failure.error.kind, ErrorKind::Cancelled);
		assert_eq!(failure.error.location, Some(loc(9 + charge as u32)));
		assert!(failure.cleanup_error.is_none());
		assert!(e.is_idle());
		assert_eq!(
			e.invoke_args_for_test(&p.code, &mut s, 0, Args::Values(&[input]), 100)
				.unwrap(),
			Value::Bool(true)
		);
		assert!(e.is_idle());
	}
	s.release(input).unwrap();
	assert_eq!(s.live_entries(), 0);
}
