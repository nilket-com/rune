// Live differential over every method name a new fast path recognizes: a
// program that defines a script method of that name either refuses or matches
// stock. It reads no frozen observation.

/// Names the lowering dispatches on without a script target, from each table.
fn fast_path_names() -> Vec<&'static str> {
	let mut names = vec![
		"len", "clone", "push", "collect", "sort_by", "get", "contains_key", "insert", "remove",
		"is_empty", "abs", "round", "powf", "cmp", "unwrap", "expect", "is_some", "is_none",
		"is_ok", "is_err", "unwrap_or", "chars", "skip", "take", "count", "next", "sum",
		"product", "map", "filter", "filter_map", "flat_map", "enumerate", "chain", "rev",
		"fold", "reduce", "find", "any", "all", "iter",
	];
	names.extend(
		crate::function::text::Method::ALL
			.iter()
			.map(|m| m.name()),
	);
	names.extend(
		crate::function::number::Method::ALL
			.iter()
			.map(|m| m.name()),
	);
	names.extend(crate::function::iter::Name::ALL.iter().map(|n| n.as_str()));
	names.sort_unstable();
	names.dedup();
	names
}

/// One engine's outcome, reduced to what both engines must agree on.
fn legacy_shadow(program: &str) -> Json {
	let mut sources = Sources::new();
	sources
		.insert(Source::new("shadow.rn", program).unwrap())
		.unwrap();
	let context = Context::with_default_modules().unwrap();
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
		.unwrap();
	match crate::runtime::budget::with(200_000, || vm.call(["main"], ())).call() {
		Ok(value) => json!({"value":runtime_graph(&value)}),
		Err(e) => json!({"error":error(&e, &sources)["class"]}),
	}
}

fn shadow(program: &str) -> Option<Json> {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources
		.insert(Source::new("shadow.rn", program).unwrap())
		.unwrap();
	let built = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 16).ok()?;
	let mut store = Store::new(Limits {
		entries: 4096,
		bytes: 4 << 20,
	})
	.unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let main = built.function(Hash::type_hash(["main"])).unwrap();
	Some(match executor.invoke(&built, &mut store, main, &[], 200_000) {
		Ok(root) => {
			let value = store.import(&root).unwrap();
			let runtime_graph = graph(&mut store, value);
			store.root_release(root).unwrap();
			json!({"value":runtime_graph})
		}
		Err(failure) => json!({"error":self::failure(&failure, &built, &sources)["class"]}),
	})
}

#[test]
#[ignore = "C18a live shadow differential (about 2 minutes in debug); run by the gates"]
fn shadow_contract() {
	const ARGS: [(&str, &str); 7] = [
		("", ""),
		("a", "1"),
		("a", "|x|x"),
		("a", "|x,y|x"),
		("a,b", "1,2"),
		("a,b", "1,|x|x"),
		("a,b", "|x|x,|x,y|x"),
	];
	let mut admitted = 0;
	for name in fast_path_names() {
		for (params, args) in ARGS {
			let params = if params.is_empty() {
				String::from("self")
			} else {
				format!("self,{params}")
			};
			let named = format!("struct H{{h}}impl H{{fn {name}({params}){{\"script\"}}}}");
			let tuple = format!("struct H(h);impl H{{fn {name}({params}){{\"script\"}}}}");
			let call = format!("{name}({args})");
			for program in [
				format!("{named}pub fn main(){{H{{h:1}}.{call}}}"),
				format!("{tuple}pub fn main(){{let x=H(1);x.{call}}}"),
				format!("{named}fn f(x){{x.{call}}}pub fn main(){{f(H{{h:1}})}}"),
				format!("{named}fn f(x){{x.{call};7}}pub fn main(){{f(H{{h:1}})}}"),
				format!("{named}fn f(x){{x.{call}}}pub fn main(){{f([3,1,2])}}"),
				format!("{named}pub fn main(){{\"abc\".{call}}}"),
				format!("{named}fn f(x){{let s=0;for i in 0..x.{call}{{s+=1;}}s}}pub fn main(){{f(H{{h:1}})}}"),
				format!("{named}pub fn main(){{H{{h:1}}.{call}.count()}}"),
				format!("{named}pub fn main(){{H{{h:1}}.{call}.map(|v|v).collect::<Vec>()}}"),
				format!("{named}pub fn main(){{H{{h:1}}.{call}.contains(3)}}"),
			] {
				let Some(execution) = shadow(&program) else {
					continue;
				};
				admitted += 1;
				assert_eq!(execution, legacy_shadow(&program), "{program}");
			}
		}
	}
	eprintln!("C18a shadow programs admitted: {admitted}");
	// The certain-receiver forms of every name are admitted at least.
	assert!(admitted >= 2 * fast_path_names().len(), "{admitted}");
}
