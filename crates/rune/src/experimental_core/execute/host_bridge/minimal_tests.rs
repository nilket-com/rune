//! Minimal host-boundary fixtures. No source compiler or std-module installation.
use super::*;
use crate::experimental_core::{
	code::{Builder, Function},
	heap::Limits,
};
use crate::{Context, Hash, Module};

fn location() -> Location {
	Location {
		source: 0,
		revision: 1,
		start: 1,
		end: 2,
	}
}
pub(super) fn code(context: &Context, name: &str, parameters: u32) -> Code {
	let binding = context.host(Hash::type_hash(["tiny", name])).unwrap();
	let mut builder = Builder::default();
	let id = builder.host(binding).unwrap();
	let mut function = Function::new(parameters + 1, parameters).unwrap();
	let indices: std::vec::Vec<_> = (0..parameters).collect();
	let (args_start, args_len) = function.arguments(&indices).unwrap();
	function
		.push(
			Op::Call {
				out: parameters,
				function: id,
				args_start,
				args_len,
			},
			location(),
		)
		.unwrap();
	function
		.push(Op::Return { value: parameters }, location())
		.unwrap();
	builder.push(function).unwrap();
	builder.finish().unwrap()
}
pub(super) fn executor(context: &Context) -> Executor {
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	executor
}
pub(super) fn installed(module: Module) -> Context {
	let mut context = Context::new();
	context.install(module).unwrap();
	context
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_scalar_memory_call() {
	let mut module = Module::with_item(["tiny"]).unwrap();
	module.host_function("add", |a: i64, b: i64| a + b).unwrap();
	let context = installed(module);
	let code = code(&context, "add", 2);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let a = store.root_i64(2);
	let b = store.root_i64(3);
	let result = executor
		.invoke_code(&code, &mut store, 0, &[&a, &b], 100)
		.unwrap();
	assert_eq!(result.value, Value::I64(5));
	store.root_release(result).unwrap();
	store.root_release(a).unwrap();
	store.root_release(b).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_string_alias_memory_call() {
	let mut module = Module::with_item(["tiny"]).unwrap();
	module
		.host_function("cat", |a: &str, b: &str| -> std::string::String {
			assert_eq!(a.as_ptr(), b.as_ptr());
			format!("{a}{b}")
		})
		.unwrap();
	let context = installed(module);
	let code = code(&context, "cat", 2);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let input = store.root_string("x").unwrap();
	let result = executor
		.invoke_code(&code, &mut store, 0, &[&input, &input], 100)
		.unwrap();
	assert_eq!(store.root_text(&result).unwrap(), "xx");
	store.root_release(result).unwrap();
	store.root_release(input).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_owned_variants() {
	let mut module = Module::with_item(["tiny"]).unwrap();
	module
		.host_function("o", || -> Option<std::string::String> { Some("x".into()) })
		.unwrap();
	module
		.host_function("e", || -> core::result::Result<i64, std::string::String> {
			Err("y".into())
		})
		.unwrap();
	let context = installed(module);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	for (name, tag, flag, text) in [
		("o", Type::Option, true, "x"),
		("e", Type::Result, false, "y"),
	] {
		let code = code(&context, name, 0);
		let root = executor
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap();
		let (actual_tag, actual_flag, child) = store.variant_parts(root.value).unwrap();
		assert_eq!((actual_tag, actual_flag), (tag, flag));
		assert_eq!(store.text(child.unwrap()).unwrap(), text);
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
	}
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_unwind_clears_scratch_and_is_reusable() {
	let mut module = Module::with_item(["tiny"]).unwrap();
	module
		.host_function("panic", |_: &str| -> i64 { panic!("tiny panic") })
		.unwrap();
	module.host_function("ok", || 7i64).unwrap();
	let context = installed(module);
	let failing = code(&context, "panic", 1);
	let good = code(&context, "ok", 0);
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let input = store.root_string("x").unwrap();
	let failure = executor
		.invoke_code(&failing, &mut store, 0, &[&input], 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Host);
	assert_eq!(failure.error.detail, Detail::HostPanicked);
	assert_eq!(failure.host_message.as_deref(), Some("tiny panic"));
	store.root_release(input).unwrap();
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
	let root = executor
		.invoke_code(&good, &mut store, 0, &[], 100)
		.unwrap();
	assert_eq!(root.value, Value::I64(7));
	store.root_release(root).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_identity_mismatch_has_no_effect() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};
	let calls = Arc::new(AtomicUsize::new(0));
	fn context(calls: Arc<AtomicUsize>) -> Context {
		let mut module = Module::with_item(["tiny"]).unwrap();
		module
			.host_function("f", move || {
				calls.fetch_add(1, Ordering::SeqCst);
				1i64
			})
			.unwrap();
		installed(module)
	}
	let first = context(calls.clone());
	let second = context(calls.clone());
	let code = code(&first, "f", 0);
	let mut executor = executor(&second);
	let mut store = Store::new(Limits::default()).unwrap();
	let failure = executor
		.invoke_code(&code, &mut store, 0, &[], 100)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(failure.error.location.is_none());
	assert_eq!(calls.load(Ordering::SeqCst), 0);
	assert_eq!(store.live_entries(), 0);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(miri, ignore = "focused host Miri: exact-name operator run only")]
fn minimal_malformed_shapes_drop_outputs_and_allow_reuse() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};
	for (which, shape) in [
		(0u8, HostOutputKind::Option(HostType::I64)),
		(1, HostOutputKind::Result(HostType::I64, HostType::I64)),
		(2, HostOutputKind::Scalar(HostType::I64)),
		(3, HostOutputKind::Scalar(HostType::String)),
	] {
		let calls = Arc::new(AtomicUsize::new(0));
		let captured = calls.clone();
		let mut module = Module::with_item(["tiny"]).unwrap();
		module
			.raw_function("bad", move |memory: &mut dyn stock::Memory, _, _, out| {
				captured.fetch_add(1, Ordering::SeqCst);
				match which {
					0 => memory.store(out, "wrong")?,
					1 => memory.store(out, Some(stock::Value::from(1i64)))?,
					2 => memory.store(
						out,
						Some(stock::Value::new(Some(stock::Value::from(1i64)))?),
					)?,
					3 => memory.store(out, 1i64)?,
					_ => unreachable!(),
				}
				Ok(())
			})
			.build()
			.unwrap();
		module.host_function("ok", || 7i64).unwrap();
		let mut context = installed(module);
		context
			.host_for_test(
				Hash::type_hash(["tiny", "bad"]),
				HostSignature {
					arity: 0,
					arguments: [HostType::Unit; 4],
					output: shape,
				},
			)
			.unwrap();
		let bad = code(&context, "bad", 0);
		let good = code(&context, "ok", 0);
		let mut executor = executor(&context);
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = executor
			.invoke_code(&bad, &mut store, 0, &[], 100)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
		assert_eq!(failure.error.location, Some(location()));
		assert!(failure.cleanup_error.is_none());
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		let root = executor
			.invoke_code(&good, &mut store, 0, &[], 100)
			.unwrap();
		assert_eq!(root.value, Value::I64(7));
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(calls.load(Ordering::SeqCst), 1);
	}
}

#[test]
fn typed_host_conversion_metadata_comes_from_the_real_native_error() {
	use crate::TypeHash;
	use core::cmp::Ordering;
	let mut executor = Executor::new().unwrap();
	let native = i64::from_value(stock::Value::from(Ordering::Less)).unwrap_err();
	let native = stock::VmError::with_error(
		stock::VmError::from(native),
		stock::VmErrorKind::BadArgument { arg: 0 },
	);
	let error = executor.host_error(native);
	assert_eq!(error.kind, ErrorKind::Host);
	assert_eq!(
		error.detail,
		Detail::HostExpectedNumber {
			actual: Ordering::HASH,
			arg: Some(0)
		}
	);
	assert_eq!(
		error.detail.numeric_argument(),
		Some((
			crate::experimental_core::error::HostCause::ExpectedNumber,
			0
		))
	);
	let native = bool::from_value(stock::Value::from(Ordering::Greater)).unwrap_err();
	let native = stock::VmError::with_error(
		stock::VmError::from(native),
		stock::VmErrorKind::BadArgument { arg: 1 },
	);
	let error = executor.host_error(native);
	assert_eq!(
		error.detail,
		Detail::HostExpectedType {
			expected: bool::HASH,
			actual: Ordering::HASH,
			arg: Some(1)
		}
	);
	assert_eq!(
		error.detail.numeric_argument(),
		Some((crate::experimental_core::error::HostCause::ExpectedType, 1))
	);
	assert_eq!(core::mem::size_of::<Detail>(), 32);
	assert_eq!(core::mem::size_of::<Error>(), 88);
	assert_eq!(core::mem::size_of::<Failure>(), 272);
}

#[test]
fn failure_display_keeps_native_type_and_number_conversion_messages() {
	use crate::experimental_core::frontend::Program;
	use std::string::ToString;
	let mut module = Module::with_item(["tiny"]).unwrap();
	module.host_function("expect_bool", |_: bool| ()).unwrap();
	module.host_function("expect_i64", |_: i64| ()).unwrap();
	let context = installed(module);
	for (name, expected) in [
		(
			"expect_bool",
			"Expected type `std::bool` but found `std::cmp::Ordering`",
		),
		(
			"expect_i64",
			"Expected number type, but found `std::cmp::Ordering`",
		),
	] {
		let program = Program::for_diagnostics_test(code(&context, name, 1));
		let mut executor = executor(&context);
		let mut store = Store::new(Limits::default()).unwrap();
		let input = store.root_ordering(core::cmp::Ordering::Less);
		let failure = executor
			.invoke_code(&program.code, &mut store, 0, &[&input], 100)
			.unwrap_err();
		assert_eq!(failure.host_message.as_deref(), Some(expected));
		assert_eq!(failure.message(&program).to_string(), expected);
		assert!(failure.cleanup_error.is_none());
		store.root_release(input).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		assert!(executor.is_idle());
	}
}
