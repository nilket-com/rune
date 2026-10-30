//! Unit clone intentionally preserves unit, including through wrappers.
use super::*;

#[test]
fn unit_clone_stays_unit_in_direct_and_wrapped_results() {
	let context = Context::with_default_modules().unwrap();
	for body in [
		"()",
		"().clone()",
		"std::clone::clone(())",
		"((),).clone().0",
		"[()].clone()[0]",
		"Some(()).clone().0",
		"[().clone()].clone()[0]",
		"std::clone::clone(().clone())",
		"Ok(().clone())",
		"Err(().clone())",
		"Some(().clone())",
		"[().clone()]",
		"(().clone(),)",
	] {
		let text = std::format!("pub fn main(){{{body}}}");
		let program = compile(&context, &text);
		for native in [false, true] {
			let mut store = Store::new(Limits::default()).unwrap();
			let mut executor = executor(&context);
			executor.set_native_enabled(native);
			let value = executor
				.prepare(&program, &mut store, main(&program), &[])
				.unwrap()
				.invoke(100_000)
				.unwrap();
			{
				let view = store.root_view(&value, ViewLimits::default()).unwrap();
				let node = view.root().unwrap();
				let unit = match node.kind().unwrap() {
					ViewKind::Result { .. }
					| ViewKind::Option { some: true }
					| ViewKind::List { .. } => node.child(0).unwrap(),
					ViewKind::Scalar(Scalar::Unit) => node,
					kind => panic!("unexpected result kind {kind:?} for {body}"),
				};
				assert_eq!(
					unit.kind().unwrap(),
					ViewKind::Scalar(Scalar::Unit),
					"{body}"
				);
				assert!(unit.identity().is_none());
			}
			store.root_release(value).unwrap();
			assert!(executor.is_idle());
			store.close().unwrap();
		}
	}
}
