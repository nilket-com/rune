// Test-only registrations, including providers that must never borrow std provenance.
fn native_body(_: Value) -> i64 {
	tap(99, 0);
	41
}
fn context(fixture: &str) -> Context {context_for(fixture,false)}
fn context_for(fixture:&str,fresh:bool)->Context{
	use crate::TypeHash;
	let mut context = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c14"]).unwrap();
	if fresh {m.fresh_host_function("tap",tap).unwrap();}else{m.function("tap", tap).build().unwrap();}
	m.function("native_body", native_body).build().unwrap();
	context.install(m).unwrap();
	match fixture {
		"default" => (),
		"i64-provider" => {
			let mut m = crate::Module::with_item(["c14_provider"]).unwrap();
			for name in [
				"split",
				"replace",
				"starts_with",
				"ends_with",
				"contains",
				"trim",
			] {
				m.associated_function(name, |_: i64| -> i64 {
					tap(99, 0);
					41
				})
				.unwrap();
			}
			context.install(m).unwrap();
		}
		"raw-i64-provider" => {
			let handler = context
				.lookup_function(Hash::type_hash(["c14", "native_body"]))
				.unwrap()
				.clone();
			for name in [
				"split",
				"replace",
				"starts_with",
				"ends_with",
				"contains",
				"trim",
			] {
				context
					.c13_insert_raw_fixture(
						Hash::associated_function(i64::HASH, Hash::ident(name)),
						handler.clone(),
					)
					.unwrap();
			}
		}
		_ => panic!("unreviewed C14 fixture"),
	}
	context
}
