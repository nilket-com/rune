use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};
fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 0,
		start: n,
		end: n + 1,
	}
}
fn compound_code(filler: usize) -> Code {
	compound_code_rhs(filler, Value::I64(1))
}
fn compound_code_rhs(filler: usize, rhs: Value) -> Code {
	let mut f = Function::new(3, 1).unwrap();
	let key = f.string("x").unwrap();
	for _ in 0..filler {
		f.push(
			Op::Literal {
				out: 1,
				value: Value::Unit,
			},
			loc(0),
		)
		.unwrap();
	}
	f.push(Op::Literal { out: 1, value: rhs }, loc(1)).unwrap();
	f.push(
		Op::FieldGetMut {
			out: 2,
			value: 0,
			key,
		},
		loc(2),
	)
	.unwrap();
	f.push(
		Op::AssignBinary {
			out: 2,
			lhs: 2,
			rhs: 1,
			op: arithmetic::Binary::Add,
		},
		loc(3),
	)
	.unwrap();
	f.push(
		Op::FieldSetMut {
			value: 0,
			key,
			new: 2,
		},
		loc(4),
	)
	.unwrap();
	f.push(Op::Return { value: 2 }, loc(5)).unwrap();
	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}
#[test]
fn compound_overflow_leaves_host_owned_field_unchanged() {
	let code = compound_code(0);
	let mut store = Store::new(Limits::default()).unwrap();
	let object = store
		.record(
			super::super::value::Type::Object,
			&[("x", Value::I64(i64::MAX))],
		)
		.unwrap();
	let mut e = Executor::new().unwrap();
	let error = e
		.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[object]), 100)
		.unwrap_err();
	assert_eq!(error.error.kind, ErrorKind::Overflow);
	assert_eq!(store.field(object, "x").unwrap(), Value::I64(i64::MAX));
	store.set_field(object, "after", Value::I64(1)).unwrap();
	store.release(object).unwrap();
	assert_eq!(store.live_entries(), 0);
}
#[test]
fn named_compound_fuel_boundaries_preserve_reusable_state() {
	for filler in 250..260 {
		let code = compound_code(filler);
		for fuel in 0..4 {
			let mut store = Store::new(Limits::default()).unwrap();
			let object = store
				.record(super::super::value::Type::Object, &[("x", Value::I64(1))])
				.unwrap();
			let mut e = Executor::new().unwrap();
			match e.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[object]), fuel) {
				Ok(value) => {
					assert_eq!(value, Value::I64(2));
					store.release(value).unwrap();
				}
				Err(f) => assert_eq!(f.error.kind, ErrorKind::FuelExhausted),
			}
			let current = store.field(object, "x").unwrap();
			assert!(matches!(current, Value::I64(1 | 2)));
			store.release(object).unwrap();
			assert_eq!(store.live_entries(), 0);
		}
	}
}
#[test]
fn missing_instance_accessor_is_root_only_and_detail_stays_small() {
	assert!(size_of::<Detail>() <= 32);
	let instance = TypeName::I64.builtin_hash().unwrap();
	let hash = crate::Hash::associated_function(instance, &crate::runtime::Protocol::INTO_ITER);
	let detail = Detail::MissingInstanceFunction {
		target: TypeName::I64,
		hash,
		instance,
	};
	let mut failure = Failure {
		remaining_work: None,
		numeric_context: None,
		error: Error::new(ErrorKind::Type, "store").with_detail(detail),
		missing_key: None,
		panic_message: None,
		host_message: None,
		call_chain: Vec::new(),
		cleanup_error: None,
	};
	let accessor = failure.missing_instance_function().unwrap();
	assert_eq!((accessor.hash, accessor.instance), (hash, instance));
	let p = Program::for_diagnostics_test(Builder::default().finish().unwrap());
	assert_eq!(std::format!("{}", accessor.instance_name(&p)), "std::i64");
	failure.cleanup_error = Some(failure.error);
	failure.error.detail = Detail::None;
	assert!(failure.missing_instance_function().is_none());
}

#[test]
fn iterator_exit_paths_release_object_lease_and_yielded_pairs() {
	for exit in 0..3 {
		let mut f = Function::new(6, 1).unwrap();
		f.push(Op::IterStart { out: 1, value: 0 }, loc(0)).unwrap();
		f.push(
			Op::IterNext {
				out: 2,
				iterator: 1,
				none: 5,
			},
			loc(1),
		)
		.unwrap();
		match exit {
			0 => {
				// break: release iterator explicitly, then return a scalar.
				f.push(Op::Release { value: 1 }, loc(2)).unwrap();
				f.push(
					Op::Literal {
						out: 3,
						value: Value::I64(1),
					},
					loc(3),
				)
				.unwrap();
				f.push(Op::Return { value: 3 }, loc(4)).unwrap();
			}
			1 => {
				// return from the body: frame unwind owns the iterator.
				f.push(
					Op::Literal {
						out: 3,
						value: Value::I64(1),
					},
					loc(2),
				)
				.unwrap();
				f.push(Op::Return { value: 3 }, loc(3)).unwrap();
				f.push(Op::Return { value: 3 }, loc(4)).unwrap();
			}
			_ => {
				f.push(
					Op::Literal {
						out: 3,
						value: Value::I64(0),
					},
					loc(2),
				)
				.unwrap();
				f.push(
					Op::Binary {
						out: 4,
						op: arithmetic::Binary::Div,
						lhs: 3,
						rhs: 3,
					},
					loc(3),
				)
				.unwrap();
				f.push(Op::Return { value: 4 }, loc(4)).unwrap();
			}
		}
		f.push(Op::Return { value: 0 }, loc(5)).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		let code = b.finish().unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let object = store
			.record(super::super::value::Type::Object, &[("x", Value::I64(1))])
			.unwrap();
		let mut e = Executor::new().unwrap();
		match e.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[object]), 100) {
			Ok(value) => {
				assert!(exit < 2);
				store.release(value).unwrap();
			}
			Err(f) => {
				assert_eq!(exit, 2);
				assert_eq!(f.error.kind, ErrorKind::DivisionByZero);
				assert!(f.cleanup_error.is_none());
			}
		}
		assert_eq!(store.live_entries(), 1);
		store.set_field(object, "after", Value::I64(2)).unwrap();
		store.release(object).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
fn compound_failure_groups_preserve_host_record() {
	for group in 0..3 {
		let code = compound_code_rhs(
			0,
			if group == 0 {
				Value::Bool(true)
			} else {
				Value::I64(1)
			},
		);
		let mut store = Store::new(Limits::default()).unwrap();
		let tag = if group == 2 {
			super::super::value::Type::Struct(
				super::super::nominal::intern(0xf1e1d000185, "field_test::P").unwrap(),
			)
		} else {
			super::super::value::Type::Object
		};
		let key = if group == 2 { "other" } else { "x" };
		let object = store.record(tag, &[(key, Value::I64(1))]).unwrap();
		let iterator = if group == 1 {
			Some(store.iterator(object).unwrap())
		} else {
			None
		};
		let before = store.format_value(object, true).unwrap();
		let mut e = Executor::new().unwrap();
		let failure = e
			.invoke_args_for_test(&code, &mut store, 0, Args::Values(&[object]), 100)
			.unwrap_err();
		assert_eq!(
			failure.error.kind,
			if group == 1 {
				ErrorKind::AccessConflict
			} else {
				ErrorKind::Type
			}
		);
		assert_eq!(store.format_value(object, true).unwrap(), before);
		assert!(failure.cleanup_error.is_none());
		if let Some(iterator) = iterator {
			store.release(iterator).unwrap();
		}
		store.set_field(object, key, Value::I64(2)).unwrap();
		store.release(object).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}
