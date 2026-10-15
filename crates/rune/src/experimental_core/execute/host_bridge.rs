//! Cold, closed-shape host bridge. No stock VM/environment is installed.
use super::super::value::Type;
use super::*;
use crate as rune;

use crate::function::fresh_host::{HostOutput, HostSignature, HostType};
use crate::runtime::{self as stock, FromValue};

// Fresh canonical Iterator diagnostics are explicitly Retained vs stock origins.
#[derive(crate::Any)]
#[rune(item = ::std::iter, name = Iterator)]
struct IteratorWitness;

// Best-effort bounded diagnostic copy; failure never changes the primary.
fn owned_message(value: &dyn core::fmt::Display, limit: usize) -> Option<Box<str>> {
	struct Buffer {
		text: crate::alloc::String,
		limit: usize,
	}
	impl core::fmt::Write for Buffer {
		fn write_str(&mut self, text: &str) -> core::fmt::Result {
			let end = self
				.text
				.len()
				.checked_add(text.len())
				.ok_or(core::fmt::Error)?;
			if end > self.limit {
				return Err(core::fmt::Error);
			}
			self.text
				.try_reserve_exact(text.len())
				.map_err(|_| core::fmt::Error)?;
			self.text.try_push_str(text).map_err(|_| core::fmt::Error)
		}
	}
	std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		let mut buffer = Buffer {
			text: crate::alloc::String::new(),
			limit,
		};
		core::fmt::write(&mut buffer, format_args!("{value}")).ok()?;
		buffer.text.try_into().ok()
	}))
	.ok()
	.flatten()
}

#[derive(Clone, Copy)]
enum Input {
	Plain,
	String,
	Wrong(Type),
	Function,
	TypeValue,
	Failed(Error),
}

fn input(store: &Store, value: Value, expected: HostType) -> Input {
	match value {
		Value::Heap(_) => match store.host_tag(value) {
			Ok((Type::String, true)) => Input::String,
			Ok((Type::String, false)) if expected == HostType::String => {
				Input::Failed(error(ErrorKind::AccessConflict).with_detail(Detail::Moved {
					access: super::super::error::MovedAccess::Read,
				}))
			}
			Ok((tag, _)) => Input::Wrong(tag),
			Err(error) => Input::Failed(error),
		},
		Value::Function { .. } => Input::Function,
		Value::Type(_) => Input::TypeValue,
		_ => Input::Plain,
	}
}

fn snapshot(store: &Store, value: Value) -> Result<stock::Value> {
	Ok(match value {
		Value::Unit => stock::Value::unit(),
		Value::Bool(v) => stock::Value::from(v),
		Value::I64(v) => stock::Value::from(v),
		Value::U64(v) => stock::Value::from(v),
		Value::F64(v) => stock::Value::from(v),
		Value::Char(v) => stock::Value::from(v),
		Value::Ordering(v) => stock::Value::from(v),
		Value::Heap(_) => {
			stock::Value::try_from(crate::alloc::String::try_from(store.text(value)?)?)?
		}
		_ => return Err(error(ErrorKind::InvalidCode)),
	})
}

// Probe the prefix only on a cold non-representable-input path. No host body runs.
fn prefix_conversion_succeeds(value: &stock::Value, kind: HostType) -> bool {
	match kind {
		HostType::Unit => <()>::from_value(value.clone()).is_ok(),
		HostType::Bool => bool::from_value(value.clone()).is_ok(),
		HostType::I64 => i64::from_value(value.clone()).is_ok(),
		HostType::U64 => u64::from_value(value.clone()).is_ok(),
		HostType::F64 => f64::from_value(value.clone()).is_ok(),
		HostType::Char => char::from_value(value.clone()).is_ok(),
		HostType::String => value.borrow_string_ref().is_ok(),
	}
}

// No aggregate payload is copied. Tuple needs one dummy element: an empty tuple
// is deliberately avoided: the witness must stay distinct from an inline Unit.
fn witness(tag: Type) -> Result<stock::Value> {
	Ok(match tag {
		Type::String => stock::Value::try_from(crate::alloc::String::new())?,
		Type::Vec => stock::Value::try_from(stock::Vec::new())?,
		Type::Object => stock::Value::try_from(stock::Object::new())?,
		Type::Tuple => {
			let mut values = crate::alloc::Vec::new();
			values.try_push(stock::Value::unit())?;
			stock::Value::tuple(values)?
		}
		Type::Option => stock::Value::new(Option::<stock::Value>::None)?,
		Type::Result => stock::Value::new(
			core::result::Result::<stock::Value, stock::Value>::Err(stock::Value::unit()),
		)?,
		// Canonical range witness; origin subtype diagnostic parity is Retained.
		Type::Range => stock::Value::new(stock::Range::new(
			stock::Value::from(0i64),
			stock::Value::from(0i64),
		))?,
		Type::Iterator => stock::Value::new(IteratorWitness)?,
		Type::Struct(nominal) => {
			let mut name = [0u8; 256];
			let len = super::super::nominal::name_copy(nominal, &mut name)
				.ok_or_else(|| error(ErrorKind::InvalidCode))?;
			let name =
				core::str::from_utf8(&name[..len]).map_err(|_| error(ErrorKind::InvalidCode))?;
			let hash = super::super::nominal::hash_copy(nominal)
				.ok_or_else(|| error(ErrorKind::InvalidCode))?;
			stock::Value::empty_struct(crate::sync::Arc::try_new(stock::Rtti {
				kind: stock::RttiKind::Empty,
				hash: crate::Hash::new(hash),
				variant_hash: crate::Hash::EMPTY,
				item: crate::ItemBuf::with_item(name.trim_start_matches("::").split("::"))?,
				fields: crate::runtime::new_field_map(),
			})?)?
		}
		_ => return Err(error(ErrorKind::InvalidCode)),
	})
}

fn clear(values: &mut [stock::Value]) {
	for value in values {
		*value = stock::Value::empty();
	}
}

fn leaf_type(value: &stock::Value, expected: HostType) -> Result<(usize, usize)> {
	use stock::Inline;
	let valid = matches!(
		(value.as_inline(), expected),
		(Some(Inline::Unit), HostType::Unit)
			| (Some(Inline::Bool(_)), HostType::Bool)
			| (Some(Inline::Signed(_)), HostType::I64)
			| (Some(Inline::Unsigned(_)), HostType::U64)
			| (Some(Inline::Float(_)), HostType::F64)
			| (Some(Inline::Char(_)), HostType::Char)
	);
	if valid {
		return Ok((0, 0));
	}
	if expected == HostType::String {
		let text = value
			.borrow_string_ref()
			.map_err(|_| error(ErrorKind::InvalidCode))?;
		return Ok((text.len(), 1));
	}
	Err(error(ErrorKind::InvalidCode))
}
fn leaf(store: &mut Store, value: &stock::Value, expected: HostType) -> Result<Value> {
	use stock::Inline;
	let _ = leaf_type(value, expected)?;
	Ok(match value.as_inline() {
		Some(Inline::Unit) => Value::Unit,
		Some(Inline::Bool(v)) => Value::Bool(*v),
		Some(Inline::Signed(v)) => Value::I64(*v),
		Some(Inline::Unsigned(v)) => Value::U64(*v),
		Some(Inline::Float(v)) => Value::F64(*v),
		Some(Inline::Char(v)) => Value::Char(*v),
		_ => store.string(
			&value
				.borrow_string_ref()
				.map_err(|_| error(ErrorKind::InvalidCode))?,
		)?,
	})
}

impl Executor {
	#[cold]
	#[inline(never)]
	fn host_error(&mut self, native: stock::VmError) -> Error {
		use super::super::error::HostCause;
		use stock::VmErrorKind as K;
		let arg = native.chain().iter().find_map(|at| match at.kind() {
			K::BadArgument { arg } => u8::try_from(*arg).ok(),
			_ => None,
		});
		let (kind, detail) = match native.error().kind() {
			K::ValueToIntegerCoercionError { .. } => (
				ErrorKind::IntegerCoercion,
				Detail::HostMessage {
					cause: HostCause::IntegerCoercion,
					arg,
				},
			),
			K::Panic { .. } => (ErrorKind::Panic, Detail::PanicMessage),
			K::ExpectedNumber { actual } => (
				ErrorKind::Host,
				Detail::HostExpectedNumber {
					actual: actual.type_hash(),
					arg,
				},
			),
			K::ExpectedType { expected, actual } => (
				ErrorKind::Host,
				Detail::HostExpectedType {
					expected: expected.type_hash(),
					actual: actual.type_hash(),
					arg,
				},
			),
			K::AccessError { .. } => (
				ErrorKind::Host,
				Detail::HostMessage {
					cause: HostCause::Access,
					arg,
				},
			),
			K::MissingInterfaceEnvironment => (
				ErrorKind::Host,
				Detail::HostMessage {
					cause: HostCause::MissingEnvironment,
					arg,
				},
			),
			K::BadArgumentCount { .. } => return error(ErrorKind::InvalidCode),
			_ => (
				ErrorKind::Host,
				Detail::HostMessage {
					cause: HostCause::Other,
					arg,
				},
			),
		};
		// Diagnostic allocation/Display failure never replaces the primary or retries.
		self.pending_host_message = match native.error().kind() {
			K::Panic { reason } => owned_message(reason, self.format_limits.bytes),
			_ => owned_message(native.error(), self.format_limits.bytes),
		};
		error(kind).with_detail(detail)
	}

	#[cold]
	#[inline(never)]
	fn host_output(
		&mut self,
		store: &mut Store,
		value: &stock::Value,
		shape: HostOutput,
	) -> Result<Value> {
		match shape {
			HostOutput::Scalar(kind) => {
				let (bytes, entries) = leaf_type(value, kind)?;
				store.host_room(bytes, entries)?;
				#[cfg(test)]
				fault_tests::observe("leaf-start");
				leaf(store, value, kind)
			}
			HostOutput::Option(kind) => {
				let option = value
					.borrow_ref::<Option<stock::Value>>()
					.map_err(|_| error(ErrorKind::InvalidCode))?;
				self.host_variant(
					store,
					Type::Option,
					option.is_some(),
					(*option).as_ref().map(|value| (value, kind)),
				)
			}
			HostOutput::Result(ok, err) => {
				let result = value
					.borrow_ref::<core::result::Result<stock::Value, stock::Value>>()
					.map_err(|_| error(ErrorKind::InvalidCode))?;
				match &*result {
					Ok(value) => self.host_variant(store, Type::Result, true, Some((value, ok))),
					Err(value) => self.host_variant(store, Type::Result, false, Some((value, err))),
				}
			}
		}
	}
	fn host_variant(
		&mut self,
		store: &mut Store,
		tag: Type,
		success: bool,
		payload: Option<(&stock::Value, HostType)>,
	) -> Result<Value> {
		let (bytes, entries) = payload
			.map(|(value, kind)| leaf_type(value, kind))
			.transpose()?
			.unwrap_or((0, 0));
		store.host_room(
			bytes
				.checked_add(core::mem::size_of::<Value>())
				.ok_or_else(|| error(ErrorKind::Overflow))?,
			entries + 1,
		)?;
		#[cfg(test)]
		fault_tests::observe("leaf-start");
		let child = payload
			.map(|(value, kind)| leaf(store, value, kind))
			.transpose()?;
		#[cfg(test)]
		fault_tests::observe("parent-start");
		let result = store.variant(tag, success, child);
		if let Some(child) = child {
			if let Err(cleanup) = store.release(child) {
				#[cfg(test)]
				fault_tests::observe("cleanup-error");
				if let Ok(parent) = result {
					self.pending_host_cleanup = store.release(parent).err();
					return Err(cleanup);
				}
				self.pending_host_cleanup = Some(cleanup);
			}
		}
		result
	}

	#[cold]
	#[inline(never)]
	#[allow(clippy::too_many_arguments)]
	pub(super) fn host_call(
		&mut self,
		code: &Code,
		store: &mut Store,
		function: &super::super::code::Function,
		base: usize,
		out: u32,
		id: u32,
		start: u32,
		count: u32,
	) -> Result<()> {
		let invalid = || error(ErrorKind::InvalidCode);
		let binding = code.host(id).ok_or_else(invalid)?;
		let signature: HostSignature = binding.signature;
		if !signature.valid()
			|| count != u32::from(signature.arity)
			|| count > 4
			|| out >= function.slots
		{
			return Err(invalid());
		}
		let actual = self
			.active_host_runtime
			.as_deref()
			.and_then(|runtime| runtime.function(&binding.hash))
			.ok_or_else(invalid)?;
		if !binding.handler.same_allocation(actual) {
			return Err(invalid());
		}
		let end = start.checked_add(count).ok_or_else(invalid)?;
		let arguments = function
			.args
			.get(start as usize..end as usize)
			.ok_or_else(invalid)?;
		let destination = base.checked_add(out as usize).ok_or_else(invalid)?;
		if self.slots.get(destination).is_none() {
			return Err(invalid());
		}
		let mut values = [Value::Unit; 4];
		for (i, slot) in arguments.iter().copied().enumerate() {
			if slot >= function.slots {
				return Err(invalid());
			}
			values[i] = *self
				.slots
				.get(base.checked_add(slot as usize).ok_or_else(invalid)?)
				.ok_or_else(invalid)?;
		}
		let mut stop = None;
		let mut bytes = 0usize;
		for i in 0..count as usize {
			match input(store, values[i], signature.arguments[i]) {
				Input::Plain => (),
				Input::String => {
					if !values[..i].contains(&values[i]) {
						bytes = bytes
							.checked_add(store.text(values[i])?.len())
							.ok_or_else(|| error(ErrorKind::Overflow))?;
					}
				}
				other => {
					stop = Some((i, other));
					break;
				}
			}
		}
		store.host_scratch_room(bytes)?;
		let mut scratch: [stock::Value; 5] = core::array::from_fn(|_| stock::Value::empty());
		let prefix = stop.map(|(i, _)| i).unwrap_or(count as usize);
		for i in 0..prefix {
			if matches!(
				input(store, values[i], signature.arguments[i]),
				Input::String
			) {
				if let Some(alias) = values[..i].iter().position(|value| *value == values[i]) {
					scratch[i] = scratch[alias].clone();
					continue;
				}
			}
			scratch[i] = snapshot(store, values[i])?;
		}
		if let Some((at, state)) = stop {
			for slot in &mut scratch[at..count as usize] {
				*slot = stock::Value::unit();
			}
			let earlier =
				(0..at).any(|i| !prefix_conversion_succeeds(&scratch[i], signature.arguments[i]));
			if !earlier {
				scratch[at] = match state {
					Input::Wrong(tag) => witness(tag)?,
					Input::Function => stock::Value::try_from(stock::Function::new(|| ())?)?,
					Input::TypeValue => stock::Value::from(stock::Type::new(crate::Hash::EMPTY)),
					Input::Failed(failure)
						if failure.kind == ErrorKind::AccessConflict
							&& matches!(
								failure.detail,
								Detail::Moved {
									access: super::super::error::MovedAccess::Read
								}
							) =>
					{
						let native = stock::VmError::new(stock::VmErrorKind::AccessError {
							error: stock::AccessError::fresh_moved_read(),
						})
						.with_error(stock::VmErrorKind::BadArgument { arg: at });
						clear(&mut scratch);
						return Err(self.host_error(native));
					}
					Input::Failed(failure) => {
						clear(&mut scratch);
						return Err(failure);
					}
					_ => return Err(invalid()),
				};
			}
		}
		// Value slots own the complete bounded scratch even if a handler only partly drains it.
		#[cfg(test)]
		fault_tests::observe("adapter-start");
		let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			binding.handler.call(
				&mut scratch,
				stock::Address::new(0),
				count as usize,
				stock::Output::keep(count as usize),
			)
		}));
		match called {
			Err(payload) => {
				clear(&mut scratch);
				let text = payload.downcast_ref::<&str>().copied().or_else(|| {
					payload
						.downcast_ref::<std::string::String>()
						.map(|s| s.as_str())
				});
				self.pending_host_message =
					text.and_then(|text| owned_message(&text, self.format_limits.bytes));
				return Err(error(ErrorKind::Host).with_detail(Detail::HostPanicked));
			}
			Ok(Err(native)) => {
				clear(&mut scratch);
				return Err(self.host_error(native));
			}
			Ok(Ok(())) => (),
		}
		let output = core::mem::replace(&mut scratch[count as usize], stock::Value::empty());
		clear(&mut scratch[..count as usize]);
		#[cfg(test)]
		fault_tests::observe("output-start");
		let result = self.host_output(store, &output, signature.output);
		drop(output);
		let value = result?;
		#[cfg(test)]
		fault_tests::observe("put");
		self.put(store, destination, value)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::experimental_core::{error::HostCause, frontend, heap::Limits};
	use crate::{Context, Diagnostics, Hash, Module, Source, Sources};

	fn context() -> Context {
		let mut context = Context::with_default_modules().unwrap();
		let mut module = Module::with_item(["h1focused"]).unwrap();
		module
			.fresh_host_function("add", |a: i64, b: i64| a + b)
			.unwrap();
		module
			.fresh_host_function("greet", |s: &str| -> std::string::String { s.into() })
			.unwrap();
		module
			.fresh_host_function("pair", |a: i64, s: &str| -> i64 { a + s.len() as i64 })
			.unwrap();
		module
			.fresh_host_function("none", || -> Option<i64> { None })
			.unwrap();
		module
			.fresh_host_function("err", || -> core::result::Result<i64, bool> { Err(false) })
			.unwrap();
		module
			.fresh_host_function("four", |a: i64, b: &str, c: bool, d: &str| -> i64 {
				a + b.len() as i64 + c as i64 + d.len() as i64
			})
			.unwrap();
		context.install(module).unwrap();
		context
	}

	fn compile(context: &Context, text: &str) -> Program {
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1).unwrap()
	}

	#[test]
	fn prefix_probe_checks_coercion_not_only_type() {
		assert!(!prefix_conversion_succeeds(
			&stock::Value::from(u64::MAX),
			HostType::I64
		));
		assert!(prefix_conversion_succeeds(
			&stock::Value::from(1u64),
			HostType::I64
		));
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn enrolled_scalar_and_borrowed_string_calls_execute() {
		let context = context();
		let program = compile(
			&context,
			"pub fn main(){h1focused::pair(h1focused::add(2,3),\"abc\")}",
		);
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let root = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		assert_eq!(root.value, Value::I64(8));
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
		assert!(executor.active_host_runtime.is_none());
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn absent_and_wrong_runtime_refuse_before_execution() {
		let context = context();
		let program = compile(&context, "pub fn main(){h1focused::add(2,3)}");
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		assert!(executor
			.invoke(&program, &mut store, main, &[], 1000)
			.is_err());
		let other = self::context();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(other.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		assert!(executor
			.invoke(&program, &mut store, main, &[], 1000)
			.is_err());
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn earlier_coercion_wins_over_later_moved_string() {
		let context = context();
		let program = compile(
			&context,
			"pub fn main(){let s=\"x\";drop(s);h1focused::pair(9223372036854775808u64,s)}",
		);
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let failure = executor
			.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::IntegerCoercion);
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
		assert!(executor.active_host_runtime.is_none());
	}
	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn output_shapes_and_four_borrowed_arguments_clean_up() {
		let context = context();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		for (text, tag, expected) in [
			("pub fn main(){h1focused::none()}", Type::Option, None),
			("pub fn main(){h1focused::err()}", Type::Result, None),
			(
				"pub fn main(){h1focused::four(1,\"ab\",true,\"c\")}",
				Type::I64,
				Some(Value::I64(5)),
			),
			(
				"pub fn main(){h1focused::greet(\"hello\")}",
				Type::String,
				None,
			),
		] {
			let program = compile(&context, text);
			let root = executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					1000,
				)
				.unwrap();
			assert_eq!(store.root_tag(&root).unwrap(), tag);
			if let Some(expected) = expected {
				assert_eq!(root.value, expected);
			}
			store.root_release(root).unwrap();
			assert_eq!(store.live_entries(), 0);
			assert!(executor.active_host_runtime.is_none());
		}
	}
	fn invoke_source(
		context: &Context,
		executor: &mut Executor,
		store: &mut Store,
		text: &str,
		fuel: usize,
	) -> core::result::Result<Root, std::boxed::Box<Failure>> {
		let program = compile(context, text);
		executor
			.invoke(
				&program,
				store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				fuel,
			)
			.map_err(std::boxed::Box::new)
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn fuel_and_cancel_refusals_preserve_configuration_and_reusability() {
		let context = context();
		let mut executor = Executor::new().unwrap();
		let runtime = crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap();
		executor.set_host_runtime(Some(runtime)).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let source = "pub fn main(){h1focused::add(2,3)}";
		assert!(invoke_source(&context, &mut executor, &mut store, source, 0).is_err());
		assert!(executor.configured_host_runtime.is_some());
		assert!(executor.active_host_runtime.is_none());
		executor.request_cancel();
		executor.set_host_runtime(None).unwrap();
		assert!(invoke_source(&context, &mut executor, &mut store, source, 1000).is_err());
		assert!(executor.cancelled);
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let failure = invoke_source(&context, &mut executor, &mut store, source, 1000).unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::Cancelled);
		assert!(!executor.cancelled);
		let root = invoke_source(&context, &mut executor, &mut store, source, 1000).unwrap();
		assert_eq!(root.value, Value::I64(5));
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn failed_install_publishes_no_new_certificate() {
		let mut context = Context::with_default_modules().unwrap();
		let mut first = Module::with_item(["transaction"]).unwrap();
		first.function("conflict", || 1i64).build().unwrap();
		context.install(first).unwrap();
		let mut second = Module::with_item(["transaction"]).unwrap();
		second.fresh_host_function("new", || 2i64).unwrap();
		second.fresh_host_function("conflict", || 3i64).unwrap();
		assert!(context.install(second).is_err());
		assert!(context
			.fresh_host(Hash::type_hash(["transaction", "new"]))
			.is_none());
		assert!(context
			.fresh_host(Hash::type_hash(["transaction", "conflict"]))
			.is_none());
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn host_unwind_is_once_and_keeps_primary_and_executor_clean() {
		use std::sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		};
		let calls = Arc::new(AtomicUsize::new(0));
		let captured = calls.clone();
		let mut context = Context::with_default_modules().unwrap();
		let mut module = Module::with_item(["unwind"]).unwrap();
		module
			.fresh_host_function("fail", move || -> i64 {
				captured.fetch_add(1, Ordering::SeqCst);
				panic!("native payload");
			})
			.unwrap();
		context.install(module).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = invoke_source(
			&context,
			&mut executor,
			&mut store,
			"pub fn main(){unwind::fail()}",
			1000,
		)
		.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::Host);
		assert_eq!(failure.error.detail, Detail::HostPanicked);
		assert_eq!(failure.host_message.as_deref(), Some("native payload"));
		assert!(failure.panic_message.is_none());
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
		assert!(executor.active_host_runtime.is_none());
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn output_capacity_refusal_does_not_retry_host_effects() {
		use std::sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		};
		let calls = Arc::new(AtomicUsize::new(0));
		let captured = calls.clone();
		let mut context = Context::with_default_modules().unwrap();
		let mut module = Module::with_item(["caps"]).unwrap();
		module
			.fresh_host_function("text", move || -> std::string::String {
				captured.fetch_add(1, Ordering::SeqCst);
				"payload".into()
			})
			.unwrap();
		context.install(module).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits {
			entries: 1,
			bytes: 0,
		})
		.unwrap();
		let failure = invoke_source(
			&context,
			&mut executor,
			&mut store,
			"pub fn main(){caps::text()}",
			1000,
		)
		.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::StoreCapacity);
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert_eq!(store.live_entries(), 0);
		assert!(executor.is_idle());
		assert!(executor.active_host_runtime.is_none());
	}
	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn moved_arguments_keep_type_before_access_and_first_argument_order() {
		let context = context();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		for (text, cause, arg) in [
			(
				"pub fn main(){let s=\"x\";drop(s);h1focused::add(s,1)}",
				HostCause::ExpectedNumber,
				0,
			),
			(
				"pub fn main(){let v=[1];drop(v);h1focused::greet(v)}",
				HostCause::ExpectedType,
				0,
			),
			(
				"pub fn main(){let s=\"x\";drop(s);h1focused::greet(s)}",
				HostCause::Access,
				0,
			),
			(
				"pub fn main(){let s=\"x\";drop(s);h1focused::pair(false,s)}",
				HostCause::ExpectedNumber,
				0,
			),
		] {
			let failure =
				invoke_source(&context, &mut executor, &mut store, text, 1000).unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::Host);
			match cause {
				HostCause::ExpectedNumber => {
					assert!(
						matches!(failure.error.detail,Detail::HostExpectedNumber {arg:Some(actual),..} if actual==arg)
					);
					assert_eq!(failure.error.detail.numeric_argument(), Some((cause, arg)));
				}
				HostCause::ExpectedType => {
					assert!(
						matches!(failure.error.detail,Detail::HostExpectedType {arg:Some(actual),..} if actual==arg)
					);
					assert_eq!(failure.error.detail.numeric_argument(), Some((cause, arg)));
				}
				HostCause::Access => assert_eq!(
					failure.error.detail,
					Detail::HostMessage {
						cause,
						arg: Some(arg)
					}
				),
				_ => panic!("unlisted h1focused argument cause"),
			}
			assert!(failure.host_message.is_some());
			assert!(failure.panic_message.is_none());
			assert_eq!(store.live_entries(), 0);
			assert!(executor.active_host_runtime.is_none());
			assert!(executor.is_idle());
		}
	}

	#[test]
	fn bounded_diagnostic_failure_never_escapes_or_grows_past_limit() {
		struct Bad;
		impl core::fmt::Display for Bad {
			fn fmt(&self, _: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
				panic!("formatter");
			}
		}
		assert_eq!(owned_message(&"abc", 3).as_deref(), Some("abc"));
		assert!(owned_message(&"abcd", 3).is_none());
		assert!(owned_message(&Bad, 10).is_none());
	}
	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn runtime_keeps_handler_alive_after_context_drop_and_executor_reuse() {
		let context = context();
		let program = compile(&context, "pub fn main(){h1focused::add(2,3)}");
		let runtime = crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap();
		drop(context);
		let mut executor = Executor::new().unwrap();
		executor.set_host_runtime(Some(runtime.clone())).unwrap();
		drop(runtime);
		let mut store = Store::new(Limits::default()).unwrap();
		for _ in 0..2 {
			let root = executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					1000,
				)
				.unwrap();
			assert_eq!(root.value, Value::I64(5));
			store.root_release(root).unwrap();
			assert!(executor.configured_host_runtime.is_some());
			assert!(executor.active_host_runtime.is_none());
		}
		executor.set_host_runtime(None).unwrap();
		let plain = compile(
			&Context::with_default_modules().unwrap(),
			"pub fn main(){7}",
		);
		let root = executor
			.invoke(
				&plain,
				&mut store,
				plain.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		assert_eq!(root.value, Value::I64(7));
		store.root_release(root).unwrap();
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn matching_function_pointer_in_new_handler_allocation_is_rejected() {
		fn identity(v: i64) -> i64 {
			v
		}
		fn installed() -> Context {
			let mut context = Context::with_default_modules().unwrap();
			let mut module = Module::with_item(["samefn"]).unwrap();
			module.fresh_host_function("id", identity).unwrap();
			context.install(module).unwrap();
			context
		}
		let a = installed();
		let b = installed();
		let pa = compile(&a, "pub fn main(){samefn::id(5)}");
		let pb = compile(&b, "pub fn main(){samefn::id(5)}");
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(a.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let root = executor
			.invoke(
				&pa,
				&mut store,
				pa.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap();
		store.root_release(root).unwrap();
		let failure = executor
			.invoke(
				&pb,
				&mut store,
				pb.function(Hash::type_hash(["main"])).unwrap(),
				&[],
				1000,
			)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
		assert!(failure.error.location.is_none());
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn whole_table_validates_unused_descriptor_and_both_alias_names_before_roots() {
		use crate::experimental_core::code::{Builder, Function};
		let context = context();
		let other = self::context();
		let mut a = context
			.fresh_host(Hash::type_hash(["h1focused", "add"]))
			.unwrap()
			.clone();
		a.hash = Hash::type_hash(["alias", "a"]);
		let mut b = a.clone();
		b.hash = Hash::type_hash(["alias", "b"]);
		let mut replacement = other
			.fresh_host(Hash::type_hash(["h1focused", "add"]))
			.unwrap()
			.clone();
		replacement.hash = b.hash;
		let mut builder = Builder::default();
		builder.host(&a).unwrap();
		builder.host(&b).unwrap();
		let mut f = Function::new(1, 1).unwrap();
		f.push(
			Op::Return { value: 0 },
			Location {
				source: 0,
				revision: 0,
				start: 0,
				end: 1,
			},
		)
		.unwrap();
		builder.push(f).unwrap();
		let code = builder.finish().unwrap();
		fn runtime(
			bindings: &[crate::function::fresh_host::HostBinding],
		) -> crate::sync::Arc<stock::RuntimeContext> {
			let mut functions = crate::hash::Map::default();
			for binding in bindings {
				functions
					.try_insert(binding.hash, binding.handler.clone())
					.unwrap();
			}
			crate::sync::Arc::try_new(stock::RuntimeContext::new(
				functions,
				Default::default(),
				Default::default(),
				Default::default(),
			))
			.unwrap()
		}
		let mut executor = Executor::new().unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let root = store.root_i64(9);
		for rt in [
			runtime(&[]),
			runtime(&[a.clone()]),
			runtime(&[a.clone(), replacement]),
		] {
			executor.set_host_runtime(Some(rt)).unwrap();
			executor.request_cancel();
			let before = (store.live_entries(), store.payload_bytes());
			let failure = executor
				.invoke_code(&code, &mut store, 0, &[&root], 1000)
				.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
			assert!(failure.error.location.is_none());
			assert_eq!(before, (store.live_entries(), store.payload_bytes()));
			assert_eq!(root.value, Value::I64(9));
			assert!(executor.cancelled);
			assert!(executor.is_idle());
			executor.cancelled = false;
		}
		executor.set_host_runtime(Some(runtime(&[a, b]))).unwrap();
		let returned = executor
			.invoke_code(&code, &mut store, 0, &[&root], 1000)
			.unwrap();
		assert_eq!(returned.value, Value::I64(9));
		store.root_release(returned).unwrap();
		store.root_release(root).unwrap();
	}
	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn repeated_string_handle_uses_one_bounded_snapshot_and_preserves_aliases() {
		let mut context = Context::with_default_modules().unwrap();
		let mut module = Module::with_item(["snapshot"]).unwrap();
		module
			.fresh_host_function("same", |a: &str, b: &str| -> bool {
				a.as_ptr() == b.as_ptr() && a.len() == b.len()
			})
			.unwrap();
		context.install(module).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut store = Store::new(Limits {
			entries: 4,
			bytes: 16,
		})
		.unwrap();
		let root = invoke_source(
			&context,
			&mut executor,
			&mut store,
			"pub fn main(){let s=\"abcdefghijklmnop\";snapshot::same(s,s)}",
			1000,
		)
		.unwrap();
		assert_eq!(root.value, Value::Bool(true));
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
		assert!(executor.is_idle());
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "frontend-heavy host control; native gates cover it; focused Miri uses minimal fixtures"
	)]
	fn diagnostic_budget_refusal_keeps_host_panic_primary_and_single_effect() {
		use std::sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		};
		let calls = Arc::new(AtomicUsize::new(0));
		let captured = calls.clone();
		let mut context = Context::with_default_modules().unwrap();
		let mut module = Module::with_item(["budgetpanic"]).unwrap();
		module
			.fresh_host_function("fail", move || -> i64 {
				captured.fetch_add(1, Ordering::SeqCst);
				panic!("owned payload");
			})
			.unwrap();
		context.install(module).unwrap();
		let mut executor = Executor::new().unwrap();
		executor
			.set_host_runtime(Some(
				crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
			))
			.unwrap();
		let mut limits = *executor.format_limits();
		limits.bytes = 0;
		executor.set_format_limits(limits);
		let mut store = Store::new(Limits::default()).unwrap();
		let failure = invoke_source(
			&context,
			&mut executor,
			&mut store,
			"pub fn main(){budgetpanic::fail()}",
			1000,
		)
		.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::Host);
		assert_eq!(failure.error.detail, Detail::HostPanicked);
		assert!(failure.host_message.is_none());
		assert!(failure.panic_message.is_none());
		assert!(failure.cleanup_error.is_none());
		assert_eq!(calls.load(Ordering::SeqCst), 1);
		assert!(executor.active_host_runtime.is_none());
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}

#[cfg(test)]
mod minimal_tests;

#[cfg(test)]
mod fault_tests;
