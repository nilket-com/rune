//! Owned experimental instructions. Validated code has no old Unit/Value/VM dependency.

use super::arithmetic::{Binary, Unary};
use super::error::{Error, ErrorKind, Location, Result};
use super::nominal;
use super::value::{Type, Value};
use crate::alloc::{String, Vec};
use core::sync::atomic::{AtomicU32, Ordering};

mod missing_methods;

const MAX_CODE_BYTES: usize = 16 << 20;
pub const MAX_GAP: usize = 256;

/// Selects builtin clone_with semantics or associated-method dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CloneMode {
	Associated = 0,
	Global = 1,
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
	Template {
		out: u32,
		args_start: u32,
		args_len: u32,
	},
	Print {
		value: u32,
	},
	TupleFieldGetMut {
		out: u32,
		value: u32,
		index: u32,
	},
	TupleFieldSetMut {
		value: u32,
		index: u32,
		new: u32,
	},
	TupleField {
		out: u32,
		value: u32,
		index: u32,
	},
	SetTupleField {
		value: u32,
		index: u32,
		new: u32,
	},
	Record {
		out: u32,
		tag: Type,
		args_start: u32,
		args_len: u32,
	},
	FieldGetMut {
		out: u32,
		value: u32,
		key: u32,
	},
	FieldSetMut {
		value: u32,
		key: u32,
		new: u32,
	},
	Field {
		out: u32,
		value: u32,
		key: u32,
	},
	SetField {
		value: u32,
		key: u32,
		new: u32,
	},
	Variant {
		out: u32,
		tag: Type,
		success: bool,
		payload: Option<u32>,
	},
	Try {
		out: u32,
		value: u32,
	},
	CallValue {
		out: u32,
		value: u32,
		args_start: u32,
		args_len: u32,
	},
	Range {
		out: u32,
		start: u32,
		end: u32,
	},
	IterStart {
		out: u32,
		value: u32,
	},
	IterNext {
		out: u32,
		iterator: u32,
		none: u32,
	},
	String {
		out: u32,
		text: u32,
	},
	StringFrom {
		out: u32,
		value: u32,
	},
	Panic {
		value: u32,
	},
	Clone {
		out: u32,
		value: u32,
		mode: CloneMode,
	},
	AssignBinary {
		out: u32,
		op: Binary,
		lhs: u32,
		rhs: u32,
	},

	Literal {
		out: u32,
		value: Value,
	},
	Copy {
		out: u32,
		value: u32,
	},
	Release {
		value: u32,
	},
	Unary {
		out: u32,
		op: Unary,
		value: u32,
	},
	Binary {
		out: u32,
		op: Binary,
		lhs: u32,
		rhs: u32,
	},
	Cast {
		out: u32,
		value: u32,
		tag: Type,
	},
	Is {
		out: u32,
		value: u32,
		tag: Type,
		negate: bool,
	},
	Jump {
		target: u32,
	},
	Branch {
		condition: u32,
		yes: u32,
		no: u32,
	},
	Call {
		out: u32,
		function: u32,
		args_start: u32,
		args_len: u32,
	},
	Return {
		value: u32,
	},
	List {
		out: u32,
		tag: Type,
		args_start: u32,
		args_len: u32,
	},
	Get {
		out: u32,
		value: u32,
		index: u32,
	},
	Set {
		value: u32,
		index: u32,
		new: u32,
	},
	Len {
		out: u32,
		value: u32,
	},
	Push {
		value: u32,
		new: u32,
	},
	Drop {
		value: u32,
	},
}

const CHARGED: u32 = 1 << 31;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Instruction {
	opcode: u32,
	a: u32,
	b: u32,
	c: u32,
}

impl Instruction {
	pub fn is_charged(self) -> bool {
		self.opcode & CHARGED != 0
	}

	fn charge(&mut self) {
		self.opcode |= CHARGED;
	}
}

#[derive(Clone, Copy)]
pub(crate) struct CachedInstruction {
	pub(crate) op: Op,
	pub(crate) charged: bool,
}

// Internal debug-metadata tag shared by sort and Any synthetic helpers.
// An inverted range cannot be a source expression. The historical SORT name
// is retained; helper identity is determined separately by lower::Resolve.
// Stored in existing locations; no Function, Code, Op or Frame layout changes.
pub(crate) const TRANSPARENT_SORT_LOCATION: Location = Location {
	source: u32::MAX,
	revision: u64::MAX,
	start: u32::MAX,
	end: 0,
};

pub struct Function {
	cached: Vec<CachedInstruction>,
	pub(crate) locations: Vec<Location>,
	constants: Vec<Value>,
	types: Vec<Type>,
	ranges: Vec<(u32, u32)>,
	pub(crate) strings: Vec<String>,
	pub(crate) code: Vec<Instruction>,
	pub(crate) args: Vec<u32>,
	pub(crate) slots: u32,
	pub(crate) parameters: u32,
}

impl Function {
	#[cold]
	#[inline(never)]
	pub(crate) fn is_transparent_sort(&self) -> bool {
		self.locations.first() == Some(&TRANSPARENT_SORT_LOCATION)
	}
	pub fn new(slots: u32, parameters: u32) -> Result<Self> {
		if slots == 0 || slots > 65_536 || parameters > slots {
			return Err(invalid());
		}

		Ok(Self {
			cached: Vec::new(),
			locations: Vec::new(),
			constants: Vec::new(),
			types: Vec::new(),
			ranges: Vec::new(),
			strings: Vec::new(),
			code: Vec::new(),
			args: Vec::new(),
			slots,
			parameters,
		})
	}

	pub fn string(&mut self, text: &str) -> Result<u32> {
		if text.len() > MAX_CODE_BYTES {
			return Err(invalid());
		}

		let id = u32::try_from(self.strings.len()).map_err(|_| invalid())?;
		self.strings.try_push(String::try_from(text)?)?;
		Ok(id)
	}

	pub fn push(&mut self, op: Op, location: Location) -> Result<u32> {
		if self.code.len() >= MAX_CODE_BYTES / core::mem::size_of::<Instruction>() {
			return Err(invalid());
		}

		let pc = u32::try_from(self.code.len()).map_err(|_| invalid())?;
		self.code.try_reserve(1)?;
		self.locations.try_reserve(1)?;
		let instruction = self.encode(op)?;
		self.code.try_push(instruction)?;
		self.locations.try_push(location)?;
		Ok(pc)
	}

	/// Bind a private variant predicate to this function's type pool.
	pub(super) fn variant_test(&mut self, tag: Type) -> Result<u32> {
		if !matches!(tag, Type::Struct(_)) {
			return Err(invalid());
		}
		let index = Self::pool(&mut self.types, tag)?;
		if index > VARIANT_TEST_INDEX_MASK {
			return Err(invalid());
		}
		Ok(STAGED_VARIANT_TEST_BASE | index)
	}

	pub(super) fn variant_test_tag(&self, index: u32) -> Option<super::value::Nominal> {
		match self.types.get(index as usize)? {
			Type::Struct(nominal) => Some(*nominal),
			_ => None,
		}
	}

	pub fn arguments(&mut self, args: &[u32]) -> Result<(u32, u32)> {
		let start = u32::try_from(self.args.len()).map_err(|_| invalid())?;
		let len = u32::try_from(args.len()).map_err(|_| invalid())?;

		if self
			.args
			.len()
			.checked_add(args.len())
			.filter(|n| *n <= MAX_CODE_BYTES / 4)
			.is_none()
		{
			return Err(invalid());
		}

		self.args.try_extend_from_slice(args)?;
		Ok((start, len))
	}

	pub fn patch(&mut self, pc: u32, op: Op) -> Result<()> {
		let old = *self.code.get(pc as usize).ok_or_else(invalid)?;
		let mut instruction = self.encode(op)?;
		instruction.opcode |= old.opcode & CHARGED;
		self.code[pc as usize] = instruction;
		Ok(())
	}

	pub fn len(&self) -> usize {
		self.code.len()
	}

	fn pool<T>(pool: &mut Vec<T>, value: T) -> Result<u32> {
		if pool.len() >= MAX_CODE_BYTES / core::mem::size_of::<T>().max(1) {
			return Err(invalid());
		}

		let index = u32::try_from(pool.len()).map_err(|_| invalid())?;
		pool.try_push(value)?;
		Ok(index)
	}

	fn encode(&mut self, op: Op) -> Result<Instruction> {
		use Op::*;
		let (opcode, a, b, c) = match op {
			Template {
				out,
				args_start,
				args_len,
			} => (
				34,
				out,
				Self::pool(&mut self.ranges, (args_start, args_len))?,
				0,
			),
			Print { value } => (35, value, 0, 0),
			StringFrom { out, value } => (40, out, value, 0),
			Panic { value } => (41, value, 0, 0),
			TupleFieldGetMut { out, value, index } => (36, out, value, index),
			TupleFieldSetMut { value, index, new } => (37, value, index, new),
			TupleField { out, value, index } => (32, out, value, index),
			SetTupleField { value, index, new } => (33, value, index, new),
			Record {
				out,
				tag,
				args_start,
				args_len,
			} => (
				29,
				out,
				Self::pool(&mut self.types, tag)?,
				Self::pool(&mut self.ranges, (args_start, args_len))?,
			),
			FieldGetMut { out, value, key } => (38, out, value, key),
			FieldSetMut { value, key, new } => (39, value, key, new),
			Field { out, value, key } => (30, out, value, key),
			SetField { value, key, new } => (31, value, key, new),
			Variant {
				out,
				tag,
				success,
				payload,
			} => {
				let opcode = match (tag, success, payload.is_some()) {
					(Type::Option, true, true) => 24,
					(Type::Option, false, false) => 25,
					(Type::Result, true, true) => 26,
					(Type::Result, false, true) => 27,
					_ => return Err(invalid()),
				};

				(opcode, out, payload.unwrap_or(0), 0)
			}

			Try { out, value } => (28, out, value, 0),
			CallValue {
				out,
				value,
				args_start,
				args_len,
			} => (
				23,
				out,
				value,
				Self::pool(&mut self.ranges, (args_start, args_len))?,
			),
			Range { out, start, end } => (20, out, start, end),
			IterStart { out, value } => (21, out, value, 0),
			IterNext {
				out,
				iterator,
				none,
			} => (22, out, iterator, none),
			Literal { out, value } => (0, out, Self::pool(&mut self.constants, value)?, 0),
			String { out, text } => (1, out, text, 0),
			Copy { out, value } => (2, out, value, 0),
			Clone { out, value, mode } => (3, out, value, mode as u32),
			Release { value } => (4, value, 0, 0),
			Unary { out, op, value } => (5 + op as u32, out, value, 0),
			Binary { out, op, lhs, rhs } => (100 + op as u32, out, lhs, rhs),
			AssignBinary { out, op, lhs, rhs } => (200 + op as u32, out, lhs, rhs),
			Cast { out, value, tag } => (7, out, value, Self::pool(&mut self.types, tag)?),
			Is {
				out,
				value,
				tag,
				negate,
			} => (
				8 + u32::from(negate),
				out,
				value,
				Self::pool(&mut self.types, tag)?,
			),
			Jump { target } => (10, target, 0, 0),
			Branch { condition, yes, no } => (11, condition, yes, no),
			Call {
				out,
				function,
				args_start,
				args_len,
			} => (
				12,
				out,
				function,
				Self::pool(&mut self.ranges, (args_start, args_len))?,
			),
			Return { value } => (13, value, 0, 0),
			List {
				out,
				tag,
				args_start,
				args_len,
			} => (
				14,
				out,
				Self::pool(&mut self.types, tag)?,
				Self::pool(&mut self.ranges, (args_start, args_len))?,
			),
			Get { out, value, index } => (15, out, value, index),
			Set { value, index, new } => (16, value, index, new),
			Len { out, value } => (17, out, value, 0),
			Push { value, new } => (18, value, new, 0),
			Drop { value } => (19, value, 0, 0),
		};

		Ok(Instruction { opcode, a, b, c })
	}

	/// Cache length, allocation capacity and requested bytes, for resource inspection.
	#[cfg_attr(not(test), allow(dead_code))]
	pub fn cache_layout(&self) -> Result<(usize, usize, usize)> {
		Ok((
			self.cached.len(),
			self.cached.capacity(),
			self.cached
				.capacity()
				.checked_mul(core::mem::size_of::<CachedInstruction>())
				.ok_or_else(invalid)?,
		))
	}

	#[cfg_attr(not(test), allow(dead_code))]
	fn original_buffer_bytes(&self) -> Result<usize> {
		let buffers = [
			(self.code.capacity(), core::mem::size_of::<Instruction>()),
			(self.locations.capacity(), core::mem::size_of::<Location>()),
			(self.constants.capacity(), core::mem::size_of::<Value>()),
			(self.types.capacity(), core::mem::size_of::<Type>()),
			(self.ranges.capacity(), core::mem::size_of::<(u32, u32)>()),
			(self.args.capacity(), core::mem::size_of::<u32>()),
			(self.strings.capacity(), core::mem::size_of::<String>()),
		];
		let n = buffers
			.into_iter()
			.try_fold(0usize, |n, (count, size)| {
				n.checked_add(count.checked_mul(size)?)
			})
			.ok_or_else(invalid)?;
		self.strings
			.iter()
			.try_fold(n, |n, s| n.checked_add(s.capacity()))
			.ok_or_else(invalid)
	}

	#[cfg(test)]
	pub(crate) fn cached(&self, pc: usize) -> Option<CachedInstruction> {
		self.cached.get(pc).copied()
	}

	/// D19: the executor's fetch borrows the cached instruction, so no by-value
	/// Option is constructed at the source level; the checked `get` decides
	/// presence. Whether the machine code drops the niche re-test is observed
	/// at the build, not guaranteed here.
	pub(crate) fn cached_ref(&self, pc: usize) -> Option<&CachedInstruction> {
		self.cached.get(pc)
	}

	pub(crate) fn audit_cached(&self, pc: usize, cached: CachedInstruction) -> Result<()> {
		let encoded = *self.code.get(pc).ok_or_else(invalid)?;
		if encoded.is_charged() != cached.charged || !same_op(self.decode(encoded)?, cached.op) {
			return Err(invalid());
		}

		Ok(())
	}

	fn prepare_cache(&mut self) -> Result<()> {
		let mut cached = Vec::try_with_capacity(self.code.len())?;
		for instruction in &self.code {
			cached.try_push(CachedInstruction {
				op: self.decode(*instruction)?,
				charged: instruction.is_charged(),
			})?;
		}

		debug_assert_eq!(cached.len(), cached.capacity());
		self.cached = cached;
		Ok(())
	}

	pub fn operation(&self, pc: usize) -> Result<Op> {
		self.decode(*self.code.get(pc).ok_or_else(invalid)?)
	}

	pub(crate) fn decode(&self, i: Instruction) -> Result<Op> {
		use Op::*;
		let tag = |n: u32| self.types.get(n as usize).copied().ok_or_else(invalid);
		let range = |n: u32| self.ranges.get(n as usize).copied().ok_or_else(invalid);
		let (a, b, c) = (i.a, i.b, i.c);
		Ok(match i.opcode & !CHARGED {
			34 => {
				let (args_start, args_len) = range(b)?;
				Template {
					out: a,
					args_start,
					args_len,
				}
			}

			35 => Print { value: a },
			36 => TupleFieldGetMut {
				out: a,
				value: b,
				index: c,
			},
			37 => TupleFieldSetMut {
				value: a,
				index: b,
				new: c,
			},
			32 => TupleField {
				out: a,
				value: b,
				index: c,
			},

			33 => SetTupleField {
				value: a,
				index: b,
				new: c,
			},

			29 => {
				let (args_start, args_len) = range(c)?;
				Record {
					out: a,
					tag: tag(b)?,
					args_start,
					args_len,
				}
			}

			38 => FieldGetMut {
				out: a,
				value: b,
				key: c,
			},
			40 => StringFrom { out: a, value: b },
			41 => Panic { value: a },
			39 => FieldSetMut {
				value: a,
				key: b,
				new: c,
			},
			30 => Field {
				out: a,
				value: b,
				key: c,
			},

			31 => SetField {
				value: a,
				key: b,
				new: c,
			},

			0 => Literal {
				out: a,
				value: *self.constants.get(b as usize).ok_or_else(invalid)?,
			},
			1 => String { out: a, text: b },
			2 => Copy { out: a, value: b },
			3 => Clone {
				out: a,
				value: b,
				mode: match c {
					0 => CloneMode::Associated,
					1 => CloneMode::Global,
					_ => return Err(invalid()),
				},
			},
			4 => Release { value: a },
			5 => Unary {
				out: a,
				op: super::arithmetic::Unary::Neg,
				value: b,
			},

			6 => Unary {
				out: a,
				op: super::arithmetic::Unary::Not,
				value: b,
			},

			7 => Cast {
				out: a,
				value: b,
				tag: tag(c)?,
			},

			code @ (8 | 9) => Is {
				out: a,
				value: b,
				tag: tag(c)?,
				negate: code == 9,
			},

			10 => Jump { target: a },
			11 => Branch {
				condition: a,
				yes: b,
				no: c,
			},

			12 => {
				let (args_start, args_len) = range(c)?;
				Call {
					out: a,
					function: b,
					args_start,
					args_len,
				}
			}

			13 => Return { value: a },
			14 => {
				let (args_start, args_len) = range(c)?;
				List {
					out: a,
					tag: tag(b)?,
					args_start,
					args_len,
				}
			}

			15 => Get {
				out: a,
				value: b,
				index: c,
			},

			16 => Set {
				value: a,
				index: b,
				new: c,
			},
			17 => Len { out: a, value: b },
			18 => Push { value: a, new: b },
			19 => Drop { value: a },
			20 => Range {
				out: a,
				start: b,
				end: c,
			},
			21 => IterStart { out: a, value: b },
			22 => IterNext {
				out: a,
				iterator: b,
				none: c,
			},

			23 => {
				let (args_start, args_len) = range(c)?;
				CallValue {
					out: a,
					value: b,
					args_start,
					args_len,
				}
			}

			code @ 24..=27 => Variant {
				out: a,
				tag: if code < 26 {
					Type::Option
				} else {
					Type::Result
				},
				success: code == 24 || code == 26,
				payload: if code == 25 { None } else { Some(b) },
			},

			28 => Try { out: a, value: b },
			code @ 100..=115 => Binary {
				out: a,
				op: binary_code(code - 100)?,
				lhs: b,
				rhs: c,
			},

			code @ 200..=215 => AssignBinary {
				out: a,
				op: binary_code(code - 200)?,
				lhs: b,
				rhs: c,
			},

			_ => return Err(invalid()),
		})
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub fn instructions(&self) -> &[Instruction] {
		&self.code
	}
}

// D1c: ids in the upper half are never ordinary function indices.
pub(crate) const FIRST_INTRINSIC_ID: u32 = 0x8000_0000;
pub(crate) fn reserved_intrinsic_id(id: u32) -> bool {
	id >= FIRST_INTRINSIC_ID
}
fn ordinary_function_id(index: usize) -> Result<u32> {
	if index >= FIRST_INTRINSIC_ID as usize {
		return Err(invalid());
	}
	u32::try_from(index).map_err(|_| invalid())
}
#[derive(Clone, Copy)]
pub(crate) struct CertifiedIntrinsic {
	id: u32,
	pub(super) arity: u32,
}
// Immutable policy, consulted by seal and execution. Numeric
// membership in the partition does not certify an id. The parameterized
// variant family also requires a bound Struct descriptor at seal time.
// No installer API exists.
pub(crate) const PRINT_RAW: u32 = FIRST_INTRINSIC_ID + 1;
pub(crate) const VARIANT_SUCCESS: u32 = FIRST_INTRINSIC_ID + 2;
pub(crate) const SORT_BEGIN: u32 = FIRST_INTRINSIC_ID + 0x40;
pub(crate) const SORT_LEN: u32 = FIRST_INTRINSIC_ID + 0x41;
pub(crate) const SORT_LOAD: u32 = FIRST_INTRINSIC_ID + 0x42;
pub(crate) const SORT_VALUE: u32 = FIRST_INTRINSIC_ID + 0x43;
pub(crate) const SORT_STORE: u32 = FIRST_INTRINSIC_ID + 0x44;
pub(crate) const SORT_COMMIT: u32 = FIRST_INTRINSIC_ID + 0x45;
pub(crate) const SORT_ORDERING: u32 = FIRST_INTRINSIC_ID + 0x46;
pub(crate) const METHOD_CMP: u32 = crate::function::fresh_numeric::Call::MethodCmp.id();
pub(crate) const RANGE_FROM: u32 = FIRST_INTRINSIC_ID + 0x50;
pub(crate) const RANGE_FULL: u32 = FIRST_INTRINSIC_ID + 0x51;
pub(crate) const RANGE_INCLUSIVE: u32 = FIRST_INTRINSIC_ID + 0x52;
pub(crate) const RANGE_TO_INCLUSIVE: u32 = FIRST_INTRINSIC_ID + 0x53;
pub(crate) const RANGE_TO: u32 = FIRST_INTRINSIC_ID + 0x54;
pub(crate) const OBJECT_HAS_FIELD: u32 = FIRST_INTRINSIC_ID + 0x55;
pub(crate) const ITER_METHOD: u32 = FIRST_INTRINSIC_ID + 0x56;
pub(crate) const NEXT_METHOD: u32 = FIRST_INTRINSIC_ID + 0x57;
pub(crate) const ANY_READY: u32 = FIRST_INTRINSIC_ID + 0x58;
pub(crate) const CLOSURE_CREATE: u32 = FIRST_INTRINSIC_ID + 0x59;
pub(crate) const CAPTURE_TAKE: u32 = FIRST_INTRINSIC_ID + 0x5c;
pub(crate) const MISSING_METHOD: u32 = FIRST_INTRINSIC_ID + 0x5a;
pub(crate) const MATCH_VARIANT: u32 = FIRST_INTRINSIC_ID + 0x5b;
// Staged predicates bind a Struct descriptor in the caller's checked type pool.
// Sealing replaces that index with the immutable canonical variant identity;
// the final identity remains bound to a descriptor in the caller's type pool.
const VARIANT_TEST_BASE: u32 = 0x9000_0000;
const STAGED_VARIANT_TEST_BASE: u32 = 0xa000_0000;
const VARIANT_TEST_INDEX_MASK: u32 = 0x0fff_ffff;
pub(super) fn variant_test_type(id: u32) -> Option<u32> {
	(id & !VARIANT_TEST_INDEX_MASK == STAGED_VARIANT_TEST_BASE)
		.then_some(id & VARIANT_TEST_INDEX_MASK)
}

pub(super) fn variant_test_id(id: u32) -> Option<u32> {
	(id & !VARIANT_TEST_INDEX_MASK == VARIANT_TEST_BASE).then_some(id & VARIANT_TEST_INDEX_MASK)
}

pub(crate) const FIRST_HOST: u32 = FIRST_INTRINSIC_ID + 0x100;
pub(crate) const MAX_HOST_DESCRIPTORS: usize = 4096;
fn host_id(index: usize) -> Result<u32> {
	if index >= MAX_HOST_DESCRIPTORS {
		return Err(invalid());
	}
	FIRST_HOST
		.checked_add(u32::try_from(index).map_err(|_| invalid())?)
		.ok_or_else(invalid)
}
fn host_binding(
	hosts: &[crate::function::fresh_host::HostBinding],
	id: u32,
) -> Option<&crate::function::fresh_host::HostBinding> {
	hosts.get(id.checked_sub(FIRST_HOST)? as usize)
}
const CERTIFIED_INTRINSICS: &[CertifiedIntrinsic] = &[
	CertifiedIntrinsic {
		id: PRINT_RAW,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: VARIANT_SUCCESS,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: MATCH_VARIANT,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::MethodAbs.id(),
		arity: 1,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::MethodRound.id(),
		arity: 1,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::MethodPowf.id(),
		arity: 2,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::F64Abs.id(),
		arity: 1,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::F64Round.id(),
		arity: 1,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::F64Powf.id(),
		arity: 2,
	},
	CertifiedIntrinsic {
		id: crate::function::fresh_numeric::Call::I64Abs.id(),
		arity: 1,
	},
	CertifiedIntrinsic {
		id: METHOD_CMP,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: SORT_BEGIN,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: SORT_LEN,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: SORT_LOAD,
		arity: 3,
	},
	CertifiedIntrinsic {
		id: SORT_VALUE,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: SORT_STORE,
		arity: 4,
	},
	CertifiedIntrinsic {
		id: SORT_COMMIT,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: SORT_ORDERING,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: ANY_READY,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: CLOSURE_CREATE,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: CAPTURE_TAKE,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: RANGE_FROM,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: RANGE_FULL,
		arity: 0,
	},
	CertifiedIntrinsic {
		id: RANGE_INCLUSIVE,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: RANGE_TO_INCLUSIVE,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: RANGE_TO,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: OBJECT_HAS_FIELD,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: ITER_METHOD,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: NEXT_METHOD,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: MISSING_METHOD,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0060,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0061,
		arity: 3,
	},
	CertifiedIntrinsic {
		id: 0x8000_0062,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0063,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0064,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0065,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0070,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0071,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0072,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0073,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: 0x8000_0074,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0075,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0076,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0077,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0078,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_0079,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_007a,
		arity: 1,
	},
	CertifiedIntrinsic {
		id: 0x8000_007b,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: ANY_TYPE_OF,
		arity: 2,
	},
	CertifiedIntrinsic {
		id: ANY_TYPE_NAME,
		arity: 1,
	},
];
pub(crate) const ANY_TYPE_OF: u32 = 0x8000_007c;
pub(crate) const ANY_TYPE_NAME: u32 = 0x8000_007d;

fn certified_intrinsic(table: &[CertifiedIntrinsic], id: u32) -> Option<&CertifiedIntrinsic> {
	if !reserved_intrinsic_id(id) {
		return None;
	}
	table.iter().find(|entry| entry.id == id)
}

pub struct Code {
	/// Process-unique, never 0 (the lowerer's "unstamped" marker), never
	/// reused: a function token names the program it indexes by this.
	id: u32,
	functions: Vec<Function>,
	hosts: Vec<crate::function::fresh_host::HostBinding>,
	absent: Vec<crate::Hash>,
}

/// Program identities. Exhaustion refuses `finish`; the counter never wraps.
static NEXT_CODE: AtomicU32 = AtomicU32::new(1);

pub(crate) fn allocate_code_id(counter: &AtomicU32) -> Result<u32> {
	counter
		.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
		.map_err(|_| invalid())
}

pub struct Builder {
	functions: Vec<Function>,
	hosts: Vec<crate::function::fresh_host::HostBinding>,
	absent: Vec<crate::Hash>,
}

fn invalid() -> Error {
	Error::new(ErrorKind::InvalidCode, "validate code")
}

impl Default for Builder {
	fn default() -> Self {
		Self {
			functions: Vec::new(),
			hosts: Vec::new(),
			absent: Vec::new(),
		}
	}
}

impl Builder {
	pub(crate) fn host(
		&mut self,
		binding: &crate::function::fresh_host::HostBinding,
	) -> Result<u32> {
		if !binding.signature.valid() {
			return Err(invalid());
		}
		if let Some(index) = self.hosts.iter().position(|old| old.same(binding)) {
			return host_id(index);
		}
		let id = host_id(self.hosts.len())?;
		self.hosts.try_push(binding.clone())?;
		Ok(id)
	}
	pub fn push(&mut self, function: Function) -> Result<u32> {
		let id = ordinary_function_id(self.functions.len())?;
		self.functions.try_push(function)?;
		Ok(id)
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub fn finish(self) -> Result<Code> {
		let mut code = self.finish_validated(&nominal_valid)?;
		code.for_each_type_mut(&mut |_| {});
		Ok(code)
	}

	fn finish_validated(mut self, valid: &dyn Fn(Type) -> bool) -> Result<Code> {
		if self.functions.len() > FIRST_INTRINSIC_ID as usize {
			return Err(invalid());
		}
		let total = self
			.functions
			.iter()
			.try_fold(0usize, |n, f| {
				n.checked_add(
					f.code
						.len()
						.checked_mul(core::mem::size_of::<Instruction>())?,
				)?
				.checked_add(f.args.len().checked_mul(4)?)
			})
			.ok_or_else(invalid)?;
		let total = self
			.functions
			.iter()
			.try_fold(total, |n, f| {
				f.strings
					.iter()
					.try_fold(n, |n, s| n.checked_add(s.capacity()))
			})
			.ok_or_else(invalid)?;
		let total = self
			.functions
			.iter()
			.try_fold(total, |n, f| {
				n.checked_add(
					f.locations
						.len()
						.checked_mul(core::mem::size_of::<Location>())?,
				)?
				.checked_add(
					f.constants
						.len()
						.checked_mul(core::mem::size_of::<Value>())?,
				)?
				.checked_add(f.types.len().checked_mul(core::mem::size_of::<Type>())?)?
				.checked_add(f.ranges.len().checked_mul(8)?)
			})
			.ok_or_else(invalid)?;
		let host_bytes = self
			.hosts
			.capacity()
			.checked_mul(core::mem::size_of::<crate::function::fresh_host::HostBinding>())
			.ok_or_else(invalid)?;
		let total = total
			.checked_add(host_bytes)
			.and_then(|n| {
				n.checked_add(
					self.absent
						.capacity()
						.checked_mul(core::mem::size_of::<crate::Hash>())?,
				)
			})
			.ok_or_else(invalid)?;
		if self.hosts.len() > MAX_HOST_DESCRIPTORS
			|| !self.hosts.iter().all(|host| host.signature.valid())
		{
			return Err(invalid());
		}
		if total > MAX_CODE_BYTES {
			return Err(invalid());
		}

		// Stamp only unstamped (code 0) function literals; a literal that already
		// names a program is foreign and is rejected, never relabelled as local.
		let id = allocate_code_id(&NEXT_CODE)?;
		for f in &mut self.functions {
			for constant in &mut f.constants {
				if let Value::Function { code, .. } = constant {
					if *code != 0 {
						return Err(invalid());
					}
					*code = id;
				}
			}
		}

		for i in 0..self.functions.len() {
			validate_operands(&self.functions[i], &self.functions, &self.hosts, id, valid)?;
			let f = &mut self.functions[i];
			f.code.first_mut().ok_or_else(invalid)?.charge();

			for pc in 0..f.code.len() {
				let mut targets = [None, None];
				successors(f, pc, &mut targets)?;

				for target in targets.into_iter().flatten() {
					if target <= pc {
						f.code[target].charge();
					}
				}
			}

			// Once every backward target is charged, uncharged paths only move
			// forward. A single DAG pass inserts a charge at a path's 257th op.
			let mut gap = Vec::try_with_capacity(f.code.len())?;
			gap.try_resize(f.code.len(), 0usize)?;

			for pc in 0..f.code.len() {
				let mut next = if f.code[pc].is_charged() {
					0
				} else {
					gap[pc].checked_add(1).ok_or_else(invalid)?
				};

				if next > MAX_GAP {
					f.code[pc].charge();
					next = 0;
				}

				let mut targets = [None, None];
				successors(f, pc, &mut targets)?;

				for target in targets.into_iter().flatten() {
					if target > pc && !f.code[target].is_charged() {
						gap[target] = gap[target].max(next);
					} else if target <= pc && !f.code[target].is_charged() {
						return Err(invalid());
					}
				}
			}
		}

		// Existing charge scratch has dropped before any decoded allocation.
		let entries = self
			.functions
			.iter()
			.try_fold(0usize, |n, f| n.checked_add(f.code.len()))
			.ok_or_else(invalid)?;
		let requested = entries
			.checked_mul(core::mem::size_of::<CachedInstruction>())
			.ok_or_else(invalid)?;
		let bound = (MAX_CODE_BYTES / core::mem::size_of::<Instruction>())
			.checked_mul(core::mem::size_of::<CachedInstruction>())
			.ok_or_else(invalid)?;
		// Unreachable after the unchanged encoded admission; retain a checked bound.
		if requested > bound {
			return Err(invalid());
		}

		for function in &mut self.functions {
			function.prepare_cache()?;
		}

		Ok(Code {
			id,
			functions: self.functions,
			hosts: self.hosts,
			absent: self.absent,
		})
	}
}

/// A compile owns both its builder and provisional identities from first lowering
/// through publication. Neither field escapes, and no API pairs a Builder with a
/// separately supplied Staging. The public Builder seal remains strict.
#[derive(Default)]
pub(crate) struct CompileTransaction {
	builder: Builder,
	staging: nominal::Staging,
	checked_absent: Vec<crate::Hash>,
	checked_text: u8,
	checked_pipeline: u8,
}

impl CompileTransaction {
	pub(crate) fn host(
		&mut self,
		binding: &crate::function::fresh_host::HostBinding,
	) -> Result<u32> {
		self.builder.host(binding)
	}
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn stage(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
	) -> Result<super::value::Nominal> {
		self.staging.stage(hash, name)
	}
	pub(super) fn stage_shape(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
		shape: nominal::Shape,
	) -> Result<super::value::Nominal> {
		self.staging.stage_shape(hash, name, shape)
	}
	pub(super) fn stage_variant(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
		shape: nominal::Shape,
		enum_hash: u64,
		ordinal: u32,
		fields: &[&str],
	) -> Result<super::value::Nominal> {
		self.staging
			.stage_variant_order(hash, name, shape, enum_hash, ordinal, fields)
	}

	pub(crate) fn push(&mut self, function: Function) -> Result<u32> {
		self.builder.push(function)
	}

	pub(crate) fn finish(self) -> Result<Code> {
		let mut code = self
			.builder
			.finish_validated(&|tag| tag.nominal().is_none_or(|n| self.staging.valid(n)))?;
		// All validation, decoded allocation, audit and remap allocation precede
		// global publication. No Result-returning operation follows publication.
		for f in &code.functions {
			for (pc, &cached) in f.cached.iter().enumerate() {
				f.audit_cached(pc, cached)?;
			}
		}
		let mut remap = self.staging.prepare_remap()?;
		self.staging.commit(&mut remap)?;
		code.for_each_type_mut(&mut |tag| {
			if let Type::Struct(n) = tag {
				if n.0 & nominal::PROVISIONAL != 0 {
					*n = remap[(n.0 & !nominal::PROVISIONAL) as usize];
				}
			}
		});
		#[cfg(debug_assertions)]
		{
			code.for_each_type_mut(&mut |tag| {
				debug_assert!(tag
					.nominal()
					.is_none_or(|n| n.0 & nominal::PROVISIONAL == 0));
			});
			for f in &code.functions {
				for (pc, &cached) in f.cached.iter().enumerate() {
					debug_assert!(f.audit_cached(pc, cached).is_ok());
				}
			}
		}
		Ok(code)
	}
}

impl Code {
	/// Encoded instructions reference these pools. Only the eager decoded cache
	/// duplicates tags; native region caches are built later, during invocation.
	/// After remapping type pools, finish descriptor-bound variant call selectors.
	fn for_each_type_mut(&mut self, f: &mut dyn FnMut(&mut Type)) {
		for function in &mut self.functions {
			for tag in &mut function.types {
				f(tag);
			}
			for value in &mut function.constants {
				if let Value::Type(tag) = value {
					f(tag);
				}
			}
			for (pc, cached) in function.cached.iter_mut().enumerate() {
				match &mut cached.op {
					Op::Record { tag, .. }
					| Op::List { tag, .. }
					| Op::Variant { tag, .. }
					| Op::Cast { tag, .. }
					| Op::Is { tag, .. } => f(tag),

					Op::Call {
						function: callee, ..
					} => {
						if let Some(index) = variant_test_type(*callee) {
							let Type::Struct(n) = function.types[index as usize] else {
								unreachable!("validated variant descriptor")
							};
							*callee = VARIANT_TEST_BASE | nominal::variant_id(n);
							function.code[pc].b = *callee;
						}
					}
					Op::Literal {
						value: Value::Type(tag),
						..
					} => f(tag),
					_ => (),
				}
			}
		}
	}
}

pub(crate) fn successors(f: &Function, pc: usize, out: &mut [Option<usize>; 2]) -> Result<()> {
	let instruction = f.operation(pc)?;
	*out = match instruction {
		Op::Return { .. } | Op::Panic { .. } => [None, None],
		Op::IterNext { none, .. } => [
			Some(pc.checked_add(1).ok_or_else(invalid)?),
			Some(none as usize),
		],
		Op::Jump { target } => [Some(target as usize), None],
		Op::Branch { yes, no, .. } => [Some(yes as usize), Some(no as usize)],
		_ => [Some(pc.checked_add(1).ok_or_else(invalid)?), None],
	};

	if out.iter().flatten().any(|n| *n >= f.code.len()) {
		return Err(invalid());
	}

	Ok(())
}

// A compound tuple update is one uninterrupted lowering group. The literal
// between arithmetic and writeback preserves the pre-existing sparse fuel layout.
fn validate_tuple_updates(f: &Function) -> Result<()> {
	use Op::*;
	let op = |pc| f.operation(pc);
	let interior = |target: usize| -> Result<bool> {
		Ok(match op(target)? {
			TupleFieldGetMut { .. } | TupleFieldSetMut { .. } | FieldSetMut { .. } => true,
			AssignBinary { .. } if target >= 1 => {
				matches!(
					op(target - 1)?,
					TupleFieldGetMut { .. } | FieldGetMut { .. }
				)
			}
			Literal { .. } if target >= 2 => matches!(op(target - 2)?, TupleFieldGetMut { .. }),
			_ => false,
		})
	};
	for pc in 0..f.code.len() {
		match op(pc)? {
			FieldGetMut { out, value, key } => {
				let AssignBinary {
					out: result,
					lhs,
					rhs,
					..
				} = op(pc + 1)?
				else {
					return Err(invalid());
				};
				let FieldSetMut {
					value: target,
					key: write_key,
					new,
				} = op(pc + 2)?
				else {
					return Err(invalid());
				};
				if out == value
					|| out == rhs || value == rhs
					|| result != out
					|| lhs != out || target != value
					|| f.strings.get(write_key as usize) != f.strings.get(key as usize)
					|| new != out
				{
					return Err(invalid());
				}
			}
			FieldSetMut { .. } => {
				if pc < 2 || !matches!(op(pc - 2)?, FieldGetMut { .. }) {
					return Err(invalid());
				}
			}
			TupleFieldGetMut { out, value, index } => {
				if pc == 0 || out == value || out == index {
					return Err(invalid());
				}
				let Literal {
					out: read_index,
					value: read_literal,
				} = op(pc - 1)?
				else {
					return Err(invalid());
				};
				let AssignBinary {
					out: result,
					lhs,
					rhs,
					..
				} = op(pc + 1)?
				else {
					return Err(invalid());
				};
				let Literal {
					out: write_index,
					value: write_literal,
				} = op(pc + 2)?
				else {
					return Err(invalid());
				};
				let TupleFieldSetMut {
					value: target,
					index: write_slot,
					new,
				} = op(pc + 3)?
				else {
					return Err(invalid());
				};
				if index != read_index
					|| read_literal != write_literal
					|| !matches!(read_literal, Value::I64(n) if n>=0)
					|| result != out
					|| lhs != out || rhs == out
					|| target != value
					|| new != out || write_slot != write_index
					|| write_index == value
					|| write_index == out
					|| write_index == rhs
				{
					return Err(invalid());
				}
			}
			TupleFieldSetMut { .. } => {
				if pc < 3 || !matches!(op(pc - 3)?, TupleFieldGetMut { .. }) {
					return Err(invalid());
				}
			}
			Jump { target } => {
				if interior(target as usize)? {
					return Err(invalid());
				}
			}
			Branch { yes, no, .. } => {
				if interior(yes as usize)? || interior(no as usize)? {
					return Err(invalid());
				}
			}
			IterNext { none, .. } if interior(none as usize)? => {
				return Err(invalid());
			}
			_ => (),
		}
	}
	Ok(())
}

// Locate and validate a suffix, then exclude it from every executable argument range.
fn template_args_prefix(f: &Function) -> Result<usize> {
	use super::format_spec::{Spec, WORDS};
	let mut prefix = f.args.len();
	for instruction in &f.code {
		if let Op::Template {
			args_start,
			args_len,
			..
		} = f.decode(*instruction)?
		{
			if args_len % 2 != 0 || args_len > 8192 {
				return Err(invalid());
			}
			let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
			let parts = f
				.args
				.get(args_start as usize..end as usize)
				.ok_or_else(invalid)?;
			for pair in parts.as_chunks::<2>().0 {
				if pair[1] >= 2 {
					prefix = prefix.min(pair[1].checked_sub(2).ok_or_else(invalid)? as usize);
				}
			}
		}
	}
	if prefix > f.args.len() || !(f.args.len() - prefix).is_multiple_of(WORDS) {
		return Err(invalid());
	}
	for record in f.args[prefix..].as_chunks::<WORDS>().0 {
		Spec::from_words(record)?;
	}
	for instruction in &f.code {
		if let Op::Template {
			args_start,
			args_len,
			..
		} = f.decode(*instruction)?
		{
			let end = args_start.checked_add(args_len).ok_or_else(invalid)? as usize;
			if end > prefix {
				return Err(invalid());
			}
			for pair in f.args[args_start as usize..end].as_chunks::<2>().0 {
				if pair[1] >= 2 {
					let offset = (pair[1] - 2) as usize;
					if offset < prefix
						|| !(offset - prefix).is_multiple_of(WORDS)
						|| offset
							.checked_add(WORDS)
							.is_none_or(|end| end > f.args.len())
					{
						return Err(invalid());
					}
				}
			}
		}
	}
	Ok(prefix)
}

// D21-derived validation logic preserved during extraction: argument tables hold fixed pairs.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn validate_operands(
	f: &Function,
	functions: &[Function],
	hosts: &[crate::function::fresh_host::HostBinding],
	code_id: u32,
	valid: &dyn Fn(Type) -> bool,
) -> Result<()> {
	validate_tuple_updates(f)?;
	let args_prefix = template_args_prefix(f)?;
	// Every tag operand (Is, Cast, Record, List, Variant) is drawn from this
	// pool, so one pass checks every Struct nominal the function names.
	if !f.types.iter().all(|&tag| valid(tag)) {
		return Err(invalid());
	}

	for instruction in &f.code {
		let slot = |n: u32| if n < f.slots { Ok(()) } else { Err(invalid()) };
		let args = |start: u32, len: u32| -> Result<&[u32]> {
			let end = (start as usize)
				.checked_add(len as usize)
				.ok_or_else(invalid)?;
			if end > args_prefix {
				return Err(invalid());
			}
			let values = f.args.get(start as usize..end).ok_or_else(invalid)?;

			for &v in values {
				slot(v)?;
			}

			Ok(values)
		};

		match f.decode(*instruction)? {
			Op::Template {
				out,
				args_start,
				args_len,
			} => {
				slot(out)?;
				if args_len % 2 != 0 || args_len > 8192 {
					return Err(invalid());
				}

				let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
				if end as usize > args_prefix {
					return Err(invalid());
				}
				let parts = f
					.args
					.get(args_start as usize..end as usize)
					.ok_or_else(invalid)?;
				for pair in parts.chunks_exact(2) {
					slot(pair[0])?;
					super::format_spec::load(&f.args, pair[1])?;
				}
			}

			Op::Print { value } => slot(value)?,
			Op::String { out, text } => {
				slot(out)?;
				if text as usize >= f.strings.len() {
					return Err(invalid());
				}
			}

			Op::Literal { out, value } => {
				slot(out)?;
				match value {
					Value::Heap(_) => return Err(invalid()),
					Value::Function { code, id }
						if code != code_id || id as usize >= functions.len() =>
					{
						return Err(invalid());
					}
					Value::Type(tag) if !valid(tag) => return Err(invalid()),
					_ => (),
				}
			}

			Op::Variant { out, payload, .. } => {
				slot(out)?;
				if let Some(value) = payload {
					slot(value)?;
				}
			}

			Op::Try { out, value } => {
				slot(out)?;
				slot(value)?;
			}

			Op::IterStart { out, value }
			| Op::Clone { out, value, .. }
			| Op::StringFrom { out, value }
			| Op::Copy { out, value }
			| Op::Unary { out, value, .. }
			| Op::Cast { out, value, .. }
			| Op::Is { out, value, .. }
			| Op::Len { out, value } => {
				slot(out)?;
				slot(value)?;
			}

			Op::Release { value }
			| Op::Return { value }
			| Op::Drop { value }
			| Op::Panic { value } => {
				slot(value)?;
			}

			Op::Range { out, start, end } => {
				slot(out)?;
				slot(start)?;
				slot(end)?;
			}

			Op::IterNext { out, iterator, .. } => {
				slot(out)?;
				slot(iterator)?;
			}

			Op::AssignBinary { out, lhs, rhs, .. } | Op::Binary { out, lhs, rhs, .. } => {
				slot(out)?;
				slot(lhs)?;
				slot(rhs)?;
			}

			Op::Record {
				out,
				tag,
				args_start,
				args_len,
			} => {
				slot(out)?;
				if !matches!(tag, Type::Object | Type::Struct(_)) || args_len % 2 != 0 {
					return Err(invalid());
				}

				let end = args_start.checked_add(args_len).ok_or_else(invalid)?;
				if end as usize > args_prefix {
					return Err(invalid());
				}
				let entries = f
					.args
					.get(args_start as usize..end as usize)
					.ok_or_else(invalid)?;
				for (i, pair) in entries.chunks_exact(2).enumerate() {
					let key = f.strings.get(pair[0] as usize).ok_or_else(invalid)?;
					slot(pair[1])?;

					if entries[..i * 2]
						.chunks_exact(2)
						.any(|prior| f.strings[prior[0] as usize] == *key)
					{
						return Err(invalid());
					}
				}
			}

			Op::Field { out, value, key } | Op::FieldGetMut { out, value, key } => {
				slot(out)?;
				slot(value)?;
				f.strings.get(key as usize).ok_or_else(invalid)?;
			}

			Op::SetField { value, key, new } | Op::FieldSetMut { value, key, new } => {
				slot(value)?;
				slot(new)?;
				f.strings.get(key as usize).ok_or_else(invalid)?;
			}

			Op::Get { out, value, index }
			| Op::TupleField { out, value, index }
			| Op::TupleFieldGetMut { out, value, index } => {
				slot(out)?;
				slot(value)?;
				slot(index)?;
			}

			Op::Set { value, index, new }
			| Op::SetTupleField { value, index, new }
			| Op::TupleFieldSetMut { value, index, new } => {
				slot(value)?;
				slot(index)?;
				slot(new)?;
			}

			Op::Push { value, new } => {
				slot(value)?;
				slot(new)?;
			}
			Op::Branch { condition, .. } => slot(condition)?,
			Op::Jump { .. } => (),
			Op::CallValue {
				out,
				value,
				args_start,
				args_len,
			} => {
				slot(out)?;
				slot(value)?;
				args(args_start, args_len)?;
			}

			Op::Call {
				out,
				function,
				args_start,
				args_len,
			} => {
				slot(out)?;
				if let Some(index) = variant_test_type(function) {
					if args_len != 1 || f.variant_test_tag(index).is_none() {
						return Err(invalid());
					}
				} else if let Some(identity) = variant_test_id(function) {
					if args_len != 1 || !f.types.iter().any(|tag| matches!(tag, Type::Struct(n) if n.0 & nominal::PROVISIONAL == 0 && nominal::variant_id(*n) == identity)) {
						return Err(invalid());
					}
				} else if let Some(callee) = functions.get(function as usize) {
					if args_len != callee.parameters {
						return Err(invalid());
					}
				} else {
					let arity = if let Some(host) = host_binding(hosts, function) {
						if !host.signature.valid() {
							return Err(invalid());
						}
						u32::from(host.signature.arity)
					} else {
						certified_intrinsic(CERTIFIED_INTRINSICS, function)
							.ok_or_else(invalid)?
							.arity
					};
					if args_len != arity {
						return Err(invalid());
					}
				}

				args(args_start, args_len)?;
			}

			Op::List {
				out,
				tag,
				args_start,
				args_len,
			} => {
				slot(out)?;
				if !matches!(tag, Type::Vec | Type::Tuple | Type::Struct(_)) {
					return Err(invalid());
				}

				args(args_start, args_len)?;
			}
		}
	}

	for pc in 0..f.code.len() {
		successors(f, pc, &mut [None, None])?;
	}

	Ok(())
}

/// A `Struct` tag must name an interned nominal; other tags carry no index.
#[cfg_attr(not(test), allow(dead_code))]
fn nominal_valid(tag: Type) -> bool {
	tag.nominal().is_none_or(nominal::contains)
}

impl Code {
	/// The program identity every function literal of this code carries.
	pub fn id(&self) -> u32 {
		self.id
	}

	/// Number of functions, for construction inspection listings.
	#[cfg_attr(not(test), allow(dead_code))]
	pub fn functions(&self) -> usize {
		self.functions.len()
	}

	pub fn function(&self, id: u32) -> Option<&Function> {
		self.functions.get(id as usize)
	}

	pub(crate) fn intrinsic(&self, id: u32) -> Option<CertifiedIntrinsic> {
		if let Some(host) = self.host(id) {
			return Some(CertifiedIntrinsic {
				id,
				arity: u32::from(host.signature.arity),
			});
		}
		certified_intrinsic(CERTIFIED_INTRINSICS, id).copied()
	}
	pub(crate) fn host(&self, id: u32) -> Option<&crate::function::fresh_host::HostBinding> {
		host_binding(&self.hosts, id)
	}
	pub(crate) fn validate_hosts(
		&self,
		runtime: Option<&crate::runtime::RuntimeContext>,
	) -> Result<()> {
		for binding in &self.hosts {
			let actual = runtime
				.and_then(|runtime| runtime.function(&binding.hash))
				.ok_or_else(invalid)?;
			if !binding.signature.valid() || !binding.handler.same_allocation(actual) {
				return Err(invalid());
			}
		}
		if !self.absent.is_empty() {
			self.validate_absence(runtime)?;
		}
		Ok(())
	}
}

fn binary_code(code: u32) -> Result<Binary> {
	use Binary::*;
	Ok(match code {
		0 => Add,
		1 => Sub,
		2 => Mul,
		3 => Div,
		4 => Rem,
		5 => Eq,
		6 => Ne,
		7 => Lt,
		8 => Le,
		9 => Gt,
		10 => Ge,
		11 => BitAnd,
		12 => BitOr,
		13 => BitXor,
		14 => Shl,
		15 => Shr,
		_ => return Err(invalid()),
	})
}

fn same_tag(a: Type, b: Type) -> bool {
	match (a, b) {
		(Type::Struct(a), Type::Struct(b)) => a.0 == b.0,
		_ => a == b,
	}
}

fn same_op(a: Op, b: Op) -> bool {
	match a {
		Op::Template {
			out: a_out,
			args_start: a_args_start,
			args_len: a_args_len,
		} => match b {
			Op::Template {
				out: b_out,
				args_start: b_args_start,
				args_len: b_args_len,
			} => a_out == b_out && a_args_start == b_args_start && a_args_len == b_args_len,
			_ => false,
		},

		Op::Print { value: a_value } => match b {
			Op::Print { value: b_value } => a_value == b_value,
			_ => false,
		},
		Op::TupleFieldGetMut { out, value, index } => {
			matches!(b, Op::TupleFieldGetMut { out: bo, value: bv, index: bi } if out==bo && value==bv && index==bi)
		}
		Op::TupleFieldSetMut { value, index, new } => {
			matches!(b, Op::TupleFieldSetMut { value: bv, index: bi, new: bn } if value==bv && index==bi && new==bn)
		}

		Op::TupleField {
			out: a_out,
			value: a_value,
			index: a_index,
		} => match b {
			Op::TupleField {
				out: b_out,
				value: b_value,
				index: b_index,
			} => a_out == b_out && a_value == b_value && a_index == b_index,
			_ => false,
		},
		Op::SetTupleField {
			value: a_value,
			index: a_index,
			new: a_new,
		} => match b {
			Op::SetTupleField {
				value: b_value,
				index: b_index,
				new: b_new,
			} => a_value == b_value && a_index == b_index && a_new == b_new,
			_ => false,
		},
		Op::Record {
			out: a_out,
			tag: a_tag,
			args_start: a_args_start,
			args_len: a_args_len,
		} => match b {
			Op::Record {
				out: b_out,
				tag: b_tag,
				args_start: b_args_start,
				args_len: b_args_len,
			} => {
				a_out == b_out
					&& same_tag(a_tag, b_tag)
					&& a_args_start == b_args_start
					&& a_args_len == b_args_len
			}

			_ => false,
		},
		Op::FieldGetMut { out, value, key } => {
			matches!(b, Op::FieldGetMut { out: bo, value: bv, key: bk } if out == bo && value == bv && key == bk)
		}
		Op::FieldSetMut { value, key, new } => {
			matches!(b, Op::FieldSetMut { value: bv, key: bk, new: bn } if value == bv && key == bk && new == bn)
		}
		Op::Field {
			out: a_out,
			value: a_value,
			key: a_key,
		} => match b {
			Op::Field {
				out: b_out,
				value: b_value,
				key: b_key,
			} => a_out == b_out && a_value == b_value && a_key == b_key,
			_ => false,
		},
		Op::SetField {
			value: a_value,
			key: a_key,
			new: a_new,
		} => match b {
			Op::SetField {
				value: b_value,
				key: b_key,
				new: b_new,
			} => a_value == b_value && a_key == b_key && a_new == b_new,
			_ => false,
		},
		Op::Variant {
			out: a_out,
			tag: a_tag,
			success: a_success,
			payload: a_payload,
		} => match b {
			Op::Variant {
				out: b_out,
				tag: b_tag,
				success: b_success,
				payload: b_payload,
			} => {
				a_out == b_out
					&& same_tag(a_tag, b_tag)
					&& a_success == b_success
					&& a_payload == b_payload
			}

			_ => false,
		},
		Op::Try {
			out: a_out,
			value: a_value,
		} => match b {
			Op::Try {
				out: b_out,
				value: b_value,
			} => a_out == b_out && a_value == b_value,
			_ => false,
		},
		Op::CallValue {
			out: a_out,
			value: a_value,
			args_start: a_args_start,
			args_len: a_args_len,
		} => match b {
			Op::CallValue {
				out: b_out,
				value: b_value,
				args_start: b_args_start,
				args_len: b_args_len,
			} => {
				a_out == b_out
					&& a_value == b_value
					&& a_args_start == b_args_start
					&& a_args_len == b_args_len
			}

			_ => false,
		},
		Op::Range {
			out: a_out,
			start: a_start,
			end: a_end,
		} => match b {
			Op::Range {
				out: b_out,
				start: b_start,
				end: b_end,
			} => a_out == b_out && a_start == b_start && a_end == b_end,
			_ => false,
		},
		Op::IterStart {
			out: a_out,
			value: a_value,
		} => match b {
			Op::IterStart {
				out: b_out,
				value: b_value,
			} => a_out == b_out && a_value == b_value,
			_ => false,
		},
		Op::IterNext {
			out: a_out,
			iterator: a_iterator,
			none: a_none,
		} => match b {
			Op::IterNext {
				out: b_out,
				iterator: b_iterator,
				none: b_none,
			} => a_out == b_out && a_iterator == b_iterator && a_none == b_none,
			_ => false,
		},
		Op::String {
			out: a_out,
			text: a_text,
		} => match b {
			Op::String {
				out: b_out,
				text: b_text,
			} => a_out == b_out && a_text == b_text,
			_ => false,
		},
		Op::Clone {
			out: a_out,
			value: a_value,
			mode: a_mode,
		} => match b {
			Op::Clone {
				out: b_out,
				value: b_value,
				mode: b_mode,
			} => a_out == b_out && a_value == b_value && a_mode == b_mode,
			_ => false,
		},
		Op::AssignBinary {
			out: a_out,
			op: a_op,
			lhs: a_lhs,
			rhs: a_rhs,
		} => match b {
			Op::AssignBinary {
				out: b_out,
				op: b_op,
				lhs: b_lhs,
				rhs: b_rhs,
			} => a_out == b_out && a_op == b_op && a_lhs == b_lhs && a_rhs == b_rhs,
			_ => false,
		},
		Op::Literal {
			out: a_out,
			value: a_value,
		} => match b {
			Op::Literal {
				out: b_out,
				value: b_value,
			} => a_out == b_out && same_literal(a_value, b_value),
			_ => false,
		},
		Op::Copy {
			out: a_out,
			value: a_value,
		} => match b {
			Op::Copy {
				out: b_out,
				value: b_value,
			} => a_out == b_out && a_value == b_value,
			_ => false,
		},

		Op::Release { value: a_value } => match b {
			Op::Release { value: b_value } => a_value == b_value,
			_ => false,
		},
		Op::Unary {
			out: a_out,
			op: a_op,
			value: a_value,
		} => match b {
			Op::Unary {
				out: b_out,
				op: b_op,
				value: b_value,
			} => a_out == b_out && a_op == b_op && a_value == b_value,
			_ => false,
		},
		Op::Binary {
			out: a_out,
			op: a_op,
			lhs: a_lhs,
			rhs: a_rhs,
		} => match b {
			Op::Binary {
				out: b_out,
				op: b_op,
				lhs: b_lhs,
				rhs: b_rhs,
			} => a_out == b_out && a_op == b_op && a_lhs == b_lhs && a_rhs == b_rhs,
			_ => false,
		},
		Op::Cast {
			out: a_out,
			value: a_value,
			tag: a_tag,
		} => match b {
			Op::Cast {
				out: b_out,
				value: b_value,
				tag: b_tag,
			} => a_out == b_out && a_value == b_value && same_tag(a_tag, b_tag),
			_ => false,
		},
		Op::Is {
			out: a_out,
			value: a_value,
			tag: a_tag,
			negate: a_negate,
		} => match b {
			Op::Is {
				out: b_out,
				value: b_value,
				tag: b_tag,
				negate: b_negate,
			} => {
				a_out == b_out
					&& a_value == b_value
					&& same_tag(a_tag, b_tag)
					&& a_negate == b_negate
			}
			_ => false,
		},

		Op::Jump { target: a_target } => match b {
			Op::Jump { target: b_target } => a_target == b_target,
			_ => false,
		},
		Op::Branch {
			condition: a_condition,
			yes: a_yes,
			no: a_no,
		} => match b {
			Op::Branch {
				condition: b_condition,
				yes: b_yes,
				no: b_no,
			} => a_condition == b_condition && a_yes == b_yes && a_no == b_no,
			_ => false,
		},
		Op::Call {
			out: a_out,
			function: a_function,
			args_start: a_args_start,
			args_len: a_args_len,
		} => match b {
			Op::Call {
				out: b_out,
				function: b_function,
				args_start: b_args_start,
				args_len: b_args_len,
			} => {
				a_out == b_out
					&& a_function == b_function
					&& a_args_start == b_args_start
					&& a_args_len == b_args_len
			}

			_ => false,
		},

		Op::Return { value: a_value } => match b {
			Op::Return { value: b_value } => a_value == b_value,
			_ => false,
		},
		Op::List {
			out: a_out,
			tag: a_tag,
			args_start: a_args_start,
			args_len: a_args_len,
		} => match b {
			Op::List {
				out: b_out,
				tag: b_tag,
				args_start: b_args_start,
				args_len: b_args_len,
			} => {
				a_out == b_out
					&& same_tag(a_tag, b_tag)
					&& a_args_start == b_args_start
					&& a_args_len == b_args_len
			}

			_ => false,
		},
		Op::Get {
			out: a_out,
			value: a_value,
			index: a_index,
		} => match b {
			Op::Get {
				out: b_out,
				value: b_value,
				index: b_index,
			} => a_out == b_out && a_value == b_value && a_index == b_index,
			_ => false,
		},
		Op::Set {
			value: a_value,
			index: a_index,
			new: a_new,
		} => match b {
			Op::Set {
				value: b_value,
				index: b_index,
				new: b_new,
			} => a_value == b_value && a_index == b_index && a_new == b_new,
			_ => false,
		},
		Op::Len {
			out: a_out,
			value: a_value,
		} => match b {
			Op::Len {
				out: b_out,
				value: b_value,
			} => a_out == b_out && a_value == b_value,
			_ => false,
		},
		Op::Push {
			value: a_value,
			new: a_new,
		} => match b {
			Op::Push {
				value: b_value,
				new: b_new,
			} => a_value == b_value && a_new == b_new,
			_ => false,
		},

		Op::Panic { value } => matches!(b, Op::Panic { value: bv } if value == bv),
		Op::StringFrom { out, value } => {
			matches!(b, Op::StringFrom { out: bo, value: bv } if out == bo && value == bv)
		}
		Op::Drop { value: a_value } => match b {
			Op::Drop { value: b_value } => a_value == b_value,
			_ => false,
		},
	}
}

fn same_literal(a: Value, b: Value) -> bool {
	match (a, b) {
		(Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
		(Value::Type(a), Value::Type(b)) => same_tag(a, b),
		(a, b) => a == b,
	}
}

/// Requested live allocation footprint; excludes allocator metadata, RSS and caller inputs.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct CodeFootprint {
	pub original_buffers: usize,
	pub function_records: usize,
	pub cache_payload: usize,
	// Read only through `Debug` by the footprint inspection tests.
	#[allow(dead_code)]
	pub charge_scratch: usize,
	pub resident: usize,
	pub finish_peak: usize,
}

#[cfg_attr(not(test), allow(dead_code))]
fn footprint(functions: &Vec<Function>) -> Result<CodeFootprint> {
	let function_records = functions
		.capacity()
		.checked_mul(core::mem::size_of::<Function>())
		.ok_or_else(invalid)?;
	let mut original_buffers = 0usize;
	let mut cache_payload = 0usize;
	let mut charge_scratch = 0usize;

	for function in functions {
		original_buffers = original_buffers
			.checked_add(function.original_buffer_bytes()?)
			.ok_or_else(invalid)?;
		cache_payload = cache_payload
			.checked_add(function.cache_layout()?.2)
			.ok_or_else(invalid)?;
		charge_scratch = charge_scratch.max(
			function
				.code
				.len()
				.checked_mul(core::mem::size_of::<usize>())
				.ok_or_else(invalid)?,
		);
	}

	let original = original_buffers
		.checked_add(function_records)
		.ok_or_else(invalid)?;
	Ok(CodeFootprint {
		original_buffers,
		function_records,
		cache_payload,
		charge_scratch,
		resident: original.checked_add(cache_payload).ok_or_else(invalid)?,
		finish_peak: original
			.checked_add(cache_payload.max(charge_scratch))
			.ok_or_else(invalid)?,
	})
}

impl Code {
	#[cfg_attr(not(test), allow(dead_code))]
	pub fn footprint(&self) -> Result<CodeFootprint> {
		let mut footprint = footprint(&self.functions)?;
		let hosts = self
			.hosts
			.capacity()
			.checked_mul(core::mem::size_of::<crate::function::fresh_host::HostBinding>())
			.ok_or_else(invalid)?;
		let hosts = hosts
			.checked_add(
				self.absent
					.capacity()
					.checked_mul(core::mem::size_of::<crate::Hash>())
					.ok_or_else(invalid)?,
			)
			.ok_or_else(invalid)?;
		footprint.original_buffers = footprint
			.original_buffers
			.checked_add(hosts)
			.ok_or_else(invalid)?;
		footprint.resident = footprint.resident.checked_add(hosts).ok_or_else(invalid)?;
		footprint.finish_peak = footprint
			.finish_peak
			.checked_add(hosts)
			.ok_or_else(invalid)?;
		Ok(footprint)
	}

	/// Current layout sizes, for the prospective cache contract inspection.
	#[cfg_attr(not(test), allow(dead_code))]
	pub fn cache_type_layout() -> (usize, usize, usize) {
		(
			core::mem::size_of::<CachedInstruction>(),
			core::mem::align_of::<CachedInstruction>(),
			core::mem::size_of::<Function>(),
		)
	}
}

#[cfg(test)]
mod predecode_tests {
	use super::*;

	fn location() -> Location {
		Location {
			source: 1,
			revision: 1,
			start: 0,
			end: 1,
		}
	}

	fn builder() -> Builder {
		let mut builder = Builder::default();
		for _ in 0..2 {
			let mut f = Function::new(1, 0).unwrap();
			for _ in 0..127 {
				f.push(Op::Copy { out: 0, value: 0 }, location()).unwrap();
			}

			f.push(Op::Return { value: 0 }, location()).unwrap();
			builder.push(f).unwrap();
		}

		builder
	}

	#[test]
	fn cache_failure_at_first_and_second_buffer_cleans_up() {
		let one = 128 * core::mem::size_of::<CachedInstruction>();
		assert!(one > 128 * core::mem::size_of::<usize>());

		for budget in [one - 1, 2 * one - 1] {
			let builder = builder();
			let preexisting = footprint(&builder.functions).unwrap().resident;
			rune_alloc::limit::with(budget, || {
				assert_eq!(builder.finish().err().unwrap().kind, ErrorKind::Allocation);

				// The builder predates this limit. Its buffers are released here,
				// as is any cache allocated inside it; account both explicitly.
				assert_eq!(rune_alloc::limit::get(), budget + preexisting);
			})
			.call();
		}

		let code = builder().finish().unwrap();
		let accounting = code.footprint().unwrap();
		assert_eq!(accounting.cache_payload, 2 * one);
		assert_eq!(
			accounting.resident,
			accounting.original_buffers + accounting.function_records + 2 * one
		);
		assert_eq!(accounting.finish_peak, accounting.resident);

		for function in &code.functions {
			assert_eq!(function.cache_layout().unwrap(), (128, 128, one));
		}
	}

	#[test]
	fn cached_tokens_float_bits_and_charge_flags_match_and_mismatch_is_detected() {
		let mut f = Function::new(1, 0).unwrap();
		for value in [
			Value::Function { code: 0, id: 0 },
			Value::F64(f64::from_bits(0x7ff8000000000042)),
			Value::F64(-0.0),
		] {
			f.push(Op::Literal { out: 0, value }, location()).unwrap();
		}

		f.push(Op::Return { value: 0 }, location()).unwrap();
		let mut b = Builder::default();
		b.push(f).unwrap();
		let code = b.finish().unwrap();
		let f = code.function(0).unwrap();

		for pc in 0..f.len() {
			f.audit_cached(pc, f.cached(pc).unwrap()).unwrap();
		}

		assert!(
			matches!(f.cached(0).unwrap().op,Op::Literal {value:Value::Function {code:id,id:0},..} if id==code.id())
		);
		let mut bad = f.cached(0).unwrap();
		bad.charged = !bad.charged;
		assert_eq!(
			f.audit_cached(0, bad).unwrap_err().kind,
			ErrorKind::InvalidCode
		);
		let mut bad = f.cached(2).unwrap();
		bad.op = Op::Literal {
			out: 0,
			value: Value::F64(0.0),
		};

		assert_eq!(
			f.audit_cached(2, bad).unwrap_err().kind,
			ErrorKind::InvalidCode
		);
	}
}

#[cfg(test)]
pub(super) mod transaction_tests {
	use super::super::value::Nominal;
	use super::*;

	fn location() -> Location {
		Location {
			source: 0,
			revision: 0,
			start: 0,
			end: 1,
		}
	}

	fn function(tag: Type) -> Function {
		let mut f = Function::new(2, 0).unwrap();
		f.push(
			Op::Literal {
				out: 0,
				value: Value::Type(tag),
			},
			location(),
		)
		.unwrap();
		f.push(
			Op::Is {
				out: 1,
				value: 0,
				tag,
				negate: false,
			},
			location(),
		)
		.unwrap();
		f.push(Op::Return { value: 1 }, location()).unwrap();
		f
	}

	fn variant_fixture() -> Code {
		let mut tx = CompileTransaction::default();
		let a = tx
			.stage_variant(
				0xc15b5201,
				format_args!("C15bBound::A"),
				nominal::Shape::Empty,
				0xc15b5200,
				0,
				&[],
			)
			.unwrap();
		let b = tx
			.stage_variant(
				0xc15b5202,
				format_args!("C15bBound::B"),
				nominal::Shape::Empty,
				0xc15b5200,
				1,
				&[],
			)
			.unwrap();
		let twin = tx
			.stage_variant(
				0xc15b5201,
				format_args!("C15bBound::A"),
				nominal::Shape::Tuple(1),
				0xc15b5200,
				0,
				&[],
			)
			.unwrap();
		let mut f = Function::new(2, 1).unwrap();
		let callee = f.variant_test(Type::Struct(a)).unwrap();
		f.variant_test(Type::Struct(b)).unwrap();
		f.variant_test(Type::Struct(twin)).unwrap();
		let (args_start, args_len) = f.arguments(&[0]).unwrap();
		f.push(
			Op::Call {
				out: 1,
				function: callee,
				args_start,
				args_len,
			},
			location(),
		)
		.unwrap();
		f.push(Op::Return { value: 1 }, location()).unwrap();
		tx.push(f).unwrap();
		tx.finish().unwrap()
	}

	#[test]
	fn variant_call_binding_is_published_and_runs_without_a_script_frame() {
		use crate::experimental_core::{
			execute::Executor,
			heap::{Limits, Store},
			value::Scalar,
		};
		let code = variant_fixture();
		let f = code.function(0).unwrap();
		let Op::Call {
			function: callee, ..
		} = f.cached[0].op
		else {
			panic!("call predicate")
		};
		let identity = variant_test_id(callee).unwrap();
		assert!(identity != 0 && variant_test_type(callee).is_none());
		assert_eq!(f.code[0].b, callee);
		assert!(
			code.intrinsic(callee).is_none(),
			"caller binding never widens the fixed global policy"
		);
		f.audit_cached(0, f.cached[0]).unwrap();
		assert_eq!(
			f.types[0], f.types[1],
			"both variants retain their parent enum type identity"
		);
		let mut store = Store::new(Limits::default()).unwrap();
		let a_value = store.list(f.types[0], &[]).unwrap();
		let a = store.export(a_value);
		let b_value = store.list(f.types[1], &[]).unwrap();
		let b = store.export(b_value);
		let one = store.root_i64(1);
		let twin_value = store.list(f.types[2], &[one.value]).unwrap();
		let twin = store.export(twin_value);
		let mut executor =
			Executor::with_limits(crate::experimental_core::execute::ExecutorLimits {
				frames: 1,
				slots: 32,
			})
			.unwrap();
		for (root, expected) in [(&a, true), (&b, false), (&twin, true)] {
			let result = executor
				.invoke_code(&code, &mut store, 0, &[root], 1000)
				.unwrap();
			assert_eq!(
				store.root_scalar(&result).unwrap(),
				Some(Scalar::Bool(expected))
			);
			store.root_release(result).unwrap();
			assert!(executor.is_idle());
		}
		// The public builder resolves already-global descriptor operands too.
		let mut g = Function::new(2, 1).unwrap();
		let target = g.variant_test(f.types[0]).unwrap();
		let (args_start, args_len) = g.arguments(&[0]).unwrap();
		g.push(
			Op::Call {
				out: 1,
				function: target,
				args_start,
				args_len,
			},
			location(),
		)
		.unwrap();
		g.push(Op::Return { value: 1 }, location()).unwrap();
		let mut builder = Builder::default();
		builder.push(g).unwrap();
		let public = builder.finish().unwrap();
		assert!(
			matches!(public.function(0).unwrap().cached[0].op, Op::Call { function, .. } if function == callee)
		);
		let result = executor
			.invoke_code(&public, &mut store, 0, &[&a], 1000)
			.unwrap();
		assert_eq!(
			store.root_scalar(&result).unwrap(),
			Some(Scalar::Bool(true))
		);
		store.root_release(result).unwrap();
		for root in [a, b, twin, one] {
			store.root_release(root).unwrap();
		}
		assert_eq!(store.live_entries(), 0);
		store.close().unwrap();
	}

	#[test]
	fn variant_call_ids_require_caller_descriptors_and_slot_arguments() {
		let prepared = variant_fixture();
		let tag = prepared.function(0).unwrap().types[0];
		let Type::Struct(n) = tag else { unreachable!() };
		let identity = nominal::variant_id(n);
		for (callee, descriptors, values) in [
			(STAGED_VARIANT_TEST_BASE, &[][..], &[0][..]),
			(STAGED_VARIANT_TEST_BASE, &[Type::Vec][..], &[0][..]),
			(STAGED_VARIANT_TEST_BASE | 1, &[tag][..], &[0][..]),
			(VARIANT_TEST_BASE | identity, &[][..], &[0][..]),
			(VARIANT_TEST_BASE | identity, &[tag][..], &[][..]),
			(VARIANT_TEST_BASE | identity, &[tag][..], &[2][..]),
			(VARIANT_TEST_BASE | identity, &[tag][..], &[0, 0][..]),
		] {
			let mut f = Function::new(2, 1).unwrap();
			for &tag in descriptors {
				f.types.try_push(tag).unwrap();
			}
			let (args_start, args_len) = f.arguments(values).unwrap();
			f.push(
				Op::Call {
					out: 1,
					function: callee,
					args_start,
					args_len,
				},
				location(),
			)
			.unwrap();
			f.push(Op::Return { value: 1 }, location()).unwrap();
			let mut builder = Builder::default();
			builder.push(f).unwrap();
			assert_eq!(builder.finish().err().unwrap().kind, ErrorKind::InvalidCode);
		}
	}

	#[test]
	fn strict_seal_and_staged_range_checks() {
		let tag = Type::Struct(Nominal(nominal::PROVISIONAL));
		let mut public = Builder::default();
		public.push(function(tag)).unwrap();
		assert_eq!(public.finish().err().unwrap().kind, ErrorKind::InvalidCode);
		let mut tx = CompileTransaction::default();
		tx.stage(0xc001, format_args!("Local")).unwrap();
		tx.push(function(Type::Struct(Nominal(nominal::PROVISIONAL | 1))))
			.unwrap();
		assert_eq!(tx.finish().err().unwrap().kind, ErrorKind::InvalidCode);
	}

	#[test]
	fn remap_covers_pool_constants_and_eager_cache() {
		let mut tx = CompileTransaction::default();
		let n = tx.stage(0xc002, format_args!("Remapped")).unwrap();
		tx.push(function(Type::Struct(n))).unwrap();
		let mut code = tx.finish().unwrap();
		let mut visited = 0;
		code.for_each_type_mut(&mut |tag| {
			if let Some(n) = tag.nominal() {
				assert!(nominal::contains(n));
				visited += 1;
			}
		});
		assert_eq!(visited, 4, "pool, constant, cached literal, cached Is");
		for f in &code.functions {
			for (pc, &cached) in f.cached.iter().enumerate() {
				f.audit_cached(pc, cached).unwrap();
			}
		}
	}

	pub(crate) fn allocation_refusals_publish_nothing() {
		let mut tx = CompileTransaction::default();
		let tag = Type::Struct(tx.stage(0xc003, format_args!("FinishFailure")).unwrap());
		tx.push(function(tag)).unwrap();
		rune_alloc::limit::with(0, || {
			assert_eq!(tx.finish().err().unwrap().kind, ErrorKind::Allocation);
		})
		.call();
		assert_eq!(nominal::lookup(0xc003).unwrap(), None);
		// Empty code needs no cache allocation: the first allocation is remap.
		let mut tx = CompileTransaction::default();
		tx.stage(0xc004, format_args!("RemapFailure")).unwrap();
		rune_alloc::limit::with(0, || {
			assert_eq!(tx.finish().err().unwrap().kind, ErrorKind::Allocation);
		})
		.call();
		assert_eq!(nominal::lookup(0xc004).unwrap(), None);
	}
}

#[cfg(test)]
mod tuple_update_tests {
	use super::*;
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
	};
	fn location() -> Location {
		Location {
			source: 1,
			revision: 1,
			start: 0,
			end: 1,
		}
	}
	fn function(mutable: bool, filler: usize) -> Function {
		let mut f = Function::new(6, 1).unwrap();
		f.push(
			Op::Literal {
				out: 1,
				value: Value::I64(1),
			},
			location(),
		)
		.unwrap();
		for _ in 0..filler {
			f.push(Op::Copy { out: 5, value: 1 }, location()).unwrap();
		}
		f.push(
			Op::Literal {
				out: 2,
				value: Value::I64(0),
			},
			location(),
		)
		.unwrap();
		f.push(
			if mutable {
				Op::TupleFieldGetMut {
					out: 3,
					value: 0,
					index: 2,
				}
			} else {
				Op::TupleField {
					out: 3,
					value: 0,
					index: 2,
				}
			},
			location(),
		)
		.unwrap();
		f.push(
			Op::AssignBinary {
				out: 3,
				op: Binary::Add,
				lhs: 3,
				rhs: 1,
			},
			location(),
		)
		.unwrap();
		f.push(
			Op::Literal {
				out: 4,
				value: Value::I64(0),
			},
			location(),
		)
		.unwrap();
		f.push(
			if mutable {
				Op::TupleFieldSetMut {
					value: 0,
					index: 4,
					new: 3,
				}
			} else {
				Op::SetTupleField {
					value: 0,
					index: 4,
					new: 3,
				}
			},
			location(),
		)
		.unwrap();
		f.push(Op::Return { value: 0 }, location()).unwrap();
		f
	}
	fn code(f: Function) -> Result<Code> {
		let mut b = Builder::default();
		b.push(f)?;
		b.finish()
	}
	fn run(code: &Code, fuel: usize) -> (Option<Error>, Option<super::super::value::Scalar>) {
		let mut store = Store::new(Limits::default()).unwrap();
		let one = store.root_i64(1);
		let tuple = store.root_list(Type::Tuple, &[&one]).unwrap();
		let result = Executor::new()
			.unwrap()
			.invoke_code(code, &mut store, 0, &[&tuple], fuel);
		let error = match result {
			Ok(value) => {
				store.root_release(value).unwrap();
				None
			}
			Err(f) => Some(f.error),
		};
		let child = store.root_get(&tuple, 0).unwrap();
		let scalar = store.root_scalar(&child).unwrap();
		store.root_release(child).unwrap();
		store.root_release(tuple).unwrap();
		store.root_release(one).unwrap();
		assert_eq!(store.live_entries(), 0);
		(error, scalar)
	}
	#[test]
	fn tuple_update_layout_charge_flags_and_each_fuel_cut_match_the_old_lowering() {
		for filler in 250..261 {
			let old = code(function(false, filler)).unwrap();
			let new = code(function(true, filler)).unwrap();
			let a = old.function(0).unwrap();
			let b = new.function(0).unwrap();
			assert_eq!(a.code.len(), b.code.len());
			for (pc, (left, right)) in a.code.iter().zip(&b.code).enumerate() {
				assert_eq!(
					(left.a, left.b, left.c, left.is_charged()),
					(right.a, right.b, right.c, right.is_charged())
				);
				let op = match b.operation(pc).unwrap() {
					Op::TupleFieldGetMut { out, value, index } => {
						Op::TupleField { out, value, index }
					}
					Op::TupleFieldSetMut { value, index, new } => {
						Op::SetTupleField { value, index, new }
					}
					op => op,
				};
				assert!(same_op(a.operation(pc).unwrap(), op));
			}
			for fuel in 0..5 {
				assert_eq!(
					run(&old, fuel),
					run(&new, fuel),
					"filler {filler} fuel {fuel}"
				);
			}
		}
	}
	#[test]
	fn tuple_update_validation_refuses_jumps_into_the_group_or_unpaired_writeback() {
		for offset in 1..=4 {
			let mut f = function(true, 0);
			f.patch(0, Op::Jump { target: offset + 1 }).unwrap();
			assert_eq!(code(f).err().unwrap().kind, ErrorKind::InvalidCode);
		}
		let mut f = function(true, 0);
		f.patch(2, Op::Copy { out: 3, value: 0 }).unwrap();
		assert_eq!(code(f).err().unwrap().kind, ErrorKind::InvalidCode);
	}
}

#[cfg(test)]
mod named_update_validation_tests {
	use super::*;
	#[test]
	fn branches_cannot_enter_named_update_interior_or_skip_writeback() {
		let loc = Location {
			source: 0,
			revision: 0,
			start: 0,
			end: 1,
		};
		for target in [2, 3] {
			let mut f = Function::new(4, 1).unwrap();
			let key = f.string("x").unwrap();
			f.push(Op::Jump { target }, loc).unwrap();
			f.push(
				Op::FieldGetMut {
					out: 2,
					value: 0,
					key,
				},
				loc,
			)
			.unwrap();
			f.push(
				Op::AssignBinary {
					out: 2,
					lhs: 2,
					rhs: 1,
					op: Binary::Add,
				},
				loc,
			)
			.unwrap();
			f.push(
				Op::FieldSetMut {
					value: 0,
					key,
					new: 2,
				},
				loc,
			)
			.unwrap();
			f.push(Op::Return { value: 2 }, loc).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			assert_eq!(b.finish().err().unwrap().kind, ErrorKind::InvalidCode);
		}
		for malformed in 0..4 {
			let mut f = Function::new(4, 1).unwrap();
			let key = f.string("x").unwrap();
			let other = f.string("y").unwrap();
			f.push(
				Op::FieldGetMut {
					out: 2,
					value: 0,
					key,
				},
				loc,
			)
			.unwrap();
			f.push(
				if malformed == 0 {
					Op::Copy { out: 2, value: 1 }
				} else {
					Op::AssignBinary {
						out: 2,
						lhs: 2,
						rhs: 1,
						op: Binary::Add,
					}
				},
				loc,
			)
			.unwrap();
			f.push(
				Op::FieldSetMut {
					value: if malformed == 1 { 3 } else { 0 },
					key: if malformed == 2 { other } else { key },
					new: if malformed == 3 { 1 } else { 2 },
				},
				loc,
			)
			.unwrap();
			f.push(Op::Return { value: 2 }, loc).unwrap();
			let mut b = Builder::default();
			b.push(f).unwrap();
			assert_eq!(b.finish().err().unwrap().kind, ErrorKind::InvalidCode);
		}
	}
}

#[cfg(test)]
mod clone_mode_tests {
	use super::*;
	#[test]
	fn invalid_encoded_clone_modes_are_rejected() {
		for mode in [2, 3, u32::MAX] {
			let mut f = Function::new(2, 1).unwrap();
			let loc = Location {
				source: 1,
				revision: 1,
				start: 0,
				end: 1,
			};
			f.push(
				Op::Clone {
					out: 1,
					value: 0,
					mode: CloneMode::Global,
				},
				loc,
			)
			.unwrap();
			f.push(Op::Return { value: 1 }, loc).unwrap();
			f.code[0].c = mode;
			assert_eq!(f.operation(0).err().unwrap().kind, ErrorKind::InvalidCode);
			let mut b = Builder::default();
			b.push(f).unwrap();
			assert_eq!(b.finish().err().unwrap().kind, ErrorKind::InvalidCode);
		}
	}
}

#[cfg(all(test, feature = "std", not(miri)))]
impl Function {
	pub(crate) fn emission_snapshot(&self) -> std::string::String {
		use std::fmt::Write;
		let mut text = std::format!(
			"slots={} parameters={}\nconstants={:?}\ntypes={:?}\nranges={:?}\nstrings={:?}\nargs={:?}\nlocations={:?}\n",
			self.slots,
			self.parameters,
			self.constants,
			self.types,
			self.ranges,
			self.strings,
			self.args,
			self.locations
		);
		for pc in 0..self.len() {
			writeln!(
				text,
				"{pc}: charged={} {:?}",
				self.code[pc].is_charged(),
				self.operation(pc).unwrap()
			)
			.unwrap();
		}
		text
	}
}

#[cfg(test)]
mod intrinsic_partition_tests {
	use super::*;
	#[test]
	fn ordinary_id_partition_is_exact_without_large_allocation() {
		assert_eq!(
			ordinary_function_id(FIRST_INTRINSIC_ID as usize - 1).unwrap(),
			FIRST_INTRINSIC_ID - 1
		);
		assert_eq!(
			ordinary_function_id(FIRST_INTRINSIC_ID as usize)
				.unwrap_err()
				.kind,
			ErrorKind::InvalidCode
		);
		assert_eq!(
			ordinary_function_id(usize::MAX).unwrap_err().kind,
			ErrorKind::InvalidCode
		);
		assert!(!reserved_intrinsic_id(FIRST_INTRINSIC_ID - 1));
		assert!(reserved_intrinsic_id(FIRST_INTRINSIC_ID));
		// C11c adds the certified cold capture-take intrinsic to the prior49.
		assert_eq!(CERTIFIED_INTRINSICS.len(), 50);
		assert_eq!(
			certified_intrinsic(CERTIFIED_INTRINSICS, CAPTURE_TAKE)
				.unwrap()
				.arity,
			1
		);
		assert_eq!(
			certified_intrinsic(CERTIFIED_INTRINSICS, MISSING_METHOD)
				.unwrap()
				.arity,
			2
		);
		assert_eq!(
			certified_intrinsic(CERTIFIED_INTRINSICS, METHOD_CMP)
				.unwrap()
				.arity,
			2
		);
		assert_eq!(CERTIFIED_INTRINSICS[0].id, PRINT_RAW);
		assert_eq!(CERTIFIED_INTRINSICS[0].arity, 1);
		assert_eq!(CERTIFIED_INTRINSICS[1].id, VARIANT_SUCCESS);
		assert_eq!(CERTIFIED_INTRINSICS[1].arity, 1);
		assert_ne!(PRINT_RAW, VARIANT_SUCCESS);
		for id in [0, FIRST_INTRINSIC_ID, VARIANT_SUCCESS + 1, u32::MAX] {
			assert!(certified_intrinsic(CERTIFIED_INTRINSICS, id).is_none());
		}
	}
}

#[cfg(test)]
mod template_metadata_tests {
	use super::*;

	fn location() -> Location {
		Location {
			source: 0,
			revision: 0,
			start: 0,
			end: 1,
		}
	}

	fn function() -> Function {
		let mut f = Function::new(128, 1).unwrap();
		f.string("key").unwrap();
		// Slot + descriptor pair precedes the six-word metadata suffix.
		f.arguments(&[0, 4, 0, ' ' as u32, 0, 12, 0, 0]).unwrap();
		f.push(
			Op::Template {
				out: 1,
				args_start: 0,
				args_len: 2,
			},
			location(),
		)
		.unwrap();
		f.push(Op::Return { value: 1 }, location()).unwrap();
		f
	}

	fn rejected(f: Function) {
		let mut b = Builder::default();
		b.push(f).unwrap();
		assert_eq!(b.finish().err().unwrap().kind, ErrorKind::InvalidCode);
	}

	#[test]
	fn complete_record_is_admitted_but_overlapping_executable_ranges_are_not() {
		let mut b = Builder::default();
		b.push(function()).unwrap();
		assert!(b.finish().is_ok());
		for op in [
			Op::Call {
				out: 1,
				function: 0,
				args_start: 2,
				args_len: 1,
			},
			Op::CallValue {
				out: 1,
				value: 0,
				args_start: 2,
				args_len: 1,
			},
			Op::List {
				out: 1,
				tag: Type::Vec,
				args_start: 2,
				args_len: 1,
			},
			Op::Record {
				out: 1,
				tag: Type::Object,
				args_start: 2,
				args_len: 2,
			},
			Op::Template {
				out: 1,
				args_start: 2,
				args_len: 2,
			},
		] {
			let mut f = function();
			f.patch(1, op).unwrap();
			f.push(Op::Return { value: 1 }, location()).unwrap();
			rejected(f);
		}
	}

	#[test]
	fn rejects_truncated_misaligned_and_out_of_range_metadata() {
		let mut f = function();
		f.args.pop();
		rejected(f);
		for descriptor in [5, 100, u32::MAX] {
			let mut f = function();
			f.args[1] = descriptor;
			rejected(f);
		}
		for (index, value) in [(2, 16), (3, 0xd800), (4, 3), (5, 65536), (6, 65537), (7, 2)] {
			let mut f = function();
			f.args[index] = value;
			rejected(f);
		}
	}

	#[test]
	fn function_local_suffix_offsets_survive_builder_composition() {
		let mut first = function();
		let mut second = Function::new(128, 1).unwrap();
		second
			.arguments(&[0, 0, 0, 6, 0, ' ' as u32, 0, 12, 0, 0])
			.unwrap();
		second
			.push(
				Op::Template {
					out: 1,
					args_start: 2,
					args_len: 2,
				},
				location(),
			)
			.unwrap();
		second.push(Op::Return { value: 1 }, location()).unwrap();
		assert_eq!(template_args_prefix(&first).unwrap(), 2);
		assert_eq!(template_args_prefix(&second).unwrap(), 4);
		first.args[1] = 0;
		first.args.truncate(2);
		assert_eq!(template_args_prefix(&first).unwrap(), 2);
		let mut b = Builder::default();
		b.push(first).unwrap();
		b.push(second).unwrap();
		assert!(b.finish().is_ok());
	}
}

#[cfg(test)]
mod template_lowering_tests {
	use super::*;
	use crate::{Context, Diagnostics, Source, Sources};

	fn compile(text: &str) -> super::super::frontend::Program {
		let context = Context::with_default_modules().unwrap();
		let mut sources = Sources::new();
		sources.insert(Source::memory(text).unwrap()).unwrap();
		super::super::frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7).unwrap()
	}

	#[test]
	#[cfg_attr(
		miri,
		ignore = "source compilation is covered by native tests; bounded Miri covers metadata and formatting helpers"
	)]
	fn lowering_appends_specs_after_later_call_list_and_record_args() {
		let program = compile(
			"fn id(x) { x } pub fn main() { let s=format!(\"{:>7}\", 1); let c=id(3); let v=[c]; let r=#{a:c}; s.len()+v[0]+r.a }",
		);
		let mut witnessed = false;
		for id in 0..program.code.functions() {
			let f = program.code.function(id as u32).unwrap();
			let prefix = template_args_prefix(f).unwrap();
			if prefix == f.args.len() {
				continue;
			}
			witnessed = true;
			assert_eq!(f.args.len() - prefix, 6);
			let mut kinds = [false; 3];
			for instruction in &f.code {
				match f.decode(*instruction).unwrap() {
					Op::Call {
						args_start,
						args_len,
						..
					} => {
						assert!((args_start + args_len) as usize <= prefix);
						kinds[0] = true;
					}
					Op::List {
						args_start,
						args_len,
						..
					} => {
						assert!((args_start + args_len) as usize <= prefix);
						kinds[1] = true;
					}
					Op::Record {
						args_start,
						args_len,
						..
					} => {
						assert!((args_start + args_len) as usize <= prefix);
						kinds[2] = true;
					}
					Op::Template {
						args_start,
						args_len,
						..
					} => {
						for pair in f.args[args_start as usize..(args_start + args_len) as usize]
							.as_chunks::<2>()
							.0
						{
							if pair[1] >= 2 {
								assert_eq!((pair[1] - 2) as usize, prefix);
							}
						}
					}
					_ => (),
				}
			}
			assert_eq!(kinds, [true; 3]);
		}
		assert!(witnessed);
	}

	#[test]
	#[cfg_attr(miri, ignore = "source compilation is covered by native tests")]
	fn nested_function_has_its_own_spec_fixups_and_radices_stay_refused() {
		let program =
			compile("pub fn main() { fn nested() { format!(\"{:>7}\", 1).len() } nested() }");
		assert!((0..program.code.functions()).any(|id| {
			let f = program.code.function(id as u32).unwrap();
			template_args_prefix(f).unwrap() < f.args.len()
		}));
		for kind in ["x", "X", "b", "p"] {
			let context = Context::with_default_modules().unwrap();
			let mut sources = Sources::new();
			sources
				.insert(
					Source::memory(std::format!(
						"pub fn main() {{ format!(\"{{:{kind}}}\", 1) }}"
					))
					.unwrap(),
				)
				.unwrap();
			let failure =
				super::super::frontend::compile(&mut sources, &context, &mut Diagnostics::new(), 7)
					.err()
					.unwrap();
			assert_eq!(
				failure.refusal,
				Some(super::super::error::Refusal::FormatSpec)
			);
		}
	}
}

#[cfg(all(test, target_pointer_width = "64"))]
mod template_layout_tests {
	use super::*;
	#[test]
	fn c5_metadata_preserves_runtime_layouts() {
		let sizes = [
			core::mem::size_of::<Function>(),
			core::mem::size_of::<Code>(),
			core::mem::size_of::<Op>(),
			core::mem::size_of::<Instruction>(),
			core::mem::size_of::<CachedInstruction>(),
		];
		std::println!("C5 layouts Function/Code/Op/Instruction/CachedInstruction: {sizes:?}");
		// C13 adds one 24-byte absence-proof Vec to Code only.
		assert_eq!(sizes, [200, 80, 24, 16, 32]);
	}
}

#[cfg(test)]
pub(crate) fn c11_depth_metadata(program: &super::frontend::Program) -> serde_json::Value {
	use serde_json::json;
	let main = program.function(crate::Hash::type_hash(["main"])).unwrap();
	let compare = program
		.function(crate::Hash::type_hash(["compare"]))
		.unwrap();
	let helper = program.function(crate::Hash::EMPTY).unwrap();
	let location = |id: u32| {
		let f = program.code.function(id).unwrap();
		let mut found = None;
		for pc in 0..f.instructions().len() {
			let op = f.operation(pc).unwrap();
			if matches!(op,Op::Call{function,..} if function==helper) {
				assert!(found.is_none());
				let at = f.locations[pc];
				found = Some(json!({"source":at.source,"start":at.start,"end":at.end}));
			}
		}
		found.unwrap()
	};
	json!({"main_id":main,"compare_id":compare,"helper_id":helper,"main_sort_site":location(main),"compare_sort_site":location(compare),"helper_transparent":program.code.function(helper).unwrap().is_transparent_sort(),"functions":(0..program.code.functions()).map(|id|{let f=program.code.function(id as u32).unwrap();json!({"id":id,"slots":f.slots,"parameters":f.parameters})}).collect::<std::vec::Vec<_>>()})
}
