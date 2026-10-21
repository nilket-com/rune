//! S4: a statically resolved call with the wrong argument count and a
//! program over a fresh-core capacity are located refusals, never
//! `InvalidCode` from code validation.
use super::*;
use crate::experimental_core::code::{Builder, Op};
use crate::experimental_core::error::Refusal;

fn compile(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory(source)?).unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 14)
}

/// The refusal and the source text it is located at.
fn refused<'a>(source: &'a str, context: &Context) -> (Refusal, &'a str) {
	let error = compile(source, context).err().expect("refused");
	let at = error.location.expect("located refusal");
	let refusal = error.refusal.expect("a refusal, not an internal error");
	(refusal, &source[at.start as usize..at.end as usize])
}

#[test]
fn wrong_arity_script_calls_refuse_at_the_call() {
	let context = Context::with_default_modules().unwrap();
	for (source, call) in [
		("fn f(a,b){a+b} pub fn main(){f(1)}", "f(1)"),
		("fn f(a,b){a+b} pub fn main(){f(1,2,3)}", "f(1,2,3)"),
		("fn f(a,b){a+b} fn unused(){f(1)} pub fn main(){7}", "f(1)"),
		("pub fn main(){f(1)} fn f(a,b){a+b}", "f(1)"),
		("fn z(){5} pub fn main(a){if a{z(1)}else{0}}", "z(1)"),
		(
			"mod m{pub fn f(a){a}} pub fn main(){let k=||m::f();7}",
			"m::f()",
		),
		(
			"struct M; impl M{fn v(self,a,b){a==b}} fn unused(m){M::v(m,1)} pub fn main(a,b){true}",
			"M::v(m,1)",
		),
		(
			"fn r(n){if n==0{0}else{r(n-1,1)}} pub fn main(){r(2)}",
			"r(n-1,1)",
		),
		("fn a(){b(1)} fn b(){1} pub fn main(){b();a()}", "b(1)"),
	] {
		assert_eq!(
			refused(source, &context),
			(Refusal::Arity, call),
			"{source}"
		);
	}
	for source in [
		"fn f(a,b){a+b} pub fn main(){f(1,2)}",
		"fn r(n){if n==0{0}else{r(n-1)}} pub fn main(){r(2)}",
		// A function value's count is checked when the call runs.
		"fn f(a,b){a+b} pub fn main(){let h=f;h(1)}",
	] {
		assert!(compile(source, &context).is_ok(), "{source}");
	}
}

#[test]
fn the_host_descriptor_table_refuses_its_overflow_at_the_call() {
	fn host(a: i64) -> i64 {
		a
	}
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["s4"]).unwrap();
	for i in 0..=crate::experimental_core::code::MAX_HOST_DESCRIPTORS {
		module
			.fresh_host_function(format!("h{i}").as_str(), host)
			.unwrap();
	}
	context.install(module).unwrap();
	let calls = |n: usize| {
		let mut source = std::string::String::from("pub fn main(){");
		for i in 0..n {
			source.push_str(&format!("s4::h{i}(1);"));
		}
		source + "7}"
	};
	let limit = crate::experimental_core::code::MAX_HOST_DESCRIPTORS;
	assert!(compile(&calls(limit), &context).is_ok());
	let source = calls(limit + 1);
	assert_eq!(
		refused(&source, &context),
		(Refusal::CodeLimit, format!("s4::h{limit}(1)").as_str())
	);
}

#[test]
fn the_code_budget_refuses_at_the_function_that_crosses_it() {
	let at = |start| Location {
		source: 0,
		revision: 1,
		start,
		end: start + 1,
	};
	// Each instruction counts 16 bytes and its 24-byte location: about 410,000
	// fill the 16 MiB budget. Two functions of 250,000 cross it in the second.
	let function = |start| {
		let mut f = Function::new(1, 0).unwrap();
		for _ in 0..250_000 {
			f.push(Op::Jump { target: 0 }, at(start)).unwrap();
		}
		f.push(Op::Return { value: 0 }, at(start)).unwrap();
		f
	};
	let mut builder = Builder::default();
	builder.push(function(10)).unwrap();
	assert!(builder.finish().is_ok());
	let mut builder = Builder::default();
	builder.push(function(10)).unwrap();
	builder.push(function(20)).unwrap();
	let error = builder.finish().err().unwrap();
	assert_eq!(error.refusal, Some(Refusal::CodeLimit));
	assert_eq!(error.location, Some(at(20)));
}

#[test]
fn a_frame_over_its_slot_limit_refuses_at_its_function() {
	let context = Context::with_default_modules().unwrap();
	let source = std::format!("fn wide(){{({})}} pub fn main(){{7}}", "1,".repeat(70_000));
	let (refusal, at) = refused(&source, &context);
	assert_eq!(refusal, Refusal::CodeLimit);
	assert!(at.starts_with("fn wide()"), "{}", &at[..20]);
}
