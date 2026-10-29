//! E1: live embedding contracts, with no stored observations.

use crate::experimental_core::{
	error::{ErrorKind, Refusal},
	execute::{Executor, PrepareError},
	frontend::{self, Program},
	heap::{FormatLimits, Limits, NominalShape, Store, ViewKind, ViewLimits},
	value::{Root, Scalar, Type},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};
use std::{borrow::ToOwned, string::ToString};

#[cfg(not(miri))]
mod budget_edges;
#[cfg(not(miri))]
mod metering;
use std::sync::{
	atomic::{AtomicUsize, Ordering},
	Arc,
};

fn sources(text: &str) -> Sources {
	let mut sources = Sources::new();
	sources
		.insert(Source::new("embedding.rn", text).unwrap())
		.unwrap();
	sources
}

fn compile(context: &Context, text: &str) -> Program {
	frontend::compile(&mut sources(text), context, &mut Diagnostics::new(), 1).unwrap()
}

fn executor(context: &Context) -> Executor {
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	executor
}

fn main(program: &Program) -> u32 {
	program.function(Hash::type_hash(["main"])).unwrap()
}

fn context_with_effects() -> (Context, Arc<AtomicUsize>) {
	let mut context = Context::with_default_modules().unwrap();
	let effects = Arc::new(AtomicUsize::new(0));
	let count = effects.clone();
	let mut module = crate::Module::with_crate("probe").unwrap();
	module
		.host_function("touch", move || {
			count.fetch_add(1, Ordering::SeqCst);
		})
		.unwrap();
	context.install(module).unwrap();
	(context, effects)
}

#[test]
fn public_executor_surface_keeps_exclusive_synchronous_execution() {
	use std::collections::BTreeSet;
	let tree = syn::parse_file(include_str!("../execute.rs")).unwrap();
	let mut names = BTreeSet::new();
	let mut prepared = false;
	for item in tree.items {
		let syn::Item::Impl(item) = item else {
			continue;
		};
		let syn::Type::Path(ty) = item.self_ty.as_ref() else {
			continue;
		};
		let name = &ty.path.segments.last().unwrap().ident;
		for member in item.items {
			let syn::ImplItem::Fn(method) = member else {
				continue;
			};
			if !matches!(method.vis, syn::Visibility::Public(_)) {
				continue;
			}
			if name == "Executor" {
				assert!(
					method.sig.asyncness.is_none(),
					"new async API needs an embedding review"
				);
				let method_name = method.sig.ident.to_string();
				if ["prepare", "invoke", "request_cancel"].contains(&method_name.as_str()) {
					let syn::FnArg::Receiver(receiver) = &method.sig.inputs[0] else {
						panic!("missing receiver")
					};
					assert!(receiver.reference.is_some() && receiver.mutability.is_some());
				}
				names.insert(method_name);
			} else if name == "PreparedInvocation" && method.sig.ident == "invoke" {
				assert!(method.sig.asyncness.is_none());
				let syn::FnArg::Receiver(receiver) = &method.sig.inputs[0] else {
					panic!("missing receiver")
				};
				assert!(
					receiver.reference.is_none(),
					"prepared invocation must consume its token"
				);
				prepared = true;
			}
		}
	}
	let expected: BTreeSet<_> = "new with_limits limits bookkeeping_bytes set_host_runtime request_cancel format_limits set_format_limits prepare run invoke is_idle".split_whitespace().map(str::to_owned).collect();
	assert_eq!(
		names, expected,
		"new public executor API needs an embedding review"
	);
	assert!(prepared);
	let _: for<'a> fn(
		&'a mut Executor,
		&'a Program,
		&'a mut Store,
		u32,
		&'a [&'a Root],
	) -> Result<
		crate::experimental_core::execute::PreparedInvocation<'a>,
		PrepareError,
	> = Executor::prepare;
}

#[test]
fn borrowed_view_preserves_aliases_cycles_and_shared_visit_bounds() {
	let mut store = Store::new(Limits::default()).unwrap();
	let cycle = store.root_list(Type::Vec, &[]).unwrap();
	store.root_push(&cycle, &cycle).unwrap();
	let pair = store.root_list(Type::Tuple, &[&cycle, &cycle]).unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	{
		let view = store
			.root_view(&pair, ViewLimits { nodes: 4, depth: 2 })
			.unwrap();
		let root = view.root().unwrap();
		let first = root.child(0).unwrap();
		let second = root.child(1).unwrap();
		assert_eq!(first.identity(), second.identity());
		assert_eq!(
			first.kind().unwrap(),
			ViewKind::List {
				tag: Type::Vec,
				len: 1
			}
		);
		let back = first.child(0).unwrap();
		assert_eq!(back.identity(), first.identity());
		assert!(matches!(back.child(0), Err(error) if error.kind == ErrorKind::FormatLimit));
		assert!(matches!(view.root(), Err(error) if error.kind == ErrorKind::FormatLimit));
	}
	assert_eq!((store.live_entries(), store.payload_bytes()), before);
	let view = store
		.root_view(
			&cycle,
			ViewLimits {
				nodes: 10,
				depth: 0,
			},
		)
		.unwrap();
	assert!(
		matches!(view.root().unwrap().child(0), Err(error) if error.kind == ErrorKind::FormatLimit)
	);
	store.root_release(pair).unwrap();
	store.root_release(cycle).unwrap();
	store.close().unwrap();
}

#[test]
fn view_rejects_foreign_stale_and_closed_roots_and_checks_indices() {
	let mut store = Store::new(Limits::default()).unwrap();
	let other = Store::new(Limits::default()).unwrap();
	assert!(
		matches!(store.root_view(&other.root_unit(), ViewLimits::default()), Err(error) if error.kind == ErrorKind::InvalidHandle)
	);
	let root = store.root_string("abc").unwrap();
	// A deliberately unretained duplicate probes the generation boundary.
	let stale = store.export(root.value);
	store.root_release(root).unwrap();
	assert!(
		matches!(store.root_view(&stale, ViewLimits::default()), Err(error) if error.kind == ErrorKind::InvalidHandle)
	);
	let scalar = store.root_i64(7);
	let list = store.root_list(Type::Vec, &[&scalar]).unwrap();
	let view = store
		.root_view(&list, ViewLimits { nodes: 2, depth: 1 })
		.unwrap();
	let root = view.root().unwrap();
	assert!(matches!(root.child(1), Err(error) if error.kind == ErrorKind::IndexOutOfBounds));
	assert_eq!(
		root.child(0).unwrap().kind().unwrap(),
		ViewKind::Scalar(Scalar::I64(7))
	);
	store.close().unwrap();
	assert!(
		matches!(store.root_view(&scalar, ViewLimits::default()), Err(error) if error.kind == ErrorKind::ClosedStore)
	);
}

#[test]
fn view_keeps_mutual_cycles_variants_keys_and_moved_access_without_allocating() {
	let mut store = Store::new(Limits {
		entries: 16,
		bytes: 1024,
	})
	.unwrap();
	let left = store.root_list(Type::Vec, &[]).unwrap();
	let right = store.root_list(Type::Vec, &[]).unwrap();
	store.root_push(&left, &right).unwrap();
	store.root_push(&right, &left).unwrap();
	let text = store.root_string("\u{e9}\n").unwrap();
	let some = store.root_variant(Type::Option, true, Some(&text)).unwrap();
	let none = store.root_variant(Type::Option, false, None).unwrap();
	let object = store
		.root_record(None, &[("\u{e9}", &some), ("none", &none)])
		.unwrap();
	rune_alloc::limit::with(0, || {
		let view = store.root_view(&left, ViewLimits::default()).unwrap();
		let root = view.root().unwrap();
		assert_eq!(
			root.identity(),
			root.child(0).unwrap().child(0).unwrap().identity()
		);
		let object = store.root_view(&object, ViewLimits::default()).unwrap();
		let root = object.root().unwrap();
		assert_eq!(root.field_name(0).unwrap(), "\u{e9}");
		let some = root.child(0).unwrap();
		assert_eq!(some.kind().unwrap(), ViewKind::Option { some: true });
		assert_eq!(
			some.child(0).unwrap().kind().unwrap(),
			ViewKind::String("\u{e9}\n")
		);
		let none = root.child(1).unwrap();
		assert_eq!(none.kind().unwrap(), ViewKind::Option { some: false });
		assert!(matches!(none.child(0),Err(error) if error.kind==ErrorKind::IndexOutOfBounds));
	})
	.call();
	store.invalidate(text.value).unwrap();
	assert!(
		matches!(store.root_view(&text,ViewLimits::default()),Err(error) if error.kind==ErrorKind::AccessConflict)
	);
	let view = store.root_view(&some, ViewLimits::default()).unwrap();
	assert!(
		matches!(view.root().unwrap().child(0),Err(error) if error.kind==ErrorKind::AccessConflict)
	);
	store.close().unwrap();
}

#[test]
fn prepared_empty_context_preserves_cancellation_and_reuses_clean_state() {
	let context = Context::new();
	let program = compile(&context, "pub fn main(){42}");
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	executor.request_cancel();
	drop(
		executor
			.prepare(&program, &mut store, main(&program), &[])
			.unwrap(),
	);
	let failure = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(1)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert_eq!(failure.remaining_work, Some(1));
	assert!(executor.is_idle());
	let root = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(1)
		.unwrap();
	assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(42)));
	store.root_release(root).unwrap();
	store.close().unwrap();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn prepared_drop_cancel_and_terminal_failure_never_repeat_effects() {
	let (context, effects) = context_with_effects();
	let program = compile(&context, "pub fn main(){probe::touch();panic!(\"once\")}");
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	executor.request_cancel();
	drop(
		executor
			.prepare(&program, &mut store, main(&program), &[])
			.unwrap(),
	);
	assert_eq!(effects.load(Ordering::SeqCst), 0);
	let failure = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Cancelled);
	assert_eq!(effects.load(Ordering::SeqCst), 0);
	let failure = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(1000)
		.unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Panic);
	assert!(failure.remaining_work.is_some());
	assert_eq!(effects.load(Ordering::SeqCst), 1);
	assert!(executor.is_idle());
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn prepare_distinguishes_refused_entries_from_invalid_arguments_and_bindings() {
	let (context, effects) = context_with_effects();
	let text = "pub fn main(x){probe::touch();x}";
	let program = compile(&context, text);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	assert!(
		matches!(executor.prepare(&program, &mut store, main(&program), &[]), Err(PrepareError::Invalid(error)) if error.kind == ErrorKind::ArgumentCount)
	);
	let other = Store::new(Limits::default()).unwrap();
	let foreign = other.root_unit();
	assert!(
		matches!(executor.prepare(&program, &mut store, main(&program), &[&foreign]), Err(PrepareError::Invalid(error)) if error.kind == ErrorKind::InvalidHandle)
	);
	let (different, different_effects) = context_with_effects();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(different.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let local = store.root_unit();
	assert!(
		matches!(executor.prepare(&program, &mut store, main(&program), &[&local]), Err(PrepareError::Invalid(error)) if error.kind == ErrorKind::InvalidCode)
	);
	assert_eq!(effects.load(Ordering::SeqCst), 0);
	assert_eq!(different_effects.load(Ordering::SeqCst), 0);
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let internal = compile(
		&context,
		"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S.run()}",
	);
	let entry = internal.function(Hash::type_hash(["S", "run"])).unwrap();
	assert!(matches!(
		executor.prepare(&internal, &mut store, entry, &[&local]),
		Err(PrepareError::Refused(Refusal::Entry))
	));
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn cold_remaining_work_distinguishes_setup_and_last_unit_errors() {
	let context = Context::with_default_modules().unwrap();
	let program = compile(&context, "pub fn main(){let v=[1];v[9]}");
	let mut executor = executor(&context);
	let mut store = Store::new(Limits::default()).unwrap();
	let argument = store.root_unit();
	let setup = executor
		.invoke(&program, &mut store, main(&program), &[&argument], 100)
		.unwrap_err();
	assert_eq!(setup.remaining_work, None);
	let empty = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(0)
		.unwrap_err();
	assert_eq!(empty.error.kind, ErrorKind::FuelExhausted);
	assert_eq!(empty.remaining_work, Some(0));
	let mut last_unit = false;
	for work in 1..100 {
		let failure = executor
			.prepare(&program, &mut store, main(&program), &[])
			.unwrap()
			.invoke(work)
			.unwrap_err();
		if failure.error.kind == ErrorKind::IndexOutOfBounds && failure.remaining_work == Some(0) {
			last_unit = true;
		}
		assert!(executor.is_idle());
	}
	assert!(last_unit, "no typed primary-error-at-exhaustion witness");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn budget_edges_complete_whenever_legacy_completes_including_callbacks() {
	use crate::experimental_core::{
		error::{ErrorClass, ErrorProjection},
		execute::take_remaining_work_for_test,
	};
	let context = Context::with_default_modules().unwrap();
	let mut intended = 0;
	for body in [
		"42",
		"let n=3;let x=n*2;x+1",
		"let n=0;for x in 0..5{n+=x;}n",
		"[1,2,3].iter().map(|x|x+1).sum::<i64>()",
		"let v=[3,1,2];v.sort_by(|a,b|a.cmp(b));v[0]",
	] {
		let text = std::format!("pub fn main(){{{body}}}");
		let mut legacy = crate::prepare(&mut sources(&text))
			.with_context(&context)
			.build_vm()
			.unwrap();
		let program = compile(&context, &text);
		let mut executor = executor(&context);
		let mut store = Store::new(Limits::default()).unwrap();
		let mut completed = 0;
		for work in 0..256 {
			let (old, remaining) = crate::runtime::budget::with(work, || {
				let result = legacy.call(["main"], ());
				(result, crate::runtime::budget::remaining().unwrap())
			})
			.call();
			let _ = take_remaining_work_for_test();
			let new = executor
				.prepare(&program, &mut store, main(&program), &[])
				.unwrap()
				.invoke(work);
			let new_remaining = take_remaining_work_for_test().unwrap();
			match (old, new) {
				(Ok(old), Ok(new)) => {
					assert!(new_remaining>=remaining,"execution charged more: body={body}, N={work}, legacy left={remaining}, new left={new_remaining}");
					let expected: i64 = crate::from_value(old).unwrap();
					assert_eq!(
						store.root_scalar(&new).unwrap(),
						Some(Scalar::I64(expected))
					);
					store.root_release(new).unwrap();
					completed += 1;
				}
				(Ok(_), Err(failure)) => panic!(
					"legacy completed but execution stopped: body={body}, N={work}, {failure:?}"
				),
				(Err(old), Ok(new)) => {
					assert!(matches!(
						old.error_class(),
						ErrorProjection::Class(ErrorClass::BudgetExhausted)
					));
					store.root_release(new).unwrap();
					intended += 1;
				}
				(Err(old), Err(new)) => {
					assert!(matches!(
						old.error_class(),
						ErrorProjection::Class(ErrorClass::BudgetExhausted)
					));
					assert_eq!(new.error.kind, ErrorKind::FuelExhausted);
					assert_eq!(new.remaining_work, Some(0));
				}
			}
		}
		assert!(completed > 0, "no successful coverage for {body}");
		store.close().unwrap();
	}
	assert!(
		intended > 0,
		"expected classified new-counts-less-work witnesses"
	);
	std::println!("new-counts-less-work: {intended}");
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn view_exposes_config_errors_nominals_functions_and_opaque_markers() {
	let context = Context::with_default_modules().unwrap();
	let text = "struct Named{n}struct Pair(a,b);struct Empty;enum E{N{v},T(v),Z}pub fn main(){let n=1;let v=[1];let it=v.iter();(#{\"key\":Err([1,2])},Named{n:7},Pair(1,2),Empty,E::N{v:3},E::T(4),E::Z,||n,1..4,[1].iter(),Some(7),None,||42,it)}";
	let program = compile(&context, text);
	let mut store = Store::new(Limits::default()).unwrap();
	let root = executor(&context)
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(10000)
		.unwrap();
	let view = store.root_view(&root, ViewLimits::default()).unwrap();
	let tuple = view.root().unwrap();
	let object = tuple.child(0).unwrap();
	assert_eq!(object.field_name(0).unwrap(), "key");
	let error = object.child(0).unwrap();
	assert_eq!(error.kind().unwrap(), ViewKind::Result { ok: false });
	assert_eq!(
		error.child(0).unwrap().kind().unwrap(),
		ViewKind::List {
			tag: Type::Vec,
			len: 2
		}
	);
	for (index, name, shape, variant) in [
		(1, "Named", NominalShape::Named, false),
		(2, "Pair", NominalShape::Tuple(2), false),
		(3, "Empty", NominalShape::Empty, false),
		(4, "E::N", NominalShape::Named, true),
		(5, "E::T", NominalShape::Tuple(1), true),
		(6, "E::Z", NominalShape::Empty, true),
	] {
		let node = tuple.child(index).unwrap();
		let info = node.nominal().unwrap().unwrap();
		assert!(info.name().ends_with(name), "{}", info.name());
		assert_eq!(info.shape, shape);
		assert_eq!(info.variant.is_some(), variant);
	}
	assert_eq!(tuple.child(7).unwrap().kind().unwrap(), ViewKind::Function);
	let range = tuple.child(8).unwrap();
	assert_eq!(range.kind().unwrap(), ViewKind::Opaque(Type::Range));
	assert_eq!(range.type_name().unwrap().to_string(), "std::ops::Range");
	assert_eq!(
		tuple.child(9).unwrap().kind().unwrap(),
		ViewKind::Opaque(Type::Iterator)
	);
	assert_eq!(
		tuple.child(10).unwrap().kind().unwrap(),
		ViewKind::Option { some: true }
	);
	assert_eq!(
		tuple.child(11).unwrap().kind().unwrap(),
		ViewKind::Option { some: false }
	);
	assert_eq!(tuple.child(12).unwrap().kind().unwrap(), ViewKind::Function);
	assert_eq!(
		tuple.child(13).unwrap().type_name().unwrap().to_string(),
		"std::slice::Iter"
	);
	store.root_release(root).unwrap();
	store.close().unwrap();
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn script_formatting_policy_matches_live_legacy_beyond_old_default_limits() {
	let (context, effects) = context_with_effects();
	let mut cases = std::vec::Vec::new();
	for bytes in [32767, 32768, 32769, 35000] {
		cases.push(std::format!(
			"let s=\"{}\";format!(\"{{}}{{}}\",s,s)",
			"x".repeat(bytes)
		));
	}
	for nodes in [4095, 4096, 4097] {
		cases.push(std::format!(
			"let v=[];for n in 0..{nodes}{{v.push(n);}}format!(\"{{:?}}\",v)"
		));
	}
	for depth in [31, 32, 33, 34] {
		cases.push(std::format!(
			"let v=0;for n in 0..{depth}{{v=[v];}}format!(\"{{:?}}\",v)"
		));
	}
	for body in cases {
		let text = std::format!("pub fn main(){{probe::touch();{body}}}");
		let mut legacy = crate::prepare(&mut sources(&text))
			.with_context(&context)
			.build_vm()
			.unwrap();
		effects.store(0, Ordering::SeqCst);
		let expected = legacy.call(["main"], ()).unwrap();
		let expected = expected.borrow_string_ref().unwrap().to_string();
		assert_eq!(effects.load(Ordering::SeqCst), 1);
		let program = compile(&context, &text);
		let mut executor = executor(&context);
		executor.set_format_limits(FormatLimits::permissive());
		let mut store = Store::new(Limits::default()).unwrap();
		effects.store(0, Ordering::SeqCst);
		let root = executor
			.prepare(&program, &mut store, main(&program), &[])
			.unwrap()
			.invoke(2_000_000)
			.unwrap();
		let view = store.root_view(&root, ViewLimits::default()).unwrap();
		assert_eq!(
			view.root().unwrap().kind().unwrap(),
			ViewKind::String(&expected)
		);
		assert_eq!(effects.load(Ordering::SeqCst), 1);
		store.root_release(root).unwrap();
		store.close().unwrap();
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "full default-module frontend compilation; opt in explicitly"
)]
fn uncertified_macro_refuses_before_handler_and_legacy_compile_runs_it_once() {
	use crate::alloc::prelude::TryClone;
	let mut context = Context::with_default_modules().unwrap();
	let effects = Arc::new(AtomicUsize::new(0));
	let count = effects.clone();
	let mut module = crate::Module::with_crate("probe").unwrap();
	module
		.macro_(
			"touch",
			move |_: &mut crate::macros::MacroContext<'_, '_, '_>,
			      input: &crate::macros::TokenStream| {
				count.fetch_add(1, Ordering::SeqCst);
				Ok(input.try_clone()?)
			},
		)
		.unwrap();
	context.install(module).unwrap();
	let text = "pub fn main(){probe::touch!(42)}";
	let refusal = frontend::compile(&mut sources(text), &context, &mut Diagnostics::new(), 1)
		.err()
		.unwrap();
	assert!(refusal.refusal.is_some());
	assert_eq!(effects.load(Ordering::SeqCst), 0);
	crate::prepare(&mut sources(text))
		.with_context(&context)
		.build()
		.unwrap();
	assert_eq!(effects.load(Ordering::SeqCst), 1);
}
