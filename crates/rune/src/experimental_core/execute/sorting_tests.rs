use super::*;
use crate::experimental_core::code::{Builder, Function, METHOD_CMP};
fn at() -> Location {
	Location {
		source: 11,
		revision: 1,
		start: 1,
		end: 2,
	}
}
fn code(kind: u8) -> Code {
	let mut b = Builder::default();
	b.push(super::super::lower::sorting::function(at()).unwrap())
		.unwrap();
	let mut cmp = Function::new(3, if kind == 3 { 0 } else { 2 }).unwrap();
	match kind {
		0 => {
			let (args_start, args_len) = cmp.arguments(&[0, 1]).unwrap();
			cmp.push(
				Op::Call {
					out: 2,
					function: METHOD_CMP,
					args_start,
					args_len,
				},
				at(),
			)
			.unwrap();
		}
		1 => {
			cmp.push(
				Op::Literal {
					out: 2,
					value: Value::I64(7),
				},
				at(),
			)
			.unwrap();
		}
		2 => {
			let text = cmp.string("stop").unwrap();
			cmp.push(Op::String { out: 2, text }, at()).unwrap();
			cmp.push(Op::Panic { value: 2 }, at()).unwrap();
		}
		3 => {
			cmp.push(
				Op::Literal {
					out: 2,
					value: Value::Ordering(core::cmp::Ordering::Equal),
				},
				at(),
			)
			.unwrap();
		}
		_ => unreachable!(),
	}
	cmp.push(Op::Return { value: 2 }, at()).unwrap();
	b.push(cmp).unwrap();
	b.finish().unwrap()
}
#[test]
fn merge_calls_are_ordinary_and_all_exit_paths_release_the_guard() {
	for kind in 0..4 {
		for len in 0..=5 {
			let c = code(kind);
			let mut s = Store::new(Default::default()).unwrap();
			let mut e = Executor::new().unwrap();
			let mut items = Vec::new();
			for i in (0..len).rev() {
				items.try_push(Value::I64(i)).unwrap();
			}
			let parent = s.list(super::super::value::Type::Vec, &items).unwrap();
			let baseline = (s.live_entries(), s.payload_bytes());
			let result = e.invoke_args_for_test(
				&c,
				&mut s,
				0,
				Args::Values(&[
					parent,
					Value::Function {
						code: c.id(),
						id: 1,
					},
				]),
				100000,
			);
			if kind == 0 || len < 2 {
				let value = result.unwrap();
				assert_eq!(value, Value::Unit);
			} else {
				let f = result.unwrap_err();
				assert!(f.cleanup_error.is_none());
				assert_eq!(
					f.error.kind,
					if kind == 2 {
						ErrorKind::Panic
					} else if kind == 3 {
						ErrorKind::ArgumentCount
					} else {
						ErrorKind::Type
					}
				);
			}
			assert!(e.is_idle());
			assert_eq!((s.live_entries(), s.payload_bytes()), baseline);
			for i in 0..len {
				let v = s.get(parent, i as usize).unwrap();
				let expected = if kind == 0 { i } else { len - 1 - i };
				assert_eq!(v, Value::I64(expected));
				s.release(v).unwrap();
			}
			s.release(parent).unwrap();
			assert_eq!(s.live_entries(), 0);
		}
	}
}
#[test]
#[cfg(not(miri))]
fn fuel_at_every_sort_instruction_releases_parent_lease() {
	let c = code(0);
	for fuel in 0..=512 {
		let mut s = Store::new(Default::default()).unwrap();
		let mut e = Executor::new().unwrap();
		let parent = s
			.list(
				super::super::value::Type::Vec,
				&[Value::I64(3), Value::I64(1), Value::I64(2)],
			)
			.unwrap();
		let base = (s.live_entries(), s.payload_bytes());
		let result = e.invoke_args_for_test(
			&c,
			&mut s,
			0,
			Args::Values(&[
				parent,
				Value::Function {
					code: c.id(),
					id: 1,
				},
			]),
			fuel,
		);
		if let Err(f) = result {
			assert_eq!(f.error.kind, ErrorKind::FuelExhausted);
			assert!(f.cleanup_error.is_none());
		}
		assert!(e.is_idle());
		assert_eq!((s.live_entries(), s.payload_bytes()), base);
		assert_eq!(s.len(parent).unwrap(), 3);
		s.release(parent).unwrap();
		assert_eq!(s.live_entries(), 0);
	}
}
#[test]
#[cfg_attr(miri, ignore = "default-module frontend; native admission control")]
fn named_and_noncapturing_closures_lower_without_legacy_assembly() {
	for text in [
		"pub fn main(){let v=[3,1,2];v.sort_by(|a,b|a.cmp(b));v[0]}",
		"fn cmp(a,b){a.cmp(b)} pub fn main(){let v=[3,1,2];v.sort_by(cmp);v[0]}",
		"pub fn main(){let bias=1;let v=[3,1,2];v.sort_by(|a,b|if bias==1{a.cmp(b)}else{b.cmp(a)});v[0]}",
	] {
		let cx = crate::Context::with_default_modules().unwrap();
		let mut sources = crate::Sources::new();
		sources
			.insert(crate::Source::memory(text).unwrap())
			.unwrap();
		let p =
			super::super::frontend::compile(&mut sources, &cx, &mut crate::Diagnostics::new(), 11)
				.unwrap();
		let mut s = Store::new(Default::default()).unwrap();
		let mut e = Executor::new().unwrap();
		let r = e
			.invoke(
				&p,
				&mut s,
				p.function(crate::Hash::type_hash(["main"])).unwrap(),
				&[],
				100000,
			)
			.unwrap();
		assert_eq!(
			s.root_scalar(&r).unwrap(),
			Some(super::super::value::Scalar::I64(1))
		);
		s.root_release(r).unwrap();
		assert!(e.is_idle());
		assert_eq!(s.live_entries(), 0);
	}
}
#[test]
#[cfg_attr(
	miri,
	ignore = "default-module frontend; native source arithmetic gate"
)]
fn depth_frame_and_slot_ledger() {
	let text = r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########;
	let mut cx = crate::Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c11"]).unwrap();
	module
		.fresh_host_function("tick", |_: i64, _: i64| -> i64 { 7 })
		.unwrap();
	module
		.fresh_host_function("mark", |_: &str| -> i64 { 7 })
		.unwrap();
	cx.install(module).unwrap();
	let mut sources = crate::Sources::new();
	sources
		.insert(crate::Source::memory(text).unwrap())
		.unwrap();
	let p = super::super::frontend::compile(&mut sources, &cx, &mut crate::Diagnostics::new(), 11)
		.unwrap();
	println!(
		"\nC11_DEPTH_METADATA {}",
		super::super::code::c11_depth_metadata(&p)
	);
	let helper = super::super::lower::sorting::function(at()).unwrap();
	assert_eq!(helper.slots, 25);
	for id in 0..p.code.functions() {
		let f = p.code.function(id as u32).unwrap();
		assert!(f.slots <= 64);
		eprintln!(
			"C11 depth function {id}: slots={} parameters={}",
			f.slots, f.parameters
		);
	}
	assert_eq!(core::mem::size_of::<Value>(), 16);
	for (n, frames, ticks, success) in [
		(8, 19, 9, true),
		(31, 65, 32, true),
		(32, 67, 33, true),
		(33, 69, 34, true),
		(127, 257, 128, true),
		(510, 1023, 511, true),
		(511, 1025, 511, false),
		(512, 1027, 511, false),
	] {
		assert_eq!(2 * n + 3, frames);
		assert_eq!(frames <= 1024, success);
		assert_eq!((n + 1).min(511), ticks);
	}
}
#[test]
fn sort_intrinsic_invalid_metadata_is_pure() {
	use super::super::code::{SORT_BEGIN, SORT_LEN, SORT_ORDERING};
	let c = code(0);
	let mut f = Function::new(2, 0).unwrap();
	f.arguments(&[u32::MAX, 0]).unwrap();
	let mut s = Store::new(Default::default()).unwrap();
	let mut e = Executor::new().unwrap();
	e.slots.try_push(Value::I64(4)).unwrap();
	e.slots.try_push(Value::Unit).unwrap();
	for (base, out, id, start, len) in [
		(0, 0, SORT_BEGIN, 0, 0),
		(0, u32::MAX, SORT_BEGIN, 0, 2),
		(0, 0, SORT_BEGIN, u32::MAX, 2),
		(0, 0, SORT_LEN, 0, 1),
		(0, 0, SORT_ORDERING + 1, 0, 1),
		(usize::MAX, 0, SORT_LEN, 1, 1),
	] {
		let error = e
			.sort_call(&c, &mut s, &f, base, out, id, start, len)
			.unwrap_err();
		assert_eq!(error.kind, ErrorKind::InvalidCode);
		assert_eq!(e.slots.as_slice(), &[Value::I64(4), Value::Unit]);
		assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	}
	e.slots.clear();
	assert!(e.is_idle());
}
#[test]
#[ignore = "C11 minimal Miri ordinary callback failure/unwind"]
fn miri_guard_unwind_is_slot_owned() {
	let c = code(1);
	let mut s = Store::new(Default::default()).unwrap();
	let mut e = Executor::new().unwrap();
	let parent = s
		.list(
			super::super::value::Type::Vec,
			&[Value::I64(2), Value::I64(1)],
		)
		.unwrap();
	let before = (s.live_entries(), s.payload_bytes());
	let f = e
		.invoke_args_for_test(
			&c,
			&mut s,
			0,
			Args::Values(&[
				parent,
				Value::Function {
					code: c.id(),
					id: 1,
				},
			]),
			10000,
		)
		.unwrap_err();
	assert_eq!(f.error.kind, ErrorKind::Type);
	assert!(f.cleanup_error.is_none());
	assert!(e.is_idle());
	assert_eq!((s.live_entries(), s.payload_bytes()), before);
	assert_eq!(s.get(parent, 0).unwrap(), Value::I64(2));
	s.release(parent).unwrap();
	assert_eq!(s.live_entries(), 0);
}

// Deliberately use an ordinary user frame above the synthetic helper.
fn sentinel_user_entry_code() -> Code {
	let mut b = Builder::default();
	b.push(super::super::lower::sorting::function(at()).unwrap())
		.unwrap();
	let mut cmp = Function::new(3, 2).unwrap();
	cmp.push(
		Op::Literal {
			out: 2,
			value: Value::Ordering(core::cmp::Ordering::Equal),
		},
		at(),
	)
	.unwrap();
	cmp.push(Op::Return { value: 2 }, at()).unwrap();
	b.push(cmp).unwrap();
	let mut main = Function::new(3, 2).unwrap();
	let (args_start, args_len) = main.arguments(&[0, 1]).unwrap();
	main.push(
		Op::Call {
			out: 2,
			function: 0,
			args_start,
			args_len,
		},
		at(),
	)
	.unwrap();
	main.push(Op::Return { value: 2 }, at()).unwrap();
	b.push(main).unwrap();
	b.finish().unwrap()
}
fn sentinel_probe(fuel: usize, cancel: Option<usize>) -> (Code, Option<Failure>) {
	let code = sentinel_user_entry_code();
	assert!(code.function(0).unwrap().is_transparent_sort());
	assert!(!code.function(2).unwrap().is_transparent_sort());
	assert!(code.function(2).unwrap().instructions()[0].is_charged());
	assert!(code.function(0).unwrap().instructions()[0].is_charged());
	let mut store = Store::new(Default::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let parent = store
		.list(
			super::super::value::Type::Vec,
			&[Value::I64(2), Value::I64(1)],
		)
		.unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	cancel_at_charge_for_test(cancel);
	let result = executor.invoke_args_for_test(
		&code,
		&mut store,
		2,
		Args::Values(&[
			parent,
			Value::Function {
				code: code.id(),
				id: 1,
			},
		]),
		fuel,
	);
	cancel_at_charge_for_test(None);
	let failure = match result {
		Ok(value) => {
			store.release(value).unwrap();
			None
		}
		Err(f) => Some(f),
	};
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	store.release(parent).unwrap();
	assert_eq!(store.live_entries(), 0);
	(code, failure)
}
#[test]
fn helper_internal_fuel_and_cancel_resolve_user_callsite() {
	// One charged main Call enters the helper; its first charged SORT_BEGIN
	// exhausts fuel=1 or consumes cancel-at-charge=1, before any comparator.
	for (fuel, cancel, kind) in [
		(1, None, ErrorKind::FuelExhausted),
		(10000, Some(1), ErrorKind::Cancelled),
	] {
		let (_, failure) = sentinel_probe(fuel, cancel);
		let failure = failure.unwrap();
		assert_eq!(failure.error.kind, kind);
		assert_eq!(failure.error.location, Some(at()));
		assert!(failure.call_chain.is_empty());
		assert!(failure.cleanup_error.is_none());
	}
}
#[test]
fn every_failure_location_is_user_facing_and_never_sentinel() {
	let mut failures = 0;
	for (fuel, cancel) in (0..=48).map(|fuel| (fuel, None)).chain([(10000, Some(1))]) {
		let (code, failure) = sentinel_probe(fuel, cancel);
		let Some(failure) = failure else { continue };
		failures += 1;
		for location in failure
			.error
			.location
			.iter()
			.chain(failure.call_chain.iter())
			.chain(
				failure
					.cleanup_error
					.iter()
					.filter_map(|error| error.location.as_ref()),
			) {
			assert!(location.start <= location.end);
			assert_ne!(location.source, u32::MAX);
			assert_ne!(*location, super::super::code::TRANSPARENT_SORT_LOCATION);
		}
		let program = super::super::frontend::Program::for_diagnostics_test(code);
		for rendered in [
			format!("{failure:?}"),
			format!("{}", failure.message(&program)),
			format!("{}", failure.error.message(&program)),
		] {
			assert!(!rendered.contains("source: 4294967295"));
			assert!(!rendered.contains("start: 4294967295"));
		}
	}
	assert!(failures >= 2);
}

#[test]
fn cold_primary_resolution_handles_surviving_and_popped_helpers() {
	let code = sentinel_user_entry_code();
	let mut executor = Executor::new().unwrap();
	let caller = Frame {
		function: 2,
		pc: 1,
		base: 0,
		out: None,
		callsite_pc: NO_CALLSITE,
	};
	let helper = Frame {
		function: 0,
		pc: 1,
		base: 3,
		out: Some(2),
		callsite_pc: 0,
	};
	executor.frames.try_push(caller).unwrap();
	executor.frames.try_push(helper).unwrap();
	let marker = super::super::code::TRANSPARENT_SORT_LOCATION;
	for popped in [false, true] {
		if popped {
			executor.frames.pop();
		}
		let mut failure = error(ErrorKind::Allocation).at(marker);
		executor.resolve_failure_location(&code, &mut failure);
		assert_eq!(failure.location, Some(at()));
		assert_eq!(failure.kind, ErrorKind::Allocation);
		let mut ordinary = error(ErrorKind::Type).at(at());
		executor.resolve_failure_location(&code, &mut ordinary);
		assert_eq!(ordinary.location, Some(at()));
	}
	executor.frames.clear();
	executor
		.frames
		.try_push(Frame {
			callsite_pc: NO_CALLSITE,
			..helper
		})
		.unwrap();
	let mut root = error(ErrorKind::FuelExhausted).at(marker);
	executor.resolve_failure_location(&code, &mut root);
	assert_eq!(root.location, None);
	executor.frames.clear();
	executor.resolve_failure_location(&code, &mut root);
	assert_eq!(root.location, None);
	assert!(executor.is_idle());
}
