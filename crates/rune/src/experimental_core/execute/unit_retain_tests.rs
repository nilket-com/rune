//! D11 tests-first: caller ownership, independent of retain's success payload.

use super::*;
use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Function},
	heap::Limits,
	value::Type,
};

fn location(n: u32) -> Location {
	Location {
		source: 11,
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

fn function(slots: u32, parameters: u32, ops: &[Op]) -> Function {
	let mut f = Function::new(slots, parameters).unwrap();
	for (i, op) in ops.iter().enumerate() {
		f.push(*op, location(i as u32)).unwrap();
	}

	f
}

fn code(f: Function) -> Code {
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}

fn fail(e: &mut Executor, code: &Code, store: &mut Store, args: &[Value], kind: ErrorKind) {
	let failure = e
		.invoke_args_for_test(code, store, 0, Args::Values(args), 1024)
		.unwrap_err();
	assert_eq!(failure.error.kind, kind);
	assert!(failure.cleanup_error.is_none());
	assert!(e.is_idle());
}

fn restore(store: &mut Store, value: Value) {
	// Assert the synthetic remainder before restoring the sole real host edge.
	assert_eq!(store.test_strong(value), u32::MAX - 1);
	store.test_set_strong(value, 1);
	store.release(value).unwrap();
}

#[test]
fn d11_duplicate_prefix_admit_rolls_back_and_roots_remain_usable() {
	let mut s = store();
	let child = s.string("child").unwrap();
	let full = s.string("full").unwrap();
	s.test_set_strong(full, u32::MAX);
	let before = (s.live_entries(), s.payload_bytes());
	assert_eq!(
		s.list(Type::Vec, &[child, child, full]).unwrap_err().kind,
		ErrorKind::Overflow
	);
	assert_eq!(s.test_strong(child), 1);
	assert_eq!(s.test_strong(full), u32::MAX);
	assert_eq!((s.live_entries(), s.payload_bytes()), before);
	let root = s.export(child);
	let alias = s.root_retain(&root).unwrap();
	s.root_release(root).unwrap();
	assert_eq!(s.root_text(&alias).unwrap(), "child");
	s.root_release(alias).unwrap();
	s.test_set_strong(full, 1);
	s.release(full).unwrap();
	let fresh = s.string("reuse").unwrap();
	assert_eq!(s.text(fresh).unwrap(), "reuse");
	s.release(fresh).unwrap();
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn d11_partial_call_retain_failure_releases_published_prefix() {
	let mut f = function(3, 2, &[]);
	let (args_start, args_len) = f.arguments(&[0, 1]).unwrap();
	f.push(
		Op::Call {
			out: 2,
			function: 1,
			args_start,
			args_len,
		},
		location(0),
	)
	.unwrap();
	f.push(Op::Return { value: 2 }, location(1)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.push(function(2, 2, &[Op::Return { value: 0 }])).unwrap();
	let c = b.finish().unwrap();
	let mut s = store();
	let a = s.string("a").unwrap();
	let bad = s.string("b").unwrap();
	s.test_set_strong(bad, u32::MAX - 1);
	let mut e = Executor::new().unwrap();
	fail(&mut e, &c, &mut s, &[a, bad], ErrorKind::Overflow);
	assert_eq!(s.test_strong(a), 1);
	assert_eq!(s.text(a).unwrap(), "a");
	assert_eq!(s.text(bad).unwrap(), "b");
	restore(&mut s, bad);
	s.release(a).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn d11_enter_checks_all_root_owners_before_reservation_or_acquisition() {
	let c = code(function(2, 2, &[Op::Return { value: 0 }]));
	let mut s = store();
	let other = store();
	let own = s.root_string("own").unwrap();
	let foreign = other.root_i64(1);
	let mut e = Executor::new().unwrap();
	let before = (
		e.slots.capacity(),
		e.frames.capacity(),
		s.test_strong(own.value),
	);
	assert_eq!(
		e.enter(&c, &mut s, 0, Args::Roots(&[&own, &foreign]), None)
			.unwrap_err()
			.kind,
		ErrorKind::InvalidHandle
	);
	assert!(e.is_idle());
	assert_eq!(
		(
			e.slots.capacity(),
			e.frames.capacity(),
			s.test_strong(own.value)
		),
		before
	);
	s.root_release(own).unwrap();

	// The receiving store must be closed: closing the foreign store would
	// merely repeat its owner mismatch against an open receiver.
	let scalar = s.root_i64(7);
	let closed_before = (
		e.slots.len(),
		e.slots.capacity(),
		e.frames.len(),
		e.frames.capacity(),
	);
	s.close().unwrap();

	for roots in [[&scalar, &scalar], [&scalar, &foreign]] {
		assert_eq!(
			e.enter(&c, &mut s, 0, Args::Roots(&roots), None)
				.unwrap_err()
				.kind,
			ErrorKind::ClosedStore
		);
		assert!(e.is_idle());
		assert_eq!(
			(
				e.slots.len(),
				e.slots.capacity(),
				e.frames.len(),
				e.frames.capacity()
			),
			closed_before
		);
	}
}

#[test]
fn d11_copy_and_successful_try_preserve_alias_and_replace_destination() {
	for same_slot in [false, true] {
		let mut s = store();
		let payload = s.string("payload").unwrap();
		let variant = s.variant(Type::Option, true, Some(payload)).unwrap();
		let out = if same_slot { 0 } else { 1 };
		let c = code(function(
			2,
			1,
			&[
				Op::Copy { out: 0, value: 0 },
				Op::Try { out, value: 0 },
				Op::Return { value: out },
			],
		));
		let mut e = Executor::new().unwrap();
		let result = e
			.invoke_args_for_test(&c, &mut s, 0, Args::Values(&[variant]), 100)
			.unwrap();
		assert_eq!(result, payload);
		assert_eq!(s.text(result).unwrap(), "payload");
		assert_eq!(s.test_strong(variant), 1);
		assert_eq!(s.test_strong(payload), 3);
		s.release(result).unwrap();
		s.release(variant).unwrap();
		assert_eq!(s.test_strong(payload), 1);
		s.release(payload).unwrap();
		assert!(e.is_idle());
		assert_eq!(s.live_entries(), 0);
	}

	let mut s = store();
	let mut e = Executor::new().unwrap();
	let value = s.string("copy-refusal").unwrap();
	s.test_set_strong(value, u32::MAX - 1);
	let c = code(function(
		1,
		1,
		&[Op::Copy { out: 0, value: 0 }, Op::Return { value: 0 }],
	));
	fail(&mut e, &c, &mut s, &[value], ErrorKind::Overflow);
	restore(&mut s, value);
	let payload = s.string("try-refusal").unwrap();
	let variant = s.variant(Type::Option, true, Some(payload)).unwrap();
	assert_eq!(s.test_strong(payload), 2);
	s.test_set_strong(payload, u32::MAX);
	let c = code(function(
		1,
		1,
		&[Op::Try { out: 0, value: 0 }, Op::Return { value: 0 }],
	));
	fail(&mut e, &c, &mut s, &[variant], ErrorKind::Overflow);
	assert_eq!(s.test_strong(variant), 1);
	assert_eq!(s.test_strong(payload), u32::MAX);
	assert_eq!(s.text(payload).unwrap(), "try-refusal");

	// Two real edges: the original host edge and the surviving variant child.
	s.test_set_strong(payload, 2);
	s.release(variant).unwrap();
	assert_eq!(s.test_strong(payload), 1);
	s.release(payload).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn d11_return_and_unsuccessful_try_transfer_or_cleanup_exact_edge() {
	for unsuccessful in [false, true] {
		let mut s = store();
		let value = if unsuccessful {
			s.variant(Type::Option, false, None).unwrap()
		} else {
			s.string("return").unwrap()
		};

		let op = if unsuccessful {
			Op::Try { out: 0, value: 0 }
		} else {
			Op::Return { value: 0 }
		};

		let c = code(function(1, 1, &[op, Op::Return { value: 0 }]));
		let mut e = Executor::new().unwrap();
		let result = e
			.invoke_args_for_test(&c, &mut s, 0, Args::Values(&[value]), 100)
			.unwrap();
		assert_eq!(result, value);
		assert_eq!(s.test_strong(value), 2);
		s.release(result).unwrap();
		assert_eq!(s.test_strong(value), 1);
		s.test_set_strong(value, u32::MAX - 1);
		fail(&mut e, &c, &mut s, &[value], ErrorKind::Overflow);
		restore(&mut s, value);
		assert_eq!(s.live_entries(), 0);
	}
}

/// M1: was `d11_mark_retain_failure_does_not_publish_telemetry`. The removed `Mark`
/// opcode is replaced by `Copy`, which performs the same retain into an out slot; a
/// retain overflow on a near-max-strong string publishes nothing (`restore` asserts the
/// strong count is unchanged) and the executor stays reusable.
#[test]
fn d11_copy_retain_failure_is_atomic() {
	let c = code(function(
		2,
		1,
		&[Op::Copy { out: 1, value: 0 }, Op::Return { value: 1 }],
	));
	let mut s = store();
	let v = s.string("mark").unwrap();
	s.test_set_strong(v, u32::MAX - 1);
	let mut e = Executor::new().unwrap();
	fail(&mut e, &c, &mut s, &[v], ErrorKind::Overflow);
	restore(&mut s, v);
	assert_eq!(s.live_entries(), 0);
	let v = s.string("mark-success").unwrap();
	let result = e
		.invoke_args_for_test(&c, &mut s, 0, Args::Values(&[v]), 100)
		.unwrap();
	assert_eq!(result, v);
	assert_eq!(s.text(result).unwrap(), "mark-success");
	assert_eq!(s.test_strong(v), 2);
	s.release(result).unwrap();
	assert_eq!(s.test_strong(v), 1);
	s.release(v).unwrap();
	assert!(e.is_idle());
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn d11_string_append_effect_survives_later_retain_refusal() {
	let c = code(function(
		3,
		2,
		&[
			Op::AssignBinary {
				out: 2,
				op: Binary::Add,
				lhs: 0,
				rhs: 1,
			},
			Op::Return { value: 2 },
		],
	));
	let mut s = store();
	let left = s.string("left").unwrap();
	let right = s.string("right").unwrap();
	s.test_set_strong(left, u32::MAX - 1);
	let mut e = Executor::new().unwrap();
	fail(&mut e, &c, &mut s, &[left, right], ErrorKind::Overflow);
	assert_eq!(s.text(left).unwrap(), "leftright");
	assert_eq!(s.test_strong(right), 1);
	restore(&mut s, left);
	s.release(right).unwrap();
	assert_eq!(s.live_entries(), 0);
}
