//! C6 indexing-completeness controls. Test-only custom loader; public API unchanged.
use super::*;
use crate as rune;
use crate::{Item, Source, SourceId};

struct Loader {
	loads: usize,
	name: &'static str,
}
impl compile::SourceLoader for Loader {
	fn load(
		&mut self,
		_: &Sources,
		_: SourceId,
		item: &Item,
		span: &dyn Spanned,
	) -> compile::Result<Source> {
		if item != crate::item!(foo) {
			return Err(compile::Error::msg(span, "unexpected C6 test module"));
		}
		self.loads += 1;
		Ok(Source::new(
			"foo.rn",
			std::format!(
				"pub struct Foo; impl Foo {{ fn {}(self) {{ 1 }} }}",
				self.name
			),
		)?)
	}
}

fn indexed(text: &str, loader: &mut dyn compile::SourceLoader) -> (Result<Program>, bool) {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(text).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let mut collector = Collector {
		revision: 7,
		entries: Vec::new(),
		transaction: CompileTransaction::default(),
		sort_helper: None,
		any_helper: None,
		native_helpers: crate::hash::Map::default(),
		failure: None,
		variable_moved: None,
	};
	let result = compile::compile(
		&mut UnitBuilder::default(),
		&Prelude::with_default_prelude().unwrap(),
		&mut sources,
		&mut Pool::new().unwrap(),
		&context,
		&mut NoopCompileVisitor::new(),
		&mut diagnostics,
		loader,
		&Options::default(),
		&[],
		&Statics::new(),
		&mut DefaultStorage::default(),
		Some(&mut collector),
	);
	if result.is_err() {
		return (
			Err(Error::refused(Refusal::FrontendDiagnostic)),
			diagnostics.has_error(),
		);
	}
	if let Some(error) = collector.failure {
		return (Err(error), false);
	}
	if diagnostics.has_error() {
		// Index-time failures may reserve ids but complete no Function.
		assert!(collector.entries.iter().all(|entry| entry.1.is_none()));
		return (Err(Error::refused(Refusal::FrontendDiagnostic)), true);
	}
	(collector.finish(), false)
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles source and standard Context; native CI checks indexing"
)]
fn script_numeric_names_close_method_admission_in_every_indexing_scope() {
	for name in ["abs", "round", "powf"] {
		for source in [
		"pub struct Foo; impl Foo { fn abs(self) { 1 } } pub fn main(){ (-2.0).abs() }",
		"mod nested { pub struct Foo; impl Foo { fn abs(self) { 1 } } } pub fn main(){ (-2.0).abs() }",
		"fn hidden(){ struct Foo; impl Foo { fn abs(self) { 1 } } } pub fn main(){ (-2.0).abs() }",
		"pub struct Foo; impl Foo { fn abs(x) { x } } pub fn main(){ (-2.0).abs() }",
	] {
		let source = rename(source, name);
		let (result, diagnostic) = indexed(&source, &mut NoopSourceLoader::default());
		assert!(!diagnostic, "{source}");
		assert_eq!(result.err().unwrap().refusal, Some(Refusal::Method), "{source}");
	}
		let (result, diagnostic) = indexed(
			&rename("pub fn main(){ (-2.0).abs() }", name),
			&mut NoopSourceLoader::default(),
		);
		assert!(!diagnostic);
		assert!(result.is_ok());
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles source and standard Context; native CI checks indexing"
)]
fn later_loaded_impl_closes_name_before_any_fresh_lowering() {
	for name in ["abs", "round", "powf"] {
		let mut loader = Loader { loads: 0, name };
		let (result, diagnostic) = indexed(
			&rename("mod foo; pub fn main(){ (-2.0).abs() }", name),
			&mut loader,
		);
		assert_eq!(loader.loads, 1);
		assert!(!diagnostic);
		assert_eq!(result.err().unwrap().refusal, Some(Refusal::Method));
	}
}

#[test]
#[cfg_attr(
	miri,
	ignore = "compiles source and standard Context; native CI checks indexing"
)]
fn failed_impl_indexing_never_yields_partial_numeric_admission() {
	for name in ["abs", "round", "powf"] {
		let (result, diagnostic) = indexed(
			&rename(
				"impl MissingType { fn abs(self) { 1 } } pub fn main(){ (-2.0).abs() }",
				name,
			),
			&mut NoopSourceLoader::default(),
		);
		assert!(diagnostic);
		assert_eq!(
			result.err().unwrap().refusal,
			Some(Refusal::FrontendDiagnostic)
		);
	}
}

fn rename(source: &str, name: &str) -> std::string::String {
	source
		.replace(
			".abs()",
			&std::format!(".{}({})", name, if name == "powf" { "2.0" } else { "" }),
		)
		.replace("fn abs(", &std::format!("fn {name}("))
}
