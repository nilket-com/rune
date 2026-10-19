prelude!();

/// A parameter pattern over an object or struct reads its fields with
/// `object-index-get-at`, and the assembler may give the field the same slot
/// as the argument it is read from. The field guard must end before the result
/// replaces the argument; otherwise the guard outlives the value it refers to.
#[test]
fn object_field_into_own_slot() {
	let out: i64 = rune! {
		fn f(#{x}) { x }
		f(#{x: 7})
	};
	assert_eq!(out, 7);

	let out: i64 = rune! {
		fn f(#{x, ..}) { x }
		f(#{x: 7, y: 8})
	};
	assert_eq!(out, 7);

	let out: String = rune! {
		fn f(#{s}) { s }
		f(#{s: "hi"})
	};
	assert_eq!(out, "hi");
}

#[test]
fn struct_field_into_own_slot() {
	let out: i64 = rune! {
		struct P { x, y }
		fn f(P { x, .. }) { x }
		f(P { x: 7, y: 8 })
	};
	assert_eq!(out, 7);
}
