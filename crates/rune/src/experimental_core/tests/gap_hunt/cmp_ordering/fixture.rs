// Closed typed host signatures only. A mismatched Ordering must not enter a body.
fn c9_context() -> (Context, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
	let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c9"]).unwrap();
	macro_rules! host {
		($name:literal,$ty:ty) => {{
			let calls = calls.clone();
			module
				.host_function($name, move |_: $ty| -> i64 {
					calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
					7
				})
				.unwrap();
		}};
	}
	host!("id_unit", ());
	host!("id_bool", bool);
	host!("id_i64", i64);
	host!("id_u64", u64);
	host!("id_f64", f64);
	host!("id_char", char);
	host!("id_string", &str);
	context.install(module).unwrap();
	(context, calls)
}
