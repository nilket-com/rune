//! Frozen stock oracles and the exact-self admission boundary.
use super::*;
use crate::experimental_core::{
	execute::Executor,
	heap::{Limits, Store},
	value::Value,
};

struct Case {
	name: &'static str,
	source: &'static str,
	value: i64,
	admitted_before: bool,
	admitted_after: bool,
}

const CASES: &[Case] = &[
	Case{name:"capacity",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:true},
	Case{name:"contains",source:"struct S;impl S{fn contains(self,x){x+1}fn run(self){self.contains(8)}}pub fn main(){S.run()}",value:9,admitted_before:false,admitted_after:true},
	Case{name:"resize",source:"struct S;impl S{fn resize(self,n,x){n+x}fn run(self){self.resize(3,4)}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:true},
	Case{name:"is_empty",source:"struct S;impl S{fn is_empty(self){true}fn run(self){if self.is_empty(){1}else{0}}}pub fn main(){S.run()}",value:1,admitted_before:true,admitted_after:true},
	Case{name:"path_ingress",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){S::run(S)}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"function_value_ingress",source:"struct S;impl S{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){let f=S::run;f(S)}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"reassigned",source:"struct S;impl S{fn capacity(self){7}fn run(self){self=S;self.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"closure",source:"struct S;impl S{fn capacity(self){7}fn run(self){let f=||self.capacity();f()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"alias",source:"struct S;impl S{fn capacity(self){7}fn run(self){let s=self;s.capacity()}}pub fn main(){S.run()}",value:7,admitted_before:false,admitted_after:false},
	Case{name:"enum",source:"enum E{A,B}impl E{fn capacity(self){7}fn run(self){self.capacity()}}pub fn main(){E::A.run()+E::B.run()}",value:14,admitted_before:false,admitted_after:true},
	Case{name:"admitted_native_mix",source:"struct S;impl S{fn is_some(self){true}fn run(self){if self.is_some(){1}else{0}}}pub fn main(){S.run()}",value:1,admitted_before:true,admitted_after:true},
	Case{name:"admitted_literal",source:"struct S;impl S{fn capacity(self){7}}pub fn main(){S.capacity()}",value:7,admitted_before:true,admitted_after:true},
];

fn compile_source(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(source).unwrap())
		.unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 21)
}

#[test]
fn self_receiver_stock_oracles() {
	let context = Context::with_default_modules().unwrap();
	for c in CASES {
		let mut sources = Sources::new();
		sources
			.insert(crate::Source::memory(c.source).unwrap())
			.unwrap();
		let mut vm = crate::prepare(&mut sources)
			.with_context(&context)
			.build_vm()
			.unwrap();
		let value: i64 = crate::from_value(vm.call(["main"], ()).unwrap()).unwrap();
		assert_eq!(value, c.value, "{}", c.name);
	}
}

#[test]
fn self_receiver_frozen_frontier() {
	let context = Context::with_default_modules().unwrap();
	for c in CASES {
		let result = compile_source(c.source, &context);
		assert_eq!(result.is_ok(), c.admitted_before, "{}", c.name);
		assert!(!c.admitted_before || c.admitted_after);
		if let Ok(program) = result {
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = Executor::new().unwrap();
			let root = executor
				.invoke(
					&program,
					&mut store,
					program.function(Hash::type_hash(["main"])).unwrap(),
					&[],
					10000,
				)
				.unwrap();
			assert!(
				matches!(store.import(&root).unwrap(),Value::I64(v) if v==c.value),
				"{}",
				c.name
			);
			store.root_release(root).unwrap();
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		} else if let Err(error) = result {
			assert_eq!(error.refusal, Some(Refusal::Method), "{}", c.name);
			assert!(error.location.is_some());
		}
	}
}
