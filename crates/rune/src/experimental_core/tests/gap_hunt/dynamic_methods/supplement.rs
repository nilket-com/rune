// Live checks outside the frozen oracle: the frame limit on plain recursion,
// and a dispatched name on a host-returned type the context registers.

use crate as rune;

/// A type only the context registers, returned by a dynamic host function.
#[derive(crate::Any)]
#[rune(item = ::c18b)]
struct Gadget;

fn gadget() -> Value {
	Value::new(Gadget).unwrap()
}

/// A native instance method on `Gadget`, a name the script below also defines.
#[rune::function(instance)]
fn knob(_: &Gadget) -> i64 {
	500
}

fn supplement_context(fresh: bool) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c18b"]).unwrap();
	module.ty::<Gadget>().unwrap();
	module.function_meta(knob).unwrap();
	if fresh {
		module.fresh_host_function("gadget", gadget).unwrap();
	} else {
		module.function("gadget", gadget).build().unwrap();
	}
	context.install(module).unwrap();
	context
}

fn supplement_stock(program: &str, fuel: usize) -> Result<Value, stock::VmError> {
	let context = supplement_context(false);
	let mut sources = Sources::new();
	sources
		.insert(Source::new("supplement.rn", program).unwrap())
		.unwrap();
	let mut vm = crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut Diagnostics::new())
		.build_vm()
		.unwrap();
	crate::runtime::budget::with(fuel, || vm.call(["main"], ())).call()
}

fn supplement_fresh(
	program: &str,
	fuel: usize,
) -> core::result::Result<
	core::result::Result<(), crate::experimental_core::execute::Failure>,
	crate::experimental_core::error::Error,
> {
	supplement_fresh_with_limits(program, fuel, None)
}

fn supplement_fresh_with_limits(
	program: &str,
	fuel: usize,
	limits: Option<crate::experimental_core::execute::ExecutorLimits>,
) -> core::result::Result<
	core::result::Result<(), crate::experimental_core::execute::Failure>,
	crate::experimental_core::error::Error,
> {
	let context = supplement_context(true);
	let mut sources = Sources::new();
	sources
		.insert(Source::new("supplement.rn", program).unwrap())
		.unwrap();
	let built = frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 16)?;
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = Executor::with_limits(limits.unwrap_or_default()).unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let main = built.function(Hash::type_hash(["main"])).unwrap();
	let result = executor
		.invoke(&built, &mut store, main, &[], fuel)
		.map(|root| store.root_release(root).unwrap());
	assert!(executor.is_idle());
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	Ok(result)
}

#[test]
fn plain_recursion_meets_the_frame_limit_like_dispatch() {
	use crate::experimental_core::error::ErrorKind;
	let frames = 1024;
	for program in [
		"fn spin(n){spin(n+1)}pub fn main(){spin(0)}",
		"struct P{x}impl P{fn spin(self){self.spin()}}fn f(p){p.spin()}pub fn main(){f(P{x:1})}",
	] {
		let stock = supplement_stock(program, 2000).unwrap_err();
		assert!(matches!(
			stock.error().kind(),
			crate::runtime::VmErrorKind::Halted { .. }
		));
		let failure = supplement_fresh_with_limits(program, 2000, Some(crate::experimental_core::execute::ExecutorLimits { frames: 1024, slots: 65_536 })).unwrap().unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::FrameCapacity, "{program}");
		// Every caller is in the chain; the frame that failed to call is not.
		assert_eq!(failure.call_chain.len() + 1, frames, "{program}");
	}
}

#[test]
fn host_returned_context_type_reaches_dispatch_only_through_the_boundary() {
	use crate::experimental_core::detail::Detail;
	use crate::experimental_core::error::HostDynamicReason;
	use crate::experimental_core::error::{ErrorKind, Refusal};
	// No native `twist` anywhere: stock misses on Gadget; fresh refuses the
	// Gadget at the host boundary (C10), before any dispatch.
	let twist = "struct P{x}impl P{fn twist(self){1}}fn f(x){x.twist()}pub fn main(){f(c18b::gadget())}";
	let stock = supplement_stock(twist, 200_000).unwrap_err();
	assert!(matches!(
		stock.error().kind(),
		crate::runtime::VmErrorKind::MissingInstanceFunction { .. }
	));
	let failure = supplement_fresh(twist, 200_000).unwrap().unwrap_err();
	assert_eq!(failure.error.kind, ErrorKind::Host);
	assert!(matches!(
		failure.error.detail,
		Detail::HostDynamic {
			reason: HostDynamicReason::Unsupported,
			..
		}
	));
	// A native `knob` on the context type: stock calls it; fresh refuses the
	// name at compile time, a proof that covers context types.
	let knob = "struct P{x}impl P{fn knob(self){1}}fn f(x){x.knob()}pub fn main(){f(c18b::gadget())}";
	assert_eq!(
		crate::from_value::<i64>(supplement_stock(knob, 200_000).unwrap()).unwrap(),
		500
	);
	let refused = supplement_fresh(knob, 200_000).unwrap_err();
	assert_eq!(refused.refusal, Some(Refusal::Method));
}
