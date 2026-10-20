//! C17d: terminal and range certificates and the direct-chain boundary.
use super::*;
use crate::experimental_core::error::Refusal;
use crate::function::iter_terminal::{provided, range_provided, Terminal};

fn compile(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory(source)?).unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 14)
}

fn refusal(source: &str, context: &Context) -> Option<Refusal> {
	compile(source, context).err().map(|e| e.refusal.unwrap())
}

#[test]
fn default_modules_certify_every_terminal_and_range() {
	let context = Context::with_default_modules().unwrap();
	for terminal in Terminal::ALL {
		assert!(provided(&context, terminal), "{terminal:?}");
	}
	assert!(range_provided(&context));
	let empty = Context::new();
	assert!(!range_provided(&empty));
	assert!(Terminal::ALL.into_iter().all(|t| !provided(&empty, t)));
}

#[test]
fn only_direct_chains_are_admitted() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"pub fn main(){[1,2,3].iter().skip(1).take(1).count()}",
		"pub fn main(x){(x.iter().sum::<i64>(),x.iter().product::<f64>())}",
		"pub fn main(){(0..).iter().skip(2).take(3).collect::<Vec>()}",
		"pub fn main(){#{a:1}.iter().collect::<Object>()}",
		"pub fn main(){[1].iter().collect::<Tuple>()}",
		"pub fn main(){['a'].iter().collect::<String>()}",
		"pub fn main(){(std::iter::range(0,3)).take(2).sum::<u64>()}",
		"pub fn main(){[1,2].iter().skip(1).next()}",
		"pub fn main(){let n=0;for x in [1,2].iter().take(1){n+=x;}n}",
		// A stored range reaches only the existing for and next paths.
		"pub fn main(){let it=std::iter::range(0,3);let n=0;for x in it{n+=x;}(n,it.next())}",
	] {
		assert!(compile(source, &context).is_ok(), "{source}");
	}
	for source in [
		"pub fn main(){let it=[1].iter();it.count()}",
		"pub fn main(){let it=[1].iter().skip(1);it.count()}",
		"pub fn main(){let it=[1].iter().skip(1);for x in it{}}",
		"pub fn main(){let it=std::iter::range(0,3);it.sum::<i64>()}",
		"fn f(it){it.count()} pub fn main(){f([1].iter())}",
		"fn f(){[1].iter().skip(1)} pub fn main(){f().count()}",
		"pub fn main(){[1].iter().skip(1)}",
		"pub fn main(){[1].count()}",
		"pub fn main(){[1].iter().sum()}",
		"use std::collections::HashSet; pub fn main(){[1].iter().collect::<HashSet>()}",
		"pub fn main(){[1].iter().into_iter().count()}",
		"pub fn main(){let it=[1].iter();it.next();it.skip(1).next()}",
	] {
		assert_eq!(refusal(source, &context), Some(Refusal::Method), "{source}");
	}
	for source in [
		"pub fn main(){[1].iter().count(1)}",
		"pub fn main(){[1].iter().sum::<i64>(2)}",
		"pub fn main(){std::iter::range(1)}",
	] {
		assert_eq!(refusal(source, &context), Some(Refusal::Arity), "{source}");
	}
	assert_eq!(
		refusal("pub fn main(){let f=std::iter::range;f(0,1)}", &context),
		Some(Refusal::Native)
	);
	// A script `next` closes the adapter `next` and `for` certificate (C14b).
	assert_eq!(
		refusal(
			"struct P{x} impl P{fn next(self){0}} pub fn main(){[1].iter().skip(1).next()}",
			&context
		),
		Some(Refusal::Method)
	);
}

/// Native functions are insert-once: a registration outside the std iterator
/// modules at a provider hash either fails or makes the std install fail, so
/// it never certifies or replaces a provider.
#[test]
fn foreign_provider_registrations_fail_or_stay_uncertified() {
	let foreign = || {
		let mut module = crate::Module::new();
		module
			.associated_function("count", |_: &crate::runtime::slice::Iter| 0u64)
			.unwrap();
		module
	};
	// After the std modules: the foreign install fails; the builtin stays certified.
	let mut context = Context::with_default_modules().unwrap();
	assert!(context.install(foreign()).is_err());
	assert!(provided(&context, Terminal::Count));
	// Before the std slice module (with the type it needs): the std install
	// fails and leaves its markers pending, so nothing is certified.
	let mut context = Context::new();
	for module in [
		crate::modules::iter::module().unwrap(),
		crate::modules::core::module().unwrap(),
	] {
		context.install(module).unwrap();
	}
	let mut module = foreign();
	module.ty::<crate::runtime::slice::Iter>().unwrap();
	context.install(module).unwrap();
	assert!(context
		.install(crate::modules::slice::module().unwrap())
		.is_err());
	assert!(Terminal::ALL.into_iter().all(|t| !provided(&context, t)));
	assert!(!range_provided(&context));
	// A provider name on a receiver no chain reaches certifies nothing and
	// closes nothing.
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.associated_function("count", |this: bool| u64::from(this))
		.unwrap();
	context.install(module).unwrap();
	assert!(provided(&context, Terminal::Count));
	assert!(compile("pub fn main(){[1].iter().count()}", &context).is_ok());
	// A script item with a provider name does not change the std provider.
	let context = Context::with_default_modules().unwrap();
	assert!(compile(
		"struct P{x} impl P{fn sum(self){0}} fn range(a,b){a} pub fn main(){([1].iter().sum::<i64>(),range(1,2))}",
		&context
	)
	.is_ok());
}
