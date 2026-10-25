// These checks use actual retained payloads and store accounting, independently
// of the frozen script graphs. Staging is dropped before every clone.
fn clone_tags() -> [Type; 10] {
	use crate::experimental_core::nominal::{Shape, Staging, PROVISIONAL};
	let shapes = [
		Shape::Empty,
		Shape::Tuple(0),
		Shape::Tuple(1),
		Shape::Tuple(2),
		Shape::Named,
	];
	let mut staging = Staging::default();
	let mut tags = [Type::Unit; 10];
	for (i, tag) in tags.iter_mut().enumerate() {
		let shape = shapes[i % 5];
		let nominal = staging
			.stage_variant_order(
				0xc15c_0000 + i as u64,
				format_args!("C15cHeap::T{i}"),
				shape,
				if i < 5 { 0 } else { 0xc15c_ffff },
				(i % 5) as u32,
				if shape == Shape::Named {
					&["x", "y"]
				} else {
					&[]
				},
			)
			.unwrap();
		*tag = Type::Struct(nominal);
	}
	let mut remap = staging.prepare_remap().unwrap();
	staging.commit(&mut remap).unwrap();
	for tag in &mut tags {
		let Type::Struct(n) = *tag else {
			unreachable!()
		};
		if n.0 & PROVISIONAL != 0 {
			*tag = Type::Struct(remap[(n.0 & !PROVISIONAL) as usize]);
		}
	}
	tags
}

fn clone_source(store: &mut Store, tag: Type, child: Value) -> (Value, usize) {
	use crate::experimental_core::nominal::{self, Shape};
	let Type::Struct(n) = tag else { unreachable!() };
	match nominal::shape_copy(n).unwrap() {
		Shape::Empty => (store.list(tag, &[]).unwrap(), 0),
		Shape::Tuple(n) => (
			store.list(tag, &[child, child][..n as usize]).unwrap(),
			n as usize,
		),
		Shape::Named => (store.record(tag, &[("x", child), ("y", child)]).unwrap(), 2),
	}
}

fn clone_child(store: &mut Store, value: Value) -> Value {
	let Value::Heap(h) = value else {
		unreachable!()
	};
	if matches!(store.payload(h).unwrap(), Payload::Record { .. }) {
		store.field(value, "x").unwrap()
	} else {
		store.tuple_get(value, 0).unwrap()
	}
}

#[test]
fn nominal_clone_copies_outer_retains_children_and_keeps_metadata_after_staging_drop() {
	for tag in clone_tags() {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.list(Type::Vec, &[Value::I64(1)]).unwrap();
		let Value::Heap(ch) = child else {
			unreachable!()
		};
		let (source, length) = clone_source(&mut s, tag, child);
		let copy = s.clone_value(source).unwrap();
		assert_ne!(copy, source);
		assert_eq!(s.tag(copy).unwrap(), tag);
		let Value::Heap(h) = copy else { unreachable!() };
		assert_eq!(s.entry(h).unwrap().strong, 1);
		assert_eq!(s.entry(ch).unwrap().strong as usize, 1 + 2 * length);
		assert_eq!(
			s.format_value(source, true).unwrap(),
			s.format_value(copy, true).unwrap()
		);
		if length > 0 {
			let found = clone_child(&mut s, copy);
			assert_eq!(found, child);
			s.release(found).unwrap();
			s.push(child, Value::I64(2)).unwrap();
			assert_eq!(
				s.format_value(source, true).unwrap(),
				s.format_value(copy, true).unwrap()
			);
			if matches!(s.payload(h).unwrap(), Payload::Record { .. }) {
				s.set_field(copy, "x", Value::I64(9)).unwrap();
			} else {
				s.tuple_set_mut(copy, 0, Value::I64(9)).unwrap();
			}
			let found = clone_child(&mut s, source);
			assert_eq!(found, child);
			s.release(found).unwrap();
			assert_ne!(
				s.format_value(source, true).unwrap(),
				s.format_value(copy, true).unwrap()
			);
		}
		s.release(copy).unwrap();
		s.release(source).unwrap();
		assert_eq!(s.entry(ch).unwrap().strong, 1);
		s.release(child).unwrap();
		clear(&s);
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}
}

#[test]
fn nominal_clone_allocation_bytes_entries_and_overflow_restore_source_and_owners() {
	let tags = clone_tags();
	for budget_kind in 0..3 {
		let mut failures = 0;
		let mut successes = 0;
		for tag in tags {
			for budget in (0..512).step_by(16) {
				let mut s = Store::new(Limits::default()).unwrap();
				let child = s.string("retained").unwrap();
				let Value::Heap(ch) = child else {
					unreachable!()
				};
				let (source, _) = clone_source(&mut s, tag, child);
				let before = (
					s.live,
					s.bytes,
					s.edges,
					s.entry(ch).unwrap().strong,
					s.format_value(source, true).unwrap(),
				);
				let result = match budget_kind {
					0 => limit::with(budget, || s.clone_value(source)).call(),
					1 => {
						s.limits.bytes = s.bytes + budget;
						s.clone_value(source)
					}
					_ => {
						s.limits.entries = s.live + usize::from(budget > 0);
						s.clone_value(source)
					}
				};
				match result {
					Ok(copy) => {
						successes += 1;
						assert_ne!(copy, source);
						s.release(copy).unwrap();
					}
					Err(e) => {
						failures += 1;
						assert_eq!(
							e.kind,
							if budget_kind == 0 {
								ErrorKind::Allocation
							} else {
								ErrorKind::StoreCapacity
							}
						);
					}
				}
				clear(&s);
				assert_eq!(
					(
						s.live,
						s.bytes,
						s.edges,
						s.entry(ch).unwrap().strong,
						s.format_value(source, true).unwrap()
					),
					before
				);
				s.release(source).unwrap();
				s.release(child).unwrap();
				assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
			}
		}
		assert!(failures > 0 && successes > 0);
	}
	for tag in tags {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.string("overflow").unwrap();
		let Value::Heap(ch) = child else {
			unreachable!()
		};
		let (source, length) = clone_source(&mut s, tag, child);
		if length > 0 {
			let owners = s.entry(ch).unwrap().strong;
			s.entry_mut(ch).unwrap().strong = u32::MAX;
			let before = (s.live, s.bytes, s.edges);
			assert_eq!(s.clone_value(source).unwrap_err().kind, ErrorKind::Overflow);
			assert_eq!((s.live, s.bytes, s.edges), before);
			assert_eq!(s.entry(ch).unwrap().strong, u32::MAX);
			s.entry_mut(ch).unwrap().strong = owners;
		}
		s.release(source).unwrap();
		s.release(child).unwrap();
		clear(&s);
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}
}

#[test]
fn nominal_clone_readers_and_moved_children_do_not_change_shallow_depth() {
	use crate::experimental_core::error::{Detail, MovedAccess};
	for tag in clone_tags() {
		let mut s = Store::new(Limits::default()).unwrap();
		let child = s.string("child").unwrap();
		let Value::Heap(ch) = child else {
			unreachable!()
		};
		let (source, _) = clone_source(&mut s, tag, child);
		let Value::Heap(h) = source else {
			unreachable!()
		};
		s.entry_mut(h).unwrap().readers = 1;
		s.entry_mut(ch).unwrap().readers = 1;
		let copy = s.clone_value(source).unwrap();
		assert_ne!(copy, source);
		assert_eq!(s.entry(h).unwrap().readers, 1);
		assert_eq!(s.entry(ch).unwrap().readers, 1);
		s.release(copy).unwrap();
		s.entry_mut(h).unwrap().readers = 0;
		s.entry_mut(ch).unwrap().readers = 0;
		s.invalidate(child).unwrap();
		// Retaining a moved child does not inspect or recursively clone it.
		let copy = s.clone_value(source).unwrap();
		assert_eq!(s.tag(copy).unwrap(), tag);
		s.release(copy).unwrap();
		s.invalidate(source).unwrap();
		let before = (s.live, s.bytes, s.edges, s.entry(ch).unwrap().strong);
		assert_eq!(
			s.clone_value(source).unwrap_err().detail,
			Detail::Moved {
				access: MovedAccess::Read
			}
		);
		assert_eq!(
			(s.live, s.bytes, s.edges, s.entry(ch).unwrap().strong),
			before
		);
		s.release(source).unwrap();
		s.release(child).unwrap();
		clear(&s);
		assert_eq!((s.live, s.bytes, s.edges), (0, 0, 0));
	}
}

#[test]
fn nominal_script_clones_remain_readable_after_program_drop() {
	use crate::experimental_core::{execute::Executor, frontend};
	let context = crate::Context::with_default_modules().unwrap();
	let mut sources = crate::Sources::new();
	sources.insert(crate::Source::memory("struct T(a); struct N{x} enum E{T(a),N{x}} pub fn main(){let p=T([1]);let q=std::clone::clone(p);q.0.push(2);q.0=[9];let n=N{x:[3]};let m=std::clone::clone(n);m.x.push(4);m.x=[8];(p,q,n,m,std::clone::clone(E::T([5])),std::clone::clone(E::N{x:[6]}))}").unwrap()).unwrap();
	let program =
		frontend::compile(&mut sources, &context, &mut crate::Diagnostics::new(), 15).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(crate::Hash::type_hash(["main"])).unwrap(),
			&[],
			200_000,
		)
		.unwrap();
	let value = store.import(&root).unwrap();
	let before = store.format_value(value, true).unwrap();
	assert!(before.contains("[1, 2]") && before.contains("[3, 4]"));
	drop(program);
	assert_eq!(store.format_value(value, true).unwrap(), before);
	assert!(executor.is_idle());
	store.root_release(root).unwrap();
	assert_eq!((store.live, store.bytes, store.edges), (0, 0, 0));
}

#[test]
fn runtime_nominal_copy_preserves_rtti_child_aliases_and_borrow_failure_after_unit_drop() {
	use crate::runtime::{Repr, Vec as StockVec};
	let context = crate::Context::with_default_modules().unwrap();
	let mut sources = crate::Sources::new();
	sources
		.insert(
			crate::Source::memory(
				"struct T(a); struct N{x} pub fn tuple(){T([1])} pub fn named(){N{x:[1]}}",
			)
			.unwrap(),
		)
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.build_vm()
		.unwrap();
	let roots = [
		vm.call(["tuple"], ()).unwrap(),
		vm.call(["named"], ()).unwrap(),
	];
	drop(vm);
	for root in roots {
		let Repr::Dynamic(source) = root.as_ref() else {
			panic!("script nominal expected")
		};
		{
			let _guard = source.borrow_mut().unwrap();
			assert!(root.clone_().is_err());
			// Ordinary aliases do not borrow the protected payload.
			drop(root.clone());
		}
		assert!(limit::with(0, || root.clone_()).call().is_err());
		// Dynamic cloning needs no child protocol caller, even after Unit drop.
		let copy = root.clone_().unwrap();
		let Repr::Dynamic(cloned) = copy.as_ref() else {
			panic!("script nominal expected")
		};
		assert!(core::ptr::eq(&**source.rtti(), &**cloned.rtti()));
		let original = source.borrow_ref().unwrap();
		let copied = cloned.borrow_ref().unwrap();
		assert_ne!(original.as_ptr(), copied.as_ptr());
		assert_eq!(original.len(), 1);
		assert_eq!(copied.len(), 1);
		let original_child = original[0].borrow_ref::<StockVec>().unwrap();
		let copied_child = copied[0].borrow_ref::<StockVec>().unwrap();
		assert!(core::ptr::eq(&*original_child, &*copied_child));
	}
}
