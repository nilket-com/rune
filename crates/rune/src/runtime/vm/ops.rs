use core::ops::{
	Add, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div, Mul, Rem, Sub,
};

use crate::runtime::{InstArithmeticOp, InstBitwiseOp, InstShiftOp, Protocol, VmErrorKind};

/// The error a division or a remainder which could not be performed raises.
fn divide_error(rhs: i128) -> VmErrorKind {
	if rhs == 0 {
		VmErrorKind::DivideByZero
	} else {
		VmErrorKind::Overflow
	}
}

pub(super) struct ArithmeticOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	/// The error an operation which could not be performed raises, given the
	/// right hand side it was performed against.
	///
	/// Division and remainder fail for two different reasons, so which one it
	/// was has to be told apart from the divisor: one of zero is what usually
	/// fails, but dividing the smallest signed number by `-1` overflows.
	pub(super) error: fn(i128) -> VmErrorKind,
	pub(super) i64: fn(i64, i64) -> Option<i64>,
	pub(super) u64: fn(u64, u64) -> Option<u64>,
	pub(super) f64: fn(f64, f64) -> f64,
}

impl ArithmeticOps {
	pub(super) fn from_op(op: InstArithmeticOp) -> &'static Self {
		match op {
			InstArithmeticOp::Add => &Self {
				protocol: Protocol::ADD,
				symbol: "+",
				error: |_| VmErrorKind::Overflow,
				i64: i64::checked_add,
				u64: u64::checked_add,
				f64: f64::add,
			},
			InstArithmeticOp::Sub => &Self {
				protocol: Protocol::SUB,
				symbol: "-",
				error: |_| VmErrorKind::Underflow,
				i64: i64::checked_sub,
				u64: u64::checked_sub,
				f64: f64::sub,
			},
			InstArithmeticOp::Mul => &Self {
				protocol: Protocol::MUL,
				symbol: "*",
				error: |_| VmErrorKind::Overflow,
				i64: i64::checked_mul,
				u64: u64::checked_mul,
				f64: f64::mul,
			},
			InstArithmeticOp::Div => &Self {
				protocol: Protocol::DIV,
				symbol: "/",
				error: divide_error,
				i64: i64::checked_div,
				u64: u64::checked_div,
				f64: f64::div,
			},
			InstArithmeticOp::Rem => &Self {
				protocol: Protocol::REM,
				symbol: "%",
				error: divide_error,
				i64: i64::checked_rem,
				u64: u64::checked_rem,
				f64: f64::rem,
			},
		}
	}
}

pub(super) struct AssignArithmeticOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	/// See [`ArithmeticOps::error`].
	pub(super) error: fn(i128) -> VmErrorKind,
	pub(super) i64: fn(i64, i64) -> Option<i64>,
	pub(super) u64: fn(u64, u64) -> Option<u64>,
	pub(super) f64: fn(f64, f64) -> f64,
}

impl AssignArithmeticOps {
	pub(super) fn from_op(op: InstArithmeticOp) -> &'static AssignArithmeticOps {
		match op {
			InstArithmeticOp::Add => &Self {
				protocol: Protocol::ADD_ASSIGN,
				symbol: "+=",
				error: |_| VmErrorKind::Overflow,
				i64: i64::checked_add,
				u64: u64::checked_add,
				f64: f64::add,
			},
			InstArithmeticOp::Sub => &Self {
				protocol: Protocol::SUB_ASSIGN,
				symbol: "-=",
				error: |_| VmErrorKind::Underflow,
				i64: i64::checked_sub,
				u64: u64::checked_sub,
				f64: f64::sub,
			},
			InstArithmeticOp::Mul => &Self {
				protocol: Protocol::MUL_ASSIGN,
				symbol: "*=",
				error: |_| VmErrorKind::Overflow,
				i64: i64::checked_mul,
				u64: u64::checked_mul,
				f64: f64::mul,
			},
			InstArithmeticOp::Div => &Self {
				protocol: Protocol::DIV_ASSIGN,
				symbol: "/=",
				error: divide_error,
				i64: i64::checked_div,
				u64: u64::checked_div,
				f64: f64::div,
			},
			InstArithmeticOp::Rem => &Self {
				protocol: Protocol::REM_ASSIGN,
				symbol: "%=",
				error: divide_error,
				i64: i64::checked_rem,
				u64: u64::checked_rem,
				f64: f64::rem,
			},
		}
	}
}

pub(super) struct AssignBitwiseOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	pub(super) i64: fn(&mut i64, i64),
	pub(super) u64: fn(&mut u64, u64),
	pub(super) bool: fn(&mut bool, bool),
}

impl AssignBitwiseOps {
	pub(super) fn from_ops(op: InstBitwiseOp) -> &'static Self {
		match op {
			InstBitwiseOp::BitAnd => &Self {
				protocol: Protocol::BIT_AND_ASSIGN,
				symbol: "&=",
				i64: i64::bitand_assign,
				u64: u64::bitand_assign,
				bool: bool::bitand_assign,
			},
			InstBitwiseOp::BitXor => &Self {
				protocol: Protocol::BIT_XOR_ASSIGN,
				symbol: "^=",
				i64: i64::bitxor_assign,
				u64: u64::bitxor_assign,
				bool: bool::bitxor_assign,
			},
			InstBitwiseOp::BitOr => &Self {
				protocol: Protocol::BIT_OR_ASSIGN,
				symbol: "|=",
				i64: i64::bitor_assign,
				u64: u64::bitor_assign,
				bool: bool::bitor_assign,
			},
		}
	}
}

pub(super) struct BitwiseOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	pub(super) i64: fn(i64, i64) -> i64,
	pub(super) u64: fn(u64, u64) -> u64,
	pub(super) bool: fn(bool, bool) -> bool,
}

impl BitwiseOps {
	pub(super) fn from_op(op: InstBitwiseOp) -> &'static BitwiseOps {
		match op {
			InstBitwiseOp::BitAnd => &BitwiseOps {
				protocol: Protocol::BIT_AND,
				symbol: "&",
				i64: i64::bitand,
				u64: u64::bitand,
				bool: bool::bitand,
			},
			InstBitwiseOp::BitXor => &BitwiseOps {
				protocol: Protocol::BIT_XOR,
				symbol: "^",
				i64: i64::bitxor,
				u64: u64::bitxor,
				bool: bool::bitxor,
			},
			InstBitwiseOp::BitOr => &BitwiseOps {
				protocol: Protocol::BIT_OR,
				symbol: "|",
				i64: i64::bitor,
				u64: u64::bitor,
				bool: bool::bitor,
			},
		}
	}
}

pub(super) struct AssignShiftOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	pub(super) error: fn() -> VmErrorKind,
	pub(super) i64: fn(i64, u32) -> Option<i64>,
	pub(super) u64: fn(u64, u32) -> Option<u64>,
}

impl AssignShiftOps {
	pub(super) fn from_op(op: InstShiftOp) -> &'static AssignShiftOps {
		match op {
			InstShiftOp::Shl => &Self {
				protocol: Protocol::SHL_ASSIGN,
				symbol: "<<=",
				error: || VmErrorKind::Overflow,
				i64: i64::checked_shl,
				u64: u64::checked_shl,
			},
			InstShiftOp::Shr => &Self {
				protocol: Protocol::SHR_ASSIGN,
				symbol: ">>=",
				error: || VmErrorKind::Underflow,
				i64: i64::checked_shr,
				u64: u64::checked_shr,
			},
		}
	}
}

pub(super) struct ShiftOps {
	pub(super) protocol: Protocol,
	pub(super) symbol: &'static str,
	pub(super) error: fn() -> VmErrorKind,
	pub(super) i64: fn(i64, u32) -> Option<i64>,
	pub(super) u64: fn(u64, u32) -> Option<u64>,
}

impl ShiftOps {
	pub(super) fn from_op(op: InstShiftOp) -> &'static Self {
		match op {
			InstShiftOp::Shl => &Self {
				protocol: Protocol::SHL,
				symbol: "<<",
				error: || VmErrorKind::Overflow,
				i64: i64::checked_shl,
				u64: u64::checked_shl,
			},
			InstShiftOp::Shr => &Self {
				protocol: Protocol::SHR,
				symbol: ">>",
				error: || VmErrorKind::Underflow,
				i64: i64::checked_shr,
				u64: u64::checked_shr,
			},
		}
	}
}
