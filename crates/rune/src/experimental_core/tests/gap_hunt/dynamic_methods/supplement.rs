// Live checks outside the contract cases: the frame limit on plain recursion,
// and a dispatched name on a host-returned type the context registers.

use crate as rune;

/// A type only the context registers, returned by a dynamic host function.
#[derive(crate::Any)]
#[rune(item = ::c18b)]
struct Gadget;

fn gadget() -> RuntimeValue {
	RuntimeValue::new(Gadget).unwrap()
}

/// A native instance method on `Gadget`, a name the script below also defines.
#[rune::function(instance)]
fn knob(_: &Gadget) -> i64 {
	500
}

fn supplement_context(certify: bool) -> Context {
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c18b"]).unwrap();
	module.ty::<Gadget>().unwrap();
	module.function_meta(knob).unwrap();
	if certify {
		module.host_function("gadget", gadget).unwrap();
	} else {
		module.function("gadget", gadget).build().unwrap();
	}
	context.install(module).unwrap();
	context
}

fn legacy_supplement(program: &str, fuel: usize) -> Result<RuntimeValue, stock::VmError> {
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

fn supplement(
	program: &str,
	fuel: usize,
) -> core::result::Result<
	core::result::Result<(), crate::experimental_core::execute::Failure>,
	crate::experimental_core::error::Error,
> {
	supplement_with_limits(program, fuel, None)
}

fn supplement_with_limits(
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
		let stock = legacy_supplement(program, 2000).unwrap_err();
		assert!(matches!(
			stock.error().kind(),
			crate::runtime::VmErrorKind::Halted { .. }
		));
		let failure = supplement_with_limits(
			program,
			2000,
			Some(crate::experimental_core::execute::ExecutorLimits {
				frames: 1024,
				slots: 65_536,
			}),
		)
		.unwrap()
		.unwrap_err();
		assert_eq!(failure.error.kind, ErrorKind::FrameCapacity, "{program}");
		// Every caller is in the chain; the frame that failed to call is not.
		assert_eq!(failure.call_chain.len() + 1, frames, "{program}");
	}
}

#[test]
fn host_returned_context_type_reaches_dispatch_only_through_the_boundary() {
	use crate::experimental_core::error::Refusal;
	// No native `twist` anywhere: stock misses on Gadget; new refuses the
	// undeclared Gadget output at compile time, before any host call.
	let twist =
		"struct P{x}impl P{fn twist(self){1}}fn f(x){x.twist()}pub fn main(){f(c18b::gadget())}";
	let stock = legacy_supplement(twist, 200_000).unwrap_err();
	assert!(matches!(
		stock.error().kind(),
		crate::runtime::VmErrorKind::MissingInstanceFunction { .. }
	));
	let failure = supplement(twist, 200_000).unwrap_err();
	assert_eq!(failure.refusal, Some(Refusal::Native));

	// A native `knob` on the context type: stock calls it; new refuses the
	// undeclared output first at compile time, before method proof resolution.
	let knob =
		"struct P{x}impl P{fn knob(self){1}}fn f(x){x.knob()}pub fn main(){f(c18b::gadget())}";
	assert_eq!(
		crate::from_value::<i64>(legacy_supplement(knob, 200_000).unwrap()).unwrap(),
		500
	);
	let refused = supplement(knob, 200_000).unwrap_err();
	assert_eq!(refused.refusal, Some(Refusal::Native));
}
