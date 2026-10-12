//! Owned experimental instructions. Validated code has no old Unit/Value/VM dependency.

use super::arithmetic::{Binary, Unary};
use super::error::{Error, ErrorKind, Location, Result};
use super::nominal;
use super::value::{Type, Value};
use crate::alloc::{String, Vec};
use core::sync::atomic::{AtomicU32, Ordering};

const MAX_CODE_BYTES: usize = 16 << 20;
pub const MAX_GAP: usize = 256;

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
	Clone {
		out: u32,
		value: u32,
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
			Clone { out, value } => (3, out, value, 0),
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
			3 => Clone { out: a, value: b },
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

pub struct Code {
	/// Process-unique, never 0 (the lowerer's "unstamped" marker), never
	/// reused: a function token names the program it indexes by this.
	id: u32,
	functions: Vec<Function>,
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
}

fn invalid() -> Error {
	Error::new(ErrorKind::InvalidCode, "validate code")
}

impl Default for Builder {
	fn default() -> Self {
		Self {
			functions: Vec::new(),
		}
	}
}

impl Builder {
	pub fn push(&mut self, function: Function) -> Result<u32> {
		let id = u32::try_from(self.functions.len()).map_err(|_| invalid())?;
		self.functions.try_push(function)?;
		Ok(id)
	}

	#[cfg_attr(not(test), allow(dead_code))]
	pub fn finish(self) -> Result<Code> {
		self.finish_validated(&nominal_valid)
	}

	fn finish_validated(mut self, valid: &dyn Fn(Type) -> bool) -> Result<Code> {
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
			validate_operands(&self.functions[i], &self.functions, id, valid)?;
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
}

impl CompileTransaction {
	pub(crate) fn stage(
		&mut self,
		hash: u64,
		name: core::fmt::Arguments<'_>,
	) -> Result<super::value::Nominal> {
		self.staging.stage(hash, name)
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
			for cached in &mut function.cached {
				match &mut cached.op {
					Op::Record { tag, .. }
					| Op::List { tag, .. }
					| Op::Variant { tag, .. }
					| Op::Cast { tag, .. }
					| Op::Is { tag, .. } => f(tag),
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
		Op::Return { .. } => [None, None],
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

// D21-derived validation logic preserved during extraction: argument tables hold fixed pairs.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn validate_operands(
	f: &Function,
	functions: &[Function],
	code_id: u32,
	valid: &dyn Fn(Type) -> bool,
) -> Result<()> {
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
				let parts = f
					.args
					.get(args_start as usize..end as usize)
					.ok_or_else(invalid)?;
				for pair in parts.chunks_exact(2) {
					slot(pair[0])?;
					if pair[1] > 1 {
						return Err(invalid());
					}
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
			| Op::Clone { out, value }
			| Op::Copy { out, value }
			| Op::Unary { out, value, .. }
			| Op::Cast { out, value, .. }
			| Op::Is { out, value, .. }
			| Op::Len { out, value } => {
				slot(out)?;
				slot(value)?;
			}

			Op::Release { value } | Op::Return { value } | Op::Drop { value } => {
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

			Op::Field { out, value, key } => {
				slot(out)?;
				slot(value)?;
				f.strings.get(key as usize).ok_or_else(invalid)?;
			}

			Op::SetField { value, key, new } => {
				slot(value)?;
				slot(new)?;
				f.strings.get(key as usize).ok_or_else(invalid)?;
			}

			Op::Get { out, value, index } | Op::TupleField { out, value, index } => {
				slot(out)?;
				slot(value)?;
				slot(index)?;
			}

			Op::Set { value, index, new } | Op::SetTupleField { value, index, new } => {
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
				let callee = functions.get(function as usize).ok_or_else(invalid)?;

				if args_len != callee.parameters {
					return Err(invalid());
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
				if !matches!(tag, Type::Vec | Type::Tuple) {
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
					&& a_tag == b_tag
					&& a_args_start == b_args_start
					&& a_args_len == b_args_len
			}

			_ => false,
		},
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
				a_out == b_out && a_tag == b_tag && a_success == b_success && a_payload == b_payload
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
		} => match b {
			Op::Clone {
				out: b_out,
				value: b_value,
			} => a_out == b_out && a_value == b_value,
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
			} => a_out == b_out && a_value == b_value && a_tag == b_tag,
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
			} => a_out == b_out && a_value == b_value && a_tag == b_tag && a_negate == b_negate,
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
					&& a_tag == b_tag
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

		Op::Drop { value: a_value } => match b {
			Op::Drop { value: b_value } => a_value == b_value,
			_ => false,
		},
	}
}

fn same_literal(a: Value, b: Value) -> bool {
	match (a, b) {
		(Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
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
		footprint(&self.functions)
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
