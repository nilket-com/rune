//! Live legacy/execution parity at the template walk boundary.
use super::*;

fn compare(context: &Context, body: &str, succeeds: bool) {
	compare_inner(context, body, succeeds, false);
}

// Object's Rust Debug boundary catches semantic depth errors. Its marker
// contains an engine-owned address, so check each address and compare the
// surrounding structure while retaining both original strings in failures.
fn compare_object_text(old: &str, new: &str) {
	let (mut a, mut b) = (old, new);
	loop {
		match (a.split_once(" object at 0x"), b.split_once(" object at 0x")) {
			(Some((ap, ar)), Some((bp, br))) => {
				assert_eq!(ap, bp, "legacy={old} execution={new}");
				let (aa, at) = ar.split_once(": ").unwrap();
				let (ba, bt) = br.split_once(": ").unwrap();
				for address in [aa, ba] {
					assert!(!address.is_empty() && address.bytes().all(|v| v.is_ascii_hexdigit()));
					assert_ne!(u64::from_str_radix(address, 16).unwrap(), 0);
				}
				let marker = "Value is nested too deeply to walk, limit is 64>";
				assert!(
					at.starts_with(marker) && bt.starts_with(marker),
					"legacy={old} execution={new}"
				);
				(a, b) = (at, bt);
			}
			(None, None) => {
				assert_eq!(a, b, "legacy={old} execution={new}");
				break;
			}
			_ => panic!("legacy={old} execution={new}"),
		}
	}
}

fn compare_inner(context: &Context, body: &str, succeeds: bool, object_debug: bool) {
	let text = std::format!("pub fn main(){{{body}}}");
	let mut legacy = crate::prepare(&mut sources(&text))
		.with_context(context)
		.build_vm()
		.unwrap();
	let old = legacy.call(["main"], ());
	let program = compile(context, &text);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(context);
	executor.set_format_limits(FormatLimits::permissive());
	let new = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(100_000);
	if succeeds {
		let old = old.unwrap();
		let new = new.unwrap();
		let view = store.root_view(&new, ViewLimits::default()).unwrap();
		let node = view.root().unwrap();
		let ViewKind::String(text) = node.kind().unwrap() else {
			panic!("format! did not return a string");
		};
		let old = old.borrow_string_ref().unwrap();
		if object_debug {
			compare_object_text(old.as_ref(), text);
		} else {
			assert_eq!(old.as_ref(), text, "{body}");
		}
		store.root_release(new).unwrap();
	} else {
		let old = old.unwrap_err();
		let new = new.unwrap_err();
		assert!(matches!(
			old.error().kind(),
			crate::runtime::VmErrorKind::MaxValueDepth { max: 64 }
		));
		assert_eq!(new.error.kind, ErrorKind::ValueDepth, "{body}");
		assert_eq!(
			new.error.detail,
			crate::experimental_core::error::Detail::ValueDepth
		);
		assert_eq!(old.error().to_string(), new.message(&program).to_string());
	}
	assert!(executor.is_idle());
	store.close().unwrap();
}

#[test]
fn permissive_templates_match_legacy_depth_for_every_container_and_scalar_leaf() {
	let context = Context::with_default_modules().unwrap();
	for leaf in ["1", "()", "true", "'x'", "\"text\"", "[]", "None"] {
		for wrap in ["[a]", "(a,)", "Some(a)", "Ok(a)", "Wrap{value:a}"] {
			// Walk levels include the formatting receiver and the visited root.
			for levels in [63, 64, 65] {
				let wrappers = levels - 2;
				let body = std::format!(
					"struct Wrap{{value}}let a={leaf};for _ in 0..{wrappers}{{a={wrap};}}format!(\"{{:?}}\",a)"
				);
				compare(&context, &body, levels <= 64);
			}
		}
	}
}

#[test]
fn permissive_templates_bound_cycles_and_preserve_repeated_aliases() {
	let context = Context::with_default_modules().unwrap();
	compare(&context, "let a=[];a.push(a);format!(\"{:?}\",a)", false);
	compare(
		&context,
		"let a=[];let b=[];a.push(b);b.push(a);format!(\"{:?}\",a)",
		false,
	);
	compare(
		&context,
		"let a=[1,2];let b=[];for _ in 0..100{b.push(a);}format!(\"{:?}\",b)",
		true,
	);
	// Cover the Object Debug exception alone and across other container walks.
	// Multiple keys also exercise continuation/accounting after a caught child.
	for wrappers in [62, 63, 64, 65] {
		for leaf in ["1", "()", "true", "'x'", "\"text\"", "[]", "None"] {
			for wrap in [
				"#{x:a}",
				"#{x:[a],y:2}",
				"[#{x:a}]",
				"#{x:Wrap{value:a}}",
				"Some(#{x:a})",
			] {
				let body = std::format!("struct Wrap{{value}}let a={leaf};for _ in 0..{wrappers}{{a={wrap};}}format!(\"{{:?}}\",a)");
				compare_inner(&context, &body, true, true);
			}
		}
	}
	for body in [
		"let a=#{x:()};a.x=a;format!(\"{:?}\",a)",
		"let a=#{x:()};let b=#{x:a};a.x=b;format!(\"{:?}\",a)",
		"let a=#{x:()};a.x=[a];format!(\"{:?}\",a)",
	] {
		compare_inner(&context, body, true, true);
	}
	// A host resource limit is never converted into an Object Debug marker.
	let program = compile(
		&context,
		"pub fn main(){let a=#{x:#{x:1}};format!(\"{:?}\",a)}",
	);
	let mut store = Store::new(Limits::default()).unwrap();
	let mut executor = executor(&context);
	executor.set_format_limits(FormatLimits {
		depth: 1,
		..FormatLimits::permissive()
	});
	let error = executor
		.prepare(&program, &mut store, main(&program), &[])
		.unwrap()
		.invoke(100_000)
		.unwrap_err();
	assert_eq!(error.error.kind, ErrorKind::FormatLimit);
	store.close().unwrap();
	// Resource policy stays permissive beyond the old default 64KiB ceiling.
	let body = std::format!("let s=\"{}\";format!(\"{{}}{{}}\",s,s)", "x".repeat(35_000));
	compare(&context, &body, true);
}
