//! Stock's stable bottom-up merge order, expressed as ordinary charged code.
use super::super::code::{
	SORT_BEGIN, SORT_COMMIT, SORT_LEN, SORT_LOAD, SORT_ORDERING, SORT_STORE, SORT_VALUE,
};
use super::*;
impl Lower<'_, '_> {
	#[cold]
	#[inline(never)]
	pub(super) fn call_sort_by(
		&mut self,
		call: &hir::ExprCall<'_>,
		target: hir::ExprId,
		out: u32,
		span: Span,
	) -> Result<()> {
		if !self.context.sort_by() || self.constants.sort_by_script {
			return Err(self.refused(span, Refusal::Method));
		}
		if call.args.len() != 1 {
			return Err(self.refused(span, Refusal::Arity));
		}
		let args = self.operands(&[target, call.args[0]])?;
		let (args_start, args_len) = self.function.arguments(&args)?;
		let at = self.location(span);
		let function = (self.resolve)(Resolve::Sort, at)?;
		self.emit(
			Op::Call {
				out,
				function,
				args_start,
				args_len,
			},
			span,
		)?;
		Ok(())
	}
}
// Slot ledger (25): parent0, cmp1, guard2, len3, width4, bank5,
// start6, mid7, end8, i9, j10, k11, temp12, cond13, lhs-index14,
// rhs-index15, lhs16, rhs17, ordering18, Greater19, one20, two21,
// zero22, other-bank23, unit24. Only lhs/rhs/ordering own transient values.
struct Assembler {
	f: Function,
	at: Location,
	labels: Vec<u32>,
	fixes: Vec<(u32, usize, Option<u32>)>,
}
impl Assembler {
	fn emit(&mut self, op: Op) -> Result<()> {
		self.f.push(op, self.at)?;
		Ok(())
	}
	fn label(&mut self, id: usize) {
		self.labels[id] = self.f.len() as u32;
	}
	fn jump(&mut self, label: usize) -> Result<()> {
		let pc = self.f.push(Op::Jump { target: 0 }, self.at)?;
		self.fixes.try_push((pc, label, None))?;
		Ok(())
	}
	// A branch's false destination is the next instruction (fallthrough).
	fn branch(&mut self, condition: u32, label: usize) -> Result<()> {
		let pc = self.f.push(
			Op::Branch {
				condition,
				yes: 0,
				no: 0,
			},
			self.at,
		)?;
		self.fixes.try_push((pc, label, Some(condition)))?;
		Ok(())
	}
	fn lit(&mut self, out: u32, value: i64) -> Result<()> {
		self.emit(Op::Literal {
			out,
			value: Value::I64(value),
		})
	}
	fn copy(&mut self, out: u32, value: u32) -> Result<()> {
		self.emit(Op::Copy { out, value })
	}
	fn bin(&mut self, out: u32, op: Binary, lhs: u32, rhs: u32) -> Result<()> {
		self.emit(Op::Binary { out, op, lhs, rhs })
	}
	fn call(&mut self, out: u32, id: u32, args: &[u32]) -> Result<()> {
		let (args_start, args_len) = self.f.arguments(args)?;
		self.emit(Op::Call {
			out,
			function: id,
			args_start,
			args_len,
		})
	}
	fn min_len(&mut self, out: u32, a: u32, b: u32, label: usize) -> Result<()> {
		self.bin(out, Binary::Add, a, b)?;
		self.bin(13, Binary::Le, out, 3)?;
		self.branch(13, label)?;
		self.copy(out, 3)?;
		self.label(label);
		Ok(())
	}
}
#[cold]
#[inline(never)]
pub(in crate::experimental_core) fn function(_at: Location) -> Result<Function> {
	let mut a = Assembler {
		f: Function::new(25, 2)?,
		at: super::super::code::TRANSPARENT_SORT_LOCATION,
		labels: Vec::new(),
		fixes: Vec::new(),
	};
	a.labels.try_resize(16, u32::MAX)?;
	// Labels: outer0, finish1, runs2, swap3, merge4, right5, left6,
	// copied7, run-done8, mid-clamp9, end-clamp10, ordering-ok11.
	a.call(2, SORT_BEGIN, &[0, 1])?;
	a.call(3, SORT_LEN, &[2])?;
	a.lit(4, 1)?;
	a.lit(5, 0)?;
	a.lit(20, 1)?;
	a.lit(21, 2)?;
	a.lit(22, 0)?;
	a.emit(Op::Literal {
		out: 19,
		value: Value::Ordering(core::cmp::Ordering::Greater),
	})?;
	a.label(0);
	a.bin(13, Binary::Ge, 4, 3)?;
	a.branch(13, 1)?;
	a.copy(6, 22)?;
	a.bin(23, Binary::Sub, 20, 5)?;
	a.label(2);
	a.bin(13, Binary::Ge, 6, 3)?;
	a.branch(13, 3)?;
	a.min_len(7, 6, 4, 9)?;
	a.bin(12, Binary::Mul, 4, 21)?;
	a.min_len(8, 6, 12, 10)?;
	a.copy(9, 6)?;
	a.copy(10, 7)?;
	a.copy(11, 6)?;
	a.label(4);
	a.bin(13, Binary::Ge, 11, 8)?;
	a.branch(13, 8)?;
	a.bin(13, Binary::Ge, 9, 7)?;
	a.branch(13, 5)?;
	a.bin(13, Binary::Ge, 10, 8)?;
	a.branch(13, 6)?;
	a.call(14, SORT_LOAD, &[2, 5, 9])?;
	a.call(15, SORT_LOAD, &[2, 5, 10])?;
	a.call(16, SORT_VALUE, &[2, 14])?;
	a.call(17, SORT_VALUE, &[2, 15])?;
	let (args_start, args_len) = a.f.arguments(&[16, 17])?;
	a.emit(Op::CallValue {
		out: 18,
		value: 1,
		args_start,
		args_len,
	})?;
	a.emit(Op::Release { value: 17 })?;
	a.emit(Op::Release { value: 16 })?;
	a.emit(Op::Is {
		out: 13,
		value: 18,
		tag: Type::Ordering,
		negate: false,
	})?;
	a.branch(13, 11)?;
	a.call(24, SORT_ORDERING, &[18])?;
	a.label(11);
	a.bin(13, Binary::Eq, 18, 19)?;
	a.branch(13, 5)?;
	a.label(6);
	a.call(14, SORT_LOAD, &[2, 5, 9])?;
	a.call(24, SORT_STORE, &[2, 23, 11, 14])?;
	a.bin(9, Binary::Add, 9, 20)?;
	a.jump(7)?;
	a.label(5);
	a.call(15, SORT_LOAD, &[2, 5, 10])?;
	a.call(24, SORT_STORE, &[2, 23, 11, 15])?;
	a.bin(10, Binary::Add, 10, 20)?;
	a.label(7);
	a.bin(11, Binary::Add, 11, 20)?;
	a.jump(4)?;
	a.label(8);
	a.copy(6, 8)?;
	a.jump(2)?;
	a.label(3);
	a.copy(5, 23)?;
	a.bin(4, Binary::Mul, 4, 21)?;
	a.jump(0)?;
	a.label(1);
	a.call(24, SORT_COMMIT, &[2, 5])?;
	a.emit(Op::Release { value: 2 })?;
	a.emit(Op::Return { value: 24 })?;
	for (pc, label, condition) in a.fixes {
		let target = *a
			.labels
			.get(label)
			.ok_or_else(|| Error::new(ErrorKind::InvalidCode, "sort label"))?;
		if target == u32::MAX {
			return Err(Error::new(ErrorKind::InvalidCode, "sort label"));
		}
		a.f.patch(
			pc,
			match condition {
				Some(condition) => Op::Branch {
					condition,
					yes: target,
					no: pc + 1,
				},
				None => Op::Jump { target },
			},
		)?;
	}
	Ok(a.f)
}
