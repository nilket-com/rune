#[cfg(miri)]
macro_rules! miri_rows {
	($helper:ident; $($name:ident => $row:expr),+ $(,)?) => {
		$(#[test] fn $name() { $helper($row); })+
	};
}

prelude!();

/// A parameter pattern over an object or struct reads its fields with
/// `object-index-get-at`, and the assembler may give the field the same slot
/// as the argument it is read from. The field guard must end before the result
/// replaces the argument; otherwise the guard outlives the value it refers to.
#[cfg(not(miri))]
#[test]
fn object_field_into_own_slot() {
	for row in 0..3 {
		object_field_into_own_slot_case(row);
	}
}

fn object_field_into_own_slot_case(row: usize) {
	match row {
		0 => {
			let out: i64 = rune! {
				fn f(#{x}) { x }
				f(#{x: 7})
			};
			assert_eq!(out, 7);
		}
		1 => {
			let out: i64 = rune! {
				fn f(#{x, ..}) { x }
				f(#{x: 7, y: 8})
			};
			assert_eq!(out, 7);
		}
		2 => {
			let out: String = rune! {
				fn f(#{s}) { s }
				f(#{s: "hi"})
			};
			assert_eq!(out, "hi");
		}
		_ => panic!("invalid Miri row"),
	}
}

#[cfg(miri)]
miri_rows!(object_field_into_own_slot_case;
	object_field_into_own_slot__row_0 => 0,
	object_field_into_own_slot__row_1 => 1,
	object_field_into_own_slot__row_2 => 2
);

#[test]
fn struct_field_into_own_slot() {
	let out: i64 = rune! {
		struct P { x, y }
		fn f(P { x, .. }) { x }
		f(P { x: 7, y: 8 })
	};
	assert_eq!(out, 7);
}
