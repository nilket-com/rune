//! C17d iterator terminals: the stock argument and element conversions and
//! the per-element steps the expanded chains call. Traversal itself is
//! emitted as charged instructions.
use super::super::error::{Detail, HostCause, IntTarget, IntValue, TypeName};
use super::*;
use crate::function::iter_terminal::Number;

impl Store {
	fn terminal_type(&self, value: Value) -> Result<TypeName> {
		self.diagnostic_type(value)
			.ok_or_else(|| err(ErrorKind::InvalidCode))
	}

	/// A `skip`/`take` count, converted as the stock native `usize` argument #1.
	pub(crate) fn adapter_count(&self, n: Value) -> Result<usize> {
		let int = IntValue::of(n).ok_or_else(|| match self.terminal_type(n) {
			Ok(found) => err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
				cause: HostCause::ExpectedNumber,
				arg: 1,
				found,
			}),
			Err(e) => e,
		})?;
		usize::try_from(int.wide()).map_err(|_| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
				cause: HostCause::IntegerCoercion,
				arg: 1,
				value: int,
				to: IntTarget::Usize,
			})
		})
	}

	/// `std::iter::range(start, end)`: both native `i64` arguments convert in
	/// order before the iterator exists.
	#[cold]
	#[inline(never)]
	pub(crate) fn range_iterator(&mut self, start: Value, end: Value) -> Result<Value> {
		let start = self.range_bound(start, 0)?;
		let end = self.range_bound(end, 1)?;
		self.admit(Payload::RangeIterator { next: start, end })
	}

	fn range_bound(&self, value: Value, arg: u8) -> Result<i64> {
		let Some(int) = IntValue::of(value) else {
			return Err(
				err(ErrorKind::Type).with_detail(Detail::NumericArgumentNumber {
					cause: HostCause::ExpectedNumber,
					arg,
					found: self.terminal_type(value)?,
				}),
			);
		};
		i64::try_from(int.wide()).map_err(|_| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::NumericArgumentCoerce {
				cause: HostCause::IntegerCoercion,
				arg,
				value: int,
				to: IntTarget::I64,
			})
		})
	}

	/// An element converted as the stock `FromValue` of the `sum`/`product`
	/// type. The error names no argument: the conversion is inside the native.
	pub(crate) fn number_element(&self, value: Value, target: Number) -> Result<Value> {
		if target == Number::F64 {
			return match value {
				Value::F64(_) => Ok(value),
				_ => Err(err(ErrorKind::Type).with_detail(Detail::ExpectedType {
					expected: TypeName::F64,
					found: self.terminal_type(value)?,
				})),
			};
		}
		let Some(int) = IntValue::of(value) else {
			return Err(err(ErrorKind::Type).with_detail(Detail::ExpectedNumber {
				found: self.terminal_type(value)?,
			}));
		};
		let wide = int.wide();
		let (converted, to) = match target {
			Number::I64 => (i64::try_from(wide).ok().map(Value::I64), IntTarget::I64),
			_ => (u64::try_from(wide).ok().map(Value::U64), IntTarget::U64),
		};
		converted.ok_or_else(|| {
			err(ErrorKind::IntegerCoercion).with_detail(Detail::Coerce { value: int, to })
		})
	}

	/// One `sum` (or `product`) step in the accumulator's type, which the
	/// seed fixed. Overflow is the stock "Numerical overflow".
	pub(crate) fn number_fold(&self, acc: Value, element: Value, product: bool) -> Result<Value> {
		let overflow = || err(ErrorKind::Overflow);
		Ok(match acc {
			Value::I64(a) => {
				let Value::I64(b) = self.number_element(element, Number::I64)? else {
					return Err(err(ErrorKind::InvalidCode));
				};
				Value::I64(
					if product {
						a.checked_mul(b)
					} else {
						a.checked_add(b)
					}
					.ok_or_else(overflow)?,
				)
			}
			Value::U64(a) => {
				let Value::U64(b) = self.number_element(element, Number::U64)? else {
					return Err(err(ErrorKind::InvalidCode));
				};
				Value::U64(
					if product {
						a.checked_mul(b)
					} else {
						a.checked_add(b)
					}
					.ok_or_else(overflow)?,
				)
			}
			Value::F64(a) => {
				let Value::F64(b) = self.number_element(element, Number::F64)? else {
					return Err(err(ErrorKind::InvalidCode));
				};
				Value::F64(if product { a * b } else { a + b })
			}
			_ => return Err(err(ErrorKind::InvalidCode)),
		})
	}

	/// Inserts a `(String, value)` element into the collected Object. The key
	/// text is copied: its handle stays readable (stock moves it out).
	#[cold]
	#[inline(never)]
	pub(crate) fn collect_insert(&mut self, object: Value, element: Value) -> Result<()> {
		let pair = match element {
			Value::Heap(h) => match self.payload(h)? {
				Payload::List {
					tag: Type::Tuple,
					values,
				} => Some(values.as_slice()),
				_ => None,
			},
			Value::Unit => Some(&[][..]),
			_ => None,
		};
		let Some(pair) = pair else {
			return Err(err(ErrorKind::Type).with_detail(Detail::ExpectedType {
				expected: TypeName::Tuple,
				found: self.terminal_type(element)?,
			}));
		};
		let &[key, value] = pair else {
			return Err(err(ErrorKind::Type).with_detail(Detail::TupleLength {
				actual: pair.len(),
				expected: 2,
			}));
		};
		if self.tag(key).ok() != Some(Type::String) {
			return Err(err(ErrorKind::Type).with_detail(Detail::ExpectedType {
				expected: TypeName::String,
				found: self.terminal_type(key)?,
			}));
		}
		self.set_key(object, indexing::RecordKey::Dynamic(key), value)
	}

	/// The collected Vec's elements as a new Tuple, in order.
	#[cold]
	#[inline(never)]
	pub(crate) fn collect_tuple(&mut self, vec: Value) -> Result<Value> {
		let Value::Heap(h) = vec else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let Payload::List {
			tag: Type::Vec,
			values,
		} = self.payload(h)?
		else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let mut copy = Vec::try_with_capacity(values.len())?;
		for &value in values {
			copy.try_push(value)?;
		}
		self.list(Type::Tuple, &copy)
	}

	/// Appends a String or char element, as stock `collect::<String>`.
	#[cold]
	#[inline(never)]
	pub(crate) fn collect_append(&mut self, string: Value, element: Value) -> Result<()> {
		if !matches!(element, Value::Char(_)) && self.tag(element).ok() != Some(Type::String) {
			return Err(err(ErrorKind::Type).with_detail(Detail::ExpectedType {
				expected: TypeName::String,
				found: self.terminal_type(element)?,
			}));
		}
		self.pipeline_append(string, element).map(|_| ())
	}
}
