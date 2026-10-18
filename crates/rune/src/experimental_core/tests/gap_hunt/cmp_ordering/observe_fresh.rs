// Typed baseline observer. Unmapped values/metadata are explicit failures, never parity.
use crate::experimental_core::{
	error::{Detail, ErrorKind, TypeName},
	value::{Root, Scalar, Type, Value},
};

fn fresh_value(store: &mut Store, value: Value, depth: usize) -> Json {
	assert!(depth <= 16, "C9 fresh observation depth");
	let scalar = match value {
		Value::Unit => Some(crate::runtime::Value::unit()),
		Value::Bool(v) => Some(crate::runtime::Value::from(v)),
		Value::I64(v) => Some(crate::runtime::Value::from(v)),
		Value::U64(v) => Some(crate::runtime::Value::from(v)),
		Value::F64(v) => Some(crate::runtime::Value::from(v)),
		Value::Char(v) => Some(crate::runtime::Value::from(v)),
		Value::Ordering(v) => Some(crate::runtime::Value::from(v)),
		_ => None,
	};
	if let Some(value) = scalar {
		return value_projection(&value, depth);
	}
	let Ok(tag) = store.tag(value) else {
		return json!({"unmapped_value":format!("{value:?}")});
	};
	use crate::TypeHash;
	let (hash, payload) = match tag {
		Type::String => (
			crate::alloc::String::HASH,
			json!({"type":"String","hex":hex(store.text(value).unwrap().as_bytes())}),
		),
		Type::Vec | Type::Tuple => {
			let mut values = Vec::new();
			for index in 0..store.len(value).unwrap() {
				let child = store.get(value, index).unwrap();
				values.push(fresh_value(store, child, depth + 1));
				store.release(child).unwrap();
			}
			if tag == Type::Vec {
				(
					crate::runtime::Vec::HASH,
					json!({"type":"Vec","values":values}),
				)
			} else {
				(
					crate::runtime::OwnedTuple::HASH,
					json!({"type":"Tuple","values":values}),
				)
			}
		}
		Type::Option | Type::Result => {
			// Borrowed child: the parent stays owned, projection never mutates it.
			let (_, success, child) = store.variant_parts(value).unwrap();
			let child = child.map(|v| fresh_value(store, v, depth + 1));
			if tag == Type::Option {
				(
					Option::<crate::runtime::Value>::HASH,
					json!({"type":"Option","value":child}),
				)
			} else {
				(
					Result::<crate::runtime::Value, crate::runtime::Value>::HASH,
					json!({"type":"Result","variant":if success{"Ok"}else{"Err"},"value":child.unwrap()}),
				)
			}
		}
		_ => return json!({"unmapped_value":format!("{value:?}"),"actual_tag":format!("{tag:?}")}),
	};
	json!({"type_hash":hash.to_string(),"payload":payload})
}

// Construct diagnostic identities only from actual typed metadata. No expected-row lookup.
fn fresh_type_info(
	name: TypeName,
	program: &frontend::Program,
) -> Option<crate::runtime::TypeInfo> {
	use crate::runtime::{self as rt, TypeInfo};
	Some(match name {
		TypeName::Unit | TypeName::Tuple => TypeInfo::named::<rt::OwnedTuple>(),
		TypeName::Bool => TypeInfo::named::<bool>(),
		TypeName::Function => TypeInfo::named::<rt::Function>(),
		TypeName::Type => TypeInfo::named::<rt::Type>(),
		TypeName::Struct(nominal) => return fresh_struct_info(nominal, program),
		TypeName::Ordering => TypeInfo::named::<core::cmp::Ordering>(),
		TypeName::I64 => TypeInfo::named::<i64>(),
		TypeName::U64 => TypeInfo::named::<u64>(),
		TypeName::F64 => TypeInfo::named::<f64>(),
		TypeName::Char => TypeInfo::named::<char>(),
		TypeName::String => TypeInfo::named::<crate::alloc::String>(),
		TypeName::Vec => TypeInfo::named::<rt::Vec>(),
		TypeName::Object => TypeInfo::named::<rt::Object>(),
		TypeName::Option => TypeInfo::named::<Option<rt::Value>>(),
		TypeName::Result => TypeInfo::named::<Result<rt::Value, rt::Value>>(),
		TypeName::Range => TypeInfo::named::<rt::Range>(),
		_ => return None,
	})
}

fn fresh_primary_kind(
	error: &crate::experimental_core::error::Error,
	program: &frontend::Program,
) -> Option<crate::runtime::VmErrorKind> {
	use crate::runtime::VmErrorKind as K;
	Some(match error.detail {
		Detail::Binary { op, lhs, rhs } if error.kind == ErrorKind::Type => {
			use crate::experimental_core::error::BinarySymbol as B;
			let op = match op {
				B::Eq => "==",
				B::Ne => "!=",
				B::Lt => "<",
				B::Le => "<=",
				B::Gt => ">",
				B::Ge => ">=",
				_ => return None,
			};
			K::UnsupportedBinaryOperation {
				op,
				lhs: fresh_type_info(lhs, program)?,
				rhs: fresh_type_info(rhs, program)?,
			}
		}
		Detail::MissingInstanceFunction {
			target,
			hash,
			instance,
		} if error.kind == ErrorKind::Type => {
			let info = fresh_type_info(target, program)?;
			if info.type_hash() != instance {
				return None;
			}
			K::MissingInstanceFunction {
				hash,
				instance: info,
			}
		}
		Detail::NumericArgumentNumber { found, .. } | Detail::ExpectedNumber { found }
			if error.kind == ErrorKind::Type =>
		{
			K::ExpectedNumber {
				actual: fresh_type_info(found, program)?,
			}
		}
		Detail::NumericArgumentType {
			expected, found, ..
		}
		| Detail::ExpectedType { expected, found }
			if error.kind == ErrorKind::Type =>
		{
			K::ExpectedType {
				expected: fresh_type_info(expected, program)?,
				actual: fresh_type_info(found, program)?,
			}
		}
		Detail::NoDisplay { found } if error.kind == ErrorKind::Type => {
			K::MissingProtocolFunction {
				protocol: &crate::runtime::Protocol::DISPLAY_FMT,
				instance: fresh_type_info(found, program)?,
			}
		}
		Detail::CmpArgumentAccess {
			access: crate::experimental_core::error::MovedAccess::Read,
			..
		} if error.kind == ErrorKind::AccessConflict => K::AccessError {
			error: crate::runtime::AccessError::fresh_moved_read(),
		},
		Detail::SharedArgument { readers, .. } if error.kind == ErrorKind::AccessConflict => {
			K::AccessError {
				error: crate::runtime::AccessError::fresh_shared_for_test(readers),
			}
		}
		Detail::Moved {
			access: crate::experimental_core::error::MovedAccess::Read,
		} if error.kind == ErrorKind::AccessConflict => K::AccessError {
			error: crate::runtime::AccessError::fresh_moved_read(),
		},
		Detail::NumericArgumentCoerce { value, to, .. } | Detail::Coerce { value, to }
			if error.kind == ErrorKind::IntegerCoercion =>
		{
			use crate::experimental_core::error::IntTarget;
			match to {
				IntTarget::I64 if !value.negative && value.magnitude > i64::MAX as u64 => {
					K::ValueToIntegerCoercionError {
						from: value.magnitude.into(),
						to: "i64",
					}
				}
				IntTarget::U64 if value.negative => K::ValueToIntegerCoercionError {
					from: i64::try_from(-i128::from(value.magnitude)).ok()?.into(),
					to: "u64",
				},
				_ => return None,
			}
		}
		Detail::FloatComparison { lhs_bits, rhs_bits } if error.kind == ErrorKind::Type => {
			K::IllegalFloatComparison {
				lhs: f64::from_bits(lhs_bits),
				rhs: f64::from_bits(rhs_bits),
			}
		}
		Detail::HostExpectedType {
			expected, actual, ..
		} if error.kind == ErrorKind::Host => K::ExpectedType {
			expected: fresh_hash_info(expected, program)?,
			actual: fresh_hash_info(actual, program)?,
		},
		Detail::HostExpectedNumber { actual, .. } if error.kind == ErrorKind::Host => {
			K::ExpectedNumber {
				actual: fresh_hash_info(actual, program)?,
			}
		}
		Detail::None => match error.kind {
			ErrorKind::Overflow => K::Overflow,
			ErrorKind::DivisionByZero => K::DivideByZero,
			_ => return None,
		},
		_ => return None,
	})
}

fn fresh_failure(
	failure: &crate::experimental_core::execute::Failure,
	program: &crate::experimental_core::frontend::Program,
) -> Json {
	let error = &failure.error;
	let location = error
		.location
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}));
	if error.kind == ErrorKind::Unsupported
		&& error.operation == "Method"
		&& error.refusal.is_none()
	{
		return json!({"status":"runtime-unsupported","kind":"Unsupported","operation":error.operation,"refusal":null,"location":location,"raw":format!("{failure:?}")});
	}
	if error.kind == ErrorKind::AccessConflict && error.detail == Detail::WriteBorrowed {
		let kind = format!("{:?}", error.detail);
		let frames = if failure.call_chain.is_empty() {
			"[]".into()
		} else {
			serde_json::to_string(
				&failure
					.call_chain
					.iter()
					.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
					.collect::<Vec<_>>(),
			)
			.unwrap()
		};
		let sites = error
			.location
			.iter()
			.map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}))
			.collect::<Vec<_>>();
		return json!({"status":"error","class":format!("{:?}",error.kind),"kind":kind,"typed":{"raw_kind":kind},"display":failure.message(program).to_string(),"chain":[],"sites":sites,"raw":format!("{failure:?}")});
	}
	let Some(kind) = fresh_primary_kind(error, program) else {
		return json!({"status":"unmapped-error","actual_kind":format!("{:?}",error.kind),"actual_detail":format!("{:?}",error.detail),"operation":error.operation,"location":location,"raw":format!("{failure:?}")});
	};
	let class = if matches!(
		kind,
		crate::runtime::VmErrorKind::IllegalFloatComparison { .. }
	) {
		"IllegalFloatComparison".into()
	} else {
		stock_class(&kind)
	};
	let chain = error
		.detail
		.numeric_argument()
		.map(|(_, arg)| {
			let kind = crate::runtime::VmErrorKind::BadArgument { arg: arg.into() };
			vec![
				json!({"kind":format!("{kind:?}"),"typed":typed_kind(&kind),"display":kind.to_string()}),
			]
		})
		.unwrap_or_default();
	let call_chain = failure
		.call_chain
		.iter()
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
		.collect::<Vec<_>>();
	let frames = if call_chain.is_empty() {
		"[]".into()
	} else {
		serde_json::to_string(&call_chain).unwrap()
	};
	let sites = error
		.location
		.iter()
		.map(|at| json!({"source":at.source,"start":at.start,"end":at.end,"frames":frames}))
		.collect::<Vec<_>>();
	json!({"status":"error","class":class,"kind":format!("{kind:?}"),"typed":typed_kind(&kind),"display":failure.message(program).to_string(),"chain":chain,"sites":sites,"raw":format!("{failure:?}")})
}

fn observe_fresh(source: &str, mode: &str) -> Json {
	let (context, calls) = c9_context();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("c9.rn", source).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	let compiled = frontend::compile(&mut sources, &context, &mut diagnostics, 9);
	let mut store = Store::new(Limits {
		entries: 4096,
		bytes: 4 << 20,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mut invoked = false;
	let mut returned_root = false;
	let mut no_cleanup_error = true;
	let mut call_chain = Vec::new();
	let mut failure_site_ops = Vec::new();
	let mut actual_error = Json::Null;
	let mut arguments: Vec<Root> = Vec::new();
	if let Some(pair) = mode.strip_prefix("c9-f64x2-") {
		let (a, b) = pair.split_once('-').unwrap();
		arguments.push(store.root_f64(f64::from_bits(u64::from_str_radix(a, 16).unwrap())));
		arguments.push(store.root_f64(f64::from_bits(u64::from_str_radix(b, 16).unwrap())));
	} else {
		assert!(matches!(mode, "c9-main" | "c9-host"));
	}
	let outcome = match compiled {
		Err(error) => {
			json!({"status":if error.refusal.is_some(){"refused"}else{"unmapped-compile-error"},"kind":format!("{:?}",error.kind),"operation":error.operation,"refusal":error.refusal.map(|r|r.name()),"location":error.location.map(|at|json!({"source":at.source,"start":at.start,"end":at.end})),"raw":format!("{error:?}")})
		}
		Ok(program) => {
			invoked = true;
			let refs = arguments.iter().collect::<Vec<_>>();
			match executor.invoke(
				&program,
				&mut store,
				program.function(Hash::type_hash(["main"])).unwrap(),
				&refs,
				FUEL,
			) {
				Ok(root) => {
					returned_root = true;
					let value = store.import(&root).unwrap();
					if let Value::Ordering(v) = value {
						assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::Ordering(v)));
						assert_eq!(store.root_tag(&root).unwrap(), Type::Ordering);
					}
					let projected = fresh_value(&mut store, value, 0);
					store.root_release(root).unwrap();
					if projected.get("unmapped_value").is_some() {
						json!({"status":"unmapped-value","value":projected})
					} else {
						json!({"status":"value","value":projected})
					}
				}
				Err(failure) => {
					no_cleanup_error = failure.cleanup_error.is_none();
					call_chain = failure
						.call_chain
						.iter()
						.map(|at| json!({"source":at.source,"start":at.start,"end":at.end}))
						.collect();
					actual_error = json!({"kind":format!("{:?}",failure.error.kind),"detail":format!("{:?}",failure.error.detail),"operation":failure.error.operation,"refusal":failure.error.refusal.map(|r|r.name())});
					if let Some(at) = failure.error.location {
						for id in 0..program.code.functions() {
							let f = program.code.function(id as u32).unwrap();
							for (pc, location) in f.locations.iter().enumerate() {
								if *location == at {
									failure_site_ops.push(
										json!({"function":id,"pc":pc,"operation":format!("{:?}",f.operation(pc).unwrap())}),
									);
								}
							}
						}
					}
					fresh_failure(&failure, &program)
				}
			}
		}
	};
	for root in arguments {
		store.root_release(root).unwrap();
	}
	let row_live = store.live_entries();
	let row_bytes = store.payload_bytes();
	let row_idle = executor.is_idle();
	let mut reuse_sources = Sources::new();
	reuse_sources
		.insert(Source::memory("pub fn main(){7}").unwrap())
		.unwrap();
	let reuse_program =
		frontend::compile(&mut reuse_sources, &context, &mut Diagnostics::new(), 10).unwrap();
	let reuse = match executor.invoke(
		&reuse_program,
		&mut store,
		reuse_program.function(Hash::type_hash(["main"])).unwrap(),
		&[],
		FUEL,
	) {
		Ok(root) => {
			let result = store.root_scalar(&root).unwrap() == Some(Scalar::I64(7));
			store.root_release(root).unwrap();
			result
		}
		Err(_) => false,
	};
	json!({"outcome":outcome,"invoked":invoked,"host_calls":calls.load(std::sync::atomic::Ordering::SeqCst),"diagnostics":format!("{:?}",diagnostics.diagnostics()),"fresh_audit":{"no_cleanup_error":no_cleanup_error,"returned_root":returned_root,"row_live_entries":row_live,"row_payload_bytes":row_bytes,"row_idle":row_idle,"reuse":reuse,"live_entries":store.live_entries(),"payload_bytes":store.payload_bytes(),"idle":executor.is_idle(),"call_chain":call_chain,"failure_site_ops":failure_site_ops,"actual_error":actual_error}})
}

fn fresh_struct_info(
	nominal: crate::experimental_core::value::Nominal,
	program: &frontend::Program,
) -> Option<crate::runtime::TypeInfo> {
	Some(crate::runtime::TypeInfo::rtti(fresh_struct_rtti(
		nominal, program,
	)?))
}

fn fresh_struct_rtti(
	nominal: crate::experimental_core::value::Nominal,
	program: &frontend::Program,
) -> Option<crate::sync::Arc<crate::runtime::Rtti>> {
	if let Some(rtti) = crate::experimental_core::nominal::variant_rtti(nominal) {
		return Some(rtti);
	}
	use crate::experimental_core::code::Op;
	use crate::experimental_core::nominal;
	use crate::runtime::{Rtti, RttiKind};
	let hash = crate::Hash::new(nominal::hash_copy(nominal)?);
	let mut buffer = [0u8; 256];
	let len = nominal::name_copy(nominal, &mut buffer)?;
	let name = core::str::from_utf8(&buffer[..len]).ok()?;
	let mut shape: Option<Vec<&str>> = None;
	for id in 0..program.code.functions() {
		let function = program.code.function(u32::try_from(id).ok()?)?;
		for pc in 0..function.instructions().len() {
			let Op::Record {
				tag: Type::Struct(actual),
				args_start,
				args_len,
				..
			} = function.operation(pc).ok()?
			else {
				continue;
			};
			if actual != nominal {
				continue;
			}
			let end = args_start.checked_add(args_len)?;
			let descriptors = function.args.get(args_start as usize..end as usize)?;
			if descriptors.len() % 2 != 0 {
				return None;
			}
			let mut names = Vec::new();
			for descriptor in descriptors.as_chunks::<2>().0 {
				names.push(function.strings.get(descriptor[0] as usize)?.as_str());
			}
			if shape.as_ref().is_some_and(|old| *old != names) {
				return None;
			}
			shape = Some(names);
		}
	}
	let mut fields = crate::runtime::new_field_map();
	for (slot, name) in shape?.into_iter().enumerate() {
		let name: crate::alloc::Box<str> =
			crate::alloc::String::try_from(name).ok()?.try_into().ok()?;
		if fields.try_insert(name, slot).ok()?.is_some() {
			return None;
		}
	}
	let rtti = Rtti {
		variant_index: 0,
		kind: RttiKind::Struct,
		hash,
		variant_hash: Hash::EMPTY,
		item: crate::ItemBuf::with_item(name.trim_start_matches("::").split("::")).ok()?,
		fields,
	};
	crate::sync::Arc::try_new(rtti).ok()
}

fn fresh_hash_info(hash: Hash, program: &frontend::Program) -> Option<crate::runtime::TypeInfo> {
	// Known builtin hashes only. An unknown host identity remains unmapped.
	for name in [
		TypeName::Unit,
		TypeName::Bool,
		TypeName::I64,
		TypeName::U64,
		TypeName::F64,
		TypeName::Char,
		TypeName::Ordering,
		TypeName::String,
		TypeName::Vec,
		TypeName::Object,
		TypeName::Option,
		TypeName::Result,
		TypeName::Range,
		TypeName::Function,
		TypeName::Type,
	] {
		if name.builtin_hash() == Some(hash) {
			return fresh_type_info(name, program);
		}
	}
	None
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles standard Context/source; metadata consumer runs natively"
)]
fn struct_diagnostic_shape_comes_from_ordered_record_descriptors() {
	use crate::experimental_core::code::Op;
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("struct C9Shape{x,y} pub fn main(){C9Shape{y:2,x:1}}").unwrap())
		.unwrap();
	let context = Context::with_default_modules().unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 17).unwrap();
	let mut nominal = None;
	for id in 0..program.code.functions() {
		let f = program.code.function(id as u32).unwrap();
		for pc in 0..f.instructions().len() {
			if let Op::Record {
				tag: Type::Struct(n),
				..
			} = f.operation(pc).unwrap()
			{
				nominal = Some(n);
			}
		}
	}
	let nominal = nominal.unwrap();
	let rtti = fresh_struct_rtti(nominal, &program).unwrap();
	assert!(matches!(rtti.kind, crate::runtime::RttiKind::Struct));
	assert_eq!(
		rtti.hash,
		Hash::new(crate::experimental_core::nominal::hash_copy(nominal).unwrap())
	);
	assert_eq!(rtti.variant_hash, Hash::EMPTY);
	assert_eq!(rtti.item.to_string(), "C9Shape");
	assert_eq!(rtti.fields.get("x"), Some(&0));
	assert_eq!(rtti.fields.get("y"), Some(&1));
	let missing =
		crate::experimental_core::nominal::intern(0xC901_85FF_ABCD, "NoRecordHere").unwrap();
	assert!(fresh_struct_rtti(missing, &program).is_none());
}

#[test]
fn binary_comparison_projection_uses_only_actual_typed_metadata() {
	use crate::experimental_core::{
		code::Builder,
		error::{BinarySymbol as B, Error},
	};
	use crate::runtime::VmErrorKind as K;
	use crate::TypeHash;
	let program = frontend::Program::for_diagnostics_test(Builder::default().finish().unwrap());
	for (symbol, expected_op) in [
		(B::Eq, "=="),
		(B::Ne, "!="),
		(B::Lt, "<"),
		(B::Le, "<="),
		(B::Gt, ">"),
		(B::Ge, ">="),
	] {
		for (rhs, expected_rhs) in [(TypeName::I64, i64::HASH), (TypeName::Bool, bool::HASH)] {
			let error = Error::new(ErrorKind::Type, "actual typed comparison").with_detail(
				Detail::Binary {
					op: symbol,
					lhs: TypeName::Ordering,
					rhs,
				},
			);
			let K::UnsupportedBinaryOperation { op, lhs, rhs } =
				fresh_primary_kind(&error, &program).unwrap()
			else {
				panic!("wrong typed projection")
			};
			assert_eq!(op, expected_op);
			assert_eq!(lhs.type_hash(), core::cmp::Ordering::HASH);
			assert_eq!(rhs.type_hash(), expected_rhs);
		}
	}
	for symbol in [
		B::Add,
		B::Sub,
		B::Mul,
		B::Div,
		B::Rem,
		B::BitAnd,
		B::BitOr,
		B::BitXor,
		B::Shl,
		B::Shr,
		B::AddAssign,
		B::SubAssign,
		B::MulAssign,
		B::DivAssign,
		B::RemAssign,
		B::BitAndAssign,
		B::BitOrAssign,
		B::BitXorAssign,
		B::ShlAssign,
		B::ShrAssign,
	] {
		let error = Error::new(ErrorKind::Type, "non-comparison").with_detail(Detail::Binary {
			op: symbol,
			lhs: TypeName::Ordering,
			rhs: TypeName::I64,
		});
		assert!(fresh_primary_kind(&error, &program).is_none());
	}
	for (lhs, rhs) in [
		(TypeName::VecIter, TypeName::I64),
		(TypeName::Ordering, TypeName::VecIter),
	] {
		let error = Error::new(ErrorKind::Type, "unknown type").with_detail(Detail::Binary {
			op: B::Eq,
			lhs,
			rhs,
		});
		assert!(fresh_primary_kind(&error, &program).is_none());
	}
	let error = Error::new(ErrorKind::Overflow, "wrong kind").with_detail(Detail::Binary {
		op: B::Eq,
		lhs: TypeName::Ordering,
		rhs: TypeName::I64,
	});
	assert!(fresh_primary_kind(&error, &program).is_none());
}
