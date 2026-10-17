fn context(fixture: &str) -> Context {
	assert_eq!(fixture, "default");
	let mut c = Context::with_default_modules().unwrap();
	let mut m = crate::Module::with_item(["c15"]).unwrap();
	m.function("tap", tap).build().unwrap();
	c.install(m).unwrap();
	c
}
