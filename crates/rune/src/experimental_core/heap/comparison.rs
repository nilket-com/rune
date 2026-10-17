//! Read-only structural comparisons. Sharing is expanded; no identity shortcut.
//! Every Value-level pair counts towards the stock walk limit (root = 1).
use super::{Payload, Store};
use crate::alloc::{String, Vec};
use crate::experimental_core::{
	arithmetic::{self, Binary},
	error::{BinarySymbol, Detail, Error, ErrorKind, Result, TypeName},
	value::{Type, Value},
};
use core::cmp::Ordering;

#[derive(Clone, Copy)]
enum Item<'a> {
	Value(Value),
	Text(&'a str),
	Pair(&'a str, Value),
}

#[derive(Clone, Copy)]
enum Sequence<'a> {
	Values(&'a [Value]),
	Fields(&'a [(String, Value)]),
	Pairs(&'a [(String, Value)]),
	Range(i64, i64),
	UnsignedRange(u64, u64),
	CharRange(char, char),
	One(Option<Value>),
	Pair(&'a str, Value),
	Bounds(Value, Value),
}
impl<'a> Sequence<'a> {
	fn get(self, n: u64) -> Option<Item<'a>> {
		let index = usize::try_from(n).ok();
		Some(match self {
			Self::Values(v) => Item::Value(*v.get(index?)?),
			Self::Fields(v) => Item::Value(v.get(index?)?.1),
			Self::Pairs(v) => {
				let (key, value) = v.get(index?)?;
				Item::Pair(key, *value)
			}
			Self::Range(start, end) => {
				let value = i128::from(start) + i128::from(n);
				if value >= i128::from(end) {
					return None;
				}
				Item::Value(Value::I64(i64::try_from(value).ok()?))
			}
			Self::UnsignedRange(start, end) => {
				let value = u128::from(start) + u128::from(n);
				if value >= u128::from(end) {
					return None;
				}
				Item::Value(Value::U64(u64::try_from(value).ok()?))
			}
			Self::CharRange(start, end) => {
				let mut value = u64::from(start as u32).checked_add(n)?;
				if (start as u32) < 0xd800 && value >= 0xd800 {
					value = value.checked_add(0x800)?;
				}
				if value >= u64::from(end as u32) {
					return None;
				}
				Item::Value(Value::Char(char::from_u32(u32::try_from(value).ok()?)?))
			}
			Self::One(value) if n == 0 => Item::Value(value?),
			Self::Pair(key, _) if n == 0 => Item::Text(key),
			Self::Pair(_, value) if n == 1 => Item::Value(value),
			Self::Bounds(start, _) if n == 0 => Item::Value(start),
			Self::Bounds(_, end) if n == 1 => Item::Value(end),
			_ => return None,
		})
	}
}

enum Task<'a> {
	Pair(Item<'a>, Item<'a>, usize),
	Sequence(Sequence<'a>, Sequence<'a>, u64, usize),
	Object(&'a [(String, Value)], &'a [(String, Value)], usize, usize),
}

fn error(kind: ErrorKind) -> Error {
	Error::new(kind, "primitive operation")
}

impl Store {
	fn item_type(&self, item: Item<'_>) -> Result<TypeName> {
		match item {
			Item::Text(_) => Ok(TypeName::String),
			Item::Pair(..) => Ok(TypeName::Tuple),
			Item::Value(Value::Heap(h)) => {
				self.payload(h)?;
				self.diagnostic_type(Value::Heap(h))
					.ok_or_else(|| error(ErrorKind::InvalidCode))
			}
			Item::Value(v) => TypeName::primitive(v).ok_or_else(|| error(ErrorKind::InvalidCode)),
		}
	}

	fn unsupported_pair(&self, op: Binary, a: Item<'_>, b: Item<'_>) -> Error {
		match (self.item_type(a), self.item_type(b)) {
			(Ok(lhs), Ok(rhs)) => error(ErrorKind::Type).with_detail(Detail::Binary {
				op: BinarySymbol::from_op(op, false),
				lhs,
				rhs,
			}),
			(Err(e), _) | (_, Err(e)) => e,
		}
	}

	fn expected_pair(&self, expected: TypeName, found: Item<'_>) -> Error {
		match self.item_type(found) {
			Ok(found) => {
				error(ErrorKind::Type).with_detail(Detail::ExpectedType { expected, found })
			}
			Err(error) => error,
		}
	}

	fn item_text<'a>(&'a self, item: Item<'a>) -> Result<Option<&'a str>> {
		Ok(match item {
			Item::Text(text) => Some(text),
			Item::Value(Value::Heap(h)) => match self.payload(h)? {
				Payload::String(text) => Some(text),
				_ => None,
			},
			_ => None,
		})
	}

	fn list_view<'a>(&'a self, item: Item<'a>, tag: Type) -> Result<Option<Sequence<'a>>> {
		Ok(match item {
			Item::Pair(key, value) if tag == Type::Tuple => Some(Sequence::Pair(key, value)),
			Item::Value(Value::Unit) if tag == Type::Tuple => Some(Sequence::Values(&[])),
			Item::Value(Value::Heap(h)) => match self.payload(h)? {
				Payload::List {
					tag: actual,
					values,
				} if *actual == tag => Some(Sequence::Values(values)),
				_ => None,
			},
			_ => None,
		})
	}

	fn iterable_view<'a>(&'a self, item: Item<'a>) -> Result<Sequence<'a>> {
		match item {
			Item::Pair(key, value) => return Ok(Sequence::Pair(key, value)),
			Item::Value(Value::Unit) => return Ok(Sequence::Values(&[])),
			Item::Value(Value::Heap(h)) => match self.payload(h)? {
				Payload::List { values, .. } => return Ok(Sequence::Values(values)),
				Payload::Record {
					tag: Type::Object,
					fields,
				} => return Ok(Sequence::Pairs(fields)),
				Payload::Range { start, end } => return Ok(Sequence::Range(*start, *end)),
				Payload::ValueRange { start, end } => {
					return Ok(match self.range_iteration_bounds(*start, *end)? {
						(Value::I64(a), Value::I64(b)) => Sequence::Range(a, b),
						(Value::U64(a), Value::U64(b)) => Sequence::UnsignedRange(a, b),
						(Value::Char(a), Value::Char(b)) => Sequence::CharRange(a, b),
						_ => return Err(error(ErrorKind::InvalidCode)),
					});
				}

				Payload::Variant {
					tag: Type::Option,
					payload,
					..
				} => return Ok(Sequence::One(*payload)),
				_ => {}
			},
			_ => {}
		}
		Err(
			error(ErrorKind::Type).with_detail(Detail::ComparisonNotIterable {
				found: self.item_type(item)?,
			}),
		)
	}

	/// Iterative DFS: a non-equal child decides every enclosing comparison.
	/// Continuations retain only borrowed views; errors leave roots/edges untouched.
	pub(crate) fn compare(&self, op: Binary, a: Value, b: Value) -> Result<Value> {
		let equality = matches!(op, Binary::Eq | Binary::Ne);
		let mut stack = Vec::new();
		let mut first = Some(Task::Pair(Item::Value(a), Item::Value(b), 1));
		let mut result = Some(Ordering::Equal);
		while let Some(task) = first.take().or_else(|| stack.pop()) {
			let decided = match task {
				Task::Sequence(a, b, index, depth) => match (a.get(index), b.get(index)) {
					(Some(a_child), Some(b_child)) => {
						stack.try_push(Task::Sequence(a, b, index + 1, depth))?;
						stack.try_push(Task::Pair(a_child, b_child, depth + 1))?;
						None
					}
					(None, None) => None,
					(None, Some(_)) => Some(Some(Ordering::Less)),
					(Some(_), None) => Some(Some(Ordering::Greater)),
				},
				Task::Object(a, b, index, depth) => {
					if let Some((key, value)) = a.get(index) {
						if let Some((_, rhs)) = b.iter().find(|(k, _)| k == key) {
							stack.try_push(Task::Object(a, b, index + 1, depth))?;
							stack.try_push(Task::Pair(
								Item::Value(*value),
								Item::Value(*rhs),
								depth + 1,
							))?;
							None
						} else {
							Some(Some(Ordering::Less))
						}
					} else {
						None
					}
				}
				Task::Pair(a, b, depth) => {
					if depth > 64 {
						return Err(error(ErrorKind::ValueDepth).with_detail(Detail::ValueDepth));
					}
					// Validate handles before producing type diagnostics, without changing ownership.
					let lhs = self.item_type(a)?;
					let rhs = self.item_type(b)?;
					if lhs == TypeName::String {
						let left = self
							.item_text(a)?
							.ok_or_else(|| error(ErrorKind::InvalidCode))?;
						let right = self
							.item_text(b)?
							.ok_or_else(|| self.expected_pair(TypeName::String, b))?;
						Some(Some(left.cmp(right)))
					} else if let Some(left) = self.list_view(
						a,
						if lhs == TypeName::Vec {
							Type::Vec
						} else {
							Type::Tuple
						},
					)? {
						// Unit is inline on the left, even though it is an empty iterable RHS.
						if matches!(a, Item::Value(Value::Unit)) {
							if matches!(b, Item::Value(Value::Unit)) {
								None
							} else {
								return Err(self.unsupported_pair(op, a, b));
							}
						} else {
							let right = if equality {
								self.iterable_view(b)?
							} else {
								let tag = if lhs == TypeName::Vec {
									Type::Vec
								} else {
									Type::Tuple
								};
								self.list_view(b, tag)?
									.ok_or_else(|| self.expected_pair(lhs, b))?
							};
							stack.try_push(Task::Sequence(left, right, 0, depth))?;
							None
						}
					} else if let Item::Value(Value::Heap(h)) = a {
						match self.payload(h)? {
							Payload::Record { tag, fields } => {
								if *tag == Type::Object && !equality {
									return Err(self.unsupported_pair(op, a, b));
								}
								let other = match b {
									Item::Value(Value::Heap(h)) => match self.payload(h)? {
										Payload::Record { tag: t, fields } if t == tag => {
											Some(fields)
										}
										_ => None,
									},
									_ => None,
								};
								let Some(other) = other else {
									return Err(if *tag == Type::Object {
										self.expected_pair(lhs, b)
									} else {
										self.unsupported_pair(op, a, b)
									});
								};
								if equality && fields.len() != other.len() {
									Some(Some(Ordering::Less))
								} else {
									stack.try_push(if *tag == Type::Object {
										Task::Object(fields, other, 0, depth)
									} else {
										Task::Sequence(
											Sequence::Fields(fields),
											Sequence::Fields(other),
											0,
											depth,
										)
									})?;
									None
								}
							}
							Payload::Variant {
								tag,
								success,
								payload,
							} => {
								let other = match b {
									Item::Value(Value::Heap(h)) => match self.payload(h)? {
										Payload::Variant {
											tag: t,
											success,
											payload,
										} if t == tag => Some((*success, *payload)),
										_ => None,
									},
									_ => None,
								}
								.ok_or_else(|| self.expected_pair(lhs, b))?;
								if *success != other.0 {
									Some(Some(success.cmp(&other.0)))
								} else {
									stack.try_push(Task::Sequence(
										Sequence::One(*payload),
										Sequence::One(other.1),
										0,
										depth,
									))?;
									None
								}
							}
							payload @ (Payload::Range { .. } | Payload::ValueRange { .. }) => {
								let bounds = payload.range_values()?;
								let other = match b {
									Item::Value(Value::Heap(h)) => match self.payload(h)? {
										payload @ (Payload::Range { .. }
										| Payload::ValueRange { .. }) => Some(payload.range_values()?),
										_ => None,
									},
									_ => None,
								}
								.ok_or_else(|| self.expected_pair(lhs, b))?;
								stack.try_push(Task::Sequence(
									Sequence::Bounds(bounds.0, bounds.1),
									Sequence::Bounds(other.0, other.1),
									0,
									depth,
								))?;
								None
							}

							_ => return Err(self.unsupported_pair(op, a, b)),
						}
					} else if let (Item::Value(a_value), Item::Value(b_value)) = (a, b) {
						if matches!(a_value, Value::I64(_) | Value::U64(_))
							&& !matches!(b_value, Value::I64(_) | Value::U64(_))
							&& !matches!(b_value, Value::Heap(_) | Value::Function { .. })
						{
							return Err(error(ErrorKind::Type)
								.with_detail(Detail::ExpectedNumber { found: rhs }));
						}
						if let (Value::Type(a), Value::Type(b)) = (a_value, b_value) {
							Some(Some(type_hash(a)?.cmp(&type_hash(b)?)))
						} else {
							let eq =
								arithmetic::binary(Binary::Eq, a_value, b_value).map_err(|e| {
									if e.kind == ErrorKind::Type {
										self.unsupported_pair(op, a, b)
									} else {
										e
									}
								})?;
							if eq == Value::Bool(true) {
								None
							} else if equality {
								Some(Some(Ordering::Less))
							} else {
								let lt = arithmetic::binary(Binary::Lt, a_value, b_value)?;
								let gt = arithmetic::binary(Binary::Gt, a_value, b_value)?;
								Some(if lt == Value::Bool(true) {
									Some(Ordering::Less)
								} else if gt == Value::Bool(true) {
									Some(Ordering::Greater)
								} else {
									None
								})
							}
						}
					} else {
						return Err(self.unsupported_pair(op, a, b));
					}
				}
			};
			if let Some(other) = decided {
				if other != Some(Ordering::Equal) {
					result = other;
					break;
				}
			}
		}
		arithmetic::compare(
			op,
			result == Some(Ordering::Equal),
			result == Some(Ordering::Less),
			result == Some(Ordering::Greater),
		)
	}
}

fn type_hash(tag: Type) -> Result<u64> {
	use crate::TypeHash;
	Ok(match tag {
		Type::Unit => <()>::HASH.into_inner(),
		Type::Bool => bool::HASH.into_inner(),
		Type::I64 => i64::HASH.into_inner(),
		Type::U64 => u64::HASH.into_inner(),
		Type::F64 => f64::HASH.into_inner(),
		Type::Char => char::HASH.into_inner(),
		Type::Ordering => core::cmp::Ordering::HASH.into_inner(),
		Type::String => String::HASH.into_inner(),
		Type::Vec => crate::runtime::Vec::HASH.into_inner(),
		Type::Tuple => crate::runtime::OwnedTuple::HASH.into_inner(),
		Type::Function => crate::runtime::Function::HASH.into_inner(),
		Type::Struct(nominal) => crate::experimental_core::nominal::hash_copy(nominal)
			.ok_or_else(|| error(ErrorKind::InvalidCode))?,
		_ => return Err(error(ErrorKind::InvalidCode)),
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use rune_alloc::limit;

	fn store() -> Store {
		Store::new(super::super::Limits::default()).unwrap()
	}

	#[test]
	fn comparison_stack_failure_and_nested_handle_errors_do_not_mutate_store() {
		let mut s = store();
		let inner = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let outer = s.list(Type::Tuple, &[inner]).unwrap();
		let before = s.state_for_test();
		assert_eq!(
			limit::with(0, || s.compare(Binary::Eq, outer, outer))
				.call()
				.unwrap_err()
				.kind,
			ErrorKind::Allocation
		);
		assert_eq!(before, s.state_for_test());
		assert_eq!(
			s.compare(Binary::Eq, outer, outer).unwrap(),
			Value::Bool(true)
		);
		s.invalidate(inner).unwrap();
		let before = s.state_for_test();
		let e = s.compare(Binary::Eq, outer, outer).unwrap_err();
		assert_eq!(e.kind, ErrorKind::AccessConflict);
		assert_eq!(
			e.detail,
			Detail::Moved {
				access: crate::experimental_core::error::MovedAccess::Read
			}
		);
		assert_eq!(before, s.state_for_test());
	}

	#[test]
	fn scalar_comparisons_need_no_allocation_and_nan_remains_unordered() {
		let s = store();
		for op in [Binary::Eq, Binary::Lt, Binary::Le, Binary::Gt, Binary::Ge] {
			assert_eq!(
				limit::with(0, || s.compare(
					op,
					Value::F64(f64::NAN),
					Value::F64(f64::NAN)
				))
				.call()
				.unwrap(),
				Value::Bool(false)
			);
		}
		assert_eq!(
			limit::with(0, || s.compare(Binary::Eq, Value::Unit, Value::Unit))
				.call()
				.unwrap(),
			Value::Bool(true)
		);
	}

	#[test]
	fn exact_walk_depth_cycles_and_early_difference_leave_store_unchanged() {
		let mut s = store();
		let mut v = Value::I64(1);
		for _ in 0..63 {
			let next = s.list(Type::Vec, &[v]).unwrap();
			s.release(v).unwrap();
			v = next;
		}
		assert_eq!(s.compare(Binary::Eq, v, v).unwrap(), Value::Bool(true));
		let deeper = s.list(Type::Vec, &[v]).unwrap();
		let before = s.state_for_test();
		let e = s.compare(Binary::Eq, deeper, deeper).unwrap_err();
		assert_eq!(
			(e.kind, e.detail),
			(ErrorKind::ValueDepth, Detail::ValueDepth)
		);
		assert_eq!(before, s.state_for_test());
		let cycle = s.list(Type::Vec, &[]).unwrap();
		s.push(cycle, cycle).unwrap();
		assert_eq!(
			s.compare(Binary::Eq, cycle, cycle).unwrap_err().kind,
			ErrorKind::ValueDepth
		);
		let a = s.list(Type::Vec, &[Value::I64(1), cycle]).unwrap();
		let b = s.list(Type::Vec, &[Value::I64(2), cycle]).unwrap();
		assert_eq!(s.compare(Binary::Eq, a, b).unwrap(), Value::Bool(false));
	}

	#[test]
	fn nested_leaf_details_and_prefix_precedence_are_exact() {
		let mut s = store();
		let text = s.string("a").unwrap();
		let a = s.list(Type::Vec, &[text]).unwrap();
		let b = s
			.list(Type::Tuple, &[Value::I64(1), Value::I64(2)])
			.unwrap();
		assert_eq!(
			s.compare(Binary::Eq, a, b).unwrap_err().detail,
			Detail::ExpectedType {
				expected: TypeName::String,
				found: TypeName::I64
			}
		);
		let pair = s.list(Type::Tuple, &[text, Value::I64(3)]).unwrap();
		let seq = s.list(Type::Vec, &[pair]).unwrap();
		let object = s.record(Type::Object, &[("a", Value::I64(3))]).unwrap();
		assert_eq!(
			s.compare(Binary::Eq, seq, object).unwrap(),
			Value::Bool(true)
		);
	}
}
