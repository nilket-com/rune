//! Testing and benchmarking.

use crate as rune;
use crate::alloc::{self, try_format, Vec};
use crate::compile;
use crate::macros::{quote, FormatArgs, MacroContext, TokenStream};
use crate::runtime::Function;
use crate::{docstring, Any, ContextError, Module};

fn expand_debug_format(
	cx: &mut MacroContext<'_, '_, '_>,
	name: &str,
	variable: &str,
	span: crate::ast::Span,
) -> compile::Result<TokenStream> {
	let format = cx.lit(name)?;
	let mut variable = cx.ident(variable)?;
	variable.span = span;
	let input = quote!(#format, #variable).into_token_stream(cx)?;
	let format = FormatArgs::parse(cx, &input)?;
	let expanded = format.expand_at(cx, span)?;
	let tokens = expanded.into_token_stream(cx)?;
	Ok(tokens)
}

/// Testing and benchmarking.
#[rune::module(::std::test)]
pub fn module() -> Result<Module, ContextError> {
	let mut m = Module::from_meta(self::module__meta)?.with_unique("std::test");

	m.macro_meta(assert)?;
	#[cfg(feature = "execution")]
	m.mark_builtin_macro();
	m.macro_meta(assert_eq)?;
	#[cfg(feature = "execution")]
	m.mark_builtin_macro();
	m.macro_meta(assert_ne)?;
	#[cfg(feature = "execution")]
	m.mark_builtin_macro();

	m.ty::<Bencher>()?.docs(docstring! {
		/// A type to perform benchmarks.
		///
		/// This is the type of the argument to any function which is annotated with `#[bench]`
	})?;

	m.function_meta(Bencher::iter)?;
	Ok(m)
}

/// A helper type to capture benchmarks.
#[derive(Default, Any)]
#[rune(module = crate, item = ::std::test)]
pub struct Bencher {
	fns: Vec<Function>,
}

impl Bencher {
	/// Coerce bencher into its underlying functions.
	pub fn into_functions(self) -> Vec<Function> {
		self.fns
	}

	/// Run a benchmark using the given closure.
	#[rune::function]
	fn iter(&mut self, f: Function) -> alloc::Result<()> {
		self.fns.try_push(f)
	}
}

/// Assert that the expression provided as an argument is true, or cause a vm
/// panic.
///
/// The second argument can optionally be used to format a panic message.
///
/// This is useful when writing test cases.
///
/// # Examples
///
/// ```rune
/// let value = 42;
///
/// assert!(value == 42, "Value was not what was expected, instead it was {}", value);
/// ```
#[rune::macro_]
pub(crate) fn assert(
	cx: &mut MacroContext<'_, '_, '_>,
	stream: &TokenStream,
) -> compile::Result<TokenStream> {
	use crate as rune;

	let mut it = cx.exprs(stream)?.into_iter().peekable();

	let Some(expr) = it.next() else {
		return Err(compile::Error::msg(
			cx.input_span(),
			"expected an expression to assert",
		));
	};

	let message = if it.peek().is_some() {
		Some(FormatArgs::from_exprs(cx, it)?)
	} else {
		None
	};

	let output = if let Some(message) = &message {
		let expanded = message.expand(cx)?;

		quote!(if !(#expr) {
			::std::panic("assertion failed: " + (#expanded));
		})
	} else {
		let message = try_format!("assertion failed: {}", cx.stringify(&expr)?);
		let message = cx.lit(&message)?;

		quote!(if !(#expr) {
			::std::panic(#message);
		})
	};

	Ok(output.into_token_stream(cx)?)
}

/// Assert that the two arguments provided are equal, or cause a vm panic.
///
/// The third argument can optionally be used to format a panic message.
///
/// # Examples
///
/// ```rune
/// let value = 42;
///
/// assert_eq!(value, 42, "Value was not 42, instead it was {}", value);
/// ```
#[rune::macro_]
pub(crate) fn assert_eq(
	cx: &mut MacroContext<'_, '_, '_>,
	stream: &TokenStream,
) -> compile::Result<TokenStream> {
	use crate as rune;

	let mut it = cx.exprs(stream)?.into_iter().peekable();

	let (Some(left), Some(right)) = (it.next(), it.next()) else {
		return Err(compile::Error::msg(
			cx.input_span(),
			"expected two expressions to compare",
		));
	};

	let message = if it.peek().is_some() {
		Some(FormatArgs::from_exprs(cx, it)?)
	} else {
		None
	};

	let output = if let Some(message) = &message {
		let message = message.expand(cx)?;
		let span = cx.macro_span();
		let left_format = expand_debug_format(cx, "\nleft: {:?}", "left", span)?;
		let right_format = expand_debug_format(cx, "\nright: {:?}", "right", span)?;

		quote! {{
			let left = #left;
			let right = #right;

			if !(left == right) {
				let message = #message;
				message += (#left_format);
				message += (#right_format);
				::std::panic("assertion failed (left == right): " + message);
			}
		}}
	} else {
		let message = cx.lit("assertion failed (left == right):")?;
		let span = cx.macro_span();
		let left_format = expand_debug_format(cx, "\nleft: {:?}", "left", span)?;
		let right_format = expand_debug_format(cx, "\nright: {:?}", "right", span)?;

		quote! {{
			let left = #left;
			let right = #right;

			if !(left == right) {
				let message = ::std::string::String::from(#message);
				message += (#left_format);
				message += (#right_format);
				::std::panic(message);
			}
		}}
	};

	Ok(output.into_token_stream(cx)?)
}

/// Assert that the two arguments provided are not equal, or cause a vm panic.
///
/// The third argument can optionally be used to format a panic message.
///
/// # Examples
///
/// ```rune
/// let value = 42;
///
/// assert_ne!(value, 10, "Value was 10");
/// ```
#[rune::macro_]
pub(crate) fn assert_ne(
	cx: &mut MacroContext<'_, '_, '_>,
	stream: &TokenStream,
) -> compile::Result<TokenStream> {
	use crate as rune;

	let mut it = cx.exprs(stream)?.into_iter().peekable();

	let (Some(left), Some(right)) = (it.next(), it.next()) else {
		return Err(compile::Error::msg(
			cx.input_span(),
			"expected two expressions to compare",
		));
	};

	let message = if it.peek().is_some() {
		Some(FormatArgs::from_exprs(cx, it)?)
	} else {
		None
	};

	let output = if let Some(message) = &message {
		let message = message.expand(cx)?;
		let span = cx.macro_span();
		let left_format = expand_debug_format(cx, "\nleft: {:?}", "left", span)?;
		let right_format = expand_debug_format(cx, "\nright: {:?}", "right", span)?;

		quote! {{
			let left = #left;
			let right = #right;

			if !(left != right) {
				let message = #message;
				message += (#left_format);
				message += (#right_format);
				::std::panic("assertion failed (left != right): " + message);
			}
		}}
	} else {
		let message = cx.lit("assertion failed (left != right):")?;
		let span = cx.macro_span();
		let left_format = expand_debug_format(cx, "\nleft: {:?}", "left", span)?;
		let right_format = expand_debug_format(cx, "\nright: {:?}", "right", span)?;

		quote! {{
			let left = #left;
			let right = #right;

			if !(left != right) {
				let message = ::std::string::String::from(#message);
				message += (#left_format);
				message += (#right_format);
				::std::panic(message);
			}
		}}
	};

	Ok(output.into_token_stream(cx)?)
}
