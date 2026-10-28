//! Live stock/fresh coverage for S8 tuple and unit behavior.
use super::*;

const CASES: &[(&str, &str)] = &[
	("is_tuple", r###"() is Tuple"###),
	("is_not_tuple", r###"() is not Tuple"###),
	("is_unit", r###"() is Unit"###),
	("is_bool", r###"() is bool"###),
	("is_i64", r###"() is i64"###),
	("is_string", r###"() is String"###),
	("is_vec", r###"() is Vec"###),
	("is_object", r###"() is Object"###),
	("is_option", r###"() is Option"###),
	("is_result", r###"() is Result"###),
	("cast_i64", r###"() as i64"###),
	("cast_bool", r###"() as bool"###),
	("cast_char", r###"() as char"###),
	("debug", r###"format!("{:?}", ())"###),
	("display", r###"format!("{}", ())"###),
	("precision", r###"format!("{:.2?}", ())"###),
	("width", r###"format!("{:>8?}", ())"###),
	("len", r###"().len()"###),
	("iter", r###"().iter()"###),
	("get", r###"().get(0)"###),
	("is_empty", r###"().is_empty()"###),
	("clone", r###"().clone()"###),
	("tuple_eq", r###"() == ()"###),
	("tuple_ne", r###"() != ()"###),
	("tuple_cmp", r###"() < ()"###),
	("tuple_hash", r###"std::hash::hash(())"###),
	("unit_pattern", r###"match () { () => 1, _ => 2 }"###),
	("tuple_pattern", r###"match () { (..) => 1, _ => 2 }"###),
	("let_unit", r###"{ let () = (); 7 }"###),
	("let_tuple", r###"{ let (..) = (); 7 }"###),
	("array", r###"[(), ()]"###),
	("vec_index", r###"[()][0]"###),
	("tuple_inside", r###"((), 1)"###),
	("object_value", r###"#{k: ()}"###),
	("object_key", r###"{ let x = #{}; x[()] = 1; x }"###),
	("as_key", r###"{ let x = #{}; x[()] }"###),
	("source_call", r###"{ fn z() { () } z() is Tuple }"###),
	("named_struct", r###"{ struct P; () is P }"###),
	("some_tuple", r###"Some(()) is Tuple"###),
	("empty_vector", r###"[] is Tuple"###),
	("tuple_like", r###"(1,) is Tuple"###),
	("option_test", r###"() is Option"###),
	("type_name", r###"std::any::type_name(())"###),
	("tuple_one_is", r###"(1,) is Tuple"###),
	("tuple_two_is", r###"(1, 2) is Tuple"###),
	("tuple_one_is_not", r###"(1,) is not Tuple"###),
	("unit_variable_is", r###"{ let x = (); x is Tuple }"###),
	("tuple_variable_is", r###"{ let x = (1, 2); x is Tuple }"###),
	("unit_is_not_vec", r###"() is not Vec"###),
	("tuple_is_vec", r###"(1,) is Vec"###),
	("vec_is_vec", r###"[1] is Vec"###),
	("object_is_object", r###"#{x: 1} is Object"###),
	("some_is_option", r###"Some(1) is Option"###),
	("ok_is_result", r###"Ok(1) is Result"###),
	("nominal_is_self", r###"{ struct P; P is P }"###),
	(
		"nominal_value_is_self",
		r###"{ struct P; let x = P; x is P }"###,
	),
	(
		"nominal_value_is_tuple",
		r###"{ struct P; let x = P; x is Tuple }"###,
	),
	("unit_order_le", r###"() <= ()"###),
	("unit_order_gt", r###"() > ()"###),
	("unit_compare_method", r###"().cmp(())"###),
	("unit_tuple_eq", r###"() == (1,)"###),
	("unit_tuple_order", r###"() < (1,)"###),
	("unit_index", r###"()[0]"###),
	("unit_iter_count", r###"().iter().count()"###),
	("unit_into_vec", r###"().iter().collect::<Vec>()"###),
	(
		"nonempty_tuple_pattern_unit",
		r###"match (1,) { () => 1, _ => 2 }"###,
	),
	(
		"nonempty_tuple_pattern_open",
		r###"match (1,) { (..) => 1, _ => 2 }"###,
	),
	(
		"unit_pattern_literal",
		r###"match () { () => 1, (..) => 2, _ => 3 }"###,
	),
	(
		"unit_pattern_nonempty",
		r###"match () { (a,) => 1, _ => 2 }"###,
	),
	("unit_let_nonempty", r###"{ let (a,) = (); a }"###),
	("unit_fn_return_eq", r###"{ fn u() { () } u() == () }"###),
	(
		"unit_fn_return_tuple",
		r###"{ fn u() { () } u() is Tuple }"###,
	),
	("unit_type_name", r###"std::any::type_name_of_val(())"###),
	("tuple_type_name", r###"std::any::type_name_of_val((1,))"###),
	("unit_type_of", r###"std::any::type_of_val(())"###),
	("unit_hash_ops", r###"std::ops::hash(())"###),
	("tuple_hash_ops", r###"std::ops::hash((1,))"###),
	("unit_vec_membership", r###"[()].contains(())"###),
	(
		"unit_object_key_map",
		r###"{ let x = #{}; x[()] = 1; x }"###,
	),
	("unit_option_collection", r###"[Some(()), None]"###),
	("unit_debug_hex", r###"format!("{:#?}", ())"###),
	("unit_display_width", r###"format!("{:>8}", ())"###),
	("unit_cast_tuple", r###"() as Tuple"###),
	("tuple_cast_tuple", r###"(1,) as Tuple"###),
	("unit_typeof_correct", r###"std::any::Type::of_val(())"###),
	(
		"unit_typeof_equality",
		r###"std::any::Type::of_val(()) == std::any::Type::of_val((1,))"###,
	),
	("unit_cmp_nonempty", r###"().cmp((1,))"###),
	("tuple_cmp_unit", r###"(1,).cmp(())"###),
	("tuple_cmp_self", r###"(1,).cmp((1,))"###),
	("tuple_cmp_other", r###"(1,).cmp((2,))"###),
	("unit_cmp_scalar", r###"().cmp(1)"###),
	("range_full_cmp", r###"(..).cmp(..)"###),
	("range_from_cmp", r###"(1..).cmp(2..)"###),
	("range_to_cmp", r###"(..1).cmp(..2)"###),
	("range_exclusive_cmp", r###"(1..3).cmp(1..4)"###),
	("range_inclusive_cmp", r###"(1..=3).cmp(1..=4)"###),
	("range_to_inclusive_cmp", r###"(..=1).cmp(..=2)"###),
	("range_inclusive_equal", r###"(0..=3).cmp(0..=3)"###),
	("range_inclusive_start", r###"(1..=9).cmp(2..=0)"###),
	("range_from_greater", r###"(2..).cmp(1..)"###),
	("range_to_equal", r###"(..3).cmp(..3)"###),
	("nested_tuple_cmp", r###"((1, 2),).cmp(((1, 3),))"###),
	("nested_vec_cmp", r###"[(1, 2)].cmp([(1, 3)])"###),
	("nested_option_cmp", r###"Some((1, 2)).cmp(Some((1, 3)))"###),
	("nested_result_cmp", r###"Ok((1, 2)).cmp(Ok((1, 3)))"###),
	("mixed_tuple_cmp", r###"(1,).cmp((true,))"###),
	("unit_eq_host", r###"{ fn u() { () } u() == () }"###),
	(
		"tuple_two_pattern_one",
		r###"match (1,2) { (x,) => 1, _ => 2 }"###,
	),
	(
		"unit_tuple_anypattern",
		r###"match () { (x,..) => 1, _ => 2 }"###,
	),
	("unit_is_char", r###"() is char"###),
	("unit_is_u64", r###"() is u64"###),
	("unit_is_f64", r###"() is f64"###),
	("unit_is_ordering", r###"() is std::cmp::Ordering"###),
	("unit_is_function", r###"() is Function"###),
	("unit_is_type", r###"() is std::any::Type"###),
];

fn accepted_refusal(name: &str, class: &str) -> bool {
	matches!(
		(name, class),
		("get", "Some(Method)") | ("unit_hash_ops" | "tuple_hash_ops", "Some(Native)")
	)
}

#[test]
fn nested_nan_cmp_preserves_stock_diagnostic() {
	let root = std::env::temp_dir().join(format!("rune-s8-nan-{}", std::process::id()));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	let source = root.join("nan.rn");
	fs::write(&source, "pub fn main() { (0.0 / 0.0,).cmp((0.0,)) }\n").unwrap();
	let stock = observe(&source, "main", "stock");
	let fresh = observe(&source, "main", "fresh");
	assert_eq!(stock.phase, "error");
	assert!(
		stock.class.starts_with("stock:IllegalFloatComparison"),
		"{}",
		stock.class
	);
	assert_eq!(fresh.phase, "error");
	assert_eq!(fresh.class, "IllegalFloatComparison");
	assert_eq!(stock.message, fresh.message);
}

#[test]
fn legacy_float_hash_and_bad_format_have_compile_time_refusals() {
	let root =
		std::env::temp_dir().join(format!("rune-error-class-refusals-{}", std::process::id()));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	for (name, expression, stock_class, refusal) in [
		(
			"float-hash",
			"std::ops::hash(0.0 / 0.0)",
			"stock:IllegalFloatOperation",
			"Some(Native)",
		),
		(
			"bad-hex",
			"format!(\"{:x}\", true)",
			"stock:IllegalFormat",
			"Some(FormatSpec)",
		),
	] {
		let source = root.join(format!("{name}.rn"));
		fs::write(&source, format!("pub fn main() {{ {expression} }}\n")).unwrap();
		let stock = observe(&source, "main", "stock");
		let fresh = observe(&source, "main", "fresh");
		assert_eq!(stock.phase, "error", "{name}");
		assert!(
			stock.class.starts_with(stock_class),
			"{name}: {}",
			stock.class
		);
		assert_eq!(
			(fresh.phase.as_str(), fresh.class.as_str()),
			("refused", refusal),
			"{name}"
		);
	}
}

#[test]
fn shared_error_class_examples_run_live_on_both_engines() {
	let root = std::env::temp_dir().join(format!("rune-error-class-live-{}", std::process::id()));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	for (name, expression, stock_class, fresh_class) in [
		(
			"cast-target",
			"1 as bool",
			"stock:UnsupportedAsTarget",
			"InvalidCast",
		),
		(
			"cast-operand",
			"() as i64",
			"stock:UnsupportedAs",
			"InvalidCast",
		),
		(
			"missing-index",
			"[1][3]",
			"IndexOutOfBounds",
			"IndexOutOfBounds",
		),
		("bad-call", "1()", "stock:UnsupportedCallFn", "Type"),
		(
			"shift-negative",
			"1 << -1",
			"IntegerCoercion",
			"IntegerCoercion",
		),
		("shift-overflow", "1 << 64", "Overflow", "Overflow"),
		(
			"arithmetic-overflow",
			"9223372036854775807 + 1",
			"Overflow",
			"Overflow",
		),
		(
			"division-by-zero",
			"1 / 0",
			"DivisionByZero",
			"DivisionByZero",
		),
	] {
		let source = root.join(format!("{name}.rn"));
		fs::write(&source, format!("pub fn main() {{ {expression} }}\n")).unwrap();
		let stock = observe(&source, "main", "stock");
		let fresh = observe(&source, "main", "fresh");
		assert_eq!(
			(stock.phase.as_str(), fresh.phase.as_str()),
			("error", "error"),
			"{name}"
		);
		assert!(
			stock.class.starts_with(stock_class),
			"{name}: {}",
			stock.class
		);
		assert_eq!(fresh.class, fresh_class, "{name}");
	}
}

#[test]
fn structural_cmp_depth_and_cycle_match_live_stock() {
	let root = std::env::temp_dir().join(format!("rune-s8-depth-{}", std::process::id()));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	for (name, expression, succeeds) in [
		(
			"depth63",
			"{ let x = 1; for _ in 0..63 { x = [x]; } x.cmp(x) }",
			true,
		),
		(
			"depth64",
			"{ let x = 1; for _ in 0..64 { x = [x]; } x.cmp(x) }",
			true,
		),
		(
			"depth65",
			"{ let x = 1; for _ in 0..65 { x = [x]; } x.cmp(x) }",
			false,
		),
		(
			"depth100",
			"{ let x = 1; for _ in 0..100 { x = [x]; } x.cmp(x) }",
			false,
		),
		(
			"depth1000",
			"{ let x = 1; for _ in 0..1000 { x = [x]; } x.cmp(x) }",
			false,
		),
		("cycle", "{ let x = []; x.push(x); x.cmp(x) }", false),
	] {
		let source = root.join(format!("{name}.rn"));
		fs::write(&source, format!("pub fn main() {{ {expression} }}\n")).unwrap();
		let stock = observe(&source, "main", "stock");
		let fresh = observe(&source, "main", "fresh");
		if succeeds {
			assert_eq!(
				(stock.phase.as_str(), stock.value.as_str()),
				("value", "Equal"),
				"{name}"
			);
			assert_eq!(
				(fresh.phase.as_str(), fresh.value.as_str()),
				("value", "ordering:Equal:Equal"),
				"{name}"
			);
		} else {
			assert_eq!(
				(stock.phase.as_str(), stock.class.as_str()),
				("error", "stock:MaxValueDepth { max: 64 }"),
				"{name}"
			);
			assert_eq!(
				(fresh.phase.as_str(), fresh.class.as_str()),
				("error", "ValueDepth"),
				"{name}"
			);
			assert_eq!(stock.message, fresh.message, "{name}");
		}
	}
}

fn equivalent_error(name: &str, stock: &Observation, fresh: &Observation) -> bool {
	if stock.class == fresh.class {
		return true;
	}
	if matches!(
		name,
		"cast_i64" | "cast_bool" | "cast_char" | "unit_cast_tuple" | "tuple_cast_tuple"
	) {
		// S8 cast-failure: identical operand/target in the source; stock calls
		// this UnsupportedAs and the independent executor calls it InvalidCast.
		return stock.class.starts_with("stock:UnsupportedAs ") && fresh.class == "InvalidCast";
	}
	stock
		.class
		.starts_with("stock:Panic { reason: Panic { inner: ")
		&& stock.class.ends_with(" } }")
		&& fresh.class == "Panic"
		&& stock.message == fresh.message
}

#[test]
fn unit_and_tuple_audit_matches_live_stock_or_declared_refusal() {
	let root = std::env::temp_dir().join(format!(
		"rune-s8-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	for &(name, expression) in CASES {
		let source = root.join(format!("{name}.rn"));
		fs::write(&source, format!("pub fn main() {{ {expression} }}\n")).unwrap();
		let stock = observe(&source, "main", "stock");
		let fresh = observe(&source, "main", "fresh");
		if fresh.phase == "refused" {
			if stock.phase == "compile-error" {
				assert_eq!(fresh.class, "Some(FrontendDiagnostic)", "{name}");
			} else {
				assert!(
					accepted_refusal(name, &fresh.class),
					"{name}: unexpected refusal {}",
					fresh.class
				);
			}
			continue;
		}
		assert_eq!(
			stock.phase, fresh.phase,
			"{name}: stock={} fresh={} ({})",
			stock.phase, fresh.phase, fresh.message
		);
		match stock.phase.as_str() {
			"value" => {
				if name == "iter" {
					assert!(
						stock.value.starts_with("<::std::slice::Iter object at 0x"),
						"{name}"
					);
					assert!(
						fresh.value.starts_with("<::std::slice::Iter object at 0x"),
						"{name}"
					);
				} else if matches!(
					name,
					"unit_compare_method"
						| "unit_cmp_nonempty"
						| "tuple_cmp_unit" | "tuple_cmp_self"
						| "tuple_cmp_other"
						| "range_full_cmp" | "range_from_cmp"
						| "range_to_cmp" | "range_exclusive_cmp"
						| "range_inclusive_cmp"
						| "range_to_inclusive_cmp"
						| "range_inclusive_equal"
						| "range_inclusive_start"
						| "range_from_greater"
						| "range_to_equal" | "nested_tuple_cmp"
						| "nested_vec_cmp" | "nested_option_cmp"
						| "nested_result_cmp"
				) {
					// The fresh observer prefixes its typed Ordering witness; both
					// engines must still report the same concrete variant.
					assert!(
						matches!(stock.value.as_str(), "Less" | "Equal" | "Greater"),
						"{name}"
					);
					assert_eq!(
						fresh.value,
						format!("ordering:{}:{}", stock.value, stock.value),
						"{name}"
					);
				} else {
					assert_eq!(stock.value, fresh.value, "{name}");
				}
			}
			"error" => assert!(
				equivalent_error(name, &stock, &fresh),
				"{name}: stock={} fresh={}",
				stock.class,
				fresh.class
			),
			_ => panic!("{name}: unexpected observation phase {}", stock.phase),
		}
	}
}

fn compile_s8(source: &str) -> frontend::Program {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	frontend::compile(&mut sources, &context, &mut diagnostics, 1)
		.unwrap_or_else(|error| panic!("{error:?}: {:?}", diagnostics.diagnostics()))
}

#[test]
fn public_tuple_hash_is_shared_but_source_test_uses_two_exact_tags() {
	use crate::experimental_core::{arithmetic::Binary, code::Op, value::Type};
	use crate::TypeHash;
	assert_eq!(<()>::HASH, crate::runtime::OwnedTuple::HASH);
	let program = compile_s8("pub fn main(x) { x is Tuple }");
	let id = program.function(Hash::type_hash(["main"])).unwrap();
	let function = program.code.function(id).unwrap();
	let ops: std::vec::Vec<_> = function
		.instructions()
		.iter()
		.map(|op| function.decode(*op).unwrap())
		.collect();
	let tags: std::vec::Vec<_> = ops
		.iter()
		.filter_map(|op| match op {
			Op::Is {
				tag, negate: false, ..
			} => Some(*tag),
			_ => None,
		})
		.collect();
	assert_eq!(tags, [Type::Unit, Type::Tuple]);
	assert!(ops.iter().any(|op| matches!(
		op,
		Op::Binary {
			op: Binary::BitOr,
			..
		}
	)));
	let not_program = compile_s8("pub fn main(x) { x is not Tuple }");
	let function = not_program
		.code
		.function(not_program.function(Hash::type_hash(["main"])).unwrap())
		.unwrap();
	let ops: std::vec::Vec<_> = function
		.instructions()
		.iter()
		.map(|op| function.decode(*op).unwrap())
		.collect();
	assert!(ops.iter().any(|op| matches!(
		op,
		Op::Binary {
			op: Binary::BitAnd,
			..
		}
	)));
}

#[test]
fn host_created_empty_tuple_preserves_type_patterns_and_ownership() {
	use crate::experimental_core::value::{Scalar, Type};
	for (source, expected) in [
		("pub fn main(x) { x is Tuple }", Scalar::Bool(true)),
		("pub fn main(x) { x is not Tuple }", Scalar::Bool(false)),
		(
			"pub fn main(x) { match x { () => 1, _ => 2 } }",
			Scalar::I64(1),
		),
		(
			"pub fn main(x) { match x { (..) => 1, _ => 2 } }",
			Scalar::I64(1),
		),
		("pub fn main(x) { x.len() }", Scalar::U64(0)),
		("pub fn main(x) { x.is_empty() }", Scalar::Bool(true)),
	] {
		let program = compile_s8(source);
		let entry = program.function(Hash::type_hash(["main"])).unwrap();
		let mut store = Store::new(Limits::default()).unwrap();
		let input = store.root_list(Type::Tuple, &[]).unwrap();
		let mut executor = Executor::new().unwrap();
		let output = executor
			.invoke(&program, &mut store, entry, &[&input], 10_000)
			.unwrap();
		assert_eq!(
			store.root_scalar(&output).unwrap(),
			Some(expected),
			"{source}"
		);
		store.root_release(output).unwrap();
		store.root_release(input).unwrap();
		assert!(executor.is_idle());
		assert_eq!(store.live_entries(), 0);
		assert_eq!(store.payload_bytes(), 0);
	}
}

const RECEIVER_SWEEP: &[(&str, &str)] = &[
	("sweep-unit-cmp", r###"(()).cmp(())"###),
	("sweep-unit-eq", r###"(()).eq(())"###),
	("sweep-unit-partial_cmp", r###"(()).partial_cmp(())"###),
	("sweep-unit-hash", r###"(()).hash()"###),
	("sweep-unit-clone", r###"(()).clone()"###),
	("sweep-unit-len", r###"(()).len()"###),
	("sweep-tuple1-cmp", r###"((1,)).cmp((1,))"###),
	("sweep-tuple1-eq", r###"((1,)).eq((1,))"###),
	(
		"sweep-tuple1-partial_cmp",
		r###"((1,)).partial_cmp((1,))"###,
	),
	("sweep-tuple1-hash", r###"((1,)).hash()"###),
	("sweep-tuple1-clone", r###"((1,)).clone()"###),
	("sweep-tuple1-len", r###"((1,)).len()"###),
	("sweep-tuple2-cmp", r###"((1, 2)).cmp((1, 2))"###),
	("sweep-tuple2-eq", r###"((1, 2)).eq((1, 2))"###),
	(
		"sweep-tuple2-partial_cmp",
		r###"((1, 2)).partial_cmp((1, 2))"###,
	),
	("sweep-tuple2-hash", r###"((1, 2)).hash()"###),
	("sweep-tuple2-clone", r###"((1, 2)).clone()"###),
	("sweep-tuple2-len", r###"((1, 2)).len()"###),
	("sweep-bool-cmp", r###"(true).cmp(true)"###),
	("sweep-bool-eq", r###"(true).eq(true)"###),
	("sweep-bool-partial_cmp", r###"(true).partial_cmp(true)"###),
	("sweep-bool-hash", r###"(true).hash()"###),
	("sweep-bool-clone", r###"(true).clone()"###),
	("sweep-bool-len", r###"(true).len()"###),
	("sweep-i64-cmp", r###"(1).cmp(1)"###),
	("sweep-i64-eq", r###"(1).eq(1)"###),
	("sweep-i64-partial_cmp", r###"(1).partial_cmp(1)"###),
	("sweep-i64-hash", r###"(1).hash()"###),
	("sweep-i64-clone", r###"(1).clone()"###),
	("sweep-i64-len", r###"(1).len()"###),
	("sweep-u64-cmp", r###"(1u).cmp(1u)"###),
	("sweep-u64-eq", r###"(1u).eq(1u)"###),
	("sweep-u64-partial_cmp", r###"(1u).partial_cmp(1u)"###),
	("sweep-u64-hash", r###"(1u).hash()"###),
	("sweep-u64-clone", r###"(1u).clone()"###),
	("sweep-u64-len", r###"(1u).len()"###),
	("sweep-f64-cmp", r###"(1.0).cmp(1.0)"###),
	("sweep-f64-eq", r###"(1.0).eq(1.0)"###),
	("sweep-f64-partial_cmp", r###"(1.0).partial_cmp(1.0)"###),
	("sweep-f64-hash", r###"(1.0).hash()"###),
	("sweep-f64-clone", r###"(1.0).clone()"###),
	("sweep-f64-len", r###"(1.0).len()"###),
	("sweep-char-cmp", r###"('x').cmp('x')"###),
	("sweep-char-eq", r###"('x').eq('x')"###),
	("sweep-char-partial_cmp", r###"('x').partial_cmp('x')"###),
	("sweep-char-hash", r###"('x').hash()"###),
	("sweep-char-clone", r###"('x').clone()"###),
	("sweep-char-len", r###"('x').len()"###),
	("sweep-string-cmp", r###"("a").cmp("a")"###),
	("sweep-string-eq", r###"("a").eq("a")"###),
	("sweep-string-partial_cmp", r###"("a").partial_cmp("a")"###),
	("sweep-string-hash", r###"("a").hash()"###),
	("sweep-string-clone", r###"("a").clone()"###),
	("sweep-string-len", r###"("a").len()"###),
	("sweep-vec-cmp", r###"([1]).cmp([1])"###),
	("sweep-vec-eq", r###"([1]).eq([1])"###),
	("sweep-vec-partial_cmp", r###"([1]).partial_cmp([1])"###),
	("sweep-vec-hash", r###"([1]).hash()"###),
	("sweep-vec-clone", r###"([1]).clone()"###),
	("sweep-vec-len", r###"([1]).len()"###),
	("sweep-object-cmp", r###"(#{x:1}).cmp(#{x:1})"###),
	("sweep-object-eq", r###"(#{x:1}).eq(#{x:1})"###),
	(
		"sweep-object-partial_cmp",
		r###"(#{x:1}).partial_cmp(#{x:1})"###,
	),
	("sweep-object-hash", r###"(#{x:1}).hash()"###),
	("sweep-object-clone", r###"(#{x:1}).clone()"###),
	("sweep-object-len", r###"(#{x:1}).len()"###),
	("sweep-option-cmp", r###"(Some(1)).cmp(Some(1))"###),
	("sweep-option-eq", r###"(Some(1)).eq(Some(1))"###),
	(
		"sweep-option-partial_cmp",
		r###"(Some(1)).partial_cmp(Some(1))"###,
	),
	("sweep-option-hash", r###"(Some(1)).hash()"###),
	("sweep-option-clone", r###"(Some(1)).clone()"###),
	("sweep-option-len", r###"(Some(1)).len()"###),
	("sweep-result-cmp", r###"(Ok(1)).cmp(Ok(1))"###),
	("sweep-result-eq", r###"(Ok(1)).eq(Ok(1))"###),
	(
		"sweep-result-partial_cmp",
		r###"(Ok(1)).partial_cmp(Ok(1))"###,
	),
	("sweep-result-hash", r###"(Ok(1)).hash()"###),
	("sweep-result-clone", r###"(Ok(1)).clone()"###),
	("sweep-result-len", r###"(Ok(1)).len()"###),
	(
		"sweep-nominal-cmp",
		r###"{ struct P {x} P{x:1}.cmp(P{x:1}) }"###,
	),
	(
		"sweep-nominal-eq",
		r###"{ struct P {x} P{x:1}.eq(P{x:1}) }"###,
	),
	(
		"sweep-nominal-partial_cmp",
		r###"{ struct P {x} P{x:1}.partial_cmp(P{x:1}) }"###,
	),
	(
		"sweep-nominal-hash",
		r###"{ struct P {x} P{x:1}.hash() }"###,
	),
	(
		"sweep-nominal-clone",
		r###"{ struct P {x} P{x:1}.clone() }"###,
	),
	("sweep-nominal-len", r###"{ struct P {x} P{x:1}.len() }"###),
];

#[test]
fn admitted_receiver_method_sweep_never_refuses_at_runtime() {
	let root = std::env::temp_dir().join(format!(
		"rune-s8-methods-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	));
	fs::create_dir(&root).unwrap();
	let _scratch = Scratch(root.clone());
	for &(name, expression) in RECEIVER_SWEEP {
		let source = root.join(format!("{name}.rn"));
		fs::write(&source, format!("pub fn main() {{ {expression} }}\n")).unwrap();
		let stock = observe(&source, "main", "stock");
		let fresh = observe(&source, "main", "fresh");
		assert_ne!(
			fresh.class, "Unsupported",
			"{name}: admitted runtime Unsupported"
		);
		if fresh.phase == "refused" {
			let expected = if stock.phase == "compile-error" {
				"Some(FrontendDiagnostic)"
			} else {
				"Some(Method)"
			};
			assert_eq!(fresh.class, expected, "{name}");
			continue;
		}
		assert_eq!(
			stock.phase, fresh.phase,
			"{name}: stock={} fresh={} ({})",
			stock.phase, fresh.phase, fresh.message
		);
		match stock.phase.as_str() {
			"value" if name.ends_with("-cmp") => {
				assert!(
					matches!(stock.value.as_str(), "Less" | "Equal" | "Greater"),
					"{name}"
				);
				assert_eq!(
					fresh.value,
					format!("ordering:{}:{}", stock.value, stock.value),
					"{name}"
				);
			}
			"value" => assert_eq!(stock.value, fresh.value, "{name}"),
			"error" => assert!(
				equivalent_error(name, &stock, &fresh),
				"{name}: stock={} fresh={}",
				stock.class,
				fresh.class
			),
			_ => panic!("{name}: unexpected observation phase {}", stock.phase),
		}
	}
}
