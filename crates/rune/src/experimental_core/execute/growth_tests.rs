//! B3: transactional growth on the real call-setup path.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};

fn frame() -> Frame {
	Frame {
		function: 0,
		pc: 0,
		base: 0,
		out: None,
		callsite_pc: NO_CALLSITE,
	}
}
fn store() -> Store {
	Store::new(Limits {
		entries: 16,
		bytes: 4096,
	})
	.unwrap()
}
fn caller(site: usize) -> Executor {
	let mut e = Executor::new().unwrap();
	e.frames = Vec::try_with_capacity(if site == 0 { 3 } else { 8 }).unwrap();
	e.slots = Vec::try_with_capacity(if site == 1 { 3 } else { 16 }).unwrap();
	e.diagnostics = Vec::try_with_capacity(if site == 2 { 3 } else { 8 }).unwrap();
	for i in 0..3 {
		e.frames
			.try_push(Frame {
				base: i,
				pc: i + 10,
				..frame()
			})
			.unwrap();
		e.slots.try_push(Value::I64(i as i64 + 41)).unwrap();
	}
	e
}
fn witness(
	e: &Executor,
) -> (
	std::vec::Vec<(u32, usize, usize, Option<usize>, u32)>,
	std::vec::Vec<Value>,
	std::vec::Vec<Location>,
) {
	(
		e.frames
			.iter()
			.map(|f| (f.function, f.pc, f.base, f.out, f.callsite_pc))
			.collect(),
		e.slots.iter().copied().collect(),
		e.diagnostics.iter().copied().collect(),
	)
}

// Setup errors carry primary and cleanup errors inline without allocating.
#[test]
#[allow(clippy::result_large_err)]
fn each_real_reservation_failure_restores_three_callers_and_allows_retry() {
	for site in 0..3 {
		let mut e = caller(site);
		let mut s = store();
		let before = witness(&e);
		let fail = rune_alloc::limit::with(0, || {
			e.push_call(&mut s, 4, Args::Values(&[Value::I64(7)]), frame())
		})
		.call()
		.unwrap_err();
		assert_eq!(fail.error.kind, ErrorKind::Allocation, "site {site}");
		assert!(fail.cleanup_error.is_none());
		assert_eq!(witness(&e), before);
		assert!(e.diagnostics.capacity() >= e.frames.len());
		e.push_call(
			&mut s,
			4,
			Args::Slots {
				base: 0,
				indices: &[2, 0, 1],
			},
			frame(),
		)
		.unwrap();
		assert_eq!(
			&e.slots[3..],
			&[Value::I64(43), Value::I64(41), Value::I64(42), Value::Unit]
		);
		e.leave(&mut s).unwrap();
		assert_eq!(witness(&e), before);
		while !e.frames.is_empty() {
			e.leave(&mut s).unwrap();
		}
		assert!(e.is_idle());
	}
}

#[test]
fn each_retain_failure_rolls_back_only_its_own_window_in_reverse() {
	for j in [0, 1, 3] {
		let mut s = store();
		let good = s.string("good").unwrap();
		let bad = s.string("bad").unwrap();
		s.test_set_strong(bad, u32::MAX);
		let mut args = [good; 4];
		args[j] = bad;
		let mut e = caller(9);
		let before = witness(&e);
		let fail = e
			.push_call(&mut s, 5, Args::Values(&args), frame())
			.unwrap_err();
		assert_eq!(fail.error.kind, ErrorKind::Overflow);
		assert!(fail.cleanup_error.is_none());
		assert_eq!(witness(&e), before);
		assert_eq!(s.test_strong(good), 1);
		assert_eq!(s.test_strong(bad), u32::MAX);
		s.test_set_strong(bad, 1);
		e.push_call(&mut s, 5, Args::Values(&args), frame())
			.unwrap();
		e.leave(&mut s).unwrap();
		assert_eq!(witness(&e), before);
		assert_eq!(s.test_strong(good), 1);
		assert_eq!(s.test_strong(bad), 1);
		s.release(good).unwrap();
		s.release(bad).unwrap();
	}
}

#[test]
fn rollback_cleanup_error_reaches_invocation_without_masking_original() {
	let mut b = Builder::default();
	let mut f = Function::new(2, 2).unwrap();
	f.push(Op::Return { value: 0 }, Location::default())
		.unwrap();
	b.push(f).unwrap();
	let code = b.finish().unwrap();
	let mut s = store();
	let a = s.string("a").unwrap();
	let bad = s.string("bad").unwrap();
	s.test_set_strong(bad, u32::MAX);
	let mut e = Executor::new().unwrap();
	// Reports a cleanup failure after actually balancing the retained prefix.
	let secondary = Error::new(ErrorKind::AccessConflict, "rollback injection");
	e.rollback_error = Some(secondary);
	let f = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[a, bad]), 100)
		.unwrap_err();
	assert_eq!(f.error.kind, ErrorKind::Overflow);
	assert_eq!(f.cleanup_error, Some(secondary));
	assert!(f.call_chain.is_empty());
	assert!(e.is_idle());
	assert_eq!(s.test_strong(a), 1);
	s.test_set_strong(bad, 1);
	let result = e
		.invoke_args_for_test(&code, &mut s, 0, Args::Values(&[a, bad]), 100)
		.unwrap();
	s.release(result).unwrap();
	s.release(a).unwrap();
	s.release(bad).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn logical_limits_validate_without_allocating_the_maximum() {
	for limits in [
		ExecutorLimits {
			frames: 0,
			slots: 1,
		},
		ExecutorLimits {
			frames: 1,
			slots: 0,
		},
		ExecutorLimits {
			frames: usize::MAX,
			slots: 1,
		},
		ExecutorLimits {
			frames: 1,
			slots: usize::MAX,
		},
	] {
		assert_eq!(
			Executor::with_limits(limits).err().unwrap().kind,
			ErrorKind::FrameCapacity
		);
	}
	let e = Executor::with_limits(ExecutorLimits {
		frames: 1_000_000,
		slots: 1_000_000,
	})
	.unwrap();
	assert_eq!(
		e.capacities(),
		(INITIAL_FRAMES, INITIAL_SLOTS, INITIAL_FRAMES)
	);
	let mut e = Executor::with_limits(ExecutorLimits {
		frames: 1,
		slots: 1,
	})
	.unwrap();
	let mut s = store();
	e.push_call(&mut s, 1, Args::Values(&[]), frame()).unwrap();
	let before = witness(&e);
	assert_eq!(
		e.push_call(&mut s, 1, Args::Values(&[]), frame())
			.unwrap_err()
			.error
			.kind,
		ErrorKind::FrameCapacity
	);
	assert_eq!(witness(&e), before);
	e.leave(&mut s).unwrap();
	assert!(e.is_idle());
}

#[cfg(feature = "fresh-core-native")]
#[test]
fn native_room_accounts_for_every_physical_buffer_and_logical_limit() {
	let limits = ExecutorLimits {
		frames: 100,
		slots: 1000,
	};
	for caps in [(4, 200, 8), (8, 200, 4), (8, 17, 8)] {
		let room = region_room(3, 16, 16, limits, caps);
		assert_eq!(room.frames, caps.0.min(caps.2) - 3);
		assert_eq!(room.slots, caps.1 - 16);
	}
	let room = region_room(
		3,
		16,
		16,
		ExecutorLimits {
			frames: 3,
			slots: 16,
		},
		(8, 200, 8),
	);
	assert_eq!(room.frames, 0);
	assert_eq!(room.slots, 0);
	assert_eq!(region_room(3, 16, 15, limits, (8, 200, 8)).slots, 0);
}

#[cfg(all(
	feature = "fresh-core-native",
	target_arch = "x86_64",
	target_os = "linux"
))]
#[test]
#[cfg_attr(miri, ignore = "Miri: executes generated native code")]
fn tight_native_room_falls_back_then_warmed_native_path_keeps_capacities() {
	use crate::experimental_core::{frontend, region::hook};
	use crate::{Context, Diagnostics, Hash, Source, Sources};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory("fn add(a,b) { a+b } pub fn main(x) { let total=0; for i in 0..x { total=add(total,i); } total }").unwrap()).unwrap();
	let p = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap();
	let main = p.function(Hash::type_hash(["main"])).unwrap();
	let width = p.code.function(main).unwrap().slots as usize;
	for tight in 0..3 {
		hook::uninstall();
		let mut s = store();
		let x = s.root_i64(12);
		let mut e = Executor::new().unwrap();
		let reference = e.invoke(&p, &mut s, main, &[&x], 10000).unwrap();
		let reference_value = s.root_scalar(&reference).unwrap();
		s.root_release(reference).unwrap();
		let reference_fuel = hook::take_final_fuel();
		e.frames = Vec::try_with_capacity(if tight == 0 { 1 } else { 8 }).unwrap();
		e.diagnostics = Vec::try_with_capacity(if tight == 2 { 1 } else { 8 }).unwrap();
		e.slots = Vec::try_with_capacity(if tight == 1 { width } else { 128 }).unwrap();
		let install = hook::install_native(&p.code, None);
		assert!(install.loaded > 0);
		let out = e.invoke(&p, &mut s, main, &[&x], 10000).unwrap();
		assert_eq!(s.root_scalar(&out).unwrap(), reference_value);
		s.root_release(out).unwrap();
		assert_eq!(hook::take_final_fuel(), reference_fuel);
		assert!(e.is_idle());
		assert_eq!(s.live_entries(), 0);
		let stats = hook::take_stats().unwrap();
		assert!(stats.refused_room > 0, "tight buffer {tight}: {stats:?}");
		let capacities = e.capacities();
		// This path allocates a RangeIterator; only bookkeeping capacities are checked.
		let out = e.invoke(&p, &mut s, main, &[&x], 10000).unwrap();
		s.root_release(out).unwrap();
		assert_eq!(e.capacities(), capacities);
		let stats = hook::uninstall().unwrap();
		assert!(stats.native_entries > 0);
		assert_eq!(stats.refused_room, 0);
		assert_eq!(stats.mismatches, 0);
	}
}

fn recursive_code() -> Code {
	use crate::experimental_core::arithmetic::Binary;
	let mut f = Function::new(4, 1).unwrap();
	let (args_start, args_len) = f.arguments(&[0]).unwrap();
	for (pc, op) in [
		Op::Literal {
			out: 1,
			value: Value::I64(1),
		},
		Op::Binary {
			out: 2,
			op: Binary::Le,
			lhs: 0,
			rhs: 1,
		},
		Op::Branch {
			condition: 2,
			yes: 6,
			no: 3,
		},
		Op::Binary {
			out: 0,
			op: Binary::Sub,
			lhs: 0,
			rhs: 1,
		},
		Op::Call {
			out: 3,
			function: 0,
			args_start,
			args_len,
		},
		Op::Return { value: 3 },
		Op::Return { value: 0 },
	]
	.into_iter()
	.enumerate()
	{
		f.push(
			op,
			Location {
				source: 3,
				revision: 1,
				start: pc as u32,
				end: pc as u32 + 1,
			},
		)
		.unwrap();
	}
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}

#[test]
#[allow(clippy::result_large_err)]
fn configured_depth_exceeds_old_limit_and_warmed_scalar_calls_allocate_nothing() {
	let code = recursive_code();
	let mut s = store();
	let x = s.root_i64(1100);
	let mut default = Executor::new().unwrap();
	let failed = default
		.invoke_code(&code, &mut s, 0, &[&x], 10000)
		.unwrap_err();
	assert_eq!(failed.error.kind, ErrorKind::FrameCapacity);
	assert_eq!(failed.error.location.unwrap().start, 4);
	assert_eq!(failed.call_chain.len(), MAX_FRAMES - 1);
	assert!(failed.cleanup_error.is_none());
	assert!(default.is_idle());
	let mut e = Executor::with_limits(ExecutorLimits {
		frames: 2048,
		slots: 8192,
	})
	.unwrap();
	let warm = e.invoke_code(&code, &mut s, 0, &[&x], 10000).unwrap();
	s.root_release(warm).unwrap();
	let capacities = e.capacities();
	assert!(capacities.0 >= 1100 && capacities.1 >= 4400 && capacities.2 >= 1100);
	for _ in 0..2 {
		let out = rune_alloc::limit::with(0, || e.invoke_code(&code, &mut s, 0, &[&x], 10000))
			.call()
			.unwrap();
		assert_eq!(
			s.root_scalar(&out).unwrap(),
			Some(super::super::value::Scalar::I64(1))
		);
		s.root_release(out).unwrap();
		assert_eq!(e.capacities(), capacities);
		assert!(e.is_idle());
	}
	assert_eq!(s.live_entries(), 0);
	std::eprintln!(
		"B3 Frame={} Location={} Value={} capacities={capacities:?} modeled_bytes={}",
		core::mem::size_of::<Frame>(),
		core::mem::size_of::<Location>(),
		core::mem::size_of::<Value>(),
		e.bookkeeping_bytes().unwrap()
	);
}
