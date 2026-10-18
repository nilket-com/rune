//! Diagnostic controls shared by native tests and Miri.
use crate::experimental_core::frontend;
use crate::runtime as stock;
use crate::Hash;
use std::{format, string::ToString};

#[test]
fn missing_method_without_context_preserves_identity() {
	use crate::runtime::{TypeHash, TypeOf, VmErrorKind};
	let instance = <i64 as TypeHash>::HASH;
	let hash = Hash::associated_function(instance, Hash::ident("absent"));
	let error = stock::VmError::from(VmErrorKind::MissingInstanceFunction {
		hash,
		instance: <i64 as TypeOf>::type_info(),
	});
	assert!(error.stacktrace().is_empty());
	assert_eq!(error.to_string(), "Type `std::i64` has no matching method");
	let identity = error.missing_instance_function().unwrap();
	assert_eq!(identity.hash, hash);
	assert_eq!(identity.instance, instance);
	assert!(format!("{error:?}").contains(&hash.to_string()));
}

#[test]
fn source_method_names_require_tokens_and_hash_identity() {
	use crate::experimental_core::{
		code::{Builder, Function, Op},
		detail::{Detail, TypeName},
		error::{Error, ErrorKind, Location},
	};
	let instance = Hash::type_hash(["receiver"]);
	let hash = Hash::associated_function(instance, Hash::ident("méthode"));
	let render = |source: &str, receiver, revision| {
		let mut function = Function::new(1, 0).unwrap();
		function
			.push(Op::Return { value: 0 }, Location::default())
			.unwrap();
		let mut builder = Builder::default();
		builder.push(function).unwrap();
		let mut program = frontend::Program::for_diagnostics_test(builder.finish().unwrap());
		program
			.sources
			.try_push(frontend::SourceRecord {
				id: 0,
				revision: 16,
				name: crate::alloc::String::new(),
				text: crate::alloc::String::try_from(source).unwrap(),
			})
			.unwrap();
		Error::new(ErrorKind::Type, "diagnostic control")
			.with_detail(Detail::MissingInstanceFunction {
				target: TypeName::I64,
				hash,
				instance: receiver,
			})
			.at(Location {
				source: 0,
				revision,
				start: 0,
				end: 0,
			})
			.message(&program)
			.to_string()
	};
	for source in [
		"x.méthode()",
		"identity(x).méthode()",
		"x. /* name follows a comment */ méthode()",
		"let s = \"x.decoy()\"; x.méthode()",
	] {
		assert_eq!(
			render(source, instance, 16),
			"Type `std::i64` has no method named `méthode`"
		);
	}
	for source in [
		"let méthode = 1;",
		"// x.méthode()\nx.other()",
		"let s = \"x.méthode()\"; x.other()",
		"x.0",
		"0..1",
	] {
		assert_eq!(
			render(source, instance, 16),
			"Type `std::i64` has no matching method"
		);
	}
	assert_eq!(
		render("x.méthode()", Hash::EMPTY, 16),
		"Type `std::i64` has no matching method"
	);
	assert_eq!(
		render("x.méthode()", instance, 17),
		"Type `std::i64` has no matching method"
	);
}
