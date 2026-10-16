//! Cold diagnostic resolution for transparent synthetic sort and Any frames.
use super::*;
impl Executor {
	// The saved frame is passed explicitly: a failed return may have popped it.
	#[cold]
	#[inline(never)]
	fn diagnostic_location(
		&self,
		code: &Code,
		mut index: usize,
		mut frame: Frame,
		mut at: Location,
	) -> Option<Location> {
		while code.function(frame.function)?.is_transparent_sort() {
			if frame.callsite_pc == NO_CALLSITE {
				return None;
			}
			index = index.checked_sub(1)?;
			let caller = *self.frames.get(index)?;
			at = *code
				.function(caller.function)?
				.locations
				.get(frame.callsite_pc as usize)?;
			frame = caller;
		}
		Some(at)
	}
	// The dispatch loop keeps C10's e.at(...) error exits. Only a synthetic
	// primary needs resolution, once, before cleanup destroys the frames.
	#[cold]
	#[inline(never)]
	pub(super) fn resolve_failure_location(&self, code: &Code, error: &mut Error) {
		if error.location != Some(super::super::code::TRANSPARENT_SORT_LOCATION) {
			return;
		}
		error.location = (|| {
			let index = self.frames.len().checked_sub(1)?;
			let frame = *self.frames.get(index)?;
			let function = code.function(frame.function)?;
			if function.is_transparent_sort() {
				return self.diagnostic_location(
					code,
					index,
					frame,
					super::super::code::TRANSPARENT_SORT_LOCATION,
				);
			}
			// A failed Return may have popped the helper. Its caller was
			// committed to the next pc before the ordinary call entered it.
			let call_pc = frame.pc.checked_sub(1)?;
			let instruction = function.cached_ref(call_pc)?;
			let Op::Call {
				function: callee, ..
			} = instruction.op
			else {
				return None;
			};
			if !code.function(callee)?.is_transparent_sort() {
				return None;
			}
			function.locations.get(call_pc).copied()
		})();
	}
	#[cold]
	#[inline(never)]
	pub(super) fn collect_failure_chain(&mut self, code: &Code) {
		for index in (1..self.frames.len()).rev() {
			let frame = self.frames[index];
			// A helper's own caller appears when its user callee resolves the helper.
			// The helper frame itself contributes no duplicate caller window.
			if frame.callsite_pc == NO_CALLSITE
				|| code
					.function(frame.function)
					.is_some_and(|f| f.is_transparent_sort())
			{
				continue;
			}
			let caller = self.frames[index - 1];
			let Some(at) = code
				.function(caller.function)
				.and_then(|f| f.locations.get(frame.callsite_pc as usize))
				.copied()
			else {
				continue;
			};
			let Some(at) = self.diagnostic_location(code, index - 1, caller, at) else {
				continue;
			};
			if self.diagnostics.try_push(at).is_err() {
				break;
			}
		}
	}
}
