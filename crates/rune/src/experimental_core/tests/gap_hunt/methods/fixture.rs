// All fixtures are test-only and make actual stock registrations.
use crate as rune;
#[derive(rune::Any)]
#[rune(item = ::c13_foreign)]
struct Opaque;
fn native_body(_: Value) -> i64 {
	TAPE.with(|t| t.borrow_mut().push(json!({"native-body":true})));
	41
}
fn context(fixture: &str) -> Context {
	context_for(fixture, false)
}
fn context_for(fixture: &str, fresh: bool) -> Context {
	use crate::TypeHash;
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_item(["c13"]).unwrap();
	if fresh {
		module.fresh_host_function("tap", tap).unwrap();
		module.fresh_host_function("tap_bool", tap_bool).unwrap();
	} else {
		module.function("tap", tap).build().unwrap();
		module.function("tap_bool", tap_bool).build().unwrap();
	}
	module.function("native_body", native_body).build().unwrap();
	context.install(module).unwrap();
	match fixture {
		"default" => (),
		"opaque-provider" => {
			let mut m = crate::Module::with_item(["c13_foreign"]).unwrap();
			m.ty::<Opaque>().unwrap();
			m.associated_function("unrecognized", |_: &Opaque| -> i64 {
				native_body(Value::unit())
			})
			.unwrap();
			context.install(m).unwrap();
		}
		"i64-provider" | "bool-provider" | "unit-provider" => {
			let mut m = crate::Module::with_item(["c13_foreign"]).unwrap();
			match fixture {
				"i64-provider" => {
					m.associated_function("unrecognized", |_: i64| -> i64 {
						native_body(Value::unit())
					})
					.unwrap();
				}
				"bool-provider" => {
					m.associated_function("unrecognized", |_: bool| -> i64 {
						native_body(Value::unit())
					})
					.unwrap();
				}
				"unit-provider" => {
					m.associated_function("unrecognized", |_: ()| -> i64 {
						native_body(Value::unit())
					})
					.unwrap();
				}
				_ => unreachable!(),
			}
			context.install(m).unwrap();
		}
		"raw-i64-provider" | "raw-local-provider" => {
			let receiver = if fixture == "raw-i64-provider" {
				i64::HASH
			} else {
				Hash::type_hash(["Local"])
			};
			let hash = Hash::associated_function(receiver, Hash::ident("unrecognized"));
			let handler = context
				.lookup_function(Hash::type_hash(["c13", "native_body"]))
				.unwrap()
				.clone();
			context.c13_insert_raw_fixture(hash, handler).unwrap();
		}
		"failed-install-provider" => {
			// The first associated function remains installed after the second conflicts.
			let mut m = crate::Module::with_item(["c13_partial"]).unwrap();
			m.associated_function("unrecognized", |_: i64| -> i64 {
				native_body(Value::unit())
			})
			.unwrap();
			m.associated_function("abs", |_: i64| -> i64 { 99 })
				.unwrap();
			assert!(context.install(m).is_err());
			assert!(context
				.lookup_function(Hash::associated_function(
					i64::HASH,
					Hash::ident("unrecognized")
				))
				.is_some());
		}
		_ => panic!("unreviewed C13 fixture"),
	}
	context
}
