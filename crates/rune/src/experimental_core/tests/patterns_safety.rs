//! C3 construction, ownership and fresh-fuel checks; no stock-budget parity.
use crate::experimental_core::{
	code::{Builder, Function},
	error::{ErrorKind, Location},
	execute::Executor,
	frontend::{self, Program},
	heap::{Limits, Store},
	value::{Handle, Scalar, Type, Value},
};
use crate::{Context, Diagnostics, Hash, Source, Sources};

#[test]
#[cfg_attr(miri, ignore = "P3 default-Context diagnostic negatives; native suite")]
fn moved_index_set_context_is_vec_only() {
	use crate::experimental_core::error::{Detail, MovedAccess};
	let context = Context::with_default_modules().unwrap();
	for (body, vector, kind) in [
		(
			"let v=[1];let a=v;drop(a);v[0]=9;",
			true,
			ErrorKind::AccessConflict,
		),
		(
			"let v=(1,);let a=v;drop(a);v.0=9;",
			false,
			ErrorKind::AccessConflict,
		),
		("let v=(1,);let a=v;drop(a);v[0]=9;", false, ErrorKind::Type),
		(
			"let v=#{x:1};let a=v;drop(a);v[\"x\"]=9;",
			false,
			ErrorKind::AccessConflict,
		),
	] {
		let mut sources = Sources::new();
		sources
			.insert(Source::memory(std::format!("pub fn main(){{{body}}}")).unwrap())
			.unwrap();
		let program =
			frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 9).unwrap();
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		for _ in 0..2 {
			let failure = executor
				.invoke(&program, &mut store, main, &[], 1000)
				.unwrap_err();
			assert_eq!(failure.error.kind, kind, "{body}");
			if kind == ErrorKind::Type {
				assert!(!matches!(
					failure.error.detail,
					Detail::MovedArgument { .. }
				));
				assert!(failure.cleanup_error.is_none());
				idle(&executor, &store);
				continue;
			}
			let expected = if vector {
				Detail::MovedArgument {
					access: MovedAccess::Write,
					arg: 0,
				}
			} else {
				Detail::Moved {
					access: MovedAccess::Write,
				}
			};
			assert_eq!(failure.error.detail, expected, "{body}");
			assert!(failure.cleanup_error.is_none());
			idle(&executor, &store);
		}
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "C16a opt-in minimal-Context Vec local ownership witness"
)]
fn vec_local_host_shapes_moved_values_and_reuse() {
	let mut rows = 0;
	for (pattern, result, lengths) in [
		("[a,b]", "a+b", &[2usize, 1][..]),
		("[..]", "7", &[0usize][..]),
		("[a,..]", "a", &[2usize, 0][..]),
	] {
		let source = std::format!("pub fn main(v){{let{pattern}=v;{result}}}");
		let program = compile(&source);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		for &len in lengths {
			let mut store = Store::new(Limits::default()).unwrap();
			let a = store.root_i64(1);
			let b = store.root_i64(2);
			let values = [&a, &b];
			let vector = store.root_list(Type::Vec, &values[..len]).unwrap();
			let mut executor = Executor::new().unwrap();
			let expected = match pattern {
				"[a,b]" if len == 2 => Some(3),
				"[..]" => Some(7),
				"[a,..]" if len != 0 => Some(1),
				_ => None,
			};
			match executor.invoke(&program, &mut store, main, &[&vector], 100) {
				Ok(root) => {
					assert_eq!(store.root_scalar(&root).unwrap(), expected.map(Scalar::I64));
					store.root_release(root).unwrap();
				}
				Err(failure) => {
					assert_eq!(expected, None);
					assert_eq!(failure.error.kind, ErrorKind::Panic);
					assert!(failure.cleanup_error.is_none());
				}
			}
			rows += 1;
			store.invalidate(vector.value).unwrap();
			let failure = executor
				.invoke(&program, &mut store, main, &[&vector], 100)
				.unwrap_err();
			assert_eq!(failure.error.kind, ErrorKind::AccessConflict);
			assert_eq!(
				std::format!("{}", failure.message(&program)),
				"Cannot read the value: it has been moved"
			);
			let at = failure.error.location.unwrap();
			let start = source.find(pattern).unwrap() as u32;
			assert_eq!((at.start, at.end), (start, start + pattern.len() as u32));
			assert!(failure.cleanup_error.is_none());
			rows += 1;
			store.root_release(vector).unwrap();
			store.root_release(a).unwrap();
			store.root_release(b).unwrap();
			idle(&executor, &store);
		}
	}
	let program = compile("pub fn main(){let[(a,b),[c,..]]=[(1,2),[3]];a+b+c}");
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	for _ in 0..2 {
		let root = executor
			.invoke(&program, &mut store, main, &[], 100)
			.unwrap();
		assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(6)));
		store.root_release(root).unwrap();
		idle(&executor, &store);
		rows += 1;
	}
	assert_eq!(rows, 12);
	println!("C16a Vec local Miri rows {rows}");
}

fn compile(source: &str) -> Program {
	// The tuple/Vec-only fixtures need no std module installation.
	let context = Context::new();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	frontend::compile(&mut sources, &context, &mut diagnostics, 9)
		.unwrap_or_else(|e| panic!("{e:?}: {:?}", diagnostics.diagnostics()))
}

fn idle(executor: &Executor, store: &Store) {
	assert!(executor.is_idle());
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
#[cfg_attr(miri, ignore = "C16c opt-in minimal-Context expression-call ownership")]
fn expression_calls_order_failures_and_reuse() {
	for (source, expected) in [
		(
			"fn inc(x){x+1}fn make(){inc}pub fn main(){make()(6)}",
			Ok(7),
		),
		(
			"fn inc(x){x+1}fn dec(x){x-1}pub fn main(){let f=inc;(f)({f=dec;7})}",
			Ok(6),
		),
		(
			"fn inc(x){x+1}fn dec(x){x-1}pub fn main(){let f=inc;({f})({f=dec;7})}",
			Ok(8),
		),
		("pub fn main(){let n=7;let fs=[||n];fs[0]()}", Ok(7)),
		(
			"fn make(v){move||v[0]}pub fn main(){let v=[7];let n=make(v)();v[0]=9;n}",
			Err(ErrorKind::AccessConflict),
		),
		(
			"fn inc(x){x+1}fn make(){inc}pub fn main(){make()(1/0)}",
			Err(ErrorKind::DivisionByZero),
		),
		(
			"fn inc(x){x+1}fn make(){inc}pub fn main(){make()()}",
			Err(ErrorKind::ArgumentCount),
		),
		("pub fn main(){({7})([1])}", Err(ErrorKind::Type)),
	] {
		let program = compile(source);
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		for _ in 0..2 {
			match executor.invoke(&program, &mut store, main, &[], 1000) {
				Ok(root) => {
					assert_eq!(
						store.root_scalar(&root).unwrap(),
						expected.ok().map(Scalar::I64)
					);
					store.root_release(root).unwrap();
				}
				Err(failure) => {
					assert_eq!(Some(failure.error.kind), expected.err());
					assert!(failure.error.location.is_some());
					assert!(failure.cleanup_error.is_none());
				}
			}
			idle(&executor, &store);
		}
	}
}

#[test]
fn malformed_handles_refuse_all_pattern_access_primitives() {
	let mut store = Store::new(Limits::default()).unwrap();
	let bad = Value::Heap(Handle {
		slot: u32::MAX,
		generation: 0,
	});
	assert_eq!(store.tag(bad).unwrap_err().kind, ErrorKind::InvalidHandle);
	assert_eq!(store.len(bad).unwrap_err().kind, ErrorKind::InvalidHandle);
	assert_eq!(
		store.tuple_get(bad, 0).unwrap_err().kind,
		ErrorKind::InvalidHandle
	);
	assert_eq!(store.live_entries(), 0);
	assert_eq!(store.payload_bytes(), 0);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "C3 opt-in minimal-Context source cleanup; measured separately"
)]
fn missed_bindings_guard_false_and_loop_reuse_release_every_owner() {
	let p=compile("pub fn main(){let a=([1],1);let n=0;while let (x,1)=a{n+=x[0];a=([2],2);}match [n]{[x] if false=>x,[y]=>y,_=>0}}");
	let main = p.function(Hash::type_hash(["main"])).unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::new().unwrap();
	for _ in 0..3 {
		let root = executor.invoke(&p, &mut store, main, &[], 100).unwrap();
		assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(1)));
		store.root_release(root).unwrap();
		idle(&executor, &store);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "C3 opt-in minimal-Context source early exits; measured separately"
)]
fn nested_guard_return_break_continue_and_error_balance() {
	for (source,expected) in [
		("pub fn main(){match ([1],2){(x,2) if {return x[0];}=>0,_=>9}}",Ok(1)),
		("pub fn main(){'outer:loop{match ([1],2){(x,2) if {break 'outer x[0];}=>0,_=>9};}}",Ok(1)),
		("pub fn main(){let i=0;'outer:while i<3{i+=1;match [i]{[x] if {continue 'outer;}=>0,_=>9};}i}",Ok(3)),
		("pub fn main(){match ([1],2){(x,2) if 1/0==0=>x[0],_=>9}}",Err(ErrorKind::DivisionByZero)),
	] {
		let p=compile(source);let main=p.function(Hash::type_hash(["main"])).unwrap();
		let mut store=Store::new(Limits::default()).unwrap();let mut executor=Executor::new().unwrap();
		match (executor.invoke(&p,&mut store,main,&[],100),expected) {
			(Ok(root),Ok(n))=>{assert_eq!(store.root_scalar(&root).unwrap(),Some(Scalar::I64(n)));store.root_release(root).unwrap();}
			(Err(failure),Err(kind))=>{assert_eq!(failure.error.kind,kind);assert!(failure.cleanup_error.is_none());}
			_=>panic!("unexpected early-exit outcome"),
		}
		idle(&executor,&store);
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "C3 opt-in minimal-Context host tuple boundary; measured separately"
)]
fn host_empty_nonempty_and_moved_tuple_patterns_keep_access_order() {
	for (pattern, expected) in [("()", [11, 22]), ("(..)", [11, 11])] {
		let source = std::format!("pub fn main(a){{match a{{{pattern}=>11,_=>22}}}}");
		let p = compile(&source);
		let main = p.function(Hash::type_hash(["main"])).unwrap();
		for (len, n) in expected.into_iter().enumerate() {
			let mut store = Store::new(Limits::default()).unwrap();
			let scalar = store.root_i64(1);
			let refs = if len == 0 {
				std::vec![]
			} else {
				std::vec![&scalar]
			};
			let tuple = store.root_list(Type::Tuple, &refs).unwrap();
			let mut executor = Executor::new().unwrap();
			let root = executor
				.invoke(&p, &mut store, main, &[&tuple], 100)
				.unwrap();
			assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(n)));
			store.root_release(root).unwrap();
			store.invalidate(tuple.value).unwrap();
			let failed = executor
				.invoke(&p, &mut store, main, &[&tuple], 100)
				.unwrap_err();
			assert_eq!(failed.error.kind, ErrorKind::AccessConflict);
			assert_eq!(
				std::format!("{}", failed.message(&p)),
				"Cannot read the value: it has been moved"
			);
			let at = failed.error.location.unwrap();
			let start = source.find(pattern).unwrap() as u32;
			assert_eq!((at.start, at.end), (start, start + pattern.len() as u32));
			assert!(failed.cleanup_error.is_none());
			store.root_release(tuple).unwrap();
			store.root_release(scalar).unwrap();
			idle(&executor, &store);
		}
	}
}

// Rebuild only SOURCE LOCATIONS as pc identifiers. No production hook or
// instruction is added. Decode/re-encode preserves instruction count/charges.
fn pc_locations(program: &Program) -> Program {
	let mut builder = Builder::default();
	for id in 0..program.code.functions() {
		let original = program.code.function(id as u32).unwrap();
		let mut function = Function::new(original.slots, original.parameters).unwrap();
		for text in &original.strings {
			function.string(text).unwrap();
		}
		function.arguments(&original.args).unwrap();
		for (pc, instruction) in original.instructions().iter().enumerate() {
			function
				.push(
					original.decode(*instruction).unwrap(),
					Location {
						source: id as u32,
						revision: 9,
						start: pc as u32,
						end: pc as u32 + 1,
					},
				)
				.unwrap();
		}
		builder.push(function).unwrap();
	}
	let code = builder.finish().unwrap();
	for id in 0..code.functions() {
		let old = program.code.function(id as u32).unwrap();
		let new = code.function(id as u32).unwrap();
		assert_eq!(old.instructions().len(), new.instructions().len());
		for (a, b) in old.instructions().iter().zip(new.instructions()) {
			assert_eq!(a.is_charged(), b.is_charged());
			assert_eq!(
				std::format!("{:?}", old.decode(*a).unwrap()),
				std::format!("{:?}", new.decode(*b).unwrap())
			);
		}
	}
	Program::for_diagnostics_test(code)
}

#[test]
#[cfg_attr(
	miri,
	ignore = "C3 source fuel-frontier sweep is native-only; focused cleanup Miri separate"
)]
fn exact_fresh_fuel_frontiers_keep_pc_and_balanced_state() {
	let mut wide = std::string::String::from("pub fn main(){match 999{");
	for n in 0..80 {
		wide.push_str(&std::format!("{n}=>{n},"));
	}
	wide.push_str("_=>99}}");
	for (name, source, value) in [
		(
			"small",
			"pub fn main(){match ([1],2){(x,3)=>0,(x,y) if false=>y,(x,y)=>x[0]+y,_=>9}}",
			3,
		),
		(
			"loop",
			"pub fn main(){let a=([1],1);let n=0;while let (x,1)=a{n+=x[0];a=([2],2);}n}",
			1,
		),
		("wide", wide.as_str(), 99),
	] {
		let original = compile(source);
		let main = original.function(Hash::type_hash(["main"])).unwrap();
		let program = pc_locations(&original);
		let function = program.code.function(main).unwrap();
		if name == "wide" {
			assert!(function.instructions().len() > 256);
		}
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		let mut frontiers = std::vec::Vec::new();
		let mut minimum = None;
		for fuel in 0..=20 {
			match executor.invoke(&program, &mut store, main, &[], fuel) {
				Ok(root) => {
					assert_eq!(store.root_scalar(&root).unwrap(), Some(Scalar::I64(value)));
					store.root_release(root).unwrap();
					if minimum.is_none() {
						minimum = Some(fuel);
					}
				}
				Err(failed) => {
					assert!(minimum.is_none());
					assert_eq!(failed.error.kind, ErrorKind::FuelExhausted);
					assert!(failed.cleanup_error.is_none());
					let pc = failed.error.location.unwrap();
					assert_eq!(pc.source, main);
					assert!(function.instructions()[pc.start as usize].is_charged());
					frontiers.push(pc.start);
				}
			}
			idle(&executor, &store);
		}
		let minimum = minimum.expect("finite fresh fuel bound");
		assert_eq!(frontiers.len(), minimum);
		std::println!("C3 FUEL {name} minimum={minimum} pcs={frontiers:?}");
		match name {
			"small" => {
				assert_eq!(minimum, 1);
				assert_eq!(frontiers, [0]);
			}
			"loop" => {
				assert_eq!(minimum, 3);
				assert_eq!(frontiers, [0, 6, 6]);
			}
			"wide" => {
				assert_eq!(minimum, 2);
				assert_eq!(frontiers, [0, 359]);
			}
			_ => unreachable!(),
		}
	}
}

#[test]
#[cfg_attr(miri, ignore = "C3 opt-in ownership witness; measured separately")]
fn block_if_let_binding_survives_root_release_and_guard_break_releases_temps() {
	for source in [
		"pub fn main(){let a=([1],);if let (x,)={a}{x[0]=9;x[0]}else{0}}",
		"pub fn main(){'outer:loop{match ([1],){(x,) if {let t=[x, [9]];break 'outer t[1][0];}=>0,_=>0};}}",
	] {
		let p=compile(source);
		let main=p.function(Hash::type_hash(["main"])).unwrap();
		let mut store=Store::new(Limits::default()).unwrap();
		let mut executor=Executor::new().unwrap();
		let root=executor.invoke(&p,&mut store,main,&[],100).unwrap();
		assert_eq!(store.root_scalar(&root).unwrap(),Some(Scalar::I64(9)));
		store.root_release(root).unwrap();
		idle(&executor,&store);
	}
}

#[test]
#[cfg_attr(miri, ignore = "C3 source fuel sweep; focused cleanup Miri separate")]
fn pattern_kinds_guards_and_nested_loop_exits_balance_at_every_fuel_frontier() {
	let context = Context::with_default_modules().unwrap();
	for (name,source,expected) in [
		("kinds", "struct P{a} pub fn main(){match (true,5u64,'a',\"x\",Some([1]),P{a:2}){(true,5u64,'a',\"x\",Some([x]),P{a:y})=>x+y,_=>0}}",3),
		("false-guards", "pub fn main(){match ([9],){(x,) if {let t=[x];false}=>0,(x,) if {let t=[x];false}=>0,(x,) if {let t=[x];false}=>0,(x,)=>x[0],_=>0}}",9),
		("loop-three", "pub fn main(){let a=([0],1);while let (x,1)=a{let n=x[0]+1;a=([n],if n<3{1}else{0});}a.0[0]}",3),
		("guard-return", "pub fn main(){match ([9],){(x,)=>{let a=([1],);while let (y,)=a{match y{[..] if {let t=[x,y];return t[0][0];}=>0,_=>0};}0},_=>0}}",9),
		("guard-break", "pub fn main(){match ([9],){(x,)=>{'outer:while let (y,)=([1],){match y{[..] if {let t=[x,y];break 'outer t[0][0];}=>0,_=>0};}},_=>0}}",9),
		("guard-continue", "pub fn main(){let n=0;match ([9],){(x,)=>{let a=([1],1);'outer:while let (y,1)=a{n+=1;if n==3{a=([1],0);}match y{[..] if {let t=[x,y];continue 'outer;}=>0,_=>0};}n},_=>0}}",3),
		("body-return", "pub fn main(){match ([9],){(x,)=>{while let (y,)=([1],){match y{[..]=>return x[0],_=>0};}0},_=>0}}",9),
	] {
		let mut sources=Sources::new();sources.insert(Source::memory(source).unwrap()).unwrap();
		let original=frontend::compile(&mut sources,&context,&mut Diagnostics::new(),9).unwrap();
		let main=original.function(Hash::type_hash(["main"])).unwrap();
		let program=pc_locations(&original);
		let mut store=Store::new(Limits::default()).unwrap();let mut executor=Executor::new().unwrap();
		let mut frontiers=std::vec::Vec::new();let mut minimum=None;
		for fuel in 0..=32 {
			match executor.invoke(&program,&mut store,main,&[],fuel) {
				Ok(root)=>{assert_eq!(store.root_scalar(&root).unwrap(),Some(Scalar::I64(expected)),"{name}");store.root_release(root).unwrap();minimum.get_or_insert(fuel);}
				Err(failure)=>{assert!(minimum.is_none(),"{name}");assert_eq!(failure.error.kind,ErrorKind::FuelExhausted,"{name}");assert!(failure.cleanup_error.is_none());let pc=failure.error.location.unwrap();assert!(program.code.function(pc.source).unwrap().instructions()[pc.start as usize].is_charged());frontiers.push((pc.source,pc.start));}
			}
			idle(&executor,&store);
		}
		assert!(minimum.is_some(),"finite fuel bound: {name}");
		std::println!("C3 FUEL {name} minimum={minimum:?} pcs={frontiers:?}");
	}
}

#[test]
#[cfg_attr(miri, ignore = "C16b opt-in minimal-Context loop ownership witness")]
fn loop_binders_control_flow_failures_and_reuse() {
	let cases = [
		("let n=0;for(a,b)in[]{n+=a+b;}n", Ok(0)),
		("let n=0;for(a,b)in[(1,2),(3,4)]{n+=a+b;}n", Ok(10)),
		("let n=0;for[a,b]in[[1,2],[3,4]]{n+=a+b;}n", Ok(10)),
		(
			"let n=0;for(a,[b,c])in[(1,[2,3]),(4,[5,6])]{n+=a+b+c;}n",
			Ok(21),
		),
		("let n=0;for[a,..]in[[1],[2,3]]{n+=a;}n", Ok(3)),
		("let n=0;for(_,b)in[([1],2),([3],4)]{n+=b;}n", Ok(6)),
		(
			"let n=0;for(a,b)in[(1,2),(3,4)]{if a==1{continue;}n+=a+b;}n",
			Ok(7),
		),
		("let n=0;for(a,b)in[(1,2),(3,4)]{n+=a+b;break;}n", Ok(3)),
		(
			"let n=0;'outer:for(a,b)in[(1,2),(3,4)]{for x in [1,2]{n+=a+b;continue 'outer;}}n",
			Ok(10),
		),
		("for(a,b)in[(1,2),(3,)]{}7", Err(ErrorKind::Panic)),
		("for[a,b]in[[1,2],[3,4]]{return a+b;}0", Ok(3)),
		(
			"for(a,b)in[(1,2)]{let z=0;return a/z;}0",
			Err(ErrorKind::DivisionByZero),
		),
	];
	let mut rows = 0;
	for (body, expected) in cases {
		let program = compile(&std::format!("pub fn main(){{{body}}}"));
		let main = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let mut executor = Executor::new().unwrap();
		for _ in 0..2 {
			match executor.invoke(&program, &mut store, main, &[], 1000) {
				Ok(root) => {
					assert_eq!(
						store.root_scalar(&root).unwrap(),
						Some(Scalar::I64(expected.unwrap()))
					);
					store.root_release(root).unwrap();
				}
				Err(failure) => {
					assert_eq!(Err(failure.error.kind), expected);
					assert!(failure.cleanup_error.is_none());
				}
			}
			idle(&executor, &store);
			rows += 1;
		}
	}
	assert_eq!(rows, 24);
	println!("C16b loop Miri rows {rows}");
}
