//! The fresh-core frontend: compiles Rune sources into a [`Program`].
//!
//! The ordinary Rune compiler front end resolves and checks the sources; each admitted
//! function is then lowered to fresh-core code. The legacy `UnitBuilder` is metadata
//! scratch only: no function is assembled into a legacy unit.

use super::code::{Builder, Code, Function};
use super::error::{Error, ErrorKind, Location, Refusal, Result};
use crate::alloc::{String, Vec};
use crate::ast::Spanned;
use crate::compile::{
	self, NoopCompileVisitor, NoopSourceLoader, Options, Pool, Prelude, UnitBuilder,
};
use crate::runtime::unit::DefaultStorage;
use crate::{Context, Diagnostics, Hash, Sources, Statics};

/// A copy of one compiled source, kept so a host can render error locations after
/// the [`Sources`] are gone.
pub struct SourceRecord {
	/// The source id ([`Location::source`]).
	pub id: u32,
	/// The revision passed to [`compile()`] ([`Location::revision`]).
	pub revision: u64,
	/// The source name.
	pub name: String,
	/// The source text.
	pub text: String,
}

/// A compiled program, run by an
/// [`Executor`](crate::experimental_core::execute::Executor).
///
/// Function values made by one program are foreign to every other program.
pub struct Program {
	pub(crate) code: Code,
	functions: Vec<Hash>,
	pub(crate) sources: Vec<SourceRecord>,
}

impl Program {
	#[cfg(test)]
	pub(crate) fn for_diagnostics_test(code: Code) -> Self {
		Self {
			code,
			functions: Vec::new(),
			sources: Vec::new(),
		}
	}

	/// Source records for rendering error locations after frontend teardown.
	pub fn sources(&self) -> &[SourceRecord] {
		&self.sources
	}

	/// The id of the function whose item hash is `hash`, for
	/// [`Executor::invoke`](crate::experimental_core::execute::Executor::invoke).
	///
	/// For a top-level `pub fn main`, `hash` is `Hash::type_hash(["main"])`.
	pub fn function(&self, hash: Hash) -> Option<u32> {
		self.functions
			.iter()
			.position(|h| *h == hash)
			.map(|n| n as u32)
	}
}

pub(crate) struct Collector {
	revision: u64,
	entries: Vec<(Hash, Option<Function>, Option<Location>)>,
	pub(crate) failure: Option<Error>,
}

impl Collector {
	pub(crate) fn revision(&self) -> u64 {
		self.revision
	}

	pub(crate) fn reserve(&mut self, hash: Hash) -> Result<u32> {
		if let Some(id) = self.entries.iter().position(|(h, _, _)| *h == hash) {
			return Ok(id as u32);
		}

		if self.entries.len() >= 65_536 {
			return Err(Error::new(ErrorKind::FrameCapacity, "function registry"));
		}

		let id = self.entries.len() as u32;
		self.entries.try_push((hash, None, None))?;
		Ok(id)
	}

	pub(crate) fn lower(
		&mut self,
		hash: Hash,
		source: u32,
		exprs: &crate::hir::Exprs<'_>,
		item: &crate::hir::ItemFn<'_>,
		constants: &crate::query::QueryInner<'_>,
		context: &Context,
		refused_functions: &[(Hash, Refusal)],
	) -> Result<()> {
		let id = self.reserve(hash)?;
		if self.entries[id as usize].1.is_some() {
			return Err(Error::new(ErrorKind::InvalidCode, "duplicate function"));
		}

		let revision = self.revision;
		let result = super::lower::function(
			exprs,
			item,
			source,
			revision,
			&mut |hash, at| {
				if let Some((_, operation)) = refused_functions.iter().find(|(h, _)| *h == hash) {
					return Err(Error::refused(*operation).at(at));
				}

				if context.lookup_function(hash).is_some() {
					return Err(Error::refused(Refusal::Native).at(at));
				}

				let id = self.reserve(hash)?;
				self.entries[id as usize].2.get_or_insert(at);
				Ok(id)
			},
			constants,
			context.fresh_stdio_println(),
		);
		let function = match result {
			Ok(function) => function,
			Err(error) => {
				self.failure.get_or_insert(error);
				return Err(error);
			}
		};

		self.entries[id as usize].1 = Some(function);
		Ok(())
	}

	fn finish(self) -> Result<Program> {
		let mut builder = Builder::default();
		let mut functions = Vec::new();

		for (hash, function, location) in self.entries {
			let function = function.ok_or_else(|| {
				let mut error = Error::refused(Refusal::UnresolvedFunction);
				error.location = location;
				error
			})?;
			functions.try_push(hash)?;
			builder.push(function)?;
		}

		Ok(Program {
			code: builder.finish()?,
			functions,
			sources: Vec::new(),
		})
	}
}

/// Compiles `sources` against `context` for the fresh core.
///
/// Anything outside the admitted subset is a compile-time refusal: an [`Error`] with
/// kind [`ErrorKind::Unsupported`] and a [`Refusal`] category. Compiler errors are
/// reported to `diagnostics` and returned as [`Refusal::FrontendDiagnostic`]. Nothing
/// falls back to the stock VM; that choice is the caller's.
///
/// `println!` is admitted only when the context's `::std::io::println` is the std
/// stdio builtin (as installed by [`Context::with_default_modules`]). A capture, disable
/// or custom printer is refused with [`Refusal::Native`]. When no `::std::io::println`
/// function is installed at all, the Rune front end rejects the unresolved call first, and
/// the refusal is [`Refusal::FrontendDiagnostic`].
///
/// Each struct type a program declares takes an identity in a process-wide table: see
/// [`Nominal`](crate::experimental_core::value::Nominal) for its limits.
///
/// `revision` is recorded in every [`Location`] of the program.
pub fn compile(
	sources: &mut Sources,
	context: &Context,
	diagnostics: &mut Diagnostics,
	revision: u64,
) -> Result<Program> {
	// Preflight the complete input before proportional source copies.
	let mut bytes = 0usize;
	let mut count = 0usize;

	for id in sources.source_ids() {
		let source = sources
			.get(id)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "source identity"))?;
		count += 1;
		bytes = bytes
			.checked_add(source.as_str().len())
			.and_then(|n| n.checked_add(source.name().len()))
			.ok_or_else(|| Error::refused(Refusal::SourceLimit))?;
		if count > 256 || bytes > 16 << 20 || source.name().len() > 255 {
			return Err(Error::refused(Refusal::SourceLimit));
		}
	}

	let mut retained = Vec::try_with_capacity(count)?;
	for id in sources.source_ids() {
		let source = sources
			.get(id)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "source identity"))?;
		retained.try_push(SourceRecord {
			id: id.into_index() as u32,
			revision,
			name: String::try_from(source.name())?,
			text: String::try_from(source.as_str())?,
		})?;
	}

	let mut collector = Collector {
		revision,
		entries: Vec::new(),
		failure: None,
	};

	let mut unit = UnitBuilder::default();
	let prelude = if context.has_default_modules() {
		Prelude::with_default_prelude()?
	} else {
		Prelude::default()
	};

	let mut pool = Pool::new()?;
	let mut visitor = NoopCompileVisitor::new();
	let mut loader = NoopSourceLoader::default();
	let options = Options::default();
	let statics = Statics::new();
	let mut storage = DefaultStorage::default();
	compile::compile(
		&mut unit,
		&prelude,
		sources,
		&mut pool,
		context,
		&mut visitor,
		diagnostics,
		&mut loader,
		&options,
		&[],
		&statics,
		&mut storage,
		Some(&mut collector),
	)?;

	if let Some(error) = collector.failure {
		return Err(error);
	}

	if diagnostics.has_error() {
		let mut error = Error::refused(Refusal::FrontendDiagnostic);
		for diagnostic in diagnostics.diagnostics() {
			if let crate::diagnostics::Diagnostic::Fatal(fatal) = diagnostic {
				if let Some(compile) = fatal.as_compile_error() {
					let span = compile.span();
					error.location = Some(Location {
						source: fatal.source_id().into_index() as u32,
						revision,
						start: span.start.0,
						end: span.end.0,
					});
					break;
				}
			}
		}

		return Err(error);
	}

	let mut program = collector.finish()?;
	program.sources = retained;
	Ok(program)
}
