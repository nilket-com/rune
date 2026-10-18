//! Logical invocation frames shared by the execution engines.
use core::fmt;

use crate::{ast::Span, SourceId, Sources};

use super::Unit;

/// Typed auxiliary context, separate from the logical invocation frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationContext {
	/// An argument conversion or access failed at this native boundary index.
	BadArgument {
		/// The engine's native argument index, without an inferred user label.
		arg: usize,
	},
}

/// The position of a frame in an invocation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalFrameRole {
	/// The operation which failed.
	Origin,
	/// An active caller, ordered from inner to outer.
	Caller,
}

/// A source location resolved from execution metadata.
#[derive(Clone, Copy)]
pub struct LogicalLocation<'a> {
	/// Source identity within the compilation's source collection.
	pub source_id: SourceId,
	/// The original byte span.
	pub span: Span,
	/// The stock unit which supplied this location, when applicable.
	/// Source ids from different compilations must not be conflated.
	pub unit: Option<&'a Unit>,
}

/// One logical frame. Missing debug information remains an explicit frame.
#[derive(Clone, Copy)]
pub struct LogicalFrame<'a> {
	/// Origin or caller.
	pub role: LogicalFrameRole,
	/// The location, if execution metadata could resolve it.
	pub location: Option<LogicalLocation<'a>>,
}

/// Write a logical frame using one-based character line and column numbers.
/// The sources must belong to the compilation which supplied the frame.
#[cold]
#[inline(never)]
pub fn write_logical_frame(
	f: &mut fmt::Formatter<'_>,
	frame: LogicalFrame<'_>,
	sources: &Sources,
) -> fmt::Result {
	let label = match frame.role {
		LogicalFrameRole::Origin => "at",
		LogicalFrameRole::Caller => "called from",
	};
	write!(f, "\n  {label} ")?;
	if let Some(at) = frame.location {
		if let Some(source) = sources.get(at.source_id) {
			let range = at.span.range();
			if source.as_str().get(range.clone()).is_some() {
				let (line, column) = source.find_line_column(range.start);
				return write!(f, "{}:{}:{}", source.name(), line + 1, column + 1);
			}
		}
	}
	f.write_str("<unknown>")
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Render<'a>(LogicalFrame<'a>, &'a Sources);
	impl fmt::Display for Render<'_> {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			write_logical_frame(f, self.0, self.1)
		}
	}

	#[test]
	fn d1e_renderer_bounds_unicode_and_unknown() {
		let mut sources = Sources::new();
		let id = sources
			.insert(crate::Source::new("unicode.rn", "é\nx").unwrap())
			.unwrap();
		let frame = |start, end| LogicalFrame {
			role: LogicalFrameRole::Caller,
			location: Some(LogicalLocation {
				source_id: id,
				span: Span::new(start, end),
				unit: None,
			}),
		};
		assert_eq!(
			rust_alloc::format!("{}", Render(frame(3u32, 4u32), &sources)),
			"\n  called from unicode.rn:2:1"
		);
		for (start, end) in [(1u32, 2u32), (9, 10), (4, 3)] {
			assert_eq!(
				rust_alloc::format!("{}", Render(frame(start, end), &sources)),
				"\n  called from <unknown>"
			);
		}
	}
}
