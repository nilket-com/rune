//! C17c: provider certificates, the static receiver proof and std constants.
use super::*;
use crate::experimental_core::error::Refusal;
use crate::function::fresh_number::{Method, PAIRS};

fn compile(source: &str, context: &Context) -> Result<Program> {
	let mut sources = Sources::new();
	sources.insert(crate::Source::memory(source)?).unwrap();
	super::compile(&mut sources, context, &mut Diagnostics::new(), 13)
}

fn refusal(source: &str, context: &Context) -> Option<Refusal> {
	compile(source, context).err().map(|e| e.refusal.unwrap())
}

#[test]
fn default_modules_certify_every_provider() {
	let context = Context::with_default_modules().unwrap();
	for pair in PAIRS {
		assert!(context.fresh_number_pair(pair), "{pair:?}");
	}
	for m in Method::ALL {
		assert!(context.fresh_number_method(m), "{m:?}");
	}
}

#[test]
fn shared_names_need_a_proven_receiver() {
	let context = Context::with_default_modules().unwrap();
	for source in [
		"pub fn main(){1.0.max(2.0)}",
		"pub fn main(){let x=2;x=x.max(3);x.min(1)}",
		"pub fn main(){let x=1.5;x+=1.0;x.partial_cmp(2.0)}",
		"pub fn main(){(2u64*3u64).max(1u64)}",
		"pub fn main(){let x=1.0;for i in 0..3{x=x.max(2.0).sqrt();}x}",
		"pub fn main(){f64::NAN.max(1.0).floor().round().abs().powi(2).ceil().exp().ln().sqrt().powf(1.0).min(2.0)}",
		"pub fn main(){i64::MAX.min(1).wrapping_add(2).saturating_mul(3).signum()}",
	] {
		assert!(compile(source, &context).is_ok(), "{source}");
	}
	for source in [
		"pub fn main(x){x.max(2)}",
		"pub fn main(){let x=2.0;x=[1];x.max([0])}",
		"pub fn main(){let x=2.0;x|=1;x.max(1.0)}",
		"pub fn main(){(if true{1}else{2}).max(1)}",
		"pub fn main(){let v=[1.0];v[0].partial_cmp(2.0)}",
		"pub fn main(){'a'.max('b')}",
		"pub fn main(){1.partial_cmp(2)}",
		"pub fn main(){1.max::<i64>(2)}",
		"pub fn main(){1.checked_add::<i64>(2)}",
	] {
		assert_eq!(refusal(source, &context), Some(Refusal::Method), "{source}");
	}
	assert_eq!(
		refusal("pub fn main(){1.max()}", &context),
		Some(Refusal::Arity)
	);
	assert_eq!(
		refusal("pub fn main(){let f=f64::sqrt;f(1.0)}", &context),
		Some(Refusal::Native)
	);
}

#[test]
fn unproven_receivers_need_an_absence_proof() {
	let context = Context::with_default_modules().unwrap();
	assert!(compile("pub fn main(x){x.sqrt()}", &context).is_ok());
	assert!(compile("pub fn main(x){x.checked_add(1)}", &context).is_ok());
	// A script method of the same name on any type closes the name.
	assert_eq!(
		refusal(
			"struct P{x} impl P{fn sqrt(self){0}} pub fn main(x){x.sqrt()}",
			&context
		),
		Some(Refusal::Method)
	);
	// A proven receiver does not need the absence proof.
	assert!(compile(
		"struct P{x} impl P{fn sqrt(self){0}} pub fn main(){4.0.sqrt()}",
		&context
	)
	.is_ok());
	// A host registration of the name on another receiver closes it too.
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::new();
	module
		.associated_function("sqrt", |this: bool| this)
		.unwrap();
	context.install(module).unwrap();
	assert_eq!(
		refusal("pub fn main(x){x.sqrt()}", &context),
		Some(Refusal::Method)
	);
	assert!(compile("pub fn main(){4.0.sqrt()}", &context).is_ok());
}

#[test]
fn std_constants_are_certified_against_the_context() {
	let context = Context::with_default_modules().unwrap();
	assert!(compile(
		"pub fn main(){(i64::MAX,i64::MIN,u64::MAX,f64::NAN,f64::INFINITY,f64::NEG_INFINITY,f64::MAX,f64::MIN,f64::MIN_POSITIVE,f64::EPSILON)}",
		&context
	)
	.is_ok());
	assert_eq!(
		refusal("pub fn main(){u64::MIN}", &context),
		Some(Refusal::UnresolvedConstant)
	);
	assert_eq!(
		refusal("pub fn main(){f64::consts::PI}", &context),
		Some(Refusal::UnresolvedConstant)
	);
}
