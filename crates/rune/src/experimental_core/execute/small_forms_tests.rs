//! Callback ownership tests without frontend compilation or a legacy VM.
use super::super::{
	code::{Builder, Function},
	value::Type,
};
use super::*;
fn any_code(fail: bool) -> Code {
	let mut builder = Builder::default();
	builder
		.push(super::super::lower::small_forms::any_function().unwrap())
		.unwrap();
	let at = Location {
		source: 0,
		revision: 0,
		start: 1,
		end: 2,
	};
	let mut predicate = Function::new(3, 1).unwrap();
	if fail {
		predicate
			.push(
				Op::Literal {
					out: 1,
					value: Value::I64(0),
				},
				at,
			)
			.unwrap();
		predicate
			.push(
				Op::Binary {
					out: 2,
					op: arithmetic::Binary::Div,
					lhs: 0,
					rhs: 1,
				},
				at,
			)
			.unwrap();
	} else {
		predicate
			.push(
				Op::Literal {
					out: 2,
					value: Value::Bool(false),
				},
				at,
			)
			.unwrap();
	}
	predicate.push(Op::Return { value: 2 }, at).unwrap();
	builder.push(predicate).unwrap();
	builder.finish().unwrap()
}
#[test]
fn any_callback_failure_releases_only_its_iterator_owner() {
	let code = any_code(true);
	let mut store = Store::new(Default::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let parent = store
		.list(Type::Vec, &[Value::I64(1), Value::I64(2)])
		.unwrap();
	let iter = store.iterator(parent).unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	let failure = executor
		.invoke_args_for_test(
			&code,
			&mut store,
			0,
			Args::Values(&[
				iter,
				Value::Function {
					code: code.id(),
					id: 1,
				},
			]),
			1000,
		)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::DivisionByZero);
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	assert_eq!(
		store.push(parent, Value::I64(3)).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	assert_eq!(store.iterator_next(iter).unwrap(), Some(Value::I64(2)));
	store.release(iter).unwrap();
	store.push(parent, Value::I64(3)).unwrap();
	store.release(parent).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	let success = any_code(false);
	let parent = store.list(Type::Vec, &[]).unwrap();
	let iter = store.iterator(parent).unwrap();
	let result = executor
		.invoke_args_for_test(
			&success,
			&mut store,
			0,
			Args::Values(&[
				iter,
				Value::Function {
					code: success.id(),
					id: 1,
				},
			]),
			1000,
		)
		.unwrap();
	assert_eq!(result, Value::Bool(false));
	store.release(iter).unwrap();
	store.release(parent).unwrap();
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}
#[test]
fn any_fuel_cut_drops_callback_and_frame_owners() {
	let code = any_code(false);
	let mut store = Store::new(Default::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let parent = store
		.list(Type::Vec, &[Value::I64(1), Value::I64(2)])
		.unwrap();
	let iter = store.iterator(parent).unwrap();
	let failure = executor
		.invoke_args_for_test(
			&code,
			&mut store,
			0,
			Args::Values(&[
				iter,
				Value::Function {
					code: code.id(),
					id: 1,
				},
			]),
			2,
		)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::FuelExhausted);
	assert!(failure.cleanup_error.is_none());
	assert!(executor.is_idle());
	store.release(iter).unwrap();
	store.push(parent, Value::I64(3)).unwrap();
	store.release(parent).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

/// Runs `ITER_METHOD` on `receiver` from slot 0 into slot 1 and returns the iterator.
fn iter_method(executor: &mut Executor, store: &mut Store, receiver: Value) -> Value {
	let mut function = Function::new(2, 1).unwrap();
	let (args_start, args_len) = function.arguments(&[0]).unwrap();
	executor.slots.try_push(receiver).unwrap();
	executor.slots.try_push(Value::Unit).unwrap();
	executor
		.small_forms_call(
			store,
			&function,
			0,
			1,
			super::super::code::ITER_METHOD,
			args_start,
			args_len,
		)
		.unwrap();
	let iterator = executor.slots[1];
	executor.slots.clear();
	iterator
}

#[test]
fn unit_iter_method_is_an_empty_slice_iterator() {
	let mut executor = Executor::new().unwrap();
	let mut store = Store::new(Default::default()).unwrap();
	let iterator = iter_method(&mut executor, &mut store, Value::Unit);
	assert_eq!(store.diagnostic_type(iterator), Some(TypeName::VecIter));
	assert_eq!(store.iterator_next(iterator).unwrap(), None);
	assert_eq!(store.iterator_next(iterator).unwrap(), None);
	store.release(iterator).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	assert!(executor.is_idle());
}

#[test]
fn tuple_and_object_iter_methods_lease_their_receiver() {
	let mut executor = Executor::new().unwrap();
	let mut store = Store::new(Default::default()).unwrap();
	let tuple = store
		.list(Type::Tuple, &[Value::I64(1), Value::I64(2)])
		.unwrap();
	let iterator = iter_method(&mut executor, &mut store, tuple);
	assert_eq!(store.diagnostic_type(iterator), Some(TypeName::VecIter));
	assert_eq!(store.iterator_next(iterator).unwrap(), Some(Value::I64(1)));
	store.release(iterator).unwrap();
	store.release(tuple).unwrap();

	let object = store.record(Type::Object, &[("k", Value::I64(3))]).unwrap();
	let iterator = iter_method(&mut executor, &mut store, object);
	assert_eq!(store.diagnostic_type(iterator), Some(TypeName::ObjectIter));
	// The live iterator keeps the object read-borrowed.
	assert!(store.set_field(object, "z", Value::I64(4)).is_err());
	let pair = store.iterator_next(iterator).unwrap().unwrap();
	assert_eq!(store.get(pair, 1).unwrap(), Value::I64(3));
	store.release(pair).unwrap();
	assert_eq!(store.iterator_next(iterator).unwrap(), None);
	store.release(iterator).unwrap();
	store.set_field(object, "z", Value::I64(4)).unwrap();
	store.release(object).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	assert!(executor.is_idle());
}
