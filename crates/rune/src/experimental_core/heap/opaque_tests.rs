//! Publication, ownership, allocation and generation controls for cold metadata.
use super::*;
use crate::experimental_core::error::{Detail, TypeName};

#[test]
fn opaque_move_reservation_fails_before_payload_or_lease_changes() {
	let mut s = Store::new(Limits::default()).unwrap();
	let text = s.string("abc").unwrap();
	let it = s.pipeline_chars(text).unwrap();
	let before = (
		s.live_entries(),
		s.payload_bytes(),
		s.bookkeeping_bytes().unwrap(),
	);
	let error = rune_alloc::limit::with(0, || s.invalidate(it).unwrap_err()).call();
	assert_eq!(error.kind, ErrorKind::Allocation);
	assert_eq!(
		(
			s.live_entries(),
			s.payload_bytes(),
			s.bookkeeping_bytes().unwrap()
		),
		before
	);
	assert_eq!(s.diagnostic_type(it), Some(TypeName::Chars));
	assert!(s.opaque_names.is_empty());
	assert_eq!(
		s.invalidate(text).unwrap_err().kind,
		ErrorKind::AccessConflict
	);
	s.invalidate(it).unwrap();
	assert_eq!(s.diagnostic_type(it), Some(TypeName::Chars));
	s.invalidate(text).unwrap();
	s.release(text).unwrap();
	s.release(it).unwrap();
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	s.close().unwrap();
	assert!(s.opaque_names.is_empty());
}

#[test]
fn stale_opaque_metadata_cannot_name_a_released_or_reused_slot() {
	let mut s = Store::new(Limits::default()).unwrap();
	let object = s.record(Type::Object, &[]).unwrap();
	let it = s.iterator(object).unwrap();
	let Value::Heap(old) = it else {
		panic!("iterator handle")
	};
	s.invalidate(it).unwrap();
	s.release(object).unwrap();
	assert_eq!(s.diagnostic_type(it), Some(TypeName::ObjectIter));
	s.release(it).unwrap();
	assert_eq!(s.moved_iterator_name(old), None);
	let fresh = s.range_iterator(Value::I64(0), Value::I64(2)).unwrap();
	let Value::Heap(new) = fresh else {
		panic!("iterator handle")
	};
	assert_eq!(old.slot, new.slot);
	assert_ne!(old.generation, new.generation);
	assert_eq!(s.diagnostic_type(fresh), Some(TypeName::RangeIter));
	s.invalidate(fresh).unwrap();
	assert_eq!(s.diagnostic_type(fresh), Some(TypeName::RangeIter));
	assert_eq!(s.moved_iterator_name(old), None);
	s.release(fresh).unwrap();
	for _ in 0..128 {
		let it = s.range_iterator(Value::I64(0), Value::I64(1)).unwrap();
		s.invalidate(it).unwrap();
		s.release(it).unwrap();
		assert!(s.opaque_names.len() <= 8);
	}
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
	s.close().unwrap();
	assert!(s.opaque_names.is_empty());
}

#[test]
fn formatting_constructors_close_invalid_list_and_variant_tags() {
	let mut s = Store::new(Limits::default()).unwrap();
	for tag in [
		Type::Bool,
		Type::String,
		Type::Object,
		Type::Range,
		Type::Iterator,
	] {
		assert_eq!(s.list(tag, &[]).unwrap_err().kind, ErrorKind::Type);
	}
	for tag in [
		Type::Bool,
		Type::String,
		Type::Tuple,
		Type::Range,
		Type::Iterator,
	] {
		assert_eq!(
			s.variant(tag, true, Some(Value::Unit)).unwrap_err().kind,
			ErrorKind::Type
		);
	}
	// Only a private, unchecked forged payload can reach these invariant branches.
	let list = s
		.admit(Payload::List {
			tag: Type::Bool,
			values: Vec::new(),
		})
		.unwrap();
	let variant = s
		.admit(Payload::Variant {
			tag: Type::String,
			success: true,
			payload: None,
		})
		.unwrap();
	for value in [list, variant] {
		assert_eq!(
			s.format_value(value, true).unwrap_err().kind,
			ErrorKind::InvalidCode
		);
		s.release(value).unwrap();
	}
	assert_eq!((s.live_entries(), s.payload_bytes()), (0, 0));
}

#[test]
fn unpublished_nominal_has_no_runtime_name_or_comparison_hash() {
	let mut s = Store::new(Limits::default()).unwrap();
	let tag = Type::Struct(crate::experimental_core::value::Nominal(u32::MAX));
	// The opaque Nominal can only be forged inside the crate. The compiler
	// publishes it before construction; Store::record does not publish metadata.
	let forged = s.record(tag, &[]).unwrap();
	assert_eq!(
		s.format_value(forged, true).unwrap_err().kind,
		ErrorKind::InvalidCode
	);
	assert_eq!(
		s.method_cmp(forged, Value::Unit).unwrap_err().kind,
		ErrorKind::InvalidCode
	);
	s.release(forged).unwrap();
}

#[test]
fn moved_opaque_formats_before_payload_access_with_the_same_object_identity() {
	let mut s = Store::new(Limits::default()).unwrap();
	let it = s.range_iterator(Value::I64(0), Value::I64(2)).unwrap();
	let before = s.format_value(it, true).unwrap();
	s.invalidate(it).unwrap();
	assert_eq!(s.format_value(it, true).unwrap(), before);
	assert!(matches!(
		s.format_value(it, false).unwrap_err().detail,
		Detail::NoDisplay {
			found: TypeName::RangeIter
		}
	));
	s.release(it).unwrap();
}
