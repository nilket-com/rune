prelude!();

/// A `for` loop over a temporary iterable such as `o.iter()` releases it on
/// every exit path, so the object it borrows can be written after the loop.
#[test]
fn temporary_iterable_is_released_at_loop_exit() {
	let out: i64 = rune! {
		let o = #{a: 1};
		for p in o.iter() {}
		o.b = 2;
		for p in o.iter() { break; }
		o.c = 3;
		for p in o.iter() { continue; }
		o.d = 4;
		'outer: for i in [1, 2] { for p in o.iter() { break 'outer; } }
		o.e = 5;
		'outer: for i in [1, 2] { for p in o.iter() { continue 'outer; } }
		o.f = 6;
		'block: { for p in o.iter() { break 'block; } }
		o.g = 7;
		o.len()
	};
	assert_eq!(out, 7);
}

/// An iterator bound to a variable keeps its borrow after a loop over it.
#[test]
fn bound_iterator_keeps_its_borrow() {
	assert_vm_error!(
		"let o = #{a: 1}; let it = o.iter(); for p in it {} o.c = 3; it",
		VmErrorKind::AccessError { .. } => {}
	);
}
