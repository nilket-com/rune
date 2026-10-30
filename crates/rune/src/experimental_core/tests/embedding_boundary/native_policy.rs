use super::*;
use crate::experimental_core::region::cache;

#[test]
fn executor_native_policy_bypasses_cache_and_preserves_outcomes_when_toggled() {
	// An isolated thread proves the disabled path does not even decide cache
	// environment policy, including when the first invocation exhausts its work.
	std::thread::spawn(|| {
		cache::enable_for_test(true);
		let context = Context::with_default_modules().unwrap();
		for body in ["let n=0;let i=0;while i<100{n+=i;i+=1;}n", "let v=[1];v[9]"] {
			let program = compile(&context, &std::format!("pub fn main(){{{body}}}"));
			let mut executor = executor(&context);
			assert!(executor.native_enabled());
			let mut store = Store::new(Limits::default()).unwrap();
			let mut observations = std::vec::Vec::new();
			for native in [false, true, false] {
				let cache_before = cache::cache_state_for_test();
				let counters_before = cache::counters_for_test();
				let environment_before = cache::enabled_decided_for_test();
				executor.set_native_enabled(native);
				assert_eq!(executor.native_enabled(), native);
				let mut rows = std::vec::Vec::new();
				for work in [0, 1, 5, 1000] {
					let outcome = executor
						.prepare(&program, &mut store, main(&program), &[])
						.unwrap()
						.invoke(work);
					rows.push(match outcome {
						Ok(root) => {
							let value = std::format!("{:?}", store.root_scalar(&root).unwrap());
							store.root_release(root).unwrap();
							value
						}
						Err(error) => std::format!(
							"{:?}|{:?}|{}",
							error.error.kind,
							error.remaining_work,
							error.message(&program)
						),
					});
					assert!(executor.is_idle());
				}
				if !native {
					assert_eq!(cache::cache_state_for_test(), cache_before);
					assert_eq!(cache::counters_for_test(), counters_before);
					assert_eq!(cache::enabled_decided_for_test(), environment_before);
				}
				if observations.is_empty() {
					assert!(cache::summary(program.code.id()).is_none());
				} else {
					assert_eq!(&rows, &observations[0]);
					assert!(cache::summary(program.code.id()).is_some());
				}
				observations.push(rows);
			}
			store.close().unwrap();
		}
	})
	.join()
	.unwrap();
}
