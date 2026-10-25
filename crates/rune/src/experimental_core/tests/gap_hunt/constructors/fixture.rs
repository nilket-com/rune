fn context(fixture: &str) -> Context {
	context_for(fixture, false)
}
fn context_for(fixture: &str, certify: bool) -> Context {
	assert_eq!(fixture, "default");
	let mut c = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c15"]).unwrap();
	if certify {
		m.host_function("tap", tap).unwrap();
	} else {
		m.function("tap", tap).build().unwrap();
	}
	c.install(m).unwrap();
	c
}
