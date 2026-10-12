//! Soft outer VM yields must not truncate a native call's hard total budget.
prelude!();
use crate::runtime::{budget, VmOutcome};

fn execute(source: &str, total: usize, slice: usize) -> (Result<i64, String>, usize, usize) {
	let context = Context::with_default_modules().unwrap();
	let mut sources = crate::tests::sources(source);
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::tests::vm(&context, &mut sources, &mut diagnostics, false).unwrap();
	let mut execution = vm.execute(["main"], ()).unwrap();
	let mut left = total;
	let mut yields = 0;
	loop {
		let (outcome, remaining) = budget::with_slice(left, slice, || {
			let outcome = execution.resume().complete();
			(outcome, budget::remaining().unwrap())
		})
		.call();
		assert!(remaining <= left);
		left = remaining;
		match outcome {
			Ok(VmOutcome::Complete(value)) => {
				return (
					crate::from_value(value).map_err(|e| e.to_string()),
					left,
					yields,
				)
			}
			Ok(VmOutcome::Limited) if left > 0 => yields += 1,
			Ok(VmOutcome::Limited) => return (Err("hard limit".into()), left, yields),
			Err(error) => return (Err(error.to_string()), left, yields),
			_ => panic!("unexpected yielded outcome"),
		}
	}
}

#[test]
fn native_calls_cross_soft_slices_and_nested_closures_share_the_total() {
	for (body, expected) in [
		("(0..100000).iter().count()", 100000),
		("(0..100000).iter().fold(0, |a, x| a + x)", 4999950000),
		("(0..100000).iter().collect::<Vec>().len()", 100000),
		("let n = 0; for i in 0..100000 { n += i; } n", 4999950000),
	] {
		let source = format!("pub fn main() {{ {body} }}");
		let (result, left, _) = execute(&source, 2000000, 10000);
		assert_eq!(result.unwrap(), expected, "{body}");
		assert!(left < 1900000);
	}
}

#[test]
fn sliced_and_unsliced_charge_exactly_the_same_permits() {
	for body in [
		"42",
		"let n = 0; for i in 0..5000 { n += i; } n",
		"(0..30000).iter().count()",
		"(0..500).iter().fold(0, |a, x| a + x)",
	] {
		let source = format!("pub fn main() {{ {body} }}");
		let (result, left, _) = execute(&source, 100000, 100000);
		let required = 100000 - left;
		for slice in [1, 10000, 25001, 100000] {
			let (actual, remaining, _) = execute(&source, required, slice);
			assert_eq!(actual, result, "{body}, slice {slice}, required {required}");
			assert_eq!(remaining, 0);
			assert!(execute(&source, required - 1, slice).0.is_err());
		}
	}
	assert!(execute("pub fn main() { 42 }", 0, 10000).0.is_err());
}

#[test]
fn hard_native_exhaustion_is_terminal() {
	let (result, left, _) = execute("pub fn main() { (0..).iter().count() }", 50000, 10000);
	assert!(result.unwrap_err().contains("Exhausted the budget"));
	assert_eq!(left, 0);
}

#[test]
fn nested_wrappers_and_unwind_restore_both_settings() {
	budget::with_slice(50, 7, || {
		assert_eq!(budget::remaining(), Some(50));
		assert_eq!(budget::YieldPoint::take().threshold(), 43);
		budget::with(3, || {
			assert_eq!(budget::remaining(), Some(3));
			assert_eq!(budget::YieldPoint::take().threshold(), 0);
		})
		.call();
		assert_eq!(budget::remaining(), Some(50));
		assert_eq!(budget::YieldPoint::take().threshold(), 43);
		let result = std::panic::catch_unwind(|| {
			budget::with_slice(20, 2, || {
				assert_eq!(budget::YieldPoint::take().threshold(), 18);
				panic!("restore");
			})
			.call()
		});
		assert!(result.is_err());
		assert_eq!(budget::remaining(), Some(50));
		assert_eq!(budget::YieldPoint::take().threshold(), 43);
	})
	.call();
	assert_eq!(budget::remaining(), None);
	assert_eq!(budget::YieldPoint::take().threshold(), 0);
}

#[test]
#[should_panic(expected = "must make progress")]
fn zero_slice_is_rejected() {
	budget::with_slice(1, 0, || ()).call();
}

#[test]
#[should_panic(expected = "must be finite")]
fn unlimited_slice_total_is_rejected() {
	budget::with_slice(usize::MAX, 1, || ()).call();
}

#[test]
fn pure_vm_yields_spend_exactly_one_slice() {
	let source = "pub fn main() { let n = 0; for i in 0..10000 { n += i; } n }";
	let context = Context::with_default_modules().unwrap();
	let mut sources = crate::tests::sources(source);
	let mut diagnostics = Diagnostics::new();
	let mut vm = crate::tests::vm(&context, &mut sources, &mut diagnostics, false).unwrap();
	let mut execution = vm.execute(["main"], ()).unwrap();
	let mut left = 100000;
	let mut yields = 0;
	loop {
		let previous = left;
		let (outcome, remaining) = budget::with_slice(left, 123, || {
			let outcome = execution.resume().complete();
			(outcome, budget::remaining().unwrap())
		})
		.call();
		left = remaining;
		match outcome.unwrap() {
			VmOutcome::Limited => {
				assert_eq!(previous - left, 123);
				yields += 1;
			}
			VmOutcome::Complete(value) => {
				assert_eq!(crate::from_value::<i64>(value).unwrap(), 49995000);
				assert!(yields > 1);
				assert!(previous - left <= 123);
				break;
			}
			_ => panic!("unexpected yield"),
		}
	}
}

#[test]
fn awaited_nested_execution_crosses_the_outer_soft_threshold() {
	let source =
		"async fn child() { (0..100000).iter().count() } pub async fn main() { child().await }";
	let (result, _, _) = execute(source, 2000000, 10);
	assert_eq!(result.unwrap(), 100000);
}

#[test]
fn native_generator_driving_crosses_the_outer_soft_threshold() {
	let source = "fn numbers() { for i in 0..10000 { yield i; } } pub fn main() { numbers().iter().count() }";
	let (result, _, _) = execute(source, 2000000, 10);
	assert_eq!(result.unwrap(), 10000);
}
