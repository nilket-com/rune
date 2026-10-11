//! Sealed-boundary obligations that need crate-private access: generation
//! retirement near `u32::MAX`, and the owned-root disposition when a release is
//! refused before versus after the root's edge is released.

use crate::experimental_core::{
	error::ErrorKind,
	heap::{Limits, Store},
	value::{ReleaseError, Type},
};

fn store(entries: usize) -> Store {
	Store::new(Limits {
		entries,
		bytes: 4096,
	})
	.unwrap()
}

#[test]
fn a_slot_whose_generation_is_exhausted_retires_permanently() {
	let mut s = store(2);
	let first = s.string("first").unwrap();

	// The next release moves the generation to u32::MAX and the slot stays reusable.
	let first = s.set_generation_for_test(first, u32::MAX - 1);
	let before = s.free_slots_for_test();
	s.release(first).unwrap();
	assert_eq!(s.free_slots_for_test(), before + 1);
	assert_eq!(s.live_entries(), 0);

	// Reuse the slot at generation u32::MAX; releasing it then exhausts the
	// generation: the slot is retired for good, accounting stays exact, and the
	// store keeps working on its remaining slot.
	let second = s.string("second").unwrap();
	assert_eq!(s.text(second).unwrap(), "second");
	assert_eq!(s.live_entries(), 1);
	s.release(second).unwrap();
	assert_eq!(
		s.free_slots_for_test(),
		before,
		"the exhausted slot is not offered again"
	);
	assert_eq!(s.live_entries(), 0);
	assert_eq!(s.payload_bytes(), 0);
	assert_eq!(s.text(second).unwrap_err().kind, ErrorKind::InvalidHandle);
	let third = s.string("third").unwrap();
	assert_eq!(s.live_entries(), 1);

	// Only one slot remains usable; the second admission is a capacity refusal.
	assert_eq!(
		s.string("fourth").unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	s.release(third).unwrap();
	assert_eq!(s.live_entries(), 0);
}

#[test]
fn a_last_release_refused_by_its_retirement_checks_returns_the_owner() {
	let mut s = store(4);
	let one = s.root_i64(1);
	let list = s.root_list(Type::Vec, &[&one]).unwrap();

	// Deliberate internal misuse: take a reader lease, then drop the lease's own
	// retained reference so the host root is the last owner while a reader is
	// still registered. Its release is then an AccessConflict, checked before
	// the reference changes: the edge is intact and the root comes back.
	let raw = s.import(&list).unwrap();
	let iter = s.begin_iter(raw).unwrap();
	s.release(raw).unwrap();
	let list = match s.root_release(list) {
		Err(ReleaseError::Refused { root, error }) => {
			assert_eq!(error.kind, ErrorKind::AccessConflict);
			root
		}
		other => panic!("expected refusal, got {other:?}"),
	};

	assert_eq!(s.live_entries(), 1);
	assert_eq!(
		s.root_len(&list).unwrap(),
		1,
		"the owner still names a live entry"
	);

	// Ending the lease releases the last reference; the returned root is then
	// stale and is refused unchanged.
	s.end_iter(iter).unwrap();
	assert_eq!(s.live_entries(), 0);

	match s.root_release(list) {
		Err(ReleaseError::Refused { error, .. }) => {
			assert_eq!(error.kind, ErrorKind::InvalidHandle)
		}
		other => panic!("expected refusal, got {other:?}"),
	}
}

#[test]
fn a_release_failing_after_the_edge_is_consumed_reports_failed() {
	let mut s = store(8);

	// parent -> child, with the child's edge malformed from the parent's point of
	// view: releasing the parent consumes the parent's edge, then the child's
	// drainage reports the first error. B5 drains everything it can.
	let child = s.string("child").unwrap();
	let parent = s.list(Type::Vec, &[child]).unwrap();
	s.release(child).unwrap();

	// Deliberate raw misuse: over-release the child so its count underflows
	// during drainage.
	s.release(child).unwrap();
	let parent_root = s.export(parent);

	match s.root_release(parent_root) {
		Err(ReleaseError::Failed(error)) => assert_eq!(error.kind, ErrorKind::InvalidHandle),
		other => panic!("expected a post-edge failure, got {other:?}"),
	}

	assert_eq!(
		s.live_entries(),
		0,
		"the parent was retired; nothing is left"
	);
}

#[test]
fn a_post_retirement_enqueue_failure_drains_and_consumes_the_owner() {
	let mut s = store(8);
	let one = s.root_i64(1);
	let source = s.root_list(Type::Vec, &[&one]).unwrap();
	let raw_source = s.import(&source).unwrap();
	let iterator = s.iterator(raw_source).unwrap();

	// Deliberate internal misuse: the iterator's lease no longer counts as a
	// reader, and the caller's own source root goes away, so the iterator holds
	// the last strong edge to the vector.
	s.zero_readers_for_test(raw_source);
	s.root_release(source).unwrap();
	assert_eq!(s.live_entries(), 2);
	let baseline = (0, 0);

	// Releasing the iterator retires it, then its lease release fails
	// (readers underflow) AFTER the source edge was queued: the first error is
	// kept, the queue drains, the owner is consumed.
	let iterator = s.export(iterator);

	match s.root_release(iterator) {
		Err(ReleaseError::Failed(error)) => assert_eq!(error.kind, ErrorKind::InvalidHandle),
		other => panic!("expected Failed, got {other:?}"),
	}

	assert_eq!(s.queued_work_for_test(), 0);
	assert_eq!((s.live_entries(), s.payload_bytes()), baseline);
	let again = s.root_string("reuse").unwrap();
	assert_eq!(s.root_text(&again).unwrap(), "reuse");
	s.root_release(again).unwrap();
	assert_eq!(s.live_entries(), 0);
}

// ---- Moved from the D21 dev crate (`experimental/0184-dev/tests/boundary.rs`): the three
// tests that need crate-private types (`Value`, `Handle`, `Instruction`, `Builder`,
// `nominal::intern`), plus the program-identity assertions of the external test.
mod moved {
	use crate::experimental_core::{
		code::{Builder, Function, Instruction, Op},
		error::{ErrorKind, Location},
		execute::Executor,
		frontend::{self, Program},
		heap::{Limits, Store},
		nominal,
		value::{Handle, Nominal, Root, Scalar, Type, Value},
	};
	use crate::{Context, Diagnostics, Hash, Source, Sources};

	fn compile(text: &str) -> Program {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap()
	}

	fn new_store() -> Store {
		Store::new(Limits {
			entries: 16,
			bytes: 4096,
		})
		.unwrap()
	}

	fn main_of(program: &Program) -> u32 {
		program.function(Hash::type_hash(["main"])).unwrap()
	}

	#[test]
	fn layout_is_compact() {
		assert_eq!(core::mem::size_of::<Value>(), 16);
		assert_eq!(core::mem::align_of::<Value>(), 8);
		assert_eq!(core::mem::size_of::<Handle>(), 8);
		assert_eq!(core::mem::size_of::<Type>(), 8);
		assert_eq!(core::mem::size_of::<Nominal>(), 4);
		assert_eq!(core::mem::size_of::<Scalar>(), 16);
		assert_eq!(core::mem::size_of::<Root>(), 24);
		assert_eq!(core::mem::align_of::<Root>(), 8);
		assert_eq!(core::mem::size_of::<Option<Value>>(), 16);
		assert_eq!(core::mem::size_of::<Instruction>(), 16);
		assert_eq!(core::mem::align_of::<Instruction>(), 4);
		assert_eq!(core::mem::align_of::<Handle>(), 4);
		assert_eq!(core::mem::align_of::<Type>(), 4);
	}

	#[test]
	fn a_foreign_function_literal_is_rejected_at_finish() {
		let loc = Location {
			source: 1,
			revision: 1,
			start: 0,
			end: 1,
		};

		for code in [1, 7, u32::MAX] {
			let mut f = Function::new(1, 0).unwrap();
			f.push(
				Op::Literal {
					out: 0,
					value: Value::Function { code, id: 0 },
				},
				loc,
			)
			.unwrap();
			f.push(Op::Return { value: 0 }, loc).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			assert_eq!(
				b.finish().err().map(|e| e.kind),
				Some(ErrorKind::InvalidCode)
			);
		}

		// The unstamped form is accepted and stamped with the program's identity.
		let mut f = Function::new(1, 0).unwrap();
		f.push(
			Op::Literal {
				out: 0,
				value: Value::Function { code: 0, id: 0 },
			},
			loc,
		)
		.unwrap();
		f.push(Op::Return { value: 0 }, loc).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		let code = b.finish().unwrap();
		let mut store = new_store();
		let out = Executor::new()
			.unwrap()
			.invoke_code(&code, &mut store, 0, &[], 100)
			.unwrap();
		assert!(store.root_is_function(&out).unwrap());
		store.root_release(out).unwrap();
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
	)]
	fn struct_types_are_interned_once_per_hash() {
		let program =
			compile("struct Point { x } pub fn main() { let p = Point { x: 1 }; p is Point }");
		let mut store = new_store();
		let out = Executor::new()
			.unwrap()
			.run(&program, &mut store, main_of(&program), &[], 1000)
			.unwrap();
		assert_eq!(store.root_scalar(&out).unwrap(), Some(Scalar::Bool(true)));
		store.root_release(out).unwrap();

		// The same hash from a second program is the same nominal (process-wide).
		let hash = Hash::type_hash(["Point"]).into_inner();
		let n1 = nominal::intern(hash, "Point").unwrap();
		let n2 = nominal::intern(hash, "other name, same hash").unwrap();
		assert_eq!(n1, n2);
		assert!(nominal::contains(n1));

		// A host record carries the interned identity.
		let one = store.root_i64(1);
		let rec = store.root_record(Some(n1), &[("x", &one)]).unwrap();
		assert_eq!(store.root_tag(&rec).unwrap(), Type::Struct(n1));
		store.root_release(rec).unwrap();
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
	)]
	fn compiled_programs_have_distinct_nonzero_identities() {
		// Moved from the external `function_tokens_carry_their_program_identity`, which can
		// no longer reach the crate-private `Program::code`.
		let producer = compile("pub fn f() { 41 } pub fn main() { f }");
		let consumer = compile("pub fn main(g) { g() + 1 }");
		assert_ne!(producer.code.id(), consumer.code.id());
		assert_ne!(producer.code.id(), 0);
	}
}
