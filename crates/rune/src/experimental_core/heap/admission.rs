//! Cold external-entry graph validation. No retain, release or payload mutation.
use super::*;
use crate::alloc::HashSet;
use crate::experimental_core::{nominal, value::Root};
use crate::function::formatting::Certificate;

#[cfg(test)]
std::thread_local! {
	static PROBE: core::cell::Cell<(bool,usize)> = const { core::cell::Cell::new((false,0)) };
}
enum AdmissionChildren<'a> {
	Small(core::array::IntoIter<Option<Value>, 3>),
	Values(core::slice::Iter<'a, Value>),
	Fields(core::slice::Iter<'a, (String, Value)>),
	ParentValues(Option<Value>, core::slice::Iter<'a, Value>),
}
impl Iterator for AdmissionChildren<'_> {
	type Item = Value;
	fn next(&mut self) -> Option<Value> {
		match self {
			Self::Small(values) => values.find_map(|v| v),
			Self::Values(values) => values.next().copied(),
			Self::Fields(fields) => fields.next().map(|(_, v)| *v),
			Self::ParentValues(parent, values) => parent.take().or_else(|| values.next().copied()),
		}
	}
}
impl Payload {
	// Deliberately separate from ownership/drop children. Every new payload
	// must classify all state that an admitted computation could later reach.
	fn admission_children(&self) -> AdmissionChildren<'_> {
		let small = |a, b, c| AdmissionChildren::Small([a, b, c].into_iter());
		match self {
			Self::Moved(_) | Self::String(_) | Self::Range { .. } | Self::RangeIterator { .. } => {
				small(None, None, None)
			}
			Self::Variant { payload, .. } | Self::OptionIterator { payload } => {
				small(*payload, None, None)
			}
			Self::ValueRange { start, end } => small(Some(*start), Some(*end), None),
			Self::ValueRangeIterator { next, end } => small(Some(*next), Some(*end), None),
			Self::VectorIterator(iter) => small(Some(Value::Heap(iter.handle)), None, None),
			Self::Record { fields, .. } => AdmissionChildren::Fields(fields.iter()),
			Self::List { values, .. } => AdmissionChildren::Values(values.iter()),
			Self::Closure { captures, .. } => AdmissionChildren::Values(captures.iter()),
			Self::SortGuard(state) => AdmissionChildren::ParentValues(
				Some(Value::Heap(state.parent)),
				state.values.iter(),
			),
			Self::OtherRange(shape) => small(shape.start, shape.end, None),
			Self::OtherRangeIterator(state) => {
				let source = state.source.map(Value::Heap);
				match &state.kind {
					boxed_iterator::Kind::Range(cursor) => {
						small(source, Some(cursor.next), cursor.end)
					}
					boxed_iterator::Kind::Split(_)
					| boxed_iterator::Kind::Chars(_)
					| boxed_iterator::Kind::Skip(_)
					| boxed_iterator::Kind::Take(_) => small(source, None, None),
				}
			}
		}
	}
}
impl Store {
	#[cfg(test)]
	pub(crate) fn format_admission_probe(fail: bool) -> usize {
		PROBE.with(|p| {
			let (_, n) = p.replace((fail, 0));
			n
		})
	}

	#[cold]
	#[inline(never)]
	pub(crate) fn validate_format_roots(
		&self,
		roots: &[&Root],
		certificate: &Certificate,
	) -> Result<()> {
		self.import_all(roots.iter().copied())?;
		#[cfg(test)]
		if PROBE.with(|p| {
			let (fail, n) = p.get();
			p.set((fail, n + 1));
			fail
		}) {
			return Err(err(ErrorKind::Allocation));
		}

		let mut visited = HashSet::new();
		let mut nominals = HashSet::new();
		let mut pending = Vec::new();
		let discover = |value: Value,
		                visited: &mut HashSet<Handle>,
		                pending: &mut Vec<Handle>|
		 -> Result<()> {
			if let Value::Heap(handle) = value {
				if visited.try_insert(handle)? {
					pending.try_push(handle)?;
				}
			}
			Ok(())
		};
		for root in roots {
			discover(root.value, &mut visited, &mut pending)?;
		}
		while let Some(handle) = pending.pop() {
			let payload = self
				.entry(handle)?
				.payload
				.as_ref()
				.ok_or_else(|| err(ErrorKind::InvalidHandle))?;
			if let Type::Struct(nominal) = payload.tag() {
				let hash = crate::Hash::new(
					nominal::hash_copy(nominal).ok_or_else(|| err(ErrorKind::InvalidCode))?,
				);
				if nominals.try_insert(hash)? && !certificate.permits(hash) {
					return Err(Error::refused(super::super::error::Refusal::Entry));
				}
			}
			for child in payload.admission_children() {
				discover(child, &mut visited, &mut pending)?;
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate as rune;
	use crate::function::formatting::State;
	use crate::runtime::Protocol;
	use crate::{Hash, Module};

	fn certificate(receiver: Hash) -> crate::sync::Arc<Certificate> {
		let mut state = State::default();
		state
			.begin(
				Hash::associated_function(receiver, &Protocol::DISPLAY_FMT),
				false,
			)
			.unwrap();
		state.snapshot().unwrap()
	}

	#[test]
	fn bounded_enum_parent_authority_distinguishes_safe_variants() {
		let unsafe_hash = Hash::type_hash(["S7b2UnsafeEnumWalk"]);
		let safe_hash = Hash::type_hash(["S7b2SafeEnumWalk"]);
		let mut staged = nominal::Staging::default();
		for (hash, name) in [
			(unsafe_hash, "S7b2UnsafeEnumWalk::A"),
			(safe_hash, "S7b2SafeEnumWalk::A"),
		] {
			staged
				.stage_variant(
					Hash::type_hash([name]).into_inner(),
					format_args!("{name}"),
					nominal::Shape::Empty,
					hash.into_inner(),
				)
				.unwrap();
		}
		let mut remap = staged.prepare_remap().unwrap();
		staged.commit(&mut remap).unwrap();
		let authority = certificate(unsafe_hash);
		let mut store = Store::new(Limits {
			entries: 8,
			bytes: 4096,
		})
		.unwrap();
		let bad = store.list(Type::Struct(remap[0]), &[]).unwrap();
		let bad = store.export(bad);
		let safe = store.list(Type::Struct(remap[1]), &[]).unwrap();
		let safe = store.export(safe);
		store.validate_format_roots(&[&safe], &authority).unwrap();
		assert_eq!(
			store
				.validate_format_roots(&[&bad], &authority)
				.unwrap_err()
				.refusal,
			Some(crate::experimental_core::error::Refusal::Entry)
		);
		store.root_release(bad).unwrap();
		store.root_release(safe).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}

	#[test]
	fn bounded_shared_and_cyclic_graphs_preserve_inputs() {
		let hash = Hash::type_hash(["S7b2AdmissionMiri"]);
		let nominal = nominal::intern(hash.into_inner(), "S7b2AdmissionMiri").unwrap();
		let certificate = certificate(hash);
		let mut store = Store::new(Limits {
			entries: 32,
			bytes: 4096,
		})
		.unwrap();
		let bad = store.root_record(Some(nominal), &[]).unwrap();
		let cycle = store.root_list(Type::Vec, &[]).unwrap();
		store.root_push(&cycle, &cycle).unwrap();
		store.root_push(&cycle, &cycle).unwrap();
		store.root_push(&cycle, &bad).unwrap();
		let before = (store.live_entries(), store.payload_bytes());
		assert_eq!(
			store
				.validate_format_roots(&[&cycle], &certificate)
				.unwrap_err()
				.refusal,
			Some(crate::experimental_core::error::Refusal::Entry)
		);
		assert_eq!((store.live_entries(), store.payload_bytes()), before);
		assert_eq!(store.root_len(&cycle).unwrap(), 3);
		let _ = store.close();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}

	#[test]
	fn bounded_deep_and_shared_safe_graphs_complete_without_a_depth_limit() {
		let certificate = certificate(Hash::type_hash(["S7b2OtherMiri"]));
		let mut store = Store::new(Limits {
			entries: 32,
			bytes: 16384,
		})
		.unwrap();
		let mut current = store.root_string("leaf").unwrap();
		for _ in 0..8 {
			let parent = store.root_list(Type::Vec, &[&current]).unwrap();
			store.root_release(current).unwrap();
			current = parent;
		}
		let references = [&current; 64];
		let graph = store.root_list(Type::Tuple, &references).unwrap();
		let before = (store.live_entries(), store.payload_bytes());
		store
			.validate_format_roots(&[&graph, &graph], &certificate)
			.unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), before);
		store.root_release(graph).unwrap();
		store.root_release(current).unwrap();
		assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	}

	#[test]
	fn bounded_authority_snapshots_share_identity_until_native_mutation() {
		let mut context = crate::Context::new();
		let mut first = Module::with_crate("authority_miri_a").unwrap();
		first.host_function("a", || 1i64).unwrap();
		context.install(first).unwrap();
		let snapshot = context.format_snapshot().unwrap();
		let runtime = context.runtime().unwrap();
		assert!(crate::sync::Arc::ptr_eq(
			&snapshot,
			runtime.formatting.as_ref().unwrap()
		));
		let mut second = Module::with_crate("authority_miri_b").unwrap();
		second.host_function("b", || 2i64).unwrap();
		context.install(second).unwrap();
		let updated = context.format_snapshot().unwrap();
		assert!(!crate::sync::Arc::ptr_eq(&snapshot, &updated));
		let second_key = Hash::type_hash(crate::item!(::authority_miri_b::b));
		// associated_function is its own inverse when the protocol is fixed.
		let receiver = Hash::associated_function(second_key, &Protocol::DISPLAY_FMT);
		assert!(snapshot.permits(receiver));
		assert!(!updated.permits(receiver));
	}
}
