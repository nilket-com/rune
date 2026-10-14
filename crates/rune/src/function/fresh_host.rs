//! Closed, synchronous fresh-host signatures. Enrollment is a trusted host contract.
#![allow(missing_docs)]
use crate::alloc::clone::TryClone;
use crate::function::{Function, Plain, Ref};
use crate::runtime::FunctionHandler;
use crate::Hash;

mod sealed {
	pub trait Argument {}
	pub trait Scalar {}
	pub trait Output {}
	pub trait Arguments {}
	pub trait Callable<A> {}
}

/// Scalar representation used by the unstable fresh host bridge.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostType {
	Unit,
	Bool,
	I64,
	U64,
	F64,
	Char,
	String,
}
/// Exactly one optional/result layer; nesting is not certified.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOutput {
	Scalar(HostType),
	Option(HostType),
	Result(HostType, HostType),
}
/// Immutable signature created only by the sealed typed API.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostSignature {
	pub(crate) arity: u8,
	pub(crate) arguments: [HostType; 4],
	pub(crate) output: HostOutput,
}
impl TryClone for HostSignature {
	fn try_clone(&self) -> crate::alloc::Result<Self> {
		Ok(*self)
	}
}
impl HostSignature {
	pub(crate) fn valid(self) -> bool {
		self.arity <= 4
			&& self.arguments[self.arity as usize..]
				.iter()
				.all(|kind| *kind == HostType::Unit)
	}
}

/// Sealed argument marker. Owned strings and mutable/reference aggregate inputs are excluded.
#[doc(hidden)]
pub trait FreshHostArgument: sealed::Argument {
	type Rust<'a>;
	const KIND: HostType;
}
/// Sealed scalar return type.
#[doc(hidden)]
pub trait FreshHostScalar: sealed::Scalar {
	const KIND: HostType;
}
/// Sealed, non-recursive return shape.
#[doc(hidden)]
pub trait FreshHostOutput: sealed::Output {
	const SHAPE: HostOutput;
}
/// Sealed tuple of zero through four argument markers.
#[doc(hidden)]
pub trait FreshHostArguments: sealed::Arguments {
	const ARITY: u8;
	const TYPES: [HostType; 4];
}
/// A Rust Fn with the closed synchronous signature; doc metadata cannot certify one.
#[doc(hidden)]
pub trait FreshHostFunction<A>: Function<A, Plain> + sealed::Callable<A> {
	fn signature() -> HostSignature;
}

macro_rules! scalar {
	($rust:ty, $kind:ident) => {
		impl sealed::Scalar for $rust {}
		impl FreshHostScalar for $rust {
			const KIND: HostType = HostType::$kind;
		}
		impl sealed::Output for $rust {}
		impl FreshHostOutput for $rust {
			const SHAPE: HostOutput = HostOutput::Scalar(HostType::$kind);
		}
	};
}
macro_rules! argument {
	($rust:ty, $kind:ident) => {
		impl sealed::Argument for $rust {}
		impl FreshHostArgument for $rust {
			type Rust<'a> = $rust;
			const KIND: HostType = HostType::$kind;
		}
	};
}
scalar!((), Unit);
scalar!(bool, Bool);
scalar!(i64, I64);
scalar!(u64, U64);
scalar!(f64, F64);
scalar!(char, Char);
scalar!(std::string::String, String);
argument!((), Unit);
argument!(bool, Bool);
argument!(i64, I64);
argument!(u64, U64);
argument!(f64, F64);
argument!(char, Char);
impl sealed::Argument for Ref<str> {}
impl FreshHostArgument for Ref<str> {
	type Rust<'a> = &'a str;
	const KIND: HostType = HostType::String;
}
impl<T: FreshHostScalar> sealed::Output for Option<T> {}
impl<T: FreshHostScalar> FreshHostOutput for Option<T> {
	const SHAPE: HostOutput = HostOutput::Option(T::KIND);
}
impl<T: FreshHostScalar, E: FreshHostScalar> sealed::Output for Result<T, E> {}
impl<T: FreshHostScalar, E: FreshHostScalar> FreshHostOutput for Result<T, E> {
	const SHAPE: HostOutput = HostOutput::Result(T::KIND, E::KIND);
}

macro_rules! arguments {
	($n:expr; $($a:ident : $index:expr),*) => {
		impl<$($a: FreshHostArgument),*> sealed::Arguments for ($($a,)*) {}
		impl<$($a: FreshHostArgument),*> FreshHostArguments for ($($a,)*) {
			const ARITY: u8 = $n;
			const TYPES: [HostType; 4] = {
				let mut types = [HostType::Unit; 4];
				$(types[$index] = $a::KIND;)*
				types
			};
		}
		impl<F, R, $($a: FreshHostArgument),*> sealed::Callable<($($a,)*)> for F
		where F: for<'a> Fn($($a::Rust<'a>),*) -> R + Function<($($a,)*), Plain, Return = R>, R: FreshHostOutput {}
	};
}
impl sealed::Arguments for () {}
impl FreshHostArguments for () {
	const ARITY: u8 = 0;
	const TYPES: [HostType; 4] = [HostType::Unit; 4];
}
impl<F, R> sealed::Callable<()> for F
where
	F: Fn() -> R + Function<(), Plain, Return = R>,
	R: FreshHostOutput,
{
}
arguments!(1; A:0);
arguments!(2; A:0,B:1);
arguments!(3; A:0,B:1,C:2);
arguments!(4; A:0,B:1,C:2,D:3);
impl<F, A> FreshHostFunction<A> for F
where
	F: Function<A, Plain> + sealed::Callable<A>,
	A: FreshHostArguments,
	F::Return: FreshHostOutput,
{
	fn signature() -> HostSignature {
		HostSignature {
			arity: A::ARITY,
			arguments: A::TYPES,
			output: F::Return::SHAPE,
		}
	}
}

/// Certificate retains the allocation of the installed handler, never a code-address token.
#[derive(Clone)]
pub(crate) struct HostBinding {
	pub(crate) hash: Hash,
	pub(crate) handler: FunctionHandler,
	pub(crate) signature: HostSignature,
}
impl TryClone for HostBinding {
	fn try_clone(&self) -> crate::alloc::Result<Self> {
		Ok(self.clone())
	}
}
impl HostBinding {
	pub(crate) fn same(&self, other: &Self) -> bool {
		self.hash == other.hash
			&& self.signature == other.signature
			&& self.handler.same_allocation(&other.handler)
	}
}
