//! Test-only registry policy. Sources are witnesses, never saved observations.
use super::*;
use crate::compile::meta::{AssociatedKind, Kind, Signature};
use crate::experimental_core::{error::Refusal, frontend};
use crate::{Diagnostics, Source, Sources, TypeHash};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::string::{String, ToString};
use std::vec::Vec;
use std::{borrow::ToOwned, format, vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
	Admit,
	Refuse(Refusal),
}
#[derive(Clone)]
struct Receiver {
	name: String,
	source: String,
	virtual_chain: bool,
}

fn receiver(context: &Context, hash: Hash) -> Option<Receiver> {
	let ty = context.types.get(&hash)?;
	let name = ty.item.to_string();
	let source = match name.as_str() {
		"::std::bool" => "true",
		"::std::char" => "'a'",
		"::std::i64" => "1",
		"::std::u64" => "1u64",
		"::std::f64" => "1.0",
		"::std::string::String" => "\"abc\"",
		"::std::vec::Vec" => "[1,2]",
		"::std::tuple::Tuple" => "(1,2)",
		"::std::object::Object" => "#{a:1}",
		"::std::option::Option" => "Some(1)",
		"::std::result::Result" => "Ok(1)",
		"::std::cmp::Ordering" => "::std::cmp::Ordering::Equal",
		"::std::any::Type" => "i64",
		"::std::ops::Function" => "(||1)",
		"::std::ops::Range" => "(0..3)",
		"::std::ops::RangeFrom" => "(0..)",
		"::std::ops::RangeFull" => "(..)",
		"::std::ops::RangeTo" => "(..3)",
		"::std::ops::RangeInclusive" => "(0..=3)",
		"::std::ops::RangeToInclusive" => "(..=3)",
		"::std::slice::Iter" => "[1,2].iter()",
		"::std::string::Chars" => "\"abc\".chars()",
		"::std::object::Iter" => "#{a:1}.iter()",
		"::std::object::Keys" => "#{a:1}.keys()",
		"::std::object::Values" => "#{a:1}.values()",
		"::std::option::Iter" => "Some(1).iter()",
		"::std::string::Split" => {
			// Three concrete native instantiations share one item name.
			if ty.type_parameters == Hash::parameters([char::HASH]) {
				"\"a b\".split(' ')"
			} else {
				"\"a b\".split(\" \")"
			}
		}
		"::std::ops::RangeIter" => "(0..3).iter()",
		"::std::ops::RangeFromIter" => "(0..).iter()",
		"::std::ops::RangeInclusiveIter" => "(0..=3).iter()",
		"::std::iter::Rev" => "[1,2].iter().rev()",
		"::std::iter::Chain" => "[1,2].iter().chain([3])",
		"::std::iter::Enumerate" => "[1,2].iter().enumerate()",
		"::std::iter::Filter" => "[1,2].iter().filter(|x|true)",
		"::std::iter::Map" => "[1,2].iter().map(|x|x)",
		"::std::iter::FilterMap" => "[1,2].iter().filter_map(|x|Some(x))",
		"::std::iter::FlatMap" => "[1,2].iter().flat_map(|x|[x])",
		"::std::iter::Peekable" => "[1,2].iter().peekable()",
		"::std::iter::Skip" => "[1,2].iter().skip(1)",
		"::std::iter::Take" => "[1,2].iter().take(1)",
		_ => return None,
	};
	let mut source = source.to_owned();
	if name.ends_with("RangeIter")
		|| name.ends_with("RangeFromIter")
		|| name.ends_with("RangeInclusiveIter")
	{
		if ty.type_parameters == Hash::parameters([u64::HASH]) {
			source = source.replace('0', "0u64").replace('3', "3u64");
		} else if ty.type_parameters == Hash::parameters([char::HASH]) {
			source = source.replace('0', "'a'").replace('3', "'c'");
		} else {
			assert_eq!(
				ty.type_parameters,
				Hash::parameters([i64::HASH]),
				"unclassified range element"
			);
		}
	}
	let virtual_chain = name.starts_with("::std::iter::");
	Some(Receiver {
		name,
		source,
		virtual_chain,
	})
}
fn in_names(name: &str, names: &str) -> bool {
	names.split_whitespace().any(|n| n == name)
}
fn iterator(r: &Receiver) -> bool {
	r.virtual_chain
		|| r.name.ends_with("Iter")
		|| in_names(&r.name, "::std::string::Chars ::std::string::Split")
}

// These are reviewed capability rules, independent of the compiler observation below.
// Unknown methods default to Method, so adding admission without reviewing this policy fails.
fn method_policy(r: &Receiver, name: &str, parameters: &[Hash], inline: bool) -> Verdict {
	let refuse = Verdict::Refuse(Refusal::Method);
	if r.virtual_chain && (!inline || r.name.ends_with("Peekable")) {
		return refuse;
	}
	if r.name == "::std::option::Option" && name == "take" {
		return if inline {
			refuse
		} else {
			Verdict::Refuse(Refusal::Arity)
		};
	}
	if iterator(r) && !inline {
		let text = in_names(&r.name, "::std::string::Chars ::std::string::Split");
		let materialized = in_names(name, "next iter len clone")
			|| (name == "any" && r.name != "::std::string::Chars")
			|| (text && in_names(name, "count skip take collect"));
		if !materialized {
			return refuse;
		}
	}
	let terminal = in_names(
		name,
		"all any count next find fold reduce map filter filter_map flat_map enumerate chain skip take",
	);
	if iterator(r) && terminal {
		return Verdict::Admit;
	}
	if iterator(r) && in_names(name, "sum product") {
		return if !in_names(&r.name, "::std::string::Chars ::std::string::Split")
			&& parameters.len() == 1
			&& [i64::HASH, u64::HASH, f64::HASH].contains(&parameters[0])
		{
			Verdict::Admit
		} else {
			refuse
		};
	}
	if iterator(r) && name == "collect" {
		return if parameters.len() == 1
			&& ([crate::runtime::Vec::HASH, crate::alloc::String::HASH].contains(&parameters[0])
				|| (!in_names(&r.name, "::std::string::Chars ::std::string::Split")
					&& [crate::runtime::Object::HASH, crate::runtime::Tuple::HASH]
						.contains(&parameters[0])))
		{
			Verdict::Admit
		} else {
			refuse
		};
	}
	if iterator(r) && name == "rev" {
		return if !in_names(
			&r.name,
			"::std::iter::Rev ::std::iter::Enumerate ::std::iter::FlatMap ::std::iter::Chain ::std::string::Chars ::std::string::Split ::std::object::Iter",
		) && !r.name.ends_with("RangeFromIter")
		{
			Verdict::Admit
		} else {
			refuse
		};
	}
	if !parameters.is_empty() {
		return refuse;
	}
	let yes = match r.name.as_str() {
		"::std::i64" | "::std::u64" => in_names(
			name,
			"abs pow checked_abs checked_add checked_sub checked_mul checked_div checked_rem checked_pow wrapping_abs wrapping_add wrapping_sub wrapping_mul wrapping_div wrapping_rem wrapping_pow saturating_abs saturating_add saturating_sub saturating_mul saturating_pow signum to_string clone cmp min max",
		),
		"::std::f64" => in_names(
			name,
			"abs ceil exp floor ln max min partial_cmp powf powi round sqrt to_string clone cmp",
		),
		"::std::char" => name == "to_i64",
		"::std::string::String" => in_names(
			name,
			"chars clone cmp contains ends_with is_empty len push replace split starts_with trim",
		),
		"::std::vec::Vec" => in_names(name, "clone cmp is_empty iter len push resize sort_by"),
		"::std::tuple::Tuple" => in_names(name, "clone cmp is_empty iter len"),
		"::std::object::Object" => in_names(
			name,
			"clone contains_key get insert is_empty iter len remove",
		),
		"::std::option::Option" => in_names(
			name,
			"clone cmp expect is_none is_some iter unwrap unwrap_or",
		),
		"::std::result::Result" => in_names(name, "clone cmp expect is_err is_ok unwrap unwrap_or"),
		"::std::ops::Function" => name == "clone",
		"::std::object::Iter" => name == "len",
		"::std::option::Iter" => name == "clone",
		_ if r.name.contains("::ops::Range") && !iterator(r) => {
			name == "cmp"
				|| (name == "iter"
					&& !in_names(
						&r.name,
						"::std::ops::RangeFull ::std::ops::RangeTo ::std::ops::RangeToInclusive",
					))
		}
		_ if r.name.ends_with("RangeIter")
			|| r.name.ends_with("RangeInclusiveIter")
			|| r.name == "::std::slice::Iter" =>
		{
			name == "len"
		}
		_ => false,
	};
	if yes {
		Verdict::Admit
	} else {
		refuse
	}
}
fn protocol_policy(r: &Receiver, name: &str, parameters: &[Hash], inline: bool) -> Verdict {
	if inline && name == "ADD_ASSIGN" {
		return Verdict::Refuse(Refusal::Assignment);
	}
	if r.virtual_chain && !inline {
		return Verdict::Refuse(Refusal::Method);
	}
	if r.virtual_chain && name != "NEXT" {
		return Verdict::Refuse(Refusal::Method);
	}
	match name {
		"NEXT" => method_policy(r, "next", parameters, inline),
		"LEN" => method_policy(r, "len", parameters, inline),
		"INTO_ITER" => {
			if iterator(r) && !r.virtual_chain
				|| matches!(method_policy(r, "iter", parameters, inline), Verdict::Admit)
			{
				Verdict::Admit
			} else {
				Verdict::Refuse(Refusal::Method)
			}
		}
		"CLONE" => method_policy(r, "clone", parameters, inline),
		"CMP" => method_policy(r, "cmp", parameters, inline),
		"PARTIAL_CMP" => method_policy(r, "partial_cmp", parameters, inline),
		"MIN" => method_policy(r, "min", parameters, inline),
		"MAX" => method_policy(r, "max", parameters, inline),
		"DEBUG_FMT" | "DISPLAY_FMT" | "EQ" | "PARTIAL_EQ" | "LT" | "LE" | "GT" | "GE" | "ADD"
		| "ADD_ASSIGN" | "INDEX_GET" | "INDEX_SET" | "IS_VARIANT" | "TRY" => Verdict::Admit,
		_ => Verdict::Refuse(Refusal::Method),
	}
}
fn scalar(context: &Context, hash: Hash) -> String {
	receiver(context, hash)
		.map(|r| r.source)
		.unwrap_or_else(|| "()".into())
}
fn arguments(
	context: &Context,
	signature: &Signature,
	receiver: Option<&Receiver>,
	name: &str,
) -> Vec<String> {
	let mut args: Vec<_> = signature
		.arguments
		.as_ref()
		.map(|a| a.iter().map(|a| scalar(context, a.base)).collect())
		.unwrap_or_default();
	if receiver.is_some() && !args.is_empty() {
		args.remove(0);
	}
	let r = receiver.map(|r| r.source.as_str()).unwrap_or("1");
	if signature.arguments.is_none() {
		args = match name {
			"cmp" | "partial_cmp" | "min" | "max" | "eq" | "partial_eq" | "ne" => vec![r.into()],
			"skip" | "take" | "nth" | "nth_back" => vec!["1".into()],
			"chain" => vec!["[3]".into()],
			"map" | "filter" | "filter_map" | "flat_map" | "any" | "all" | "find" | "position"
			| "rposition" | "for_each" | "reduce" => vec!["()".into()],
			"fold" => vec!["0".into(), "()".into()],
			_ => vec![],
		};
	}
	match name {
		"chain" => args = vec!["[3]".into()],
		"map" => args = vec!["|x|x".into()],
		"filter" | "any" | "all" | "find" | "position" | "rposition" => {
			args = vec!["|x|true".into()]
		}
		"filter_map" => args = vec!["|x|Some(x)".into()],
		"flat_map" => args = vec!["|x|[x]".into()],
		"for_each" => args = vec!["|x|()".into()],
		"fold" => args = vec!["0".into(), "|a,b|a+b".into()],
		"reduce" => args = vec!["|a,b|a+b".into()],
		"sort_by" => args = vec!["|a,b|a.cmp(b)".into()],
		"split" => args = vec!["\" \"".into()],
		_ => {}
	}
	args
}
fn observe(context: &Context, source: &str) -> Verdict {
	let mut sources = Sources::new();
	sources
		.insert(Source::new("inventory.rn", source).unwrap())
		.unwrap();
	match frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1) {
		Ok(_) => Verdict::Admit,
		Err(e) => Verdict::Refuse(
			e.refusal
				.unwrap_or_else(|| panic!("non-refusal in registry witness {source}: {e:?}")),
		),
	}
}
fn generic(context: &Context, parameters: &[Hash]) -> String {
	if parameters.is_empty() {
		return String::new();
	}
	format!(
		"::<{}>",
		parameters
			.iter()
			.map(|p| context.types.get(p).unwrap().item.to_string())
			.collect::<Vec<_>>()
			.join(",")
	)
}
fn free_policy(item: &str) -> Verdict {
	if in_names(
		item,
		"::std::any::type_name_of_val ::std::char::from_i64 ::std::clone::clone ::std::io::print ::std::io::println ::std::iter::range ::std::mem::drop ::std::panic ::std::vec::Vec::new ::std::vec::Vec::with_capacity ::std::string::String::from",
	) {
		Verdict::Admit
	} else {
		Verdict::Refuse(Refusal::Native)
	}
}
fn expression(
	r: &Receiver,
	kind: &AssociatedKind,
	name: &str,
	args: &[String],
	generics: &str,
	target: &str,
) -> String {
	match kind {
		AssociatedKind::FieldFn(p, field) => match p.name {
			"GET" => format!("{target}.{field}"),
			"SET" => format!("{target}.{field}=1"),
			_ => panic!("unclassified field protocol"),
		},
		AssociatedKind::IndexFn(p, index) => match p.name {
			"GET" => format!("{target}.{index}"),
			"SET" => format!("{target}.{index}=1"),
			_ => panic!("unclassified index protocol"),
		},
		AssociatedKind::Protocol(p) => {
			let rhs = args.first().map(String::as_str).unwrap_or(&r.source);
			match p.name {
				"DEBUG_FMT" => format!("format!(\"{{:?}}\",{target})"),
				"DISPLAY_FMT" => format!("format!(\"{{}}\",{target})"),
				"EQ" | "PARTIAL_EQ" => format!("{target}==({rhs})"),
				"LT" => format!("{target}<({rhs})"),
				"LE" => format!("{target}<=({rhs})"),
				"GT" => format!("{target}>({rhs})"),
				"GE" => format!("{target}>=({rhs})"),
				"ADD" => format!("{target}+({rhs})"),
				"ADD_ASSIGN" => format!("{target}+=({rhs})"),
				"INDEX_GET" => format!("{target}[0]"),
				"INDEX_SET" => format!("{target}[0]=1"),
				"IS_VARIANT" => match r.name.as_str() {
					"::std::option::Option" => format!("match {target} {{Some(v)=>v,None=>0}}"),
					"::std::result::Result" => format!("match {target} {{Ok(v)=>v,Err(v)=>v}}"),
					"::std::cmp::Ordering" => format!(
						"match {target} {{::std::cmp::Ordering::Less=>-1,::std::cmp::Ordering::Equal=>0,::std::cmp::Ordering::Greater=>1}}"
					),
					_ => panic!("unclassified represented variant"),
				},
				"TRY" => format!("{target}?"),
				_ => format!("{target}.{name}{generics}({})", args.join(",")),
			}
		}
		AssociatedKind::Instance(_) => format!("{target}.{name}{generics}({})", args.join(",")),
	}
}

#[test]
fn registry_verdicts_are_live_and_every_metadata_row_is_accounted_for() {
	let context = Context::with_default_modules().unwrap();
	let mut rows = Vec::new();
	let mut failures = Vec::new();
	let mut totals = BTreeMap::<String, usize>::new();
	let mut unique_witnesses = BTreeMap::<String, Verdict>::new();
	for meta in &context.meta {
		let Kind::Function {
			associated,
			signature,
			container,
			parameter_types,
			..
		} = &meta.kind
		else {
			continue;
		};
		let item = meta
			.item
			.as_ref()
			.map(ToString::to_string)
			.unwrap_or_default();
		let mut forms = Vec::<(String, Verdict)>::new();
		let unreachable = if !context.functions.contains_key(&meta.hash) {
			Some("abstract trait declaration")
		} else if let Some(container) = container {
			if let Some(r) = receiver(&context, *container) {
				let kind = associated
					.as_ref()
					.expect("associated receiver without a kind");
				let name = match kind {
					AssociatedKind::Instance(n) => n.as_ref(),
					AssociatedKind::Protocol(p) => p.method.unwrap_or(p.name),
					AssociatedKind::FieldFn(_, n) => n.as_ref(),
					AssociatedKind::IndexFn(_, _) => "index",
				};
				if matches!(kind,AssociatedKind::Protocol(p) if p.name=="HASH") {
					Some("Hasher input has no Root representation")
				} else if in_names(
					&item,
					"::std::vec::Vec::new ::std::vec::Vec::with_capacity ::std::string::String::from",
				) {
					let args = arguments(&context, signature, None, name);
					forms.push((
						format!("pub fn main(){{{item}({})}}", args.join(",")),
						free_policy(&item),
					));
					None
				} else {
					let args = arguments(&context, signature, Some(&r), name);
					let g = generic(&context, parameter_types);
					for inline in [false, true] {
						let target = if inline {
							format!("({})", r.source)
						} else {
							"x".into()
						};
						let mut expr = expression(&r, kind, name, &args, &g, &target);
						if inline
							&& matches!(kind, AssociatedKind::Instance(_))
							&& in_names(
								name,
								"map filter filter_map flat_map enumerate chain rev skip take",
							) {
							expr.push_str(".count()");
						}
						let expected = match kind {
							AssociatedKind::Instance(_) => {
								method_policy(&r, name, parameter_types, inline)
							}
							AssociatedKind::Protocol(p) => {
								protocol_policy(&r, p.name, parameter_types, inline)
							}
							AssociatedKind::FieldFn(..) | AssociatedKind::IndexFn(..) => {
								Verdict::Admit
							}
						};
						let source = if inline {
							format!("pub fn main(){{{expr}}}")
						} else {
							format!("pub fn main(){{let x={};{expr}}}", r.source)
						};
						forms.push((source, expected));
					}
					None
				}
			} else {
				Some("receiver outside the closed input/value domain")
			}
		} else {
			let args = if signature.arguments.is_none() {
				vec!["1".into()]
			} else {
				arguments(&context, signature, None, "")
			};
			forms.push((
				format!("pub fn main(){{{item}({})}}", args.join(",")),
				free_policy(&item),
			));
			None
		};
		let mut any_admit = false;
		let mut observed_forms = Vec::new();
		for (source, expected) in forms {
			let actual = *unique_witnesses
				.entry(source.clone())
				.or_insert_with(|| observe(&context, &source));
			if actual != expected {
				failures.push(format!(
					"{item} {}: expected {expected:?}, got {actual:?}: {source}",
					meta.hash
				));
			}
			any_admit |= actual == Verdict::Admit;
			observed_forms.push(
				json!({"source":source,"policy":format!("{expected:?}"),"actual":format!("{actual:?}")}),
			);
		}
		let verdict = if unreachable.is_some() {
			"not-reachable"
		} else if any_admit {
			"conditional-admission"
		} else {
			"compile-refusal"
		};
		*totals.entry(verdict.into()).or_default() += 1;
		rows.push(json!({"hash":meta.hash.to_string(),"item":item,"container":container.map(|h|h.to_string()),"parameters":parameter_types.iter().map(ToString::to_string).collect::<Vec<_>>(),"verdict":verdict,"reason":unreachable,"forms":observed_forms}));
	}
	assert!(!rows.is_empty());
	assert_eq!(rows.len(), totals.values().sum::<usize>());
	assert!(totals
		.keys()
		.all(|k| in_names(k, "not-reachable conditional-admission compile-refusal")));
	if let Some(out) = std::env::var_os("RUNE_R1B_OUT") {
		std::fs::write(out,serde_json::to_vec_pretty(&json!({"rows":rows,"counts":totals,"observations":"computed from typed Context metadata and current source compilers"})).unwrap()).unwrap();
	}
	assert!(
		failures.is_empty(),
		"{} registry policy mismatches:\n{}",
		failures.len(),
		failures.join("\n")
	);
}

#[test]
fn constructors_and_macros_have_live_closed_policies() {
	let context = Context::with_default_modules().unwrap();
	let mut constructors = 0;
	let mut macros = 0;
	for meta in &context.meta {
		let item = meta
			.item
			.as_ref()
			.map(ToString::to_string)
			.unwrap_or_default();
		match &meta.kind {
			Kind::Struct {
				constructor: Some(sig),
				fields,
				..
			} => {
				constructors += 1;
				let short = item.rsplit("::").next().unwrap();
				let range = match short {
					"Range" => Some("(1..3)"),
					"RangeFrom" => Some("(1..)"),
					"RangeFull" => Some("(..)"),
					"RangeInclusive" => Some("(1..=3)"),
					"RangeTo" => Some("(..3)"),
					"RangeToInclusive" => Some("(..=3)"),
					_ => None,
				};
				let expr = range.map(ToString::to_string).unwrap_or_else(|| {
					if matches!(fields, meta::Fields::Empty) {
						item.clone()
					} else {
						format!("{item}({})", arguments(&context, sig, None, "").join(","))
					}
				});
				let expected =
					if item.contains("ControlFlow::") || item.contains("GeneratorState::") {
						Verdict::Refuse(Refusal::Native)
					} else {
						Verdict::Admit
					};
				assert_eq!(
					observe(&context, &format!("pub fn main(){{{expr}}}")),
					expected,
					"{item}"
				);
			}
			Kind::Macro => {
				macros += 1;
				let (args, expected) = match item.as_str() {
					"::std::panic" => ("\"probe\"", Verdict::Admit),
					"::std::fmt::format" | "::std::io::print" | "::std::io::println" => {
						("\"{}\",1", Verdict::Admit)
					}
					"::std::test::assert" => ("true", Verdict::Admit),
					"::std::test::assert_eq" => ("1,1", Verdict::Admit),
					"::std::test::assert_ne" => ("1,2", Verdict::Admit),
					"::std::stringify" => ("hello", Verdict::Refuse(Refusal::FrontendDiagnostic)),
					"::std::io::dbg" => ("1", Verdict::Refuse(Refusal::FrontendDiagnostic)),
					"::std::macros::builtin::file" | "::std::macros::builtin::line" => {
						("", Verdict::Refuse(Refusal::FrontendDiagnostic))
					}
					_ => panic!("new default macro needs a routing policy: {item}"),
				};
				assert_eq!(
					observe(&context, &format!("pub fn main(){{{item}!({args})}}")),
					expected,
					"{item}"
				);
			}
			_ => {}
		}
	}
	assert!(constructors > 0 && macros > 0);
}

#[test]
fn uncertain_calls_and_async_entries_refuse_before_host_effects() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};
	let effects = Arc::new(AtomicUsize::new(0));
	let mut context = Context::with_default_modules().unwrap();
	let mut module = crate::Module::with_crate("inventory").unwrap();
	let count = effects.clone();
	module
		.host_function("first", move || {
			count.fetch_add(1, Ordering::SeqCst);
		})
		.unwrap();
	let count = effects.clone();
	module
		.function("uncertified", move || {
			count.fetch_add(1, Ordering::SeqCst);
		})
		.build()
		.unwrap();
	context.install(module).unwrap();
	for (source, expected) in [
		(
			"pub fn main(){inventory::first();inventory::uncertified()}",
			Refusal::Native,
		),
		(
			"pub async fn main(){inventory::first();42}",
			Refusal::AsyncFunction,
		),
		(
			"pub async fn work(){inventory::first();42} pub use work as main;",
			Refusal::AsyncFunction,
		),
		("pub fn main(){inventory::first();yield 1;}", Refusal::Yield),
	] {
		assert_eq!(observe(&context, source), Verdict::Refuse(expected));
		assert_eq!(effects.load(Ordering::SeqCst), 0);
	}
	let sources = BTreeSet::from([
		"pub fn main(){[1].iter().map(|x|x).count()}",
		"pub fn main(){let x=[1].iter().map(|x|x);x.count()}",
	]);
	assert_eq!(
		sources
			.iter()
			.filter(|s| observe(&context, s) == Verdict::Admit)
			.count(),
		1
	);
}
