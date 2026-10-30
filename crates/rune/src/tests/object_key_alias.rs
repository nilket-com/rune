#[cfg(miri)]
macro_rules! miri_rows {
	($helper:ident; $($name:ident => $row:expr),+ $(,)?) => {
		$(#[test] fn $name() { $helper($row); })+
	};
}

prelude!();

use crate::runtime::VmErrorKind;

/// `collect::<Object>` borrows each String key and stores a copy, so every
/// other handle to the key stays readable and owns its own bytes.
#[cfg(not(miri))]
#[test]
fn collect_copies_keys() {
	for row in 0..4 {
		collect_copies_keys_case(row);
	}
}

fn collect_copies_keys_case(row: usize) {
	match row {
		0 => {
			let out: (usize, String) = rune! {
				let k = "key";
				let o = [(k, 1)].iter().collect::<Object>();
				(o.len(), k)
			};
			assert_eq!(out, (1, String::from("key")));
		}
		1 => {
			let out: (i64, String) = rune! {
				let v = [("k", 1)];
				let o = v.iter().collect::<Object>();
				(o["k"], v[0].0)
			};
			assert_eq!(out, (1, String::from("k")));
		}
		2 => {
			let out: (i64, String) = rune! {
				let k = "k";
				let o = [(k, 1), (k, 2)].iter().collect::<Object>();
				(o[k], k)
			};
			assert_eq!(out, (2, String::from("k")));
		}
		3 => {
			let out: (bool, bool, String) = rune! {
				let k = "a";
				let o = [(k, 1)].iter().collect::<Object>();
				k.push_str("b");
				(o.contains_key("a"), o.contains_key("ab"), k)
			};
			assert_eq!(out, (true, false, String::from("ab")));
		}
		_ => panic!("invalid Miri row"),
	}
}

#[cfg(miri)]
miri_rows!(collect_copies_keys_case;
	collect_copies_keys__row_0 => 0,
	collect_copies_keys__row_1 => 1,
	collect_copies_keys__row_2 => 2,
	collect_copies_keys__row_3 => 3
);

/// `Object::insert` copies its key the same way.
#[cfg(not(miri))]
#[test]
fn insert_copies_the_key() {
	for row in 0..2 {
		insert_copies_the_key_case(row);
	}
}

fn insert_copies_the_key_case(row: usize) {
	match row {
		0 => {
			let out: (i64, String) = rune! {
				let k = "k";
				let o = #{};
				o.insert(k, 1);
				o.insert(k, 2);
				(o[k], k)
			};
			assert_eq!(out, (2, String::from("k")));
		}
		1 => {
			let out: (bool, String) = rune! {
				let k = "a";
				let o = #{};
				o.insert(k, 1);
				k.push_str("b");
				(o.contains_key("a"), k)
			};
			assert_eq!(out, (true, String::from("ab")));
		}
		_ => panic!("invalid Miri row"),
	}
}

#[cfg(miri)]
miri_rows!(insert_copies_the_key_case;
	insert_copies_the_key__row_0 => 0,
	insert_copies_the_key__row_1 => 1
);

#[cfg(not(miri))]
#[test]
fn non_string_keys_keep_their_errors() {
	for row in 0..4 {
		non_string_keys_keep_their_errors_case(row);
	}
}

fn non_string_keys_keep_their_errors_case(row: usize) {
	match row {
		0 => {
			assert_vm_error!(
				"[(1, 2)].iter().collect::<Object>()",
				VmErrorKind::ExpectedType { .. } => {}
			);
		}
		1 => {
			assert_vm_error!(
				"[([1], 2)].iter().collect::<Object>()",
				VmErrorKind::ExpectedType { .. } => {}
			);
		}
		2 => {
			assert_vm_error!(
				"[(\"k\", 1, 2)].iter().collect::<Object>()",
				VmErrorKind::ExpectedTupleLength { actual: 3, expected: 2 } => {}
			);
		}
		3 => {
			assert_vm_error!(
				"let o = #{}; o.insert(1, 2)",
				VmErrorKind::ExpectedType { .. } => {}
			);
		}
		_ => panic!("invalid Miri row"),
	}
}

#[cfg(miri)]
miri_rows!(non_string_keys_keep_their_errors_case;
	non_string_keys_keep_their_errors__row_0 => 0,
	non_string_keys_keep_their_errors__row_1 => 1,
	non_string_keys_keep_their_errors__row_2 => 2,
	non_string_keys_keep_their_errors__row_3 => 3
);
