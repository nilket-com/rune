//! Output certificates are declarations, never inferred from output samples.
use super::*;
use crate::experimental_core::{
	error::Refusal,
	execute::Executor,
	frontend,
	heap::{Limits, Store},
};
use crate::{Context, Diagnostics, Module, Source, Sources};
fn compile(
	context: &Context,
	source: &str,
) -> crate::experimental_core::error::Result<frontend::Program> {
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	frontend::compile(&mut sources, context, &mut Diagnostics::new(), 1)
}
fn value() -> crate::Value {
	crate::Value::from(7i64)
}
#[test]
fn undeclared_and_unimportable_outputs_refuse_before_host_effects() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};
	let calls = Arc::new(AtomicUsize::new(0));
	let mut module = Module::new();
	let count = calls.clone();
	module
		.host_function("undeclared", move || {
			count.fetch_add(1, Ordering::SeqCst);
			value()
		})
		.unwrap();
	let count = calls.clone();
	module
		.host_function_with_output(
			"mixed",
			move || {
				count.fetch_add(1, Ordering::SeqCst);
				value()
			},
			OutputCertificate {
				nodes: &[
					OutputShape::Union(&[1, 2]),
					OutputShape::Scalar(HostType::I64),
					OutputShape::Unsupported(Hash::EMPTY),
				],
				root: 0,
			},
		)
		.unwrap();
	for (name, certificate) in [
		(
			"bad_index",
			OutputCertificate {
				nodes: &[OutputShape::Vec(1)],
				root: 0,
			},
		),
		(
			"bad_root",
			OutputCertificate {
				nodes: &[],
				root: 0,
			},
		),
		(
			"empty_union",
			OutputCertificate {
				nodes: &[OutputShape::Union(&[])],
				root: 0,
			},
		),
	] {
		module
			.host_function_with_output(name, value, certificate)
			.unwrap();
	}
	module
		.host_function_with_output(
			"wrong_result",
			|| -> Result<crate::Value, std::string::String> { Ok(value()) },
			OutputCertificate {
				nodes: &[OutputShape::Scalar(HostType::I64)],
				root: 0,
			},
		)
		.unwrap();
	let mut context = Context::new();
	context.install(module).unwrap();
	for name in [
		"undeclared",
		"mixed",
		"bad_index",
		"bad_root",
		"empty_union",
		"wrong_result",
	] {
		assert!(context.lookup_function(Hash::type_hash([name])).is_some());
		assert!(context.host(Hash::type_hash([name])).is_none());
		for args in ["", "1"] {
			let err = compile(&context, &std::format!("pub fn main(){{{name}({args})}}"))
				.err()
				.unwrap();
			assert_eq!(err.refusal, Some(Refusal::Native));
		}
	}
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn certified_dynamic_and_typed_outputs_remain_exact() {
	let mut module = Module::new();
	module
		.host_function_with_output(
			"dynamic",
			value,
			OutputCertificate {
				nodes: &[OutputShape::Scalar(HostType::I64)],
				root: 0,
			},
		)
		.unwrap();
	module.host_function("typed", || 9i64).unwrap();
	module
		.host_function_with_output(
			"json",
			|| -> Result<crate::Value, std::string::String> { Ok(crate::Value::unit()) },
			OutputCertificate::JSON_RESULT,
		)
		.unwrap();
	let mut context = Context::new();
	context.install(module).unwrap();
	let program = compile(&context, "pub fn main(){dynamic()+typed()}").unwrap();
	let mut executor = Executor::new().unwrap();
	executor
		.set_host_runtime(Some(
			crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
		))
		.unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let root = executor
		.invoke(
			&program,
			&mut store,
			program.function(Hash::type_hash(["main"])).unwrap(),
			&[],
			100,
		)
		.unwrap();
	assert_eq!(root.value, crate::experimental_core::value::Value::I64(16));
	store.root_release(root).unwrap();
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
	assert!(compile(&context, "pub fn main(){json()}").is_ok());
}
#[test]
fn recursive_shape_graph_checks_every_union_alternative() {
	const GRAPH: &[OutputShape] = &[
		OutputShape::Vec(0),
		OutputShape::Tuple(&[0, 2]),
		OutputShape::Ordering,
		OutputShape::Object(1),
		OutputShape::Option(3),
		OutputShape::Result(4, 2),
		OutputShape::Union(&[0, 1, 2, 3, 4, 5]),
	];
	assert!(OutputCertificate {
		nodes: GRAPH,
		root: 6
	}
	.importable());
	assert!(OutputCertificate::JSON_RESULT.certifies(HostOutputKind::ResultDynamicString));
	assert!(!OutputCertificate::JSON_RESULT.certifies(HostOutputKind::Scalar(HostType::I64)));
}
