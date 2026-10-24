//! C: canonical layout with source-order evaluation and process-lifetime names.
use crate::experimental_core::{
	execute::Executor,
	frontend,
	heap::{Limits, Store},
	value::Root,
};
use crate::{Context, Diagnostics, Hash, Source, Sources};

fn compile(context: &Context, source: &str) -> frontend::Program {
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1).unwrap()
}

fn invoke(program: &frontend::Program, store: &mut Store) -> Root {
	Executor::new()
		.unwrap()
		.invoke(
			program,
			store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			100_000,
		)
		.unwrap()
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn names_layout_snapshots_and_nested_debug_match_stock() {
	let context = Context::with_default_modules().unwrap();
	let mut checked = 0;
	for (defs, expr, expected) in [
		("struct P { x, y }", "P { y: 2, x: 1 }", "P {x: 1, y: 2}"),
		(
			"struct P { x, y }",
			"{ let x = 1; let y = 2; P { y, x } }",
			"P {x: 1, y: 2}",
		),
		(
			"struct P { x, y }",
			"{ let a = 1; P { y: { a = 2; a }, x: a } }",
			"P {x: 2, y: 2}",
		),
		(
			"struct P { x, y }",
			"{ let a = 1; P { y: a, x: { a = 2; a } } }",
			"P {x: 2, y: 1}",
		),
		(
			"struct P { q } struct Q { x }",
			"P { q: Q { x: 3 } }",
			"P {q: Q {x: 3}}",
		),
		(
			"struct P { q }",
			"P { q: #{y: 2, x: 1} }",
			"P {q: {\"x\": 1, \"y\": 2}}",
		),
		("struct P { x }", "[P { x: 7 }]", "[P {x: 7}]"),
		(
			"mod m { pub struct Q { x } }",
			"m::Q { x: 4 }",
			"m::Q {x: 4}",
		),
		("struct Empty {}", "Empty {}", "Empty {}"),
		(
			"struct P { x, y } fn mark(w, x) { w.push(x); x }",
			"{ let w = []; let p = P { y: mark(w, 2), x: mark(w, 1) }; (p, w) }",
			"(P {x: 1, y: 2}, [2, 1])",
		),
	] {
		let source = std::format!("{defs} pub fn main() {{ {expr} }}");
		let program = compile(&context, &source);
		let mut store = Store::new(Limits::default()).unwrap();
		let root = invoke(&program, &mut store);
		let actual = store.root_format(&root, true).unwrap();
		let stock_source = std::format!("{defs} pub fn main() {{ format!(\"{{:?}}\", {expr}) }}");
		let stock: crate::alloc::String =
			crate::tests::run(&context, &stock_source, (), false).unwrap();
		assert_eq!(stock, expected, "stock {source}");
		assert_eq!(actual, stock, "fresh {source}");
		drop(program);
		assert_eq!(
			store.root_format(&root, true).unwrap(),
			expected,
			"name after Program drop"
		);
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		checked += 1;
	}
	assert_eq!(checked, 10);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn cross_program_identity_keeps_each_values_fields_and_early_return() {
	let context = Context::with_default_modules().unwrap();
	let a = compile(
		&context,
		"struct Redeclared { x } pub fn main() { Redeclared { x: 1 } }",
	);
	let b = compile(
		&context,
		"struct Redeclared { y } pub fn main() { Redeclared { y: 2 } }",
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let ar = invoke(&a, &mut store);
	let br = invoke(&b, &mut store);
	assert_eq!(store.root_tag(&ar).unwrap(), store.root_tag(&br).unwrap());
	drop((a, b));
	assert_eq!(store.root_format(&ar, true).unwrap(), "Redeclared {x: 1}");
	assert_eq!(store.root_format(&br, true).unwrap(), "Redeclared {y: 2}");
	store.root_release(ar).unwrap();
	store.root_release(br).unwrap();
	let source = "struct Early { x, y } pub fn main() { Early { y: return 5, x: 1 } }";
	let program = compile(&context, source);
	let root = invoke(&program, &mut store);
	assert_eq!(
		store.root_scalar(&root).unwrap(),
		Some(crate::experimental_core::value::Scalar::I64(5))
	);
	let stock: i64 = crate::tests::run(&context, source, (), false).unwrap();
	assert_eq!(stock, 5);
	store.root_release(root).unwrap();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn tuple_and_unit_constructors_are_admitted_and_release_cleanly() {
	let context = Context::with_default_modules().unwrap();
	for (source, debug) in [
		("struct T(a,b);pub fn main(){T(1,2)}", "T (1, 2)"),
		("struct U;pub fn main(){U}", "U"),
	] {
		let program = compile(&context, source);
		let mut store = Store::new(Limits::default()).unwrap();
		let root = invoke(&program, &mut store);
		let value = store.import(&root).unwrap();
		assert_eq!(store.format_value(value, true).unwrap(), debug);
		store.root_release(root).unwrap();
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}

#[test]
fn unpublished_nominal_debug_is_an_internal_invariant() {
	use crate::experimental_core::{
		error::ErrorKind,
		value::{Nominal, Type, Value},
	};
	let mut store = Store::new(Limits::default()).unwrap();
	// Internal construction seam only; public admission refuses this identity.
	let value = store
		.record(Type::Struct(Nominal(u32::MAX)), &[("x", Value::I64(1))])
		.unwrap();
	let error = store.format_value(value, true).unwrap_err();
	assert_eq!(error.kind, ErrorKind::InvalidCode);
	assert_eq!(error.operation, "struct name");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: builds a std-module Context and compiles Rune source (minutes per build); native CI runs it; opt in with --ignored"
)]
fn struct_diagnostic_uses_the_item_name_on_both_engines() {
	use std::string::ToString;
	let context = Context::with_default_modules().unwrap();
	let source = "struct NamedFailure { x } pub fn main() { let p = NamedFailure { x: 1 }; p + 1 }";
	let program = compile(&context, source);
	let mut store = Store::new(Limits::default()).unwrap();
	let failure = Executor::new()
		.unwrap()
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			1000,
		)
		.unwrap_err();
	let expected = "Cannot apply `+` to `NamedFailure` and `i64`";
	assert_eq!(failure.error.message(&program).to_string(), expected);
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.build_vm()
		.unwrap();
	assert_eq!(vm.call(["main"], ()).unwrap_err().to_string(), expected);
}

// Frontend-free ownership and shape control, also selected by the Miri runner.
#[test]
fn tuple_struct_storage_alias_mutation_and_rollback() {
	use crate::experimental_core::{
		nominal,
		value::{Type, Value},
	};
	let mut staged = nominal::Staging::default();
	staged
		.stage_shape(
			0xc150_0000_0000_0001,
			format_args!("C15Miri"),
			nominal::Shape::Tuple(1),
		)
		.unwrap();
	let mut remap = staged.prepare_remap().unwrap();
	staged.commit(&mut remap).unwrap();
	let nominal = nominal::Staging::default()
		.stage_shape(
			0xc150_0000_0000_0001,
			format_args!("C15Miri"),
			nominal::Shape::Tuple(1),
		)
		.unwrap();
	let mut store = Store::new(Limits {
		entries: 4,
		bytes: 32,
	})
	.unwrap();
	let value = store.list(Type::Struct(nominal), &[Value::I64(7)]).unwrap();
	store.retain(value).unwrap();
	store.tuple_set(value, 0, Value::I64(9)).unwrap();
	assert_eq!(store.tuple_get(value, 0).unwrap(), Value::I64(9));
	assert_eq!(store.format_value(value, true).unwrap(), "C15Miri (9)");
	assert!(store.len(value).is_err());
	assert_eq!(
		store.field_get_mut(value, "x").unwrap_err().kind,
		crate::experimental_core::error::ErrorKind::IndexOutOfBounds
	);
	assert_eq!(
		store.dynamic_get(value, Value::I64(0)).unwrap(),
		Value::I64(9)
	);
	let key = store.string("x").unwrap();
	assert!(store.dynamic_get(value, key).is_err());
	store.release(key).unwrap();
	assert!(store.tuple_set(value, 1, Value::I64(0)).is_err());
	let before = (store.live_entries(), store.payload_bytes());
	assert!(store
		.list(Type::Struct(nominal), &[Value::Unit; 3])
		.is_err());
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	let extra = store.list(Type::Struct(nominal), &[Value::Unit]).unwrap();
	let full = (store.live_entries(), store.payload_bytes());
	assert!(store.list(Type::Struct(nominal), &[Value::Unit]).is_err());
	assert_eq!((store.live_entries(), store.payload_bytes()), full);
	store.release(extra).unwrap();
	store.release(value).unwrap();
	store.release(value).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	let mut staged = nominal::Staging::default();
	staged
		.stage_shape(
			0xc150_0000_0000_0003,
			format_args!("C15Named"),
			nominal::Shape::Named,
		)
		.unwrap();
	let mut remap = staged.prepare_remap().unwrap();
	staged.commit(&mut remap).unwrap();
	let named = nominal::Staging::default()
		.stage_shape(
			0xc150_0000_0000_0003,
			format_args!("C15Named"),
			nominal::Shape::Named,
		)
		.unwrap();
	let mut named_store = Store::new(Limits::default()).unwrap();
	let named_value = named_store
		.record(Type::Struct(named), &[("x", Value::I64(1))])
		.unwrap();
	named_store.invalidate(named_value).unwrap();
	assert_eq!(
		named_store.tuple_get(named_value, 0).unwrap_err().kind,
		crate::experimental_core::error::ErrorKind::IndexOutOfBounds
	);
	named_store.release(named_value).unwrap();
	assert_eq!(
		(named_store.live_entries(), named_store.payload_bytes()),
		(0, 0)
	);
}
