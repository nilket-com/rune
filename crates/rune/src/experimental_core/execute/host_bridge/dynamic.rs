//! Transactional dynamic host output import. No protocols or recursive data walk.
use super::*;
use crate::experimental_core::error::HostDynamicReason as Reason;
use crate::runtime::{Inline, Ref, Repr};
use core::mem::size_of;

type NativeResult = core::result::Result<stock::Value, stock::Value>;
fn failed(reason: Reason, type_hash: Option<crate::Hash>) -> Error {
	error(ErrorKind::Host).with_detail(Detail::HostDynamic { reason, type_hash })
}
fn overflow() -> Error {
	error(ErrorKind::Overflow)
}
fn add(a: usize, b: usize) -> Result<usize> {
	a.checked_add(b).ok_or_else(overflow)
}
fn bytes<T>(n: usize) -> Result<usize> {
	n.checked_mul(size_of::<T>()).ok_or_else(overflow)
}

// The Ref owns an immutable guard for the entire plan and construction. An
// address is only an equality/hash token; never dereferenced or reconstructed.
enum Source {
	String(Ref<crate::alloc::String>),
	Vec(Ref<stock::Vec>),
	Tuple(Ref<stock::OwnedTuple>),
	Object(Ref<stock::Object>),
	Option(Ref<Option<stock::Value>>),
	Result(Ref<NativeResult>),
}
impl Source {
	fn read(value: stock::Value) -> Result<Self> {
		macro_rules! read {
			($ty:ty, $variant:ident) => {
				let supported = match value.try_borrow_ref::<$ty>() {
					Ok(Some(_)) => true,
					Ok(None) => false,
					Err(_) => return Err(failed(Reason::OutputAccess, Some(value.type_hash()))),
				};
				if supported {
					let hash = value.type_hash();
					return value
						.into_ref::<$ty>()
						.map(Self::$variant)
						.map_err(|_| failed(Reason::OutputAccess, Some(hash)));
				}
			};
		}
		read!(crate::alloc::String, String);
		read!(stock::Vec, Vec);
		read!(stock::OwnedTuple, Tuple);
		read!(stock::Object, Object);
		read!(Option<stock::Value>, Option);
		read!(NativeResult, Result);
		Err(failed(Reason::Unsupported, Some(value.type_hash())))
	}
	fn identity(&self) -> (u8, usize) {
		match self {
			Self::String(v) => (0, (&**v as *const crate::alloc::String).addr()),
			Self::Vec(v) => (1, (&**v as *const stock::Vec).addr()),
			Self::Tuple(v) => (2, (&**v as *const stock::OwnedTuple).addr()),
			Self::Object(v) => (3, (&**v as *const stock::Object).addr()),
			Self::Option(v) => (4, (&**v as *const Option<stock::Value>).addr()),
			Self::Result(v) => (5, (&**v as *const NativeResult).addr()),
		}
	}
	fn len(&self) -> usize {
		match self {
			Self::String(_) => 0,
			Self::Vec(v) => v.len(),
			Self::Tuple(v) => v.len(),
			Self::Object(v) => v.len(),
			Self::Option(v) => usize::from(v.is_some()),
			Self::Result(_) => 1,
		}
	}
	fn payload(&self) -> Result<usize> {
		match self {
			Self::String(v) => Ok(v.len()),
			Self::Vec(v) => bytes::<Value>(v.len()),
			Self::Tuple(v) => bytes::<Value>(v.len()),
			Self::Object(v) => v.iter().try_fold(
				bytes::<(crate::alloc::String, Value)>(v.len())?,
				|n, (key, _)| add(n, key.len()),
			),
			Self::Option(_) | Self::Result(_) => Ok(size_of::<Value>()),
		}
	}
	fn append(&self, edges: &mut Vec<Link>) -> Result<()> {
		// Capacity was checked and reserved jointly before any clone. Value::clone
		// only retains its stock allocation; it never dispatches CLONE.
		match self {
			Self::String(_) => (),
			Self::Vec(v) => {
				for child in v.iter() {
					edges.try_push(Link::Pending(child.clone()))?;
				}
			}
			Self::Tuple(v) => {
				for child in v.iter() {
					edges.try_push(Link::Pending(child.clone()))?;
				}
			}
			Self::Object(v) => {
				for (_, child) in v.iter() {
					edges.try_push(Link::Pending(child.clone()))?;
				}
			}
			Self::Option(v) => {
				if let Some(child) = &**v {
					edges.try_push(Link::Pending(child.clone()))?;
				}
			}
			Self::Result(v) => edges.try_push(Link::Pending(match &**v {
				Ok(v) | Err(v) => v.clone(),
			}))?,
		}
		Ok(())
	}
}
enum Link {
	Pending(stock::Value),
	Leaf(Value),
	Node(usize),
}
struct Node {
	original: stock::Value,
	source: Source,
	start: usize,
	len: usize,
	active: bool,
	owned: Option<Value>,
}
#[derive(Clone, Copy)]
struct Memo {
	key: (u8, usize),
	id: usize,
}
const EMPTY: Memo = Memo {
	key: (0, 0),
	id: usize::MAX,
};
struct Budget {
	remaining: usize,
	payload: usize,
	scratch: usize,
}
impl Budget {
	fn check(&self, extra: usize) -> Result<()> {
		// Conversion scratch is ordinary temporary allocation, not Store payload.
		// Checked arithmetic and fallible reservations provide the resource bound.
		add(self.scratch, extra)?;
		if self.payload > self.remaining {
			return Err(error(ErrorKind::StoreCapacity));
		}
		Ok(())
	}

	fn reserve<T>(&mut self, vector: &mut Vec<T>, additional: usize) -> Result<()> {
		let needed = add(vector.len(), additional)?;
		if needed <= vector.capacity() {
			return Ok(());
		}
		let capacity = needed
			.max(vector.capacity().checked_mul(2).ok_or_else(overflow)?)
			.max(1);
		let old = bytes::<T>(vector.capacity())?;
		// Include old + new buffers during growth, then use actual capacities.
		self.check(bytes::<T>(capacity)?)?;
		vector.try_reserve_exact(capacity.checked_sub(vector.len()).ok_or_else(overflow)?)?;
		self.scratch = add(
			self.scratch.checked_sub(old).ok_or_else(overflow)?,
			bytes::<T>(vector.capacity())?,
		)?;
		self.check(0)
	}
}
struct Plan {
	nodes: Vec<Node>,
	edges: Vec<Link>,
	memo: Vec<Memo>,
	order: Vec<usize>,
	frames: Vec<(usize, usize)>,
	budget: Budget,
}
impl Plan {
	fn new(store: &Store) -> Result<Self> {
		Ok(Self {
			nodes: Vec::new(),
			edges: Vec::new(),
			memo: Vec::new(),
			order: Vec::new(),
			frames: Vec::new(),
			budget: Budget {
				remaining: store.host_remaining_bytes()?,
				payload: 0,
				scratch: 0,
			},
		})
	}
	fn bucket(key: (u8, usize), len: usize) -> usize {
		// Storage is aligned, so mix the address before masking. Hash iteration
		// never determines plan or observable construction order.
		let mut x = (key.1 as u64) ^ u64::from(key.0);
		x ^= x >> 30;
		x = x.wrapping_mul(0xbf58476d1ce4e5b9);
		x ^= x >> 27;
		x = x.wrapping_mul(0x94d049bb133111eb);
		(x ^ (x >> 31)) as usize & (len - 1)
	}
	fn slot(memo: &[Memo], key: (u8, usize)) -> usize {
		let mut at = Self::bucket(key, memo.len());
		while memo[at].id != usize::MAX && memo[at].key != key {
			at = (at + 1) & (memo.len() - 1);
		}
		at
	}
	fn intern(&mut self, source: Source, original: stock::Value, store: &Store) -> Result<Link> {
		let key = source.identity();
		if !self.memo.is_empty() {
			let prior = self.memo[Self::slot(&self.memo, key)].id;
			if prior != usize::MAX {
				let node = &self.nodes[prior];
				if node.active {
					return Err(failed(Reason::Cycle, None));
				}
				return Ok(Link::Node(prior));
			}
		}
		let payload = add(self.budget.payload, source.payload()?)?;
		store.host_room(payload, add(self.nodes.len(), 1)?)?;
		self.budget.payload = payload;
		self.budget.check(0)?;
		if self.memo.is_empty() || (self.nodes.len() + 1) * 2 > self.memo.len() {
			let size = self.memo.len().max(8).checked_mul(2).ok_or_else(overflow)?;
			self.budget.check(bytes::<Memo>(size)?)?;
			let mut memo = Vec::try_with_capacity(size)?;
			self.budget.check(bytes::<Memo>(memo.capacity())?)?;
			memo.try_resize_with(size, || EMPTY)?;
			for entry in &self.memo {
				if entry.id != usize::MAX {
					let at = Self::slot(&memo, entry.key);
					memo[at] = *entry;
				}
			}
			self.budget.scratch = add(
				self.budget
					.scratch
					.checked_sub(bytes::<Memo>(self.memo.capacity())?)
					.ok_or_else(overflow)?,
				bytes::<Memo>(memo.capacity())?,
			)?;
			self.memo = memo;
		}
		let len = source.len();
		self.budget.reserve(&mut self.nodes, 1)?;
		self.budget.reserve(&mut self.edges, len)?;
		let start = self.edges.len();
		source.append(&mut self.edges)?;
		let id = self.nodes.len();
		self.nodes.try_push(Node {
			original,
			source,
			start,
			len,
			active: true,
			owned: None,
		})?;
		let at = Self::slot(&self.memo, key);
		self.memo[at] = Memo { key, id };
		Ok(Link::Node(id))
	}
	fn enter(&mut self, value: stock::Value, store: &Store) -> Result<Link> {
		if let Repr::Inline(value) = value.as_ref() {
			return Ok(Link::Leaf(match value {
				Inline::Unit => Value::Unit,
				Inline::Bool(v) => Value::Bool(*v),
				Inline::Signed(v) => Value::I64(*v),
				Inline::Unsigned(v) => Value::U64(*v),
				Inline::Float(v) => Value::F64(*v),
				Inline::Char(v) => Value::Char(*v),
				Inline::Ordering(v) => Value::Ordering(*v),
				Inline::Empty | Inline::Type(_) | Inline::Hash(_) => {
					return Err(failed(Reason::Unsupported, None))
				}
			}));
		}
		self.intern(Source::read(value.clone())?, value, store)
	}
	fn walk(&mut self, value: &stock::Value, store: &Store) -> Result<Link> {
		let root = self.enter(value.clone(), store)?;
		if let Link::Node(id) = root {
			self.budget.reserve(&mut self.frames, 1)?;
			self.frames.try_push((id, 0))?;
		}
		while let Some(&(id, next)) = self.frames.last() {
			if next < self.nodes[id].len {
				self.frames.last_mut().unwrap().1 += 1;
				let at = self.nodes[id].start + next;
				let Link::Pending(child) =
					core::mem::replace(&mut self.edges[at], Link::Leaf(Value::Unit))
				else {
					return Err(error(ErrorKind::InvalidCode));
				};
				let link = self.enter(child, store)?;
				let child = match link {
					Link::Node(id) if self.nodes[id].active => Some(id),
					_ => None,
				};
				self.edges[at] = link;
				if let Some(child) = child {
					self.budget.reserve(&mut self.frames, 1)?;
					self.frames.try_push((child, 0))?;
				}
			} else {
				self.nodes[id].active = false;
				self.budget.reserve(&mut self.order, 1)?;
				self.order.try_push(id)?;
				self.frames.pop();
			}
		}
		// At this point each graph allocation has one original stock edge per
		// incoming link, one retained Source guard, and one audit handle. Any
		// excess owner is an external alias or a live guard, violating enrollment.
		let mut incoming = Vec::try_with_capacity(self.nodes.len())?;
		incoming.try_resize_with(self.nodes.len(), || 0usize)?;
		for link in self.edges.iter().chain(core::iter::once(&root)) {
			if let Link::Node(id) = link {
				incoming[*id] = add(incoming[*id], 1)?;
			}
		}
		for (node, incoming) in self.nodes.iter().zip(incoming) {
			let count = match node.original.as_ref() {
				Repr::Any(any) => any.owned_ref_count(),
				_ => None,
			};
			if count != Some(add(incoming, 2)?) {
				return Err(failed(
					Reason::ExternalAlias,
					Some(node.original.type_hash()),
				));
			}
		}
		Ok(root)
	}
	fn value(&self, link: &Link) -> Result<Value> {
		match link {
			Link::Leaf(value) => Ok(*value),
			Link::Node(id) => self.nodes[*id]
				.owned
				.ok_or_else(|| error(ErrorKind::InvalidCode)),
			Link::Pending(_) => Err(error(ErrorKind::InvalidCode)),
		}
	}
	fn build(&mut self, store: &mut Store) -> Result<()> {
		store.host_room(self.budget.payload, self.nodes.len())?;
		// Preflight the maximum temporary capacity jointly before ANY fresh entry.
		// Keys are borrowed from guarded objects; their final copies are payload.
		let mut temporary = 0;
		for node in &self.nodes {
			let n = match &node.source {
				Source::Vec(_) | Source::Tuple(_) => bytes::<Value>(node.len)?,
				Source::Object(_) => bytes::<(&str, Value)>(node.len)?,
				_ => 0,
			};
			temporary = temporary.max(n);
		}
		self.budget.check(temporary)?;
		#[cfg(test)]
		fault_tests::observe("dynamic-build");
		for id in self.order.iter().copied() {
			let node = &self.nodes[id];
			let links = &self.edges[node.start..node.start + node.len];
			#[cfg(test)]
			fault_tests::observe("dynamic-node");
			let planned_bytes = node.source.payload()?;
			let before_bytes = store.payload_bytes();
			let value = match &node.source {
				Source::String(text) => store.string(text)?,
				Source::Vec(_) | Source::Tuple(_) => {
					let mut values = Vec::try_with_capacity(node.len)?;
					self.budget.check(bytes::<Value>(values.capacity())?)?;
					for link in links {
						values.try_push(self.value(link)?)?;
					}
					store.list(
						if matches!(&node.source, Source::Vec(_)) {
							Type::Vec
						} else {
							Type::Tuple
						},
						&values,
					)?
				}
				Source::Object(object) => {
					let mut fields = Vec::try_with_capacity(node.len)?;
					self.budget
						.check(bytes::<(&str, Value)>(fields.capacity())?)?;
					for ((key, _), link) in object.iter().zip(links) {
						fields.try_push((key.as_str(), self.value(link)?))?;
					}
					store.record(Type::Object, &fields)?
				}
				Source::Option(option) => store.variant(
					Type::Option,
					option.is_some(),
					links.first().map(|link| self.value(link)).transpose()?,
				)?,
				Source::Result(result) => {
					store.variant(Type::Result, result.is_ok(), Some(self.value(&links[0])?))?
				}
			};
			self.nodes[id].owned = Some(value);
			// Actual payload capacities are charged by Store::admit. Preserve
			// the joint bound even if an allocator reserves excess capacity.
			let actual_bytes = store
				.payload_bytes()
				.checked_sub(before_bytes)
				.ok_or_else(overflow)?;
			self.budget.payload = add(
				self.budget
					.payload
					.checked_sub(planned_bytes)
					.ok_or_else(overflow)?,
				actual_bytes,
			)?;
			self.budget.check(temporary)?;
		}
		Ok(())
	}
	fn release(&mut self, store: &mut Store) -> Option<Error> {
		let mut first = None;
		// Every node owns exactly one local reference. Parent edges are separate
		// retained owners. Release each local once, including shared children.
		for node in &mut self.nodes {
			if let Some(value) = node.owned.take() {
				if let Err(error) = store.release(value) {
					#[cfg(test)]
					fault_tests::observe("cleanup-error");
					first.get_or_insert(error);
				}
			}
		}
		first
	}
}

#[cold]
#[inline(never)]
pub(super) fn convert(
	executor: &mut Executor,
	store: &mut Store,
	value: &stock::Value,
) -> Result<Value> {
	let mut plan = Plan::new(store)?;
	#[cfg(test)]
	fault_tests::observe("dynamic-plan");
	let root = plan.walk(value, store)?;
	if let Err(primary) = plan.build(store) {
		executor.pending_host_cleanup = plan.release(store);
		return Err(primary);
	}
	let value = match plan.value(&root) {
		Ok(value) => value,
		Err(primary) => {
			executor.pending_host_cleanup = plan.release(store);
			return Err(primary);
		}
	};
	if let Link::Node(id) = root {
		plan.nodes[id].owned = None;
	}
	if let Some(primary) = plan.release(store) {
		executor.pending_host_cleanup = store.release(value).err();
		return Err(primary);
	}
	Ok(value)
}
