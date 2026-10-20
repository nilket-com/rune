prelude!();

use crate::runtime::VmErrorKind;

/// `collect::<Object>` borrows each String key and stores a copy, so every
/// other handle to the key stays readable and owns its own bytes.
#[test]
fn collect_copies_keys() {
	let out: (usize, String) = rune! {
		let k = "key";
		let o = [(k, 1)].iter().collect::<Object>();
		(o.len(), k)
	};
	assert_eq!(out, (1, String::from("key")));

	let out: (i64, String) = rune! {
		let v = [("k", 1)];
		let o = v.iter().collect::<Object>();
		(o["k"], v[0].0)
	};
	assert_eq!(out, (1, String::from("k")));

	let out: (i64, String) = rune! {
		let k = "k";
		let o = [(k, 1), (k, 2)].iter().collect::<Object>();
		(o[k], k)
	};
	assert_eq!(out, (2, String::from("k")));

	let out: (bool, bool, String) = rune! {
		let k = "a";
		let o = [(k, 1)].iter().collect::<Object>();
		k.push_str("b");
		(o.contains_key("a"), o.contains_key("ab"), k)
	};
	assert_eq!(out, (true, false, String::from("ab")));
}

/// `Object::insert` copies its key the same way.
#[test]
fn insert_copies_the_key() {
	let out: (i64, String) = rune! {
		let k = "k";
		let o = #{};
		o.insert(k, 1);
		o.insert(k, 2);
		(o[k], k)
	};
	assert_eq!(out, (2, String::from("k")));

	let out: (bool, String) = rune! {
		let k = "a";
		let o = #{};
		o.insert(k, 1);
		k.push_str("b");
		(o.contains_key("a"), k)
	};
	assert_eq!(out, (true, String::from("ab")));
}

#[test]
fn non_string_keys_keep_their_errors() {
	assert_vm_error!(
		"[(1, 2)].iter().collect::<Object>()",
		VmErrorKind::ExpectedType { .. } => {}
	);
	assert_vm_error!(
		"[([1], 2)].iter().collect::<Object>()",
		VmErrorKind::ExpectedType { .. } => {}
	);
	assert_vm_error!(
		"[(\"k\", 1, 2)].iter().collect::<Object>()",
		VmErrorKind::ExpectedTupleLength { actual: 3, expected: 2 } => {}
	);
	assert_vm_error!(
		"let o = #{}; o.insert(1, 2)",
		VmErrorKind::ExpectedType { .. } => {}
	);
}
