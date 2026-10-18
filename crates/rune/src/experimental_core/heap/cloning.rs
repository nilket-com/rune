//! Iterative stock clone_with semantics. Temporary roots and scratch have explicit
//! ownership; path bits are scoped to the active recursive ancestors, never a memo.
use super::*;
use crate::experimental_core::error::{Detail, TypeName};

#[derive(Clone, Copy)]
enum Kind {
	Tuple,
	Variant { tag: Type, success: bool },
}

struct Frame {
	handle: Handle,
	kind: Kind,
	next: usize,
	length: usize,
	children: Vec<Value>,
}

impl Store {
	fn clone_room(&self, start: usize, scratch: usize, extra: usize) -> Result<()> {
		let used = self
			.bytes
			.checked_sub(start)
			.and_then(|n| n.checked_add(scratch))
			.and_then(|n| n.checked_add(extra))
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		if used > self.limits.bytes.saturating_sub(start) {
			return Err(err(ErrorKind::StoreCapacity));
		}
		Ok(())
	}

	fn admit_clone(&mut self, payload: Payload, start: usize, scratch: usize) -> Result<Value> {
		self.clone_room(start, scratch, payload.bytes()?)?;
		self.admit(payload)
	}

	fn clone_leaf(&mut self, value: Value, start: usize, scratch: usize) -> Result<Value> {
		let Value::Heap(h) = value else {
			self.retain(value)?;
			return Ok(value);
		};
		if matches!(self.payload(h)?, Payload::Closure { .. }) {
			self.retain(value)?;
			return Ok(value);
		}
		let payload = match self.payload(h)? {
			Payload::String(text) => {
				self.clone_room(start, scratch, text.len())?;
				Payload::String(String::try_from(text.as_str())?)
			}
			Payload::List {
				tag: tag @ (Type::Vec | Type::Struct(_)),
				values,
			} => {
				self.clone_room(
					start,
					scratch,
					values
						.len()
						.checked_mul(size_of::<Value>())
						.ok_or_else(|| err(ErrorKind::StoreCapacity))?,
				)?;
				let mut copy = Vec::try_with_capacity(values.len())?;
				for &v in values {
					copy.try_push(v)?;
				}
				Payload::List {
					tag: *tag,
					values: copy,
				}
			}
			Payload::Record {
				tag: tag @ (Type::Object | Type::Struct(_)),
				fields,
			} => {
				let size = fields
					.len()
					.checked_mul(size_of::<(String, Value)>())
					.and_then(|n| {
						fields
							.iter()
							.try_fold(n, |n, (key, _)| n.checked_add(key.len()))
					})
					.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
				self.clone_room(start, scratch, size)?;
				let mut copy = Vec::try_with_capacity(fields.len())?;
				let mut key_bytes = 0usize;
				// Check actual capacities after every allocation; no child owner is
				// acquired until the complete payload is admitted.
				for (key, value) in fields {
					let key = String::try_from(key.as_str())?;
					key_bytes = key_bytes
						.checked_add(key.capacity())
						.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
					copy.try_push((key, *value))?;
					let actual = copy
						.capacity()
						.checked_mul(size_of::<(String, Value)>())
						.and_then(|n| n.checked_add(key_bytes))
						.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
					self.clone_room(start, scratch, actual)?;
				}
				Payload::Record {
					tag: *tag,
					fields: copy,
				}
			}
			_ => {
				return Err(err(ErrorKind::Type).with_detail(Detail::NoCloneProtocol {
					found: TypeName::from_type(self.entry(h)?.tag()?),
				}));
			}
		};
		self.admit_clone(payload, start, scratch)
	}

	fn clone_frame(
		&mut self,
		h: Handle,
		start: usize,
		scratch: &mut usize,
		stack: &mut Vec<Frame>,
	) -> Result<()> {
		if self.entry(h)?.clone_path() {
			return Err(err(ErrorKind::StoreCapacity));
		}
		let (kind, length) = match self.payload(h)? {
			Payload::List {
				tag: Type::Tuple,
				values,
			} => (Kind::Tuple, values.len()),
			Payload::Variant {
				tag,
				success,
				payload,
			} => (
				Kind::Variant {
					tag: *tag,
					success: *success,
				},
				usize::from(payload.is_some()),
			),
			_ => return Err(err(ErrorKind::InvalidCode)),
		};
		if stack.len() == stack.capacity() {
			let capacity = stack
				.capacity()
				.checked_mul(2)
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?
				.max(1);
			let old = stack
				.capacity()
				.checked_mul(size_of::<Frame>())
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
			let extra = capacity
				.checked_mul(size_of::<Frame>())
				.and_then(|n| n.checked_sub(old))
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
			self.clone_room(start, *scratch, extra)?;
			stack.try_reserve_exact(capacity - stack.len())?;
			*scratch = scratch
				.checked_sub(old)
				.and_then(|n| n.checked_add(stack.capacity().checked_mul(size_of::<Frame>())?))
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
			self.clone_room(start, *scratch, 0)?;
		}
		self.clone_room(
			start,
			*scratch,
			length
				.checked_mul(size_of::<Value>())
				.ok_or_else(|| err(ErrorKind::StoreCapacity))?,
		)?;
		let children = Vec::try_with_capacity(length)?;
		let extra = children
			.capacity()
			.checked_mul(size_of::<Value>())
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		self.clone_room(start, *scratch, extra)?;
		stack
			.push_within_capacity(Frame {
				handle: h,
				kind,
				next: 0,
				length,
				children,
			})
			.map_err(|_| err(ErrorKind::InvalidCode))?;
		*scratch = scratch
			.checked_add(extra)
			.ok_or_else(|| err(ErrorKind::StoreCapacity))?;
		self.entry_mut(h)?.set_clone_path(true);
		Ok(())
	}

	fn finish_clone(&mut self, frame: &Frame, start: usize, scratch: usize) -> Result<Value> {
		let payload = match frame.kind {
			Kind::Tuple => {
				self.clone_room(
					start,
					scratch,
					frame
						.children
						.len()
						.checked_mul(size_of::<Value>())
						.ok_or_else(|| err(ErrorKind::StoreCapacity))?,
				)?;
				let mut values = Vec::try_with_capacity(frame.children.len())?;
				for &v in &frame.children {
					values.try_push(v)?;
				}
				Payload::List {
					tag: Type::Tuple,
					values,
				}
			}
			Kind::Variant { tag, success } => Payload::Variant {
				tag,
				success,
				payload: frame.children.first().copied(),
			},
		};
		self.admit_clone(payload, start, scratch)
	}

	pub(crate) fn clone_value(&mut self, value: Value) -> Result<Value> {
		let start = self.bytes;
		let mut scratch = 0usize;
		let mut stack = Vec::<Frame>::new();
		let mut pending = Some(value);
		let mut completed = None;
		let result = (|| -> Result<Value> {
			loop {
				if let Some(value) = pending.take() {
					if let Value::Heap(h) = value {
						if matches!(
							self.entry(h)?.tag()?,
							Type::Tuple | Type::Option | Type::Result
						) {
							self.clone_frame(h, start, &mut scratch, &mut stack)?;
						} else {
							completed = Some(self.clone_leaf(value, start, scratch)?);
						}
					} else {
						completed = Some(self.clone_leaf(value, start, scratch)?);
					}
				}
				if let Some(value) = completed.take() {
					if let Some(parent) = stack.last_mut() {
						if let Err(value) = parent.children.push_within_capacity(value) {
							completed = Some(value);
							return Err(err(ErrorKind::InvalidCode));
						}
					} else {
						return Ok(value);
					}
				}
				let frame = stack
					.last_mut()
					.ok_or_else(|| err(ErrorKind::InvalidCode))?;
				if frame.next < frame.length {
					pending = Some(match self.payload(frame.handle)? {
						Payload::List { values, .. } => *values
							.get(frame.next)
							.ok_or_else(|| err(ErrorKind::InvalidCode))?,
						Payload::Variant {
							payload: Some(value),
							..
						} => *value,
						_ => return Err(err(ErrorKind::InvalidCode)),
					});
					frame.next += 1;
				} else {
					let frame = stack.pop().ok_or_else(|| err(ErrorKind::InvalidCode))?;
					self.entry_mut(frame.handle)?.set_clone_path(false);
					let result = self.finish_clone(&frame, start, scratch);
					// The payload retains children on admission. Each partial child
					// root is ours until this release, on both success and failure.
					let mut cleanup = None;
					for &child in &frame.children {
						if let Err(e) = self.release(child) {
							cleanup.get_or_insert(e);
						}
					}
					scratch = scratch
						.checked_sub(frame.children.capacity() * size_of::<Value>())
						.ok_or_else(|| err(ErrorKind::InvalidCode))?;
					completed = Some(result?);
					if let Some(e) = cleanup {
						return Err(e);
					}
				}
			}
		})();
		if result.is_err() {
			if let Some(value) = completed {
				let _ = self.release(value);
			}
			for frame in stack.into_iter().rev() {
				if let Ok(entry) = self.entry_mut(frame.handle) {
					entry.set_clone_path(false);
				}
				for child in frame.children {
					let _ = self.release(child);
				}
			}
		}
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rune_alloc::limit;

	include!("nominal_clone_tests.rs");

	fn clear(s: &Store) {
		assert!(s
			.slots
			.iter()
			.filter_map(|s| s.entry.as_ref())
			.all(|e| !e.clone_path()));
		s.assert_work();
	}

	#[test]
	fn layout_and_method_identities() {
		assert_eq!(
			(size_of::<Entry>(), core::mem::align_of::<Entry>()),
			(72, 8)
		);
		assert_eq!((size_of::<Slot>(), core::mem::align_of::<Slot>()), (80, 8));
		for (method, name) in [
			(super::super::super::error::AdmittedMethod::Len, "len"),
			(super::super::super::error::AdmittedMethod::Push, "push"),
			(super::super::super::error::AdmittedMethod::Clone, "clone"),
		] {
			assert_eq!(method.hash(), crate::Hash::ident(name));
		}
	}

	#[test]
	fn both_dispatch_modes_preserve_fuel_and_rollback_on_allocator_and_store_limits() {
		use crate::experimental_core::{
			code::{Builder, CloneMode, Function, Op},
			error::Location,
			execute::Executor,
		};
		let loc = Location {
			source: 1,
			revision: 1,
			start: 0,
			end: 1,
		};
		for mode in [CloneMode::Associated, CloneMode::Global] {
			let mut f = Function::new(2, 1).unwrap();
			f.push(
				Op::Clone {
					out: 1,
					value: 0,
					mode,
				},
				loc,
			)
			.unwrap();
			f.push(Op::Return { value: 1 }, loc).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			let code = b.finish().unwrap();
			assert!(code.function(0).unwrap().instructions()[0].is_charged());
			assert!(!code.function(0).unwrap().instructions()[1].is_charged());
			for store_budget in [false, true] {
				let mut failures = 0;
				let mut successes = 0;
				for budget in (0..2048).step_by(64) {
					let mut s = Store::new(Limits::default()).unwrap();
					let leaf = s.string("leaf").unwrap();
					let source = s.list(Type::Tuple, &[leaf, leaf]).unwrap();
					let before = (s.live, s.bytes, s.edges);
					let e = Executor::new().unwrap();
					assert_eq!(
						e.run_values(&code, &mut s, 0, &[source], 0)
							.err()
							.unwrap()
							.error
							.kind,
						ErrorKind::FuelExhausted
					);
					let e = Executor::new().unwrap();
					let result = if store_budget {
						s.limits.bytes = s.bytes + budget;
						e.run_values(&code, &mut s, 0, &[source], 1)
							.map_err(|f| f.error.kind)
					} else {
						limit::with(budget, || {
							e.run_values(&code, &mut s, 0, &[source], 1)
								.map_err(|f| f.error.kind)
						})
						.call()
					};
					match result {
						Ok(copy) => {
							successes += 1;
							s.release(copy).unwrap();
						}
						Err(e) => {
							failures += 1;
							assert_eq!(
								e,
								if store_budget {
									ErrorKind::StoreCapacity
								} else {
									ErrorKind::Allocation
								}
							);
						}
					}
					clear(&s);
					assert_eq!((s.live, s.bytes, s.edges), before);
					assert_eq!(
						s.format_value(source, true).unwrap().as_str(),
						"(\"leaf\", \"leaf\")"
					);
					s.release(source).unwrap();
					s.release(leaf).unwrap();
					assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
				}
				assert!(successes > 0 && failures > 0);
			}
		}
	}

	#[test]
	fn allocation_failures_restore_payload_edges_aliases_and_path_marks() {
		let mut failures = 0;
		let mut successes = 0;
		for budget in (0..2048).step_by(16) {
			let mut s = Store::new(Limits::default()).unwrap();
			let child = s
				.list(Type::Vec, &[Value::Bool(true), Value::Char('x')])
				.unwrap();
			let option = s.variant(Type::Option, true, Some(child)).unwrap();
			let source = s.list(Type::Tuple, &[option, option]).unwrap();
			s.retain(source).unwrap();
			let before = (
				s.live,
				s.bytes,
				s.edges,
				s.format_value(source, true).unwrap(),
			);
			match limit::with(budget, || s.clone_value(source)).call() {
				Ok(copy) => {
					successes += 1;
					s.release(copy).unwrap();
				}
				Err(e) => {
					failures += 1;
					assert_eq!(e.kind, ErrorKind::Allocation);
				}
			}
			clear(&s);
			assert_eq!(
				(
					s.live,
					s.bytes,
					s.edges,
					s.format_value(source, true).unwrap()
				),
				before
			);
			for v in [source, source, option, child] {
				s.release(v).unwrap();
			}
			assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
		}
		assert!(failures > 0 && successes > 0);
	}

	#[test]
	fn one_budget_covers_frames_children_and_new_payloads() {
		let mut failures = 0;
		let mut successes = 0;
		for headroom in (0..1024).step_by(16) {
			let mut s = Store::new(Limits::default()).unwrap();
			let child = s.string("child").unwrap();
			let source = s.list(Type::Tuple, &[child, child]).unwrap();
			let before = (s.live, s.bytes, s.edges);
			s.limits.bytes = s.bytes + headroom;
			match s.clone_value(source) {
				Ok(copy) => {
					successes += 1;
					s.release(copy).unwrap();
				}
				Err(e) => {
					failures += 1;
					assert_eq!(e.kind, ErrorKind::StoreCapacity);
				}
			}
			clear(&s);
			assert_eq!((s.live, s.bytes, s.edges), before);
			s.release(source).unwrap();
			s.release(child).unwrap();
		}
		assert!(failures > 0 && successes > 0);
	}

	#[test]
	fn repeated_dag_is_copied_per_occurrence_and_shallow_cycles_are_shared() {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let option = s.variant(Type::Option, true, Some(child)).unwrap();
		let source = s.list(Type::Tuple, &[option, option]).unwrap();
		let copy = s.clone_value(source).unwrap();
		let left = s.get(copy, 0).unwrap();
		let right = s.get(copy, 1).unwrap();
		assert_ne!(left, right);
		let leaf = s.tuple_get(left, 0).unwrap();
		s.push(leaf, Value::I64(9)).unwrap();
		assert_eq!(
			s.format_value(source, true).unwrap().as_str(),
			"(Some([1]), Some([1]))"
		);
		assert_eq!(
			s.format_value(copy, true).unwrap().as_str(),
			"(Some([1, 9]), Some([1]))"
		);
		for v in [leaf, left, right, copy, source, option, child] {
			s.release(v).unwrap();
		}
		let source = s.list(Type::Vec, &[Value::Unit]).unwrap();
		s.set(source, 0, source).unwrap();
		let copy = s.clone_value(source).unwrap();
		let child = s.get(copy, 0).unwrap();
		assert_eq!(child, source);
		clear(&s);
		s.release(child).unwrap();
		s.release(copy).unwrap();
		s.release(source).unwrap();
		s.close().unwrap();
		assert_eq!(s.live, 0);
	}

	#[test]
	fn recursive_cycle_rejection_clears_path_marks_and_preserves_source() {
		let mut s = Store::new(Limits::default()).unwrap();
		let source = s.variant(Type::Option, true, Some(Value::Unit)).unwrap();
		s.tuple_set_mut(source, 0, source).unwrap();
		let before = (s.live, s.bytes, s.edges);
		assert_eq!(
			s.clone_value(source).unwrap_err().kind,
			ErrorKind::StoreCapacity
		);
		clear(&s);
		assert_eq!((s.live, s.bytes, s.edges), before);
		let found = s.tuple_get(source, 0).unwrap();
		assert_eq!(found, source);
		s.release(found).unwrap();
		s.release(source).unwrap();
		s.close().unwrap();
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}

	#[test]
	fn moved_tombstones_preserve_type_and_read_write_take_causes() {
		use super::super::super::error::MovedAccess;
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.string("child").unwrap();
		let values = [
			s.string("x").unwrap(),
			s.list(Type::Vec, &[child]).unwrap(),
			s.list(Type::Tuple, &[child]).unwrap(),
			s.record(Type::Object, &[("x", child)]).unwrap(),
			s.record(
				Type::Struct(crate::experimental_core::value::Nominal(0)),
				&[("x", child)],
			)
			.unwrap(),
			s.variant(Type::Option, true, Some(child)).unwrap(),
			s.variant(Type::Result, false, Some(child)).unwrap(),
			s.range(0, 2).unwrap(),
		];
		for value in values {
			let tag = s.tag(value).unwrap();
			let Value::Heap(h) = value else {
				unreachable!()
			};
			let bytes = s.entry(h).unwrap().charged_bytes();
			let edges = Store::payload_edges(s.entry(h).unwrap().payload.as_ref().unwrap());
			let before = (s.live, s.bytes, s.edges);
			s.invalidate(value).unwrap();
			assert_eq!(s.tag(value).unwrap(), tag);
			assert_eq!(
				(s.live, s.bytes, s.edges),
				(before.0, before.1 - bytes, before.2 - edges)
			);
			assert_eq!(
				s.payload(h).unwrap_err().detail,
				Detail::Moved {
					access: MovedAccess::Read
				}
			);
			assert_eq!(
				s.mutable(h).unwrap_err().detail,
				Detail::Moved {
					access: MovedAccess::Write
				}
			);
			assert_eq!(
				s.invalidate(value).unwrap_err().detail,
				Detail::Moved {
					access: MovedAccess::Take
				}
			);
			assert_eq!(s.entry(h).unwrap().charged_bytes(), 0);
			assert_eq!(
				Store::payload_edges(s.entry(h).unwrap().payload.as_ref().unwrap()),
				0
			);
			s.release(value).unwrap();
			clear(&s);
		}
		s.release(child).unwrap();
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}

	#[test]
	fn marker_and_charges_are_independent_and_debug_never_exposes_raw_word() {
		let mut s = Store::new(Limits::default()).unwrap();
		let value = s.string("x").unwrap();
		let Value::Heap(h) = value else {
			unreachable!()
		};
		let entry = s.entry_mut(h).unwrap();
		let charge = entry.charged_bytes();
		let before = std::format!("{entry:?}");
		entry.set_clone_path(true);
		assert_eq!(entry.charged_bytes(), charge);
		assert_eq!(std::format!("{entry:?}"), before);
		entry.set_charged_bytes(charge);
		assert!(entry.clone_path());
		entry.set_clone_path(false);
		assert_eq!(entry.charged_bytes(), charge);
		s.release(value).unwrap();
	}
}

/// Operator-only child workload: launch the exact ignored test in a fresh process
/// with RUNE_CLONE_STRESS_CHILD=1 and explicit wall/output caps. Never ordinary CI
/// or Miri; 100,000 host construction steps and one iterative clone are fixed.
#[test]
#[ignore = "operator-only bounded 100k clone child; requires RUNE_CLONE_STRESS_CHILD=1"]
fn deep_clone_stress_child() {
	assert_eq!(std::env::var("RUNE_CLONE_STRESS_CHILD").as_deref(), Ok("1"));
	let mut s = Store::new(super::Limits {
		entries: 250_000,
		bytes: 64 << 20,
	})
	.unwrap();
	let mut source = Value::I64(1);
	for _ in 0..100_000 {
		let next = s.variant(Type::Option, true, Some(source)).unwrap();
		s.release(source).unwrap();
		source = next;
	}
	let copy = s.clone_value(source).unwrap();
	assert_eq!(s.live, 200_000);
	assert!(s
		.slots
		.iter()
		.filter_map(|s| s.entry.as_ref())
		.all(|e| !e.clone_path()));
	s.release(copy).unwrap();
	s.release(source).unwrap();
	assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	s.assert_work();
}
