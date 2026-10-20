//! Direct iterator chains: the stock argument and element conversions and
//! the per-element steps the expanded chains call (C17d terminals, C17e
//! adapters). Traversal itself is
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

/// The front and length of a reversed source, read without advancing it.
enum Reversed {
	/// `values[index..]` of a Vec or Tuple.
	List {
		root: Value,
		index: usize,
		len: usize,
	},
	Option(Option<Value>),
	/// Integers `first..=first + last`; empty when `last` is `None`.
	Signed {
		first: i64,
		last: Option<u64>,
	},
	Unsigned {
		first: u64,
		last: Option<u64>,
	},
	/// Chars from `first` with `count` elements, skipping the surrogates.
	Char {
		first: char,
		count: u64,
	},
}

/// Chars in `a..b`, skipping the surrogate block as stock's char range does.
fn char_count(a: char, b: char) -> u64 {
	let (a, b) = (a as u32, b as u32);
	if a >= b {
		return 0;
	}
	let gap = b.min(0xe000).saturating_sub(a.max(0xd800));
	u64::from(b - a - gap)
}

impl Store {
	/// `filter_map`'s result test: an Option's success, or the stock
	/// `Option<Value>` conversion error for anything else.
	pub(crate) fn option_some(&self, value: Value) -> Result<bool> {
		if let Value::Heap(h) = value {
			if let Payload::Variant {
				tag: Type::Option,
				success,
				..
			} = self.payload(h)?
			{
				return Ok(*success);
			}
		}
		Err(err(ErrorKind::Type).with_detail(Detail::ExpectedType {
			expected: TypeName::Option,
			found: self.terminal_type(value)?,
		}))
	}

	/// `protocol_into_iter`: the iterator a `for` loop would take, or stock's
	/// missing-protocol error.
	pub(crate) fn protocol_iterator(&mut self, value: Value) -> Result<Value> {
		self.iterator(value).map_err(|error| {
			if error.kind != ErrorKind::Type || error.detail != Detail::None {
				return error;
			}
			match self.terminal_type(value) {
				Ok(found) => error.with_detail(Detail::MissingIntoIter { found }),
				Err(error) => error,
			}
		})
	}

	/// The source a direct `rev` reads from the back. A source stock gives no
	/// `rev` raises its missing-method error, at the `rev` call.
	fn reversed(&self, source: Value) -> Result<Reversed> {
		let missing = || -> Result<Error> {
			let target = self.terminal_type(source)?;
			let instance = target
				.builtin_hash()
				.ok_or_else(|| err(ErrorKind::InvalidCode))?;
			Ok(
				err(ErrorKind::Type).with_detail(Detail::MissingInstanceFunction {
					target,
					instance,
					hash: crate::Hash::associated_function(
						instance,
						crate::function::iter_terminal::Closure::Rev.hash(),
					),
				}),
			)
		};
		let Value::Heap(h) = source else {
			return Err(err(ErrorKind::InvalidCode));
		};
		Ok(match self.payload(h)? {
			Payload::VectorIterator(iter) => match self.payload(iter.handle)? {
				Payload::List { values, .. } => Reversed::List {
					root: Value::Heap(iter.handle),
					index: iter.index,
					len: values.len().saturating_sub(iter.index),
				},
				_ => return Err(missing()?),
			},
			Payload::OptionIterator { payload } => Reversed::Option(*payload),
			Payload::RangeIterator { next, end } => Reversed::Signed {
				first: *next,
				last: (next < end).then(|| end.abs_diff(*next) - 1),
			},
			Payload::ValueRangeIterator { next, end } => match (*next, *end) {
				(Value::I64(a), Value::I64(b)) => Reversed::Signed {
					first: a,
					last: (a < b).then(|| b.abs_diff(a) - 1),
				},
				(Value::U64(a), Value::U64(b)) => Reversed::Unsigned {
					first: a,
					last: (a < b).then(|| b - a - 1),
				},
				(Value::Char(a), Value::Char(b)) => Reversed::Char {
					first: a,
					count: char_count(a, b),
				},
				_ => return Err(err(ErrorKind::InvalidCode)),
			},
			Payload::OtherRangeIterator(state) => {
				let boxed_iterator::Kind::Range(cursor) = &state.kind else {
					return Err(err(ErrorKind::InvalidCode));
				};
				let Some(end) = cursor.end else {
					return Err(missing()?);
				};
				match (cursor.next, end) {
					(Value::I64(a), Value::I64(b)) => Reversed::Signed {
						first: a,
						last: (!cursor.done && a <= b).then(|| b.abs_diff(a)),
					},
					(Value::U64(a), Value::U64(b)) => Reversed::Unsigned {
						first: a,
						last: (!cursor.done && a <= b).then(|| b - a),
					},
					(Value::Char(a), Value::Char(b)) => Reversed::Char {
						first: a,
						count: if cursor.done || a > b {
							0
						} else {
							char_count(a, b) + 1
						},
					},
					_ => return Err(err(ErrorKind::InvalidCode)),
				}
			}
			_ => return Err(err(ErrorKind::InvalidCode)),
		})
	}

	/// Whether a reversed source has no element left.
	pub(crate) fn rev_empty(&self, source: Value) -> Result<bool> {
		Ok(match self.reversed(source)? {
			Reversed::List { len, .. } => len == 0,
			Reversed::Option(payload) => payload.is_none(),
			Reversed::Signed { last, .. } | Reversed::Unsigned { last, .. } => last.is_none(),
			Reversed::Char { count, .. } => count == 0,
		})
	}

	/// The offset of a reversed source's last element; 0 when it is empty.
	pub(crate) fn rev_last(&self, source: Value) -> Result<u64> {
		let last = match self.reversed(source)? {
			Reversed::List { len, .. } => len.checked_sub(1).map(|n| n as u64),
			Reversed::Option(_) => Some(0),
			Reversed::Signed { last, .. } | Reversed::Unsigned { last, .. } => last,
			Reversed::Char { count, .. } => count.checked_sub(1),
		};
		Ok(last.unwrap_or(0))
	}

	/// The element at `offset` from a reversed source's front, retained.
	pub(crate) fn rev_at(&mut self, source: Value, offset: Value) -> Result<Value> {
		let Value::U64(offset) = offset else {
			return Err(err(ErrorKind::InvalidCode));
		};
		let invalid = || err(ErrorKind::InvalidCode);
		match self.reversed(source)? {
			Reversed::List { root, index, len } => {
				let offset = usize::try_from(offset).map_err(|_| invalid())?;
				if offset >= len {
					return Err(invalid());
				}
				self.get(root, index + offset)
			}
			Reversed::Option(payload) => {
				let value = payload.filter(|_| offset == 0).ok_or_else(invalid)?;
				self.retain(value)?;
				Ok(value)
			}
			Reversed::Signed { first, last } => {
				if last.is_none_or(|last| offset > last) {
					return Err(invalid());
				}
				Ok(Value::I64(first.wrapping_add(offset as i64)))
			}
			Reversed::Unsigned { first, last } => {
				if last.is_none_or(|last| offset > last) {
					return Err(invalid());
				}
				Ok(Value::U64(first + offset))
			}
			Reversed::Char { first, count } => {
				if offset >= count {
					return Err(invalid());
				}
				let mut code = first as u32 + u32::try_from(offset).map_err(|_| invalid())?;
				if (first as u32) < 0xd800 && code >= 0xd800 {
					code += 0x800;
				}
				Ok(Value::Char(char::from_u32(code).ok_or_else(invalid)?))
			}
		}
	}
}
