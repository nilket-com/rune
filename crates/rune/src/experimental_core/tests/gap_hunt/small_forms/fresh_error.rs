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
		TypeName::VecIter => TypeInfo::named::<rt::slice::Iter>(),
		TypeName::OptionIter => TypeInfo::named::<crate::modules::option::Iter>(),
 TypeName::StringSplit=>TypeInfo::named::<crate::modules::string::Split<crate::alloc::String>>(),
 TypeName::CharSplit=>TypeInfo::named::<crate::modules::string::Split<char>>(),
		TypeName::RangeIter => TypeInfo::named::<rt::range::RangeIter<i64>>(),
		TypeName::UnsignedRangeIter => TypeInfo::named::<rt::range::RangeIter<u64>>(),
		TypeName::CharRangeIter => TypeInfo::named::<rt::range::RangeIter<char>>(),
		TypeName::RangeInclusiveIter => {
			TypeInfo::named::<rt::range_inclusive::RangeInclusiveIter<i64>>()
		}
		TypeName::UnsignedRangeInclusiveIter => {
			TypeInfo::named::<rt::range_inclusive::RangeInclusiveIter<u64>>()
		}
		TypeName::CharRangeInclusiveIter => {
			TypeInfo::named::<rt::range_inclusive::RangeInclusiveIter<char>>()
		}
		TypeName::RangeFromIter => TypeInfo::named::<rt::range_from::RangeFromIter<i64>>(),
		TypeName::UnsignedRangeFromIter => TypeInfo::named::<rt::range_from::RangeFromIter<u64>>(),
		TypeName::CharRangeFromIter => TypeInfo::named::<rt::range_from::RangeFromIter<char>>(),

		TypeName::RangeTo => TypeInfo::named::<rt::RangeTo>(),
		TypeName::RangeToInclusive => TypeInfo::named::<rt::RangeToInclusive>(),
		TypeName::RangeInclusive => TypeInfo::named::<rt::RangeInclusive>(),
		TypeName::RangeFull => TypeInfo::named::<rt::RangeFull>(),
		TypeName::RangeFrom => TypeInfo::named::<rt::RangeFrom>(),
	})
}

pub(super) fn fresh_primary_kind(
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
				B::Add => "+",
				B::Sub => "-",
				B::Mul => "*",
				B::Div => "/",
				B::Rem => "%",
				B::BitAnd => "&",
				B::BitOr => "|",
				B::BitXor => "^",
				B::Shl => "<<",
				B::Shr => ">>",
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
				IntTarget::U32 if value.negative || value.magnitude > u64::from(u32::MAX) => {
					K::ValueToIntegerCoercionError {
						from: if value.negative {
							i64::try_from(-i128::from(value.magnitude)).ok()?.into()
						} else {
							value.magnitude.into()
						},
						to: "u32",
					}
				}
				IntTarget::Usize if value.negative || usize::try_from(value.magnitude).is_err() => {
					K::ValueToIntegerCoercionError {
						from: if value.negative {
							i64::try_from(-i128::from(value.magnitude)).ok()?.into()
						} else {
							value.magnitude.into()
						},
						to: "usize",
					}
				}
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
		Detail::Arity { given, expected } if error.kind == ErrorKind::Type => K::BadArgumentCount {
			actual: given as usize,
			expected: expected as usize,
		},
		Detail::SharedArgument{readers,..}=>K::AccessError{error:stock::AccessError::fresh_shared_for_test(readers)},
		Detail::ReadExclusive
		| Detail::WriteExclusive
		| Detail::TakeExclusive
		| Detail::ExclusiveArgument { .. }
			if error.kind == ErrorKind::AccessConflict =>
		{
			let mode = match error.detail {
				Detail::ReadExclusive
				| Detail::ExclusiveArgument {
					access: MovedAccess::Read,
					..
				} => 0,
				Detail::WriteExclusive
				| Detail::ExclusiveArgument {
					access: MovedAccess::Write,
					..
				} => 1,
				Detail::TakeExclusive
				| Detail::ExclusiveArgument {
					access: MovedAccess::Take,
					..
				} => 2,
				_ => return None,
			};
			K::AccessError {
				error: stock::AccessError::fresh_exclusive_for_test(mode)?,
			}
		}
		Detail::Unary { op, operand } if error.kind == ErrorKind::Type => {
			K::UnsupportedUnaryOperation {
				op: match op {
					crate::experimental_core::error::UnarySymbol::Not => "!",
					crate::experimental_core::error::UnarySymbol::Neg => "-",
				},
				operand: fresh_type_info(operand, program)?,
			}
		}
		Detail::ExpectedBool { found } if error.kind == ErrorKind::Type => K::ExpectedType {
			expected: crate::runtime::TypeInfo::named::<bool>(),
			actual: fresh_type_info(found, program)?,
		},
		Detail::UnsupportedIterRangeInclusive { start, end } => K::UnsupportedIterRangeInclusive {
			start: fresh_type_info(start, program)?,
			end: fresh_type_info(end, program)?,
		},
		Detail::UnsupportedIterRangeFrom { start } => K::UnsupportedIterRangeFrom {
			start: fresh_type_info(start, program)?,
		},
		Detail::UnsupportedIterRange { start, end } => K::UnsupportedIterRange {
			start: fresh_type_info(start, program)?,
			end: fresh_type_info(end, program)?,
		},
		Detail::MissingIndex { target } => K::MissingIndex {
			target: fresh_type_info(target, program)?,
		},
		Detail::MissingStringSlice if error.kind == ErrorKind::Panic => K::Panic {
			reason: stock::Panic::custom("missing string slice".to_owned()),
		},
		Detail::Underflow if error.kind == ErrorKind::Overflow => K::Underflow,
		Detail::None => match error.kind {
			ErrorKind::Overflow => K::Overflow,
			ErrorKind::DivisionByZero => K::DivideByZero,
			_ => return None,
		},
		_ => return None,
	})
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
fn integer_error_projection_covers_shift_and_index_targets() {
	use crate::experimental_core::arithmetic::{binary, Binary};
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(){7}").unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 12).unwrap();
	let negative = binary(Binary::Shl, FreshValue::I64(1), FreshValue::I64(-1)).unwrap_err();
	assert_eq!(
		format!("{:?}", fresh_primary_kind(&negative, &program).unwrap()),
		"ValueToIntegerCoercionError { from: Signed(-1), to: \"u32\" }"
	);
	let index = crate::experimental_core::execute::index_value(FreshValue::I64(-1)).unwrap_err();
	assert_eq!(
		format!("{:?}", fresh_primary_kind(&index, &program).unwrap()),
		"ValueToIntegerCoercionError { from: Signed(-1), to: \"usize\" }"
	);
	let underflow = binary(Binary::Shr, FreshValue::I64(1), FreshValue::I64(64)).unwrap_err();
	assert_eq!(
		format!("{:?}", fresh_primary_kind(&underflow, &program).unwrap()),
		"Underflow"
	);
}

#[test]
fn iterator_type_info_projection_matches_actual_builtin_hashes() {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::memory("pub fn main(){7}").unwrap())
		.unwrap();
	let program = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 12).unwrap();
	for name in [
		TypeName::VecIter,
		TypeName::OptionIter,
		TypeName::RangeIter,
		TypeName::UnsignedRangeIter,
		TypeName::CharRangeIter,
		TypeName::RangeInclusiveIter,
		TypeName::UnsignedRangeInclusiveIter,
		TypeName::CharRangeInclusiveIter,
		TypeName::RangeFromIter,
		TypeName::UnsignedRangeFromIter,
		TypeName::CharRangeFromIter,
	] {
		assert_eq!(
			fresh_type_info(name, &program).unwrap().type_hash(),
			name.builtin_hash().unwrap()
		);
	}
}
