//! Parameter-count obligations. A callee's parameter count is known only
//! once it is lowered, so each call that names a script function records the
//! count it passes, and the compile checks every record before it seals code.
use super::super::error::Location;
use super::*;

/// A direct call of a script function whose argument count is a recorded
/// obligation: the only way lowering builds a script [`Op::Call`].
#[must_use]
pub(crate) struct ScriptCall {
	function: u32,
	args_len: u32,
}

impl ScriptCall {
	pub(crate) fn op(self, out: u32, args_start: u32) -> Op {
		Op::Call {
			out,
			function: self.function,
			args_start,
			args_len: self.args_len,
		}
	}
}

impl CompileTransaction {
	/// Records that `function`, called at `at`, must take `args_len`
	/// arguments.
	pub(crate) fn script_call(
		&mut self,
		function: u32,
		args_len: u32,
		at: Location,
	) -> Result<ScriptCall> {
		self.require_parameters(function, args_len, at)?;
		Ok(ScriptCall { function, args_len })
	}

	/// Records that function `id`, called at `at`, must take `parameters`
	/// arguments, captures included.
	pub(crate) fn require_parameters(
		&mut self,
		id: u32,
		parameters: u32,
		at: Location,
	) -> Result<()> {
		self.arities.try_push((id, parameters, at))?;
		Ok(())
	}

	/// The first recorded call whose callee does not take the recorded
	/// count, by where it is called.
	pub(crate) fn wrong_parameters(
		&self,
		parameters: impl Fn(u32) -> Option<u32>,
	) -> Option<Location> {
		self.arities
			.iter()
			.find(|(id, expected, _)| parameters(*id).is_some_and(|n| n != *expected))
			.map(|(_, _, at)| *at)
	}
}
