#![cfg(feature = "fresh-core")]

//! The sealed host boundary through the public API only: owner checks at every
//! root operand position, root ownership semantics and function-token provenance.
//! Moved from the D21 dev crate (`experimental/0184-dev/tests/boundary.rs`); the
//! layout, hand-built-code and interner tests are in-crate (`tests/boundary_internal.rs`).

use rune::experimental_core::{
	error::ErrorKind,
	execute::Executor,
	frontend::{self, Program},
	heap::{Limits, Store},
	value::{ReleaseError, Root, Scalar, Type},
};
use rune::{Context, Diagnostics, Hash, Source, Sources};

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

/// Two stores whose first entries share slot and generation: a list `[1]` in
/// each. Inside the core the handles are identical bits.
fn pair() -> (Store, Store, Root, Root) {
	let mut a = new_store();
	let mut b = new_store();
	let one = a.root_i64(1);
	let list_a = a.root_list(Type::Vec, &[&one]).unwrap();
	let one = b.root_i64(1);
	let list_b = b.root_list(Type::Vec, &[&one]).unwrap();
	(a, b, list_a, list_b)
}

fn counters(store: &Store) -> (usize, usize) {
	(store.live_entries(), store.payload_bytes())
}

fn i64_of(store: &Store, root: &Root) -> i64 {
	match store.root_scalar(root).unwrap() {
		Some(Scalar::I64(n)) => n,
		other => panic!("not an i64: {other:?}"),
	}
}

#[test]
fn over_cap_host_requests_are_refused_before_any_scratch_or_retain() {
	// A store with room for exactly one more 16-byte element in bytes and one
	// more entry.
	let mut a = Store::new(Limits {
		entries: 2,
		bytes: 64,
	})
	.unwrap();
	let one = a.root_i64(1);
	let keep = a.root_list(Type::Vec, &[&one, &one, &one]).unwrap();
	assert_eq!(a.payload_bytes(), 48);
	let before = counters(&a);
	let b = new_store();
	let foreign = b.root_i64(2);

	// Bytes refused first (a 2-element list needs 32 > 16 remaining); nothing retained,
	// including when a foreign root sits in the request.
	assert_eq!(
		a.root_list(Type::Vec, &[&one, &foreign]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		a.root_list(Type::Vec, &[&keep, &keep]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);
	assert_eq!(
		a.root_record(None, &[("k", &keep)]).unwrap_err().kind,
		ErrorKind::StoreCapacity
	);

	// Shape refusals precede everything.
	assert_eq!(
		a.root_list(Type::String, &[]).unwrap_err().kind,
		ErrorKind::Type
	);
	assert_eq!(
		a.root_record(None, &[("k", &one), ("k", &one)])
			.unwrap_err()
			.kind,
		ErrorKind::InvalidCode
	);
	assert_eq!(counters(&a), before);
	assert_eq!(b.live_entries(), 0);

	// A one-element list fits exactly.
	let last = a.root_list(Type::Vec, &[&keep]).unwrap();
	assert_eq!(a.payload_bytes(), 64);
	assert_eq!(
		a.root_list(Type::Vec, &[]).unwrap_err().kind,
		ErrorKind::StoreCapacity,
		"entry cap"
	);
	a.root_release(last).unwrap();
	a.root_release(keep).unwrap();
	assert_eq!(a.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn invoke_refuses_a_missing_function_and_a_wrong_arity_before_touching_roots() {
	let program = compile("pub fn main(a) { a }");
	let main = main_of(&program);
	let (mut a, mut b, la, lb) = pair();
	let mut executor = Executor::new().unwrap();

	// Wrong arity with a foreign root: the arity refusal comes first.
	let failure = executor
		.invoke(&program, &mut a, main, &[&la, &lb], 10)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	let failure = executor
		.invoke(&program, &mut a, 99, &[&lb], 10)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::InvalidCode);
	assert!(executor.is_idle());
	assert_eq!(a.live_entries(), 1);
	assert_eq!(b.live_entries(), 1);
	a.root_release(la).unwrap();
	b.root_release(lb).unwrap();
}

#[test]
fn every_root_operand_position_is_owner_checked() {
	let (mut a, mut b, la, lb) = pair();
	let before = (counters(&a), counters(&b));
	let foreign = |r: Result<(), ErrorKind>| assert_eq!(r, Err(ErrorKind::InvalidHandle));
	let kind = |e: rune::experimental_core::error::Error| e.kind;

	// Scalars are branded too: a foreign scalar root is refused, never read.
	let one_b = b.root_i64(1);
	foreign(a.root_scalar(&one_b).map(drop).map_err(kind));
	foreign(a.root_tag(&one_b).map(drop).map_err(kind));
	foreign(a.root_is_function(&one_b).map(drop).map_err(kind));

	// Construction operands.
	foreign(a.root_list(Type::Vec, &[&lb]).map(drop).map_err(kind));
	foreign(a.root_list(Type::Vec, &[&la, &lb]).map(drop).map_err(kind));
	foreign(a.root_record(None, &[("x", &lb)]).map(drop).map_err(kind));
	foreign(
		a.root_variant(Type::Option, true, Some(&lb))
			.map(drop)
			.map_err(kind),
	);

	// Reads.
	foreign(a.root_get(&lb, 0).map(drop).map_err(kind));
	foreign(a.root_len(&lb).map(drop).map_err(kind));
	foreign(a.root_text(&lb).map(drop).map_err(kind));
	foreign(a.root_tag(&lb).map(drop).map_err(kind));
	foreign(a.root_field(&lb, "x").map(drop).map_err(kind));
	foreign(a.root_format(&lb, true).map(drop).map_err(kind));
	foreign(a.root_variant_parts(&lb).map(drop).map_err(kind));

	// Mutation: receiver and value positions.
	let one_a = a.root_i64(1);
	foreign(a.root_set(&lb, 0, &one_a).map_err(kind));
	foreign(a.root_set(&la, 0, &one_b).map_err(kind));
	foreign(a.root_push(&lb, &one_a).map_err(kind));
	foreign(a.root_push(&la, &one_b).map_err(kind));
	let rec = a.root_record(None, &[("x", &one_a)]).unwrap();
	let rec_b = b.root_record(None, &[("x", &one_b)]).unwrap();
	foreign(a.root_set_field(&rec_b, "x", &one_a).map_err(kind));
	foreign(a.root_set_field(&rec, "x", &one_b).map_err(kind));

	// Ownership operations.
	foreign(a.root_retain(&lb).map(drop).map_err(kind));
	foreign(a.root_invalidate(&lb).map_err(kind));

	// Release: refused, and the unchanged root comes back.
	let lb = match a.root_release(lb) {
		Err(ReleaseError::Refused { root, error }) => {
			assert_eq!(error.kind, ErrorKind::InvalidHandle);
			root
		}
		other => panic!("expected refusal, got {other:?}"),
	};

	a.root_release(rec).unwrap();
	b.root_release(rec_b).unwrap();
	assert_eq!(
		(counters(&a), counters(&b)),
		before,
		"nothing changed in either store"
	);

	// Still usable in their own stores afterwards.
	assert_eq!(a.root_len(&la).unwrap(), 1);
	assert_eq!(b.root_len(&lb).unwrap(), 1);
	a.root_release(la).unwrap();
	b.root_release(lb).unwrap();
	assert_eq!(a.live_entries(), 0);
	assert_eq!(b.live_entries(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn invoke_checks_every_argument_before_retaining_any() {
	let program = compile("pub fn main(a, b) { a.len() + b.len() }");
	let main = main_of(&program);
	let (mut a, mut b, la, lb) = pair();
	let mut executor = Executor::new().unwrap();

	for args in [[&la, &lb], [&lb, &la], [&lb, &lb]] {
		let failure = executor
			.invoke(&program, &mut a, main, &args, 10_000)
			.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::InvalidHandle);
		assert!(failure.call_chain.is_empty());
		assert!(executor.is_idle());
		assert_eq!(a.live_entries(), 1);
		assert_eq!(b.live_entries(), 1);
	}

	let out = executor
		.invoke(&program, &mut a, main, &[&la, &la], 10_000)
		.unwrap();
	assert_eq!(a.root_scalar(&out).unwrap(), Some(Scalar::U64(2)));

	// The result is branded by the store that ran: foreign to the other.
	assert_eq!(
		b.root_scalar(&out).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	a.root_release(out).unwrap();
	a.root_release(la).unwrap();
	b.root_release(lb).unwrap();
}

#[test]
fn closed_and_dropped_stores_refuse_their_roots() {
	let mut a = new_store();
	let text = a.root_string("x").unwrap();
	a.close().unwrap();
	assert_eq!(a.root_len(&text).unwrap_err().kind, ErrorKind::ClosedStore);
	let text = match a.root_release(text) {
		Err(ReleaseError::Refused { root, error }) => {
			assert_eq!(error.kind, ErrorKind::ClosedStore);
			root
		}
		other => panic!("expected refusal, got {other:?}"),
	};

	drop(a);

	// Store identities are never reused: the stale root is foreign everywhere.
	let b = new_store();
	assert_eq!(
		b.root_len(&text).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(
		b.root_scalar(&text).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
}

#[test]
fn two_owners_release_once_each_in_either_order() {
	for second_first in [false, true] {
		let mut store = new_store();
		let first = store.root_string("shared").unwrap();
		let second = store.root_retain(&first).unwrap();
		assert_eq!(store.live_entries(), 1);
		let (x, y) = if second_first {
			(second, first)
		} else {
			(first, second)
		};

		store.root_release(x).unwrap();
		assert_eq!(store.live_entries(), 1, "one owner keeps the entry");
		assert_eq!(store.root_text(&y).unwrap(), "shared");
		store.root_release(y).unwrap();
		assert_eq!(store.live_entries(), 0);
	}
}

#[test]
fn retained_reads_and_container_edges_are_counted() {
	let mut store = new_store();
	let inner = store.root_string("in").unwrap();
	let list = store.root_list(Type::Vec, &[&inner]).unwrap();
	let rec = store.root_record(None, &[("k", &inner)]).unwrap();
	assert_eq!(store.live_entries(), 3);
	let got = store.root_get(&list, 0).unwrap();
	let field = store.root_field(&rec, "k").unwrap();
	assert_eq!(store.root_text(&got).unwrap(), "in");
	assert_eq!(store.root_text(&field).unwrap(), "in");

	// The host's own root goes away; the containers and the retained reads keep the string.
	store.root_release(inner).unwrap();
	assert_eq!(store.live_entries(), 3);
	store.root_release(list).unwrap();
	store.root_release(rec).unwrap();
	assert_eq!(store.live_entries(), 1);
	store.root_release(got).unwrap();
	assert_eq!(store.live_entries(), 1);
	store.root_release(field).unwrap();
	assert_eq!(store.live_entries(), 0);
}

#[test]
fn mutation_operands_are_retained_by_the_container() {
	let mut store = new_store();
	let zero = store.root_i64(0);
	let list = store.root_list(Type::Vec, &[&zero]).unwrap();
	let text = store.root_string("t").unwrap();
	store.root_set(&list, 0, &text).unwrap();
	store.root_push(&list, &text).unwrap();
	let rec = store.root_record(None, &[("k", &zero)]).unwrap();
	store.root_set_field(&rec, "k", &text).unwrap();
	assert_eq!(store.root_len(&list).unwrap(), 2);
	store.root_release(text).unwrap();
	assert_eq!(
		store.live_entries(),
		3,
		"string kept by three container edges"
	);
	assert_eq!(store.root_format(&list, true).unwrap(), "[\"t\", \"t\"]");
	store.root_release(list).unwrap();
	store.root_release(rec).unwrap();
	assert_eq!(store.live_entries(), 0);

	// Scalar roots release nothing and are consumed.
	store.root_release(zero).unwrap();
}

#[test]
fn scalar_roots_read_back_and_heap_roots_do_not() {
	let mut store = new_store();
	assert_eq!(
		store.root_scalar(&store.root_unit()).unwrap(),
		Some(Scalar::Unit)
	);
	assert_eq!(
		store.root_scalar(&store.root_bool(true)).unwrap(),
		Some(Scalar::Bool(true))
	);
	assert_eq!(
		store.root_scalar(&store.root_u64(7)).unwrap(),
		Some(Scalar::U64(7))
	);
	assert_eq!(
		store.root_scalar(&store.root_f64(0.5)).unwrap(),
		Some(Scalar::F64(0.5))
	);
	assert_eq!(
		store.root_scalar(&store.root_char('c')).unwrap(),
		Some(Scalar::Char('c'))
	);
	assert_eq!(store.root_tag(&store.root_char('c')).unwrap(), Type::Char);
	let text = store.root_string("s").unwrap();
	assert_eq!(store.root_scalar(&text).unwrap(), None);
	assert_eq!(store.root_tag(&text).unwrap(), Type::String);
	assert!(!store.root_is_function(&text).unwrap());
	store.root_release(text).unwrap();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn function_tokens_carry_their_program_identity() {
	let producer = compile("pub fn f() { 41 } pub fn main() { f }");
	let consumer = compile("pub fn main(g) { g() + 1 }");

	// M1: the `code.id()` assertions moved in-crate
	// (`boundary_internal::compiled_programs_have_distinct_nonzero_identities`).
	let mut store = new_store();
	let mut executor = Executor::new().unwrap();
	let token = executor
		.invoke(&producer, &mut store, main_of(&producer), &[], 1000)
		.unwrap();
	assert!(store.root_is_function(&token).unwrap());
	assert_eq!(store.root_scalar(&token).unwrap(), None);

	// The token is foreign to the consumer program: a type error at the call,
	// with nothing retained and the executor idle.
	let failure = executor
		.invoke(&consumer, &mut store, main_of(&consumer), &[&token], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	assert!(failure.error.location.is_some(), "located at the call");
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);

	// Its own program calls it.
	let caller = producer.function(Hash::type_hash(["f"])).unwrap();
	let out = executor
		.invoke(&producer, &mut store, caller, &[], 1000)
		.unwrap();
	assert_eq!(i64_of(&store, &out), 41);
	store.root_release(out).unwrap();

	// Dropping the producer never frees its identity for a later program.
	drop(producer);
	let later = compile("pub fn main(g) { g() }");
	let failure = executor
		.invoke(&later, &mut store, main_of(&later), &[&token], 1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Type);
	store.root_release(token).unwrap();
}
