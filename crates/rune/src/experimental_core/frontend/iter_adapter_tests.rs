//! C17e: closure adapter certificates, function value arity and the
//! direct-chain boundary.
use super::*;
use crate::experimental_core::error::Refusal;
use crate::function::iter_terminal::{closure_provided, extended_next_provided, extended_provided};
use crate::function::iter_terminal::{Closure, Terminal};

fn compile(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory(source)?).unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 14)
}

fn refusal(source: &str, context: &Context) -> Option<Refusal> {
	compile(source, context).err().map(|e| e.refusal.unwrap())
}

#[test]
fn default_modules_certify_every_adapter_and_consumer() {
	let context = Context::with_default_modules().unwrap();
	for closure in Closure::ALL {
		assert!(closure_provided(&context, closure), "{closure:?}");
	}
	for terminal in Terminal::ALL {
		assert!(extended_provided(&context, terminal), "{terminal:?}");
	}
	assert!(extended_next_provided(&context));
	let empty = Context::new();
	assert!(Closure::ALL
		.into_iter()
		.all(|c| !closure_provided(&empty, c)));
	assert!(!extended_next_provided(&empty));
}

#[test]
fn only_direct_chains_with_known_function_values_are_admitted() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"pub fn main(){[1,2,3].iter().map(|x|x*2).filter(|x|x>2).count()}",
		"fn g(x){x+1} pub fn main(){let f=|x|x>1;([1].iter().map(g).collect::<Vec>(),[2].iter().filter(f).count())}",
		"pub fn main(){let k=2;(0..).iter().filter_map(|x|if x>k{Some(x)}else{None}).take(2).sum::<i64>()}",
		"pub fn main(){[[1],[2]].iter().flat_map(|v|v).enumerate().collect::<Vec>()}",
		"pub fn main(){[1].iter().chain([2]).fold(0,|a,x|a+x)}",
		"pub fn main(){([1,2].iter().reduce(|a,x|a*x),[1].iter().find(|x|x>0),[1].iter().map(|x|x).all(|x|x>0))}",
		"pub fn main(){(1..=4).iter().map(|x|x).rev().next()}",
		"pub fn main(){[1,2,3].iter().take(2).rev().collect::<Vec>()}",
		"pub fn main(x){x.iter().rev().count()}",
		"pub fn main(){\"a b\".split(\" \").map(|s|s+s).collect::<Vec>()}",
		"pub fn main(){let n=0;for (i,x) in [5].iter().enumerate(){n+=x;}n}",
		"struct T(a) pub fn main(){[1].iter().map(T).count()}",
	] {
		assert!(compile(source, &context).is_ok(), "{source}");
	}
	for source in [
		// Stored, passed and returned iterators, and iterator arguments.
		"pub fn main(){let it=[1].iter().map(|x|x);it.count()}",
		"fn f(it){it.count()} pub fn main(){f([1].iter().map(|x|x))}",
		"pub fn main(){[1].iter().chain([2].iter()).count()}",
		// Function values whose parameter count is not known before they run.
		"fn h(f){[1].iter().map(f).count()} pub fn main(){h(|x|x)}",
		"pub fn main(){let f=|x|x;f=|x|x+1;[1].iter().map(f).count()}",
		"pub fn main(){let o=#{f:|x|x};[1].iter().map(o.f).count()}",
		"pub fn main(){[1].iter().map(5).count()}",
		// `rev` outside its admitted shapes.
		"pub fn main(x){x.iter().map(|y|y).rev().count()}",
		"pub fn main(){[1].iter().enumerate().rev().count()}",
		"pub fn main(){\"ab\".chars().rev().count()}",
		"pub fn main(){(1..=3).iter().skip(1).rev().count()}",
		"pub fn main(){[1].iter().rev().rev().count()}",
		// Outside the admitted surface.
		"pub fn main(){[1].iter().map(|x|x).nth(0)}",
		"pub fn main(){[1].iter().map::<i64>(|x|x).count()}",
		"pub fn main(){[1].iter().map().count()}",
	] {
		assert_eq!(refusal(source, &context), Some(Refusal::Method), "{source}");
	}
	for source in [
		"pub fn main(){[1].iter().map(|a,b|a).count()}",
		"fn g(a,b){a} pub fn main(){[1].iter().filter(g).count()}",
		"pub fn main(){let f=|a|a;[].iter().fold(0,f)}",
		"pub fn main(){let k=1;[1].iter().map(move ||k).count()}",
		"struct T(a,b) pub fn main(){[1].iter().map(T).count()}",
		"pub fn main(){[1].iter().fold(|a,x|a)}",
	] {
		assert_eq!(refusal(source, &context), Some(Refusal::Arity), "{source}");
	}
	// `any` on a bare source stays the C12 small form, which takes any value.
	assert!(compile("pub fn main(f){[1].iter().any(f)}", &context).is_ok());
	assert_eq!(
		refusal("pub fn main(f){[1].iter().skip(0).any(f)}", &context),
		Some(Refusal::Method)
	);
}

/// A rival registration at a C17e provider hash either fails or leaves the
/// std iterator modules uncertified.
#[test]
fn foreign_provider_registrations_fail_or_stay_uncertified() {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.associated_function("map", |_: &crate::runtime::slice::Iter, _: i64| 0u64)
		.unwrap();
	assert!(context.install(module).is_err());
	assert!(closure_provided(&context, Closure::Map));
	// A provider name on a receiver no chain reaches closes nothing.
	let mut module = crate::Module::new();
	module
		.associated_function("fold", |this: bool| this)
		.unwrap();
	context.install(module).unwrap();
	assert!(closure_provided(&context, Closure::Fold));
	assert!(compile("pub fn main(){[1].iter().fold(0,|a,x|a+x)}", &context).is_ok());
	// A native `rev` on an iterator stock gives none closes `rev`.
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.associated_function("rev", |_: &crate::runtime::object::RuneIter| 0u64)
		.unwrap();
	context.install(module).unwrap();
	assert_eq!(
		refusal("pub fn main(){[1].iter().rev().count()}", &context),
		Some(Refusal::Method)
	);
}
