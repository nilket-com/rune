//! Independent bounded, iterative value rendering. No legacy formatting protocols.
//! Count and validate first, then allocate the output. The store cannot mutate
//! between passes. Traversal uses indices and borrowed payloads, never recursion.

use super::{Payload, Store};
use crate::alloc::{String, Vec};
use crate::experimental_core::{
	error::{Error, ErrorKind, Result},
	value::{Type, Value},
};
use core::fmt::{self, Write};
use core::mem::size_of;

// Only a proven moved native Debug receiver has argument-zero provenance.
// Stock FormatSpec handles String directly; nominal/dynamic and unknown paths
// must keep the generic failure. Read the retained tag only after failure.
#[cold]
#[inline(never)]
fn format_argument_failure(
	store: &Store,
	value: Value,
	debug: bool,
	mut detail: crate::experimental_core::error::Detail,
) -> crate::experimental_core::error::Detail {
	use crate::experimental_core::error::{Detail, MovedAccess};
	if !debug
		|| !matches!(
			detail,
			Detail::Moved {
				access: MovedAccess::Read
			}
		) {
		return detail;
	}
	let Value::Heap(handle) = value else {
		return detail;
	};
	let receiver = store
		.entry(handle)
		.ok()
		.and_then(|entry| entry.payload.as_ref());
	if matches!(
		receiver,
		Some(Payload::Moved(
			Type::Vec | Type::Tuple | Type::Object | Type::Option
		))
	) {
		detail = Detail::FormatArgumentMoved {
			arg: 0,
			access: MovedAccess::Read,
		};
	}
	detail
}

/// Host-selected limits for each complete formatting call (including all parts).
///
/// Depth starts at zero: the default accepts levels 0..=32. Scratch is the sum
/// of actual retained traversal and sorting capacities, excluding output. It is
/// not an RSS or transient allocation ceiling: growth can briefly hold old and
/// new buffers. Requested capacity is checked before allocating, actual capacity
/// immediately afterwards; an over-budget buffer is dropped before use.
///
/// Local pending-node/key-byte and scratch preflights may refuse before a later
/// child error. This is deliberate resource-first precedence at those checks,
/// not a global priority over earlier errors. Successful text is unchanged given
/// sufficient resources. Raising limits allocates nothing by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatLimits {
	/// Maximum rendered bytes, including punctuation and escaped keys.
	pub bytes: usize,
	/// Maximum values visited, including containers and repeated aliases.
	pub nodes: usize,
	/// Maximum child depth, with the root at level zero.
	pub depth: usize,
	/// Maximum retained traversal/sort allocation capacities, in bytes.
	pub scratch_bytes: usize,
}

impl Default for FormatLimits {
	fn default() -> Self {
		Self {
			bytes: 64 << 10,
			nodes: 4096,
			depth: 32,
			scratch_bytes: scratch_model(4096, 32).expect("fixed default scratch fits usize"),
		}
	}
}

fn scratch_model(nodes: usize, depth: usize) -> Option<usize> {
	nodes
		.checked_add(depth)?
		.checked_add(1)?
		.checked_mul(size_of::<Frame<'_>>().max(size_of::<usize>()))?
		.checked_mul(2)
}

// Only container continuations are retained. A level-zero container can have a
// level-32 container child: at most depth+1 frames are simultaneously live.
enum Frame<'a> {
	List {
		values: &'a [Value],
		next: usize,
		close: &'static str,
	},
	Record {
		fields: &'a [(String, Value)],
		next: usize,
		order: Vec<usize>,
		sorted: bool,
	},
	Variant,
}

#[cfg(test)]
#[derive(Default, Debug)]
struct Stats {
	scanned_keys: usize,
	comparisons: usize,
	// Logical byte pairs inspected by the test comparator, not CPU loads.
	key_bytes: usize,
	sort_allocations: usize,
	peak_scratch: usize,
}

struct Sink<'a> {
	limits: &'a FormatLimits,
	text: Option<String>,
	bytes: usize,
	nodes: usize,
	pending_nodes: usize,
	pending_bytes: usize,
	scratch_bytes: usize,
	failure: Option<Error>,
	#[cfg(test)]
	stats: Stats,
}

impl<'a> Sink<'a> {
	fn new(limits: &'a FormatLimits, text: Option<String>) -> Self {
		Self {
			limits,
			text,
			bytes: 0,
			nodes: 0,
			pending_nodes: 0,
			pending_bytes: 0,
			scratch_bytes: 0,
			failure: None,
			#[cfg(test)]
			stats: Stats::default(),
		}
	}

	fn push(&mut self, text: &str) -> Result<()> {
		let bytes = self.bytes.checked_add(text.len()).ok_or_else(limit)?;
		if bytes.checked_add(self.pending_bytes).ok_or_else(limit)? > self.limits.bytes {
			return Err(limit());
		}
		if let Some(output) = &mut self.text {
			output.try_push_str(text)?;
		}
		self.bytes = bytes;
		Ok(())
	}

	fn formatted(&mut self, value: fmt::Arguments<'_>) -> Result<()> {
		self.write_fmt(value).map_err(|_| {
			self.failure
				.unwrap_or_else(|| Error::new(ErrorKind::Type, "format value"))
		})
	}

	fn reserve<T>(&mut self, buffer: &mut Vec<T>, needed: usize) -> Result<()> {
		if needed <= buffer.capacity() {
			return Ok(());
		}
		let old = buffer
			.capacity()
			.checked_mul(size_of::<T>())
			.ok_or_else(limit)?;
		let other = self.scratch_bytes.checked_sub(old).ok_or_else(limit)?;
		let requested = needed.checked_mul(size_of::<T>()).ok_or_else(limit)?;
		if other.checked_add(requested).ok_or_else(limit)? > self.limits.scratch_bytes {
			return Err(limit());
		}
		buffer.try_reserve_exact(needed.checked_sub(buffer.len()).ok_or_else(limit)?)?;
		let actual = buffer
			.capacity()
			.checked_mul(size_of::<T>())
			.ok_or_else(limit)?;
		let total = other.checked_add(actual).ok_or_else(limit)?;
		if total > self.limits.scratch_bytes {
			// No element is used from an over-budget allocation. On traversal-stack
			// growth this also drops its owned sort vectors; the call aborts here.
			*buffer = Vec::new();
			return Err(limit());
		}
		self.scratch_bytes = total;
		#[cfg(test)]
		{
			self.stats.peak_scratch = self.stats.peak_scratch.max(total);
		}
		Ok(())
	}

	fn frame<'s>(&mut self, stack: &mut Vec<Frame<'s>>, frame: Frame<'s>) -> Result<()> {
		let needed = stack.len().checked_add(1).ok_or_else(limit)?;
		let max = self.limits.depth.checked_add(1).ok_or_else(limit)?;
		if needed > max {
			return Err(limit());
		}
		// Grow with actual traversal, never with the host's configured maximum.
		if needed > stack.capacity() {
			let desired = needed
				.max(stack.capacity().checked_mul(2).ok_or_else(limit)?)
				.min(max);
			self.reserve(stack, desired)?;
		}
		stack
			.push_within_capacity(frame)
			.map_err(|_| Error::new(ErrorKind::InvalidCode, "format stack capacity"))
	}
}

impl Write for Sink<'_> {
	fn write_str(&mut self, text: &str) -> fmt::Result {
		self.push(text).map_err(|error| {
			self.failure = Some(error);
			fmt::Error
		})
	}
}

fn limit() -> Error {
	Error::new(ErrorKind::FormatLimit, "format bytes/depth/nodes")
}
fn accounting() -> Error {
	Error::new(ErrorKind::InvalidCode, "format accounting")
}

/// Debug and Display of a type value are both stock's `Type(<hash>)`.
#[cold]
#[inline(never)]
fn type_value(tag: Type, sink: &mut Sink<'_>) -> Result<()> {
	let hash = crate::Hash::new(super::comparison::type_hash(tag)?);
	sink.formatted(format_args!("{:?}", crate::runtime::Type::new(hash)))
}

fn unsupported() -> Error {
	Error::new(ErrorKind::Unsupported, "value formatting form")
}

impl Store {
	pub(crate) fn format_value(&self, value: Value, debug: bool) -> Result<String> {
		self.format_value_with_limits(value, debug, &FormatLimits::default())
	}

	pub(crate) fn format_value_with_limits(
		&self,
		value: Value,
		debug: bool,
		limits: &FormatLimits,
	) -> Result<String> {
		self.format_parts_bounded(core::iter::once(Ok((value, debug, None))), limits, false)
	}

	/// Descriptor operands are already evaluated and the iterator is repeatable.
	#[cfg(test)]
	pub(crate) fn format_template_parts<I>(&self, parts: I, limits: &FormatLimits) -> Result<String>
	where
		I: Iterator<Item = Result<(Value, bool)>> + Clone,
	{
		self.format_parts_bounded(parts.map(|p| p.map(|(v, d)| (v, d, None))), limits, true)
	}

	pub(crate) fn format_template_specs<I>(&self, parts: I, limits: &FormatLimits) -> Result<String>
	where
		I: Iterator<Item = Result<(Value, bool, Option<crate::runtime::format::FormatSpec>)>>
			+ Clone,
	{
		self.format_parts_bounded(parts, limits, true)
	}

	fn format_parts_bounded<I>(
		&self,
		parts: I,
		limits: &FormatLimits,
		template: bool,
	) -> Result<String>
	where
		I: Iterator<Item = Result<(Value, bool, Option<crate::runtime::format::FormatSpec>)>>
			+ Clone,
	{
		if self.is_closed() {
			return Err(Error::new(ErrorKind::ClosedStore, "format value"));
		}
		let mut sink = Sink::new(limits, None);
		for part in parts.clone() {
			let (value, debug, spec) = part?;
			if let Some(spec) = spec {
				self.render_spec(value, spec, &mut sink)
					.map_err(|mut error| {
						error.detail = format_argument_failure(
							self,
							value,
							matches!(spec.format_type, crate::runtime::format::Type::Debug),
							error.detail,
						);
						error
					})?;
			} else {
				self.render(value, debug, &mut sink).map_err(|mut error| {
					error.detail = format_argument_failure(self, value, debug, error.detail);
					error
				})?;
			}
		}
		let size = sink.bytes;
		if template {
			self.admission_room(size)?;
		}
		let output = String::try_with_capacity(size)?;
		#[cfg(test)]
		let output = {
			let mut output = output;
			if self.format_capacity_extra != 0 {
				let requested = size
					.checked_add(self.format_capacity_extra)
					.ok_or_else(limit)?;
				// Real allocator capacity, not a fabricated accounting number.
				output.try_reserve_exact(requested)?;
			}
			output
		};
		if output.capacity() > limits.bytes {
			return Err(limit());
		}
		// Actual capacity remains subject to string_owned admission, independently
		// of the counted preflight (including allocator overgrowth).
		sink = Sink::new(limits, Some(output));
		for part in parts {
			let (value, debug, spec) = part?;
			if let Some(spec) = spec {
				self.render_spec(value, spec, &mut sink)
					.map_err(|mut error| {
						error.detail = format_argument_failure(
							self,
							value,
							matches!(spec.format_type, crate::runtime::format::Type::Debug),
							error.detail,
						);
						error
					})?;
			} else {
				self.render(value, debug, &mut sink).map_err(|mut error| {
					error.detail = format_argument_failure(self, value, debug, error.detail);
					error
				})?;
			}
		}
		if sink.bytes != size {
			return Err(Error::new(ErrorKind::InvalidCode, "format two-pass size"));
		}
		sink.text.ok_or_else(unsupported)
	}

	fn render_spec(
		&self,
		value: Value,
		spec: crate::runtime::format::FormatSpec,
		sink: &mut Sink<'_>,
	) -> Result<()> {
		use crate::alloc::fmt::TryWrite;
		use crate::runtime::format::{Alignment, Type as FormatType};
		let debug = matches!(spec.format_type, FormatType::Debug);
		#[derive(Clone, Copy)]
		enum Body {
			Char(char),
			Bool(bool),
			UInt(u64),
			Float(f64),
			Text(crate::experimental_core::value::Handle),
		}
		let (body, padding) = match value {
			Value::Bool(v) => (Body::Bool(v), spec.padding()),
			Value::Char(v) if !debug => (Body::Char(v), spec.padding()),
			Value::I64(v) => {
				let (v, p) = spec.int_traits(v);
				(Body::UInt(v), p)
			}
			Value::U64(v) => {
				let (v, p) = spec.uint_traits(v);
				(Body::UInt(v), p)
			}
			Value::F64(v) => {
				let (v, p) = spec.float_traits(v);
				(Body::Float(v), p)
			}
			Value::Heap(h) if !debug && matches!(self.entry(h)?.tag()?, Type::String) => {
				// Select the scalar form without reading its payload yet.
				(Body::Text(h), spec.padding())
			}
			_ => {
				return self.render(value, debug, sink);
			}
		};
		if sink
			.nodes
			.checked_add(sink.pending_nodes)
			.ok_or_else(limit)?
			>= sink.limits.nodes
		{
			return Err(limit());
		}
		sink.nodes += 1;
		let text = match body {
			Body::Text(handle) => {
				// Preserve start_value's node-budget-before-payload-read ordering.
				let Payload::String(text) = self.payload(handle)? else {
					return Err(unsupported());
				};
				Some(text.as_str())
			}
			_ => None,
		};
		let write_body = |w: &mut dyn TryWrite| -> crate::alloc::Result<()> {
			match body {
				Body::Char(v) => w.try_write_char(v),
				Body::Bool(v) => w.try_write_str(if v { "true" } else { "false" }),
				Body::UInt(v) => spec.format_unsigned(w, v),
				Body::Float(v) => spec.format_float(w, v),
				Body::Text(_) => spec.format_str(w, text.ok_or(crate::alloc::Error::FormatError)?),
			}
		};
		struct Measure {
			bytes: usize,
			chars: usize,
			max: usize,
		}
		impl TryWrite for Measure {
			fn try_write_str(&mut self, s: &str) -> crate::alloc::Result<()> {
				let bytes = self
					.bytes
					.checked_add(s.len())
					.ok_or(crate::alloc::Error::CapacityOverflow)?;
				if bytes > self.max {
					return Err(crate::alloc::Error::CapacityOverflow);
				}
				self.chars = self
					.chars
					.checked_add(s.chars().count())
					.ok_or(crate::alloc::Error::CapacityOverflow)?;
				self.bytes = bytes;
				Ok(())
			}
		}
		let remaining = sink
			.limits
			.bytes
			.checked_sub(sink.bytes)
			.and_then(|n| n.checked_sub(sink.pending_bytes))
			.ok_or_else(limit)?;
		if let Body::Float(v) = body {
			if v.is_finite()
				&& spec
					.precision
					.is_some_and(|p| p > 0 && p.checked_add(2).is_none_or(|n| n > remaining))
			{
				return Err(limit());
			}
		}
		let mut measure = Measure {
			bytes: 0,
			chars: 0,
			max: remaining,
		};
		write_body(&mut measure).map_err(|_| limit())?;
		let fill = spec.padding_width(measure.chars, 0, padding.sign);
		let extra = fill
			.checked_mul(padding.fill.len_utf8())
			.and_then(|n| n.checked_add(padding.sign.map_or(0, char::len_utf8)))
			.ok_or_else(limit)?;
		if measure
			.bytes
			.checked_add(extra)
			.is_none_or(|n| n > remaining)
		{
			return Err(limit());
		}
		struct Writer<'a, 'b>(&'a mut Sink<'b>);
		impl TryWrite for Writer<'_, '_> {
			fn try_write_str(&mut self, s: &str) -> crate::alloc::Result<()> {
				if self.0.failure.is_some() {
					return Err(crate::alloc::Error::FormatError);
				}
				self.0.push(s).map_err(|e| {
					self.0.failure = Some(e);
					crate::alloc::Error::FormatError
				})
			}
		}
		let mut writer = Writer(sink);
		let (left, right) = if padding.zero {
			(0, 0)
		} else {
			match padding.align {
				Alignment::Left => (0, fill),
				Alignment::Center => (fill / 2, fill - fill / 2),
				Alignment::Right => (fill, 0),
			}
		};
		let result = (|| -> crate::alloc::Result<()> {
			for _ in 0..left {
				writer.try_write_char(padding.fill)?;
			}
			if let Some(sign) = padding.sign {
				writer.try_write_char(sign)?;
			}
			if padding.zero {
				for _ in 0..fill {
					writer.try_write_char(padding.fill)?;
				}
			}
			write_body(&mut writer)?;
			for _ in 0..right {
				writer.try_write_char(padding.fill)?;
			}
			Ok(())
		})();
		result.map_err(|_| writer.0.failure.unwrap_or_else(unsupported))
	}

	fn render(&self, value: Value, debug: bool, sink: &mut Sink<'_>) -> Result<()> {
		let mut stack = Vec::new();
		let mut current = Some((value, debug));
		// All owned buffers drop on every return, including failures. No store
		// references or reference counts are acquired by this read-only traversal.
		let result = (|| {
			loop {
				if let Some((value, debug)) = current.take() {
					current = self
						.start_value(value, debug, sink, &mut stack)?
						.map(|v| (v, true));
					continue;
				}
				let Some(frame) = stack.last_mut() else { break };
				match frame {
					Frame::List {
						values,
						next,
						close,
					} => {
						if let Some(value) = values.get(*next) {
							if *next != 0 {
								sink.push(", ")?;
							}
							*next += 1;
							current = Some((*value, true));
							continue;
						}
						sink.push(close)?;
					}
					Frame::Record {
						fields,
						next,
						order,
						sorted,
					} => {
						if *next < fields.len() {
							let index = if *sorted { order[*next] } else { *next };
							let (key, value) = &fields[index];
							if *sorted {
								sink.pending_nodes =
									sink.pending_nodes.checked_sub(1).ok_or_else(accounting)?;
								sink.pending_bytes = sink
									.pending_bytes
									.checked_sub(key.len().checked_add(4).ok_or_else(accounting)?)
									.ok_or_else(accounting)?;
							}
							if *next != 0 {
								sink.push(", ")?;
							}
							if *sorted {
								sink.formatted(format_args!("{:?}: ", key.as_str()))?;
							} else {
								sink.push(key)?;
								sink.push(": ")?;
							}
							*next += 1;
							current = Some((*value, true));
							continue;
						}
						sink.push("}")?;
						sink.scratch_bytes = sink
							.scratch_bytes
							.checked_sub(
								order
									.capacity()
									.checked_mul(size_of::<usize>())
									.ok_or_else(accounting)?,
							)
							.ok_or_else(accounting)?;
					}
					Frame::Variant => sink.push(")")?,
				}
				stack.pop();
			}
			Ok(())
		})();
		// A reserve failure may already have dropped the stack. After any error
		// the call ends, so no scratch reservation or pending work is reused.
		if result.is_ok() {
			sink.scratch_bytes = sink
				.scratch_bytes
				.checked_sub(
					stack
						.capacity()
						.checked_mul(size_of::<Frame<'_>>())
						.ok_or_else(accounting)?,
				)
				.ok_or_else(accounting)?;
			debug_assert_eq!(sink.scratch_bytes, 0);
			debug_assert_eq!((sink.pending_nodes, sink.pending_bytes), (0, 0));
		}
		result
	}

	fn start_value<'s>(
		&'s self,
		value: Value,
		debug: bool,
		sink: &mut Sink<'_>,
		stack: &mut Vec<Frame<'s>>,
	) -> Result<Option<Value>> {
		if stack.len() > sink.limits.depth
			|| sink
				.nodes
				.checked_add(sink.pending_nodes)
				.ok_or_else(limit)?
				>= sink.limits.nodes
		{
			return Err(limit());
		}
		sink.nodes += 1;
		// Method/protocol selection precedes access to an invalidated payload.
		if !debug {
			if let Value::Heap(h) = value {
				if matches!(
					self.entry(h)?.tag()?,
					Type::Vec
						| Type::Tuple | Type::Object
						| Type::Option | Type::Result
						| Type::Struct(_) | Type::Function
				) {
					return Err(self.no_display(value));
				}
			}
		}
		match value {
			Value::Unit if debug => sink.push("()")?,
			Value::Unit => return Err(self.no_display(value)),
			Value::Bool(n) => sink.push(if n { "true" } else { "false" })?,
			Value::I64(n) => sink.formatted(format_args!("{n}"))?,
			Value::U64(n) => sink.formatted(format_args!("{n}"))?,
			Value::F64(n) => {
				let mut buffer = ryu::Buffer::new();
				sink.push(buffer.format(n))?;
			}
			Value::Ordering(v) if debug => sink.formatted(format_args!("{v:?}"))?,
			Value::Ordering(_) => return Err(self.no_display(value)),
			Value::Char(c) if debug => sink.formatted(format_args!("{c:?}"))?,
			Value::Char(c) => {
				let mut bytes = [0; 4];
				sink.push(c.encode_utf8(&mut bytes))?;
			}
			Value::Heap(handle) => match self.payload(handle)? {
				Payload::String(text) if debug => {
					sink.formatted(format_args!("{:?}", text.as_str()))?
				}
				Payload::String(text) => sink.push(text)?,
				Payload::List {
					tag: Type::Struct(n),
					values,
				} if debug => {
					let invalid = || Error::new(ErrorKind::InvalidCode, "nominal shape");
					let mut buffer = [0; 256];
					let len =
						super::super::nominal::name_copy(*n, &mut buffer).ok_or_else(invalid)?;
					let name = core::str::from_utf8(&buffer[..len]).map_err(|_| invalid())?;
					sink.push(name)?;
					match super::super::nominal::shape_copy(*n).ok_or_else(invalid)? {
						super::super::nominal::Shape::Empty => {}
						super::super::nominal::Shape::Tuple(_) => {
							sink.push(" (")?;
							sink.frame(
								stack,
								Frame::List {
									values,
									next: 0,
									close: ")",
								},
							)?;
						}
						_ => return Err(invalid()),
					}
				}
				Payload::List { tag, values } if debug => {
					let (open, close) = match tag {
						Type::Vec => ("[", "]"),
						Type::Tuple => ("(", ")"),
						_ => return Err(unsupported()),
					};
					sink.push(open)?;
					sink.frame(
						stack,
						Frame::List {
							values,
							next: 0,
							close,
						},
					)?;
				}
				Payload::Closure {
					function, captures, ..
				} if debug => {
					sink.formatted(format_args!("closure (function: {function}, env:"))?;
					sink.push("[")?;
					sink.frame(
						stack,
						Frame::List {
							values: captures,
							next: 0,
							close: "])",
						},
					)?;
				}
				Payload::Record {
					tag: Type::Object,
					fields,
				} if debug => {
					let mut needed_bytes = 2usize;
					for (key, _) in fields {
						#[cfg(test)]
						{
							sink.stats.scanned_keys += 1;
						}
						needed_bytes = needed_bytes
							.checked_add(key.len())
							.and_then(|n| n.checked_add(4))
							.ok_or_else(limit)?;
					}
					if sink
						.nodes
						.checked_add(sink.pending_nodes)
						.and_then(|n| n.checked_add(fields.len()))
						.ok_or_else(limit)?
						> sink.limits.nodes
						|| sink
							.bytes
							.checked_add(sink.pending_bytes)
							.and_then(|n| n.checked_add(needed_bytes))
							.ok_or_else(limit)? > sink.limits.bytes
					{
						return Err(limit());
					}
					let mut order = Vec::new();
					sink.reserve(&mut order, fields.len())?;
					#[cfg(test)]
					{
						sink.stats.sort_allocations += usize::from(!fields.is_empty());
					}
					for index in 0..fields.len() {
						order.try_push(index)?;
					}
					// Rely on the standard library's O(k log k) comparison bound,
					// not a particular sorting algorithm or literal coefficient.
					order.sort_unstable_by(|a, b| {
						let a = fields[*a].0.as_str();
						let b = fields[*b].0.as_str();
						#[cfg(test)]
						{
							sink.stats.comparisons += 1;
							for (a, b) in a.bytes().zip(b.bytes()) {
								sink.stats.key_bytes += 1;
								if a != b {
									return a.cmp(&b);
								}
							}
							a.len().cmp(&b.len())
						}
						#[cfg(not(test))]
						{
							a.cmp(b)
						}
					});
					sink.pending_nodes += fields.len();
					sink.pending_bytes += needed_bytes - 2;
					sink.push("{")?;
					sink.frame(
						stack,
						Frame::Record {
							fields,
							next: 0,
							order,
							sorted: true,
						},
					)?;
				}
				Payload::Record {
					tag: Type::Struct(nominal),
					fields,
				} if debug => {
					let mut buffer = [0; 256];
					let invalid = || Error::new(ErrorKind::Unsupported, "struct name");
					let len = super::super::nominal::name_copy(*nominal, &mut buffer)
						.ok_or_else(invalid)?;
					let name = core::str::from_utf8(&buffer[..len]).map_err(|_| invalid())?;
					sink.push(name)?;
					sink.push(" {")?;
					sink.frame(
						stack,
						Frame::Record {
							fields,
							next: 0,
							order: Vec::new(),
							sorted: false,
						},
					)?;
				}
				Payload::Variant {
					tag,
					success,
					payload,
				} if debug => {
					let name = match (tag, success) {
						(Type::Option, true) => "Some",
						(Type::Option, false) => "None",
						(Type::Result, true) => "Ok",
						(Type::Result, false) => "Err",
						_ => return Err(unsupported()),
					};
					sink.push(name)?;
					if let Some(value) = payload {
						sink.push("(")?;
						sink.frame(stack, Frame::Variant)?;
						return Ok(Some(*value));
					}
				}
				Payload::List {
					tag: Type::Vec | Type::Tuple | Type::Struct(_),
					..
				}
				| Payload::Record {
					tag: Type::Object, ..
				}
				| Payload::Variant {
					tag: Type::Option | Type::Result,
					..
				} if !debug => return Err(self.no_display(value)),
				_ => return Err(unsupported()),
			},
			Value::Type(tag) => type_value(tag, sink)?,
			// Stock prints its bytecode offset here; fresh names the function index,
			// as the closure form does.
			Value::Function { id, .. } if debug => {
				sink.formatted(format_args!("immediate function (function: {id})"))?
			}
			_ => return Err(unsupported()),
		}
		Ok(None)
	}

	fn no_display(&self, value: Value) -> Error {
		Error::new(ErrorKind::Type, "missing display form").with_detail(
			self.diagnostic_type(value)
				.map_or(super::super::error::Detail::None, |found| {
					super::super::error::Detail::NoDisplay { found }
				}),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::super::Limits;
	use super::*;
	use std::string::ToString;

	fn store() -> Store {
		Store::new(Limits {
			entries: 16_384,
			bytes: 8 << 20,
		})
		.unwrap()
	}

	fn state(store: &Store) -> (usize, usize, usize, std::vec::Vec<u32>) {
		(
			store.live,
			store.bytes,
			store.work.len(),
			store
				.slots
				.iter()
				.filter_map(|s| s.entry.as_ref().map(|e| e.strong))
				.collect(),
		)
	}

	fn pass(store: &Store, value: Value, limits: &FormatLimits) -> (Result<()>, Stats) {
		let mut sink = Sink::new(limits, None);
		let result = store.render(value, true, &mut sink);
		assert!(sink.stats.peak_scratch <= limits.scratch_bytes);
		(result, sink.stats)
	}

	#[test]
	fn explicit_limits_preserve_default_wrappers_and_host_output_is_uncharged() {
		let mut store = store();
		let root = store.root_string(&"a".repeat(1 << 20)).unwrap();
		let before = state(&store);
		assert_eq!(
			store.root_format(&root, false).unwrap_err().kind,
			ErrorKind::FormatLimit
		);
		let limits = FormatLimits {
			bytes: 2 << 20,
			..FormatLimits::default()
		};
		assert_eq!(
			store
				.root_format_with_limits(&root, false, &limits)
				.unwrap()
				.len(),
			1 << 20
		);
		assert_eq!(state(&store), before);
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		std::println!(
			"format Frame={} usize={} default scratch={} (retained capacity budget)",
			size_of::<Frame<'_>>(),
			size_of::<usize>(),
			FormatLimits::default().scratch_bytes
		);
		assert!(scratch_model(usize::MAX, 0).is_none());
		assert!(scratch_model(0, usize::MAX).is_none());
	}

	fn wrap(
		store: &mut Store,
		form: usize,
		value: Value,
		text: &str,
	) -> (Value, std::string::String) {
		let (next, expected) = match form {
			0 => (
				store.list(Type::Vec, &[value]).unwrap(),
				std::format!("[{text}]"),
			),
			1 => (
				store.list(Type::Tuple, &[value]).unwrap(),
				std::format!("({text})"),
			),
			2 => (
				store.record(Type::Object, &[("x", value)]).unwrap(),
				std::format!("{{\"x\": {text}}}"),
			),
			3 => (
				store.variant(Type::Option, true, Some(value)).unwrap(),
				std::format!("Some({text})"),
			),
			4 => (
				store.variant(Type::Result, true, Some(value)).unwrap(),
				std::format!("Ok({text})"),
			),
			5 => {
				let nominal = super::super::super::nominal::intern(0xb200_0001, "B2Depth").unwrap();
				(
					store
						.record(Type::Struct(nominal), &[("x", value)])
						.unwrap(),
					std::format!("B2Depth {{x: {text}}}"),
				)
			}
			_ => unreachable!(),
		};
		store.release(value).unwrap();
		(next, expected)
	}

	#[test]
	fn all_container_forms_keep_the_depth_boundary_and_can_raise_it() {
		for form in 0..6 {
			for depth in [31, 32, 33, 63, 65] {
				let mut store = store();
				let (mut value, mut text) = (Value::Unit, "()".to_string());
				for _ in 0..depth {
					(value, text) = wrap(&mut store, form, value, &text);
				}
				let before = state(&store);
				let default = store.format_value(value, true);
				if depth <= 32 {
					assert_eq!(default.unwrap().as_str(), text.as_str());
				} else {
					assert_eq!(default.unwrap_err().kind, ErrorKind::FormatLimit);
				}
				let raised = FormatLimits {
					depth: 64,
					..FormatLimits::default()
				};
				let result = store.format_value_with_limits(value, true, &raised);
				if depth <= 64 {
					assert_eq!(result.unwrap().as_str(), text.as_str());
				} else {
					assert_eq!(result.unwrap_err().kind, ErrorKind::FormatLimit);
				}
				assert_eq!(state(&store), before);
				store.release(value).unwrap();
				assert_eq!(store.live_entries(), 0);
			}
		}
	}

	#[test]
	fn local_pending_preflights_and_scratch_limits_precede_only_later_errors() {
		for invalid in [false, true] {
			for over in [false, true] {
				let mut store = store();
				let child = if invalid {
					store.string("stale").unwrap()
				} else {
					// A range has no fresh formatting form.
					store.range(0, 1).unwrap()
				};
				let key = "z".repeat(if over { 65_536 } else { 1 });
				let object = store
					.record(Type::Object, &[("a", child), (&key, Value::Unit)])
					.unwrap();
				if invalid {
					// Test-only stale generation after admission; invalidation would
					// instead leave a live generation with an AccessConflict payload.
					let Value::Heap(handle) = child else {
						unreachable!()
					};
					store.slots[handle.slot as usize].generation += 1;
				}
				let before = state(&store);
				let (result, stats) = pass(&store, object, &FormatLimits::default());
				assert_eq!(
					result.unwrap_err().kind,
					if over {
						ErrorKind::FormatLimit
					} else if invalid {
						ErrorKind::InvalidHandle
					} else {
						ErrorKind::Unsupported
					}
				);
				assert_eq!(stats.sort_allocations, usize::from(!over));
				assert_eq!(state(&store), before);
			}
		}
		let mut store = store();
		let mut deep = Value::Unit;
		for _ in 0..33 {
			let next = store.list(Type::Vec, &[deep]).unwrap();
			store.release(deep).unwrap();
			deep = next;
		}
		let object = store
			.record(
				Type::Object,
				&[("a", deep), (&"z".repeat(65_536), Value::Unit)],
			)
			.unwrap();
		assert_eq!(
			store.format_value(object, true).unwrap_err().kind,
			ErrorKind::FormatLimit
		);
		// A later oversized sibling does not globally outrank an earlier failure.
		let range = store.range(0, 1).unwrap();
		let earlier = store.list(Type::Vec, &[range, object]).unwrap();
		store.release(range).unwrap();
		assert_eq!(
			store.format_value(earlier, true).unwrap_err().kind,
			ErrorKind::Unsupported
		);
		let small = store.list(Type::Vec, &[Value::Unit]).unwrap();
		let limits = FormatLimits {
			scratch_bytes: 0,
			..FormatLimits::default()
		};
		assert_eq!(
			store
				.format_value_with_limits(small, true, &limits)
				.unwrap_err()
				.kind,
			ErrorKind::FormatLimit
		);
		// Scalar formatting needs no traversal or sort allocation.
		assert_eq!(
			store
				.format_value_with_limits(Value::I64(42), false, &limits)
				.unwrap(),
			"42"
		);
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "large sorting workload; native CI counts comparisons and scan work"
	)]
	fn key_scans_sort_work_and_aggregate_pending_nodes_are_bounded() {
		for count in [0usize, 1, 100] {
			for permutation in 0..4 {
				let mut store = store();
				let keys: std::vec::Vec<_> = (0..count)
					.map(|i| std::format!("{}{i:03}", "p".repeat(1024)))
					.collect();
				let mut indices: std::vec::Vec<_> = (0..count).collect();
				match permutation {
					1 => indices.reverse(),
					2 => {
						for i in 1..count {
							indices.swap(i, (i * 37 + 11) % (i + 1));
						}
					}
					3 => {
						for i in 1..count {
							indices.swap(i, (i * 23 + 7) % (i + 1));
						}
					}
					_ => {}
				}
				let fields: std::vec::Vec<_> = indices
					.iter()
					.map(|i| (keys[*i].as_str(), Value::Unit))
					.collect();
				let object = store.record(Type::Object, &fields).unwrap();
				let (result, stats) = pass(&store, object, &FormatLimits::default());
				assert_eq!(stats.scanned_keys, count);
				if count == 100 {
					assert_eq!(result.unwrap_err().kind, ErrorKind::FormatLimit);
					assert_eq!((stats.comparisons, stats.sort_allocations), (0, 0));
				} else {
					result.unwrap();
					assert_eq!(stats.comparisons, 0);
				}
				let limits = FormatLimits {
					bytes: 256 << 10,
					..FormatLimits::default()
				};
				let (result, stats) = pass(&store, object, &limits);
				result.unwrap();
				assert_eq!(stats.scanned_keys, count);
				if count > 1 {
					let ceil_log2 = usize::BITS as usize - (count - 1).leading_zeros() as usize;
					// C=4 is an empirical check for these four orders, not a proof
					// or an assumption about the standard library's algorithm.
					assert!(stats.key_bytes <= 4 * count * ceil_log2 * 1027, "{stats:?}");
				}
				std::println!("sort k={count} order={permutation} {stats:?}");
			}
		}
		let mut store = store();
		let keys: std::vec::Vec<_> = (0..4094).map(|i| std::format!("k{i:04}")).collect();
		let fields: std::vec::Vec<_> = keys.iter().map(|key| (key.as_str(), Value::Unit)).collect();
		let inner = store.record(Type::Object, &fields).unwrap();
		let outer = store
			.record(Type::Object, &[("a", inner), ("z", Value::Unit)])
			.unwrap();
		let (result, stats) = pass(&store, outer, &FormatLimits::default());
		assert_eq!(result.unwrap_err().kind, ErrorKind::FormatLimit);
		assert_eq!(
			stats.sort_allocations, 1,
			"inner sibling reservation must fail before sorting"
		);
		let mut tower = Value::Unit;
		for _ in 0..12 {
			let next = store.list(Type::Vec, &[tower, tower]).unwrap();
			store.release(tower).unwrap();
			tower = next;
		}
		assert_eq!(
			store.format_value(tower, true).unwrap_err().kind,
			ErrorKind::FormatLimit
		);
	}

	#[test]
	fn template_preflight_actual_capacity_and_failure_leave_all_state_unchanged() {
		let mut store = Store::new(Limits {
			entries: 2,
			bytes: 10,
		})
		.unwrap();
		let arg = store.string("aaaa").unwrap();
		let parts = core::iter::once(Ok((arg, false)));
		let before = state(&store);
		let text = store
			.format_template_parts(parts.clone(), &FormatLimits::default())
			.unwrap();
		assert_eq!(text, "aaaa");
		store.format_capacity_extra = 8;
		let text = store
			.format_template_parts(parts.clone(), &FormatLimits::default())
			.unwrap();
		assert!(text.capacity() > 6);
		assert_eq!(
			store.string_owned(text).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!(state(&store), before);
		store.format_capacity_extra = 0;
		let second = store.string("b").unwrap();
		let before = state(&store);
		assert_eq!(
			store
				.format_template_parts(parts, &FormatLimits::default())
				.unwrap_err()
				.kind,
			ErrorKind::StoreCapacity
		);
		assert_eq!(state(&store), before);
		// Host formatting needs no additional store entry even when it is full.
		assert_eq!(store.format_value(arg, false).unwrap(), "aaaa");
		store.release(second).unwrap();
		store.release(arg).unwrap();
		assert_eq!(store.live_entries(), 0);
	}

	const DEEP_CHILD: &str = "RUNE_B2_DEEP_FORMAT_CHILD";
	#[test]
	#[cfg_attr(
		miri,
		ignore = "child-process scenario; parent spawn test is native-only"
	)]
	fn deep_format_child() {
		if std::env::var_os(DEEP_CHILD).is_none() {
			return;
		}
		std::thread::Builder::new()
			.stack_size(64 << 10)
			.spawn(|| {
				let mut store = store();
				let mut value = Value::Unit;
				for _ in 0..10_000 {
					let next = store.list(Type::Vec, &[value]).unwrap();
					store.release(value).unwrap();
					value = next;
				}
				let limits = FormatLimits {
					depth: 10_000,
					nodes: 20_000,
					bytes: 100_000,
					scratch_bytes: 16 << 20,
				};
				let output = store
					.format_value_with_limits(value, true, &limits)
					.unwrap();
				assert_eq!(
					output.as_str(),
					std::format!("{}(){}", "[".repeat(10_000), "]".repeat(10_000))
				);
				store.release(value).unwrap();
				assert_eq!(store.live_entries(), 0);
			})
			.unwrap()
			.join()
			.unwrap();
	}

	#[test]
	#[cfg_attr(miri, ignore = "Miri cannot spawn subprocesses; native CI only")]
	fn deep_formatting_uses_no_native_recursion() {
		use std::process::{Command, Stdio};
		use std::time::{Duration, Instant};
		let mut child = Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"experimental_core::heap::formatting::tests::deep_format_child",
				"--test-threads=1",
			])
			.env(DEEP_CHILD, "1")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.spawn()
			.unwrap();
		let start = Instant::now();
		loop {
			if let Some(status) = child.try_wait().unwrap() {
				assert!(status.success());
				break;
			}
			if start.elapsed() > Duration::from_secs(30) {
				let _ = child.kill();
				let _ = child.wait();
				panic!("deep-format child deadline");
			}
			std::thread::sleep(Duration::from_millis(10));
		}
	}
}

#[cfg(test)]
mod scratch_tests {
	use super::super::Limits;
	use super::*;

	#[test]
	fn small_live_ancestors_use_actual_capacity_and_do_not_preallocate_maximums() {
		let mut store = Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap();
		let inner = store
			.record(Type::Object, &[("y", Value::Unit), ("x", Value::I64(1))])
			.unwrap();
		let list = store.list(Type::Vec, &[inner]).unwrap();
		let outer = store
			.record(Type::Object, &[("b", Value::Unit), ("a", list)])
			.unwrap();
		let actual_peak = 4 * size_of::<Frame<'_>>() + 4 * size_of::<usize>();
		let limits = FormatLimits {
			depth: 10_000,
			scratch_bytes: actual_peak,
			..FormatLimits::default()
		};
		let mut sink = Sink::new(&limits, None);
		store.render(outer, true, &mut sink).unwrap();
		assert_eq!(sink.stats.peak_scratch, actual_peak);
		assert_eq!(
			(sink.scratch_bytes, sink.pending_nodes, sink.pending_bytes),
			(0, 0, 0)
		);
		assert_eq!(
			store
				.format_value_with_limits(outer, true, &limits)
				.unwrap(),
			"{\"a\": [{\"x\": 1, \"y\": ()}], \"b\": ()}"
		);
		let tight = FormatLimits {
			scratch_bytes: actual_peak - 1,
			..limits
		};
		assert_eq!(
			store
				.format_value_with_limits(outer, true, &tight)
				.unwrap_err()
				.kind,
			ErrorKind::FormatLimit
		);
		store.release(outer).unwrap();
		store.release(list).unwrap();
		store.release(inner).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[cfg(test)]
mod spec_tests {
	use super::super::Limits;
	use super::*;
	use crate::runtime::format::{Alignment, Flags, FormatSpec, Type as FormatType};

	fn spec(fill: char, width: usize, precision: Option<usize>) -> FormatSpec {
		FormatSpec::new(
			Flags::default(),
			fill,
			Alignment::Right,
			core::num::NonZeroUsize::new(width),
			precision,
			FormatType::Display,
		)
	}

	#[test]
	fn moved_string_spec_and_default_share_exact_read_error() {
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 1024,
		})
		.unwrap();
		let value = store.string("a").unwrap();
		store.invalidate(value).unwrap();
		let plain = store
			.format_template_parts(
				core::iter::once(Ok((value, false))),
				&FormatLimits::default(),
			)
			.unwrap_err();
		let with_spec = store
			.format_template_specs(
				core::iter::once(Ok((value, false, Some(spec(' ', 7, None))))),
				&FormatLimits::default(),
			)
			.unwrap_err();
		assert_eq!(plain, with_spec);
		let no_nodes = FormatLimits {
			nodes: 0,
			..FormatLimits::default()
		};
		let plain = store
			.format_template_parts(core::iter::once(Ok((value, false))), &no_nodes)
			.unwrap_err();
		let with_spec = store
			.format_template_specs(
				core::iter::once(Ok((value, false, Some(spec(' ', 7, None))))),
				&no_nodes,
			)
			.unwrap_err();
		assert_eq!(
			plain, with_spec,
			"zero-node budget must precede the moved read in both paths"
		);
		store.release(value).unwrap();
		assert_eq!(store.live_entries(), 0);
	}

	#[test]
	fn d1e_formatter_argument_requires_moved_native_debug_receiver() {
		use crate::experimental_core::error::{Detail, MovedAccess};
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 1024,
		})
		.unwrap();
		let value = store.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let failure = Error::new(ErrorKind::AccessConflict, "store").with_detail(Detail::Moved {
			access: MovedAccess::Read,
		});
		assert_eq!(
			format_argument_failure(&store, value, true, failure.detail),
			failure.detail,
			"a live receiver does not prove which child failed"
		);
		store.invalidate(value).unwrap();
		assert_eq!(
			format_argument_failure(&store, value, false, failure.detail),
			failure.detail
		);
		assert_eq!(
			format_argument_failure(&store, value, true, failure.detail),
			Detail::FormatArgumentMoved {
				arg: 0,
				access: MovedAccess::Read
			}
		);
		store.release(value).unwrap();
		let text = store.string("x").unwrap();
		store.invalidate(text).unwrap();
		assert_eq!(
			format_argument_failure(&store, text, true, failure.detail),
			failure.detail,
			"String is a direct FormatSpec read, not a native conversion"
		);
		store.release(text).unwrap();
		assert_eq!(store.live_entries(), 0);
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "maximum-budget native witness; bounded scalar helper has a separate small Miri test"
	)]
	fn bounded_spec_preflight_preserves_state_and_supports_raised_limits() {
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 1 << 20,
		})
		.unwrap();
		let value = store.string("a").unwrap();
		let before = store.state_for_test();
		let parts = core::iter::once(Ok((value, false, Some(spec('😀', 65535, None)))));
		assert_eq!(
			store
				.format_template_specs(parts.clone(), &FormatLimits::default())
				.unwrap_err()
				.kind,
			ErrorKind::FormatLimit
		);
		assert_eq!(store.state_for_test(), before);
		let raised = FormatLimits {
			bytes: 1 << 20,
			..FormatLimits::default()
		};
		let text = store.format_template_specs(parts, &raised).unwrap();
		assert_eq!(text.len(), 262137);
		assert!(text.ends_with('a'));
		assert_eq!(text.chars().count(), 65535);
		assert_eq!(store.state_for_test(), before);
		let parts = core::iter::once(Ok((
			Value::F64(1.25),
			false,
			Some(spec(' ', 0, Some(65535))),
		)));
		assert_eq!(
			store
				.format_template_specs(parts.clone(), &FormatLimits::default())
				.unwrap_err()
				.kind,
			ErrorKind::FormatLimit
		);
		assert_eq!(
			store.format_template_specs(parts, &raised).unwrap().len(),
			65537
		);
		assert_eq!(store.state_for_test(), before);
		store.release(value).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[cfg(test)]
mod small_spec_tests {
	use super::super::Limits;
	use super::*;
	use crate::runtime::format::{Alignment, Flags, FormatSpec, Type as FormatType};
	#[test]
	fn small_unicode_float_and_fallback_specs_are_bounded_and_balanced() {
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 4096,
		})
		.unwrap();
		let text = store.string("αbc").unwrap();
		let spec = FormatSpec::new(
			Flags::default(),
			'😀',
			Alignment::Center,
			core::num::NonZeroUsize::new(5),
			Some(2),
			FormatType::Display,
		);
		let parts = core::iter::once(Ok((text, false, Some(spec))));
		assert_eq!(
			store
				.format_template_specs(parts.clone(), &FormatLimits::default())
				.unwrap(),
			"😀αb😀😀"
		);
		let small = FormatLimits {
			bytes: 8,
			..FormatLimits::default()
		};
		assert_eq!(
			store.format_template_specs(parts, &small).unwrap_err().kind,
			ErrorKind::FormatLimit
		);
		let spec = FormatSpec::new(
			Flags::default(),
			' ',
			Alignment::Left,
			None,
			Some(2),
			FormatType::Display,
		);
		assert_eq!(
			store
				.format_template_specs(
					core::iter::once(Ok((Value::F64(1.25), false, Some(spec)))),
					&FormatLimits::default()
				)
				.unwrap(),
			"1.25"
		);
		let spec = FormatSpec::new(
			Flags::default(),
			' ',
			Alignment::Left,
			core::num::NonZeroUsize::new(7),
			None,
			FormatType::Debug,
		);
		let expected = store.format_value(text, true).unwrap();
		assert_eq!(
			store
				.format_template_specs(
					core::iter::once(Ok((text, true, Some(spec)))),
					&FormatLimits::default()
				)
				.unwrap(),
			expected
		);
		store.release(text).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}
