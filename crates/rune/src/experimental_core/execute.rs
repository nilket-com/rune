//! The fresh-core executor: an explicit-frame interpreter over a compiled
//! [`Program`]. It uses no legacy `Value`, `Vm` or protocol caller.
//!
//! # Output
//!
//! `println!` writes each line to the locked process stdout and flushes it at once, so
//! output written before a later run-time error is kept. Output is capped at 128 KiB per
//! invocation: the cap is checked before each write, and a line that would exceed it is
//! not written and fails the invocation with [`ErrorKind::FormatLimit`]. The cap and the
//! per-line flush are declared differences from the stock `println`, which has neither.
use super::{
    arithmetic,
    code::{Code, Op},
    error::{Error, ErrorKind, Location, Result},
    frontend::Program,
    heap::Store,
    value::{Root, Value},
};
use crate::alloc::Vec;
const MAX_FRAMES: usize = 1024;
/// Entry arguments: host roots (owner-checked in `enter`, read in place) or
/// crate-internal values.
#[derive(Clone, Copy)]
enum Args<'a> {
    Roots(&'a [&'a Root]),
    #[cfg_attr(not(test), allow(dead_code))]
    Values(&'a [Value]),
}
impl Args<'_> {
    fn len(self) -> usize {
        match self {
            Self::Roots(roots) => roots.len(),
            Self::Values(values) => values.len(),
        }
    }
    fn get(self, i: usize) -> Value {
        match self {
            Self::Roots(roots) => roots[i].value,
            Self::Values(values) => values[i],
        }
    }
}
const MAX_SLOTS: usize = 65_536;
/// D14: a frame records only its callsite's pc in the caller; the Location is
/// derived on failure from the frame below it. `u32::MAX` is never a valid pc:
/// the Builder assigns pcs with `u32::try_from` and caps a function at
/// `MAX_CODE_BYTES / size_of::<Instruction>()` (1,048,576) instructions.
const NO_CALLSITE: u32 = u32::MAX;
#[derive(Clone, Copy)]
struct Frame {
    function: u32,
    /// The pc of the Call in the caller (the frame directly below), or
    /// NO_CALLSITE for the entry frame.
    callsite_pc: u32,
    pc: usize,
    base: usize,
    out: Option<usize>,
}
/// A failed invocation. Plain source diagnostics only: this record cannot root a
/// script heap.
#[derive(Debug)]
pub struct Failure {
    /// The error that stopped the invocation.
    pub error: Error,
    /// The callsite locations of the active calls, innermost caller first; empty when the
    /// failure happened in the entry function or before it ran.
    pub call_chain: Vec<Location>,
    /// A second error raised while unwinding the invocation's frames, if any.
    pub cleanup_error: Option<Error>,
}

/// Runs functions of a [`Program`] against a [`Store`].
///
/// One executor can be invoked any number of times; it is left clean after success, a
/// run-time error, a fuel halt or a cancellation.
pub struct Executor {
    frames: Vec<Frame>,
    slots: Vec<Value>,
    diagnostics: Vec<Location>,
    cancelled: bool,
    output_bytes: usize,
    /// Test-only, per executor: when set, `Print` appends here instead of stdout.
    #[cfg(test)]
    pub(crate) test_output: Option<std::vec::Vec<u8>>,
}
fn error(kind: ErrorKind) -> Error {
    Error::new(kind, "execute")
}
// `Failure` carries a located error and its call chain by value; it is the public error
// type of `invoke`, so boxing it would change the API.
#[allow(clippy::result_large_err)]
impl Executor {
    /// A new executor, with its frame, slot and call-chain buffers reserved up front.
    pub fn new() -> Result<Self> {
        Ok(Self {
            frames: Vec::try_with_capacity(MAX_FRAMES)?,
            slots: Vec::try_with_capacity(MAX_SLOTS)?,
            diagnostics: Vec::try_with_capacity(MAX_FRAMES)?,
            cancelled: false,
            output_bytes: 0,
            #[cfg(test)]
            test_output: None,
        })
    }
    /// Requests cancellation. The next charged instruction of this executor consumes the
    /// request and fails its invocation with [`ErrorKind::Cancelled`]; a refusal before
    /// the first charged instruction leaves the request pending.
    pub fn request_cancel(&mut self) {
        self.cancelled = true;
    }
    /// Single-shot entry: one invocation on an executor that is then dropped.
    /// Literally `invoke`; there is no separate execution path.
    pub fn run(
        mut self,
        program: &Program,
        store: &mut Store,
        function: u32,
        args: &[&Root],
        fuel: usize,
    ) -> core::result::Result<Root, Failure> {
        self.invoke(program, store, function, args, fuel)
    }
    /// One invocation on a prepared code object and store. The function, its
    /// arity and then every argument root (closed-store policy, store,
    /// generation) are checked before any argument is retained: a refused
    /// argument retains nothing. Argument validation and transfer use no
    /// proportional scratch: arguments go straight from roots into the frame.
    /// Invocation setup can still reserve diagnostics after a previous failure. The returned root owns the
    /// frame's single return reference. The executor is left clean and may be
    /// invoked again after success, a run-time error, a fuel halt or
    /// cancellation; a pending cancellation survives a refusal before the first
    /// charged instruction and only that instruction consumes it.
    pub fn invoke(
        &mut self,
        program: &Program,
        store: &mut Store,
        function: u32,
        args: &[&Root],
        fuel: usize,
    ) -> core::result::Result<Root, Failure> {
        self.invoke_code(&program.code, store, function, args, fuel)
    }
    /// Crate-internal entry over hand-built `Code` (tests); same checked path.
    pub(crate) fn invoke_code(
        &mut self,
        code: &Code,
        store: &mut Store,
        function: u32,
        args: &[&Root],
        fuel: usize,
    ) -> core::result::Result<Root, Failure> {
        let value = self.invoke_args(code, store, function, Args::Roots(args), fuel)?;
        Ok(store.export(value))
    }
    // D21-derived execution logic preserved during extraction: the loops read fixed
    // (slot, flag) pairs from the argument table.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn invoke_args(
        &mut self,
        code: &Code,
        store: &mut Store,
        function: u32,
        args: Args<'_>,
        mut fuel: usize,
    ) -> core::result::Result<Value, Failure> {
        // Per-invocation setup, before any instruction runs: the output cap and the
        // call-chain buffer (moved out by a previous failure; reserving it again is
        // the only allocation here and a failure leaves nothing to clean up).
        self.output_bytes = 0;
        if self.diagnostics.capacity() < MAX_FRAMES {
            if let Err(e) = self.diagnostics.try_reserve(MAX_FRAMES) {
                return Err(Failure {
                    error: Error::from(e),
                    call_chain: Vec::new(),
                    cleanup_error: None,
                });
            }
        }
        let result = (|| {
            if store.is_closed() {
                return Err(error(ErrorKind::ClosedStore));
            }
            if !self.frames.is_empty() || !self.slots.is_empty() {
                return Err(error(ErrorKind::InvalidCode));
            }
            self.enter(code, store, function, args, None)?;
            'frames: loop {
                let frame = *self
                    .frames
                    .last()
                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                let function = code
                    .function(frame.function)
                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                // Frame metadata and the function are stable until call or return.
                // No borrowed frame/slot reference crosses those mutations.
                let frame_index = self.frames.len() - 1;
                let mut pc = frame.pc;
                loop {
                    let source_pc = pc;
                    let instruction = match function.cached_ref(source_pc) {
                        Some(instruction) => instruction,
                        None => {
                            // Validated code cannot reach this, but retain the checked
                            // fetch and publish its pc before the outer cleanup.
                            self.commit_pc(frame_index, pc)?;
                            return Err(error(ErrorKind::InvalidCode));
                        }
                    };
                    let mut transferred = false;
                    let result = (|| {
                        // Charge and cancellation ride an existing instruction, no Charge opcode.
                        if instruction.charged {
                            #[cfg(test)]
                            if cancel_tick_for_test() {
                                self.cancelled = true;
                            }
                            if self.cancelled {
                                // The pending request is consumed here and only here.
                                self.cancelled = false;
                                return Err(error(ErrorKind::Cancelled));
                            }
                            fuel = fuel
                                .checked_sub(1)
                                .ok_or_else(|| error(ErrorKind::FuelExhausted))?;
                        }
                        pc += 1;
                        let get = |n: u32| -> Result<Value> {
                            self.slots
                                .get(frame.base + n as usize)
                                .copied()
                                .ok_or_else(|| error(ErrorKind::InvalidCode))
                        };
                        // D20: operands are bound per arm from the borrowed cached instruction (no
                        // whole-Op temporary). Call and CallValue name their call here; the shared
                        // Call body runs after the match, so CallValue's checks still precede
                        // commit_pc and every retain/reserve/frame publication.
                        let mut call = None;
                        match instruction.op {
                            Op::Template {
                                out,
                                args_start,
                                args_len,
                            } => {
                                let end = args_start
                                    .checked_add(args_len)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                let parts = function
                                    .args
                                    .get(args_start as usize..end as usize)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                // D18: read the parts' slots here instead of capturing `get`; the
                                // lookup and its error are identical, but `get` no longer escapes
                                // into the out-of-line formatter.
                                let base = frame.base;
                                let slots = &self.slots;
                                let text =
                                    store.format_parts(parts.chunks_exact(2).map(|pair| {
                                        let value = slots
                                            .get(base + pair[0] as usize)
                                            .copied()
                                            .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                        Ok((value, pair[1] != 0))
                                    }))?;
                                let value = store.string_owned(text)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::Print { value } => {
                                use std::io::Write;
                                let text = store.text(get(value)?)?;
                                let bytes = self
                                    .output_bytes
                                    .checked_add(text.len())
                                    .and_then(|n| n.checked_add(1))
                                    .filter(|n| *n <= 128 << 10)
                                    .ok_or_else(|| error(ErrorKind::FormatLimit))?;
                                self.output_bytes = bytes;
                                // Test-only seam, scoped to this executor: same cap and order, no stdout.
                                #[cfg(test)]
                                let captured = match self.test_output.as_mut() {
                                    Some(buffer) => {
                                        buffer.extend_from_slice(text.as_bytes());
                                        buffer.push(b'\n');
                                        true
                                    }
                                    None => false,
                                };
                                #[cfg(not(test))]
                                let captured = false;
                                // Real ordered stdout, flushed now rather than returned as a
                                // result recording. Earlier output survives later failures.
                                if !captured {
                                    let stdout = std::io::stdout();
                                    let mut sink = stdout.lock();
                                    sink.write_all(text.as_bytes())
                                        .and_then(|_| sink.write_all(b"\n"))
                                        .and_then(|_| sink.flush())
                                        .map_err(|_| error(ErrorKind::Output))?;
                                }
                            }
                            Op::Variant {
                                out,
                                tag,
                                success,
                                payload,
                            } => {
                                let value =
                                    store.variant(tag, success, payload.map(get).transpose()?)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::Try { out, value } => {
                                let value = get(value)?;
                                let (_, success, payload) = store.variant_parts(value)?;
                                if success {
                                    let payload = payload.ok_or_else(|| error(ErrorKind::Type))?;
                                    store.retain(payload)?;
                                    self.put(store, frame.base + out as usize, payload)?;
                                } else {
                                    store.retain(value)?;
                                    let out = frame.out;
                                    self.commit_pc(frame_index, pc)?;
                                    self.leave(store)?;
                                    transferred = true;
                                    if let Some(out) = out {
                                        self.put(store, out, value)?;
                                    } else {
                                        return Ok(Some(value));
                                    }
                                }
                            }

                            Op::CallValue {
                                out,
                                value,
                                args_start,
                                args_len,
                            } => {
                                let Value::Function {
                                    code: owner,
                                    id: callee,
                                } = get(value)?
                                else {
                                    return Err(error(ErrorKind::Type));
                                };
                                // A token from another program is a type error at the call,
                                // before any argument is retained.
                                if owner != code.id() {
                                    return Err(error(ErrorKind::Type));
                                }
                                let target = code
                                    .function(callee)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                if target.parameters != args_len {
                                    return Err(error(ErrorKind::Type));
                                }
                                call = Some((out, callee, args_start, args_len));
                            }
                            Op::Range { out, start, end } => {
                                let (Value::I64(start), Value::I64(end)) = (get(start)?, get(end)?)
                                else {
                                    return Err(error(ErrorKind::Type));
                                };
                                let value = store.range(start, end)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::IterStart { out, value } => {
                                let value = store.iterator(get(value)?)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::IterNext {
                                out,
                                iterator,
                                none,
                            } => {
                                if let Some(value) = store.iterator_next(get(iterator)?)? {
                                    self.put(store, frame.base + out as usize, value)?;
                                } else {
                                    pc = none as usize;
                                }
                            }

                            Op::String { out, text } => {
                                let v = store.string(
                                    function
                                        .strings
                                        .get(text as usize)
                                        .ok_or_else(|| error(ErrorKind::InvalidCode))?,
                                )?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Clone { out, value } => {
                                let tag = store.tag(get(value)?)?;
                                if !matches!(
                                    tag,
                                    super::value::Type::Vec
                                        | super::value::Type::Tuple
                                        | super::value::Type::Object
                                        | super::value::Type::Struct(_)
                                        | super::value::Type::String
                                        | super::value::Type::Unit
                                        | super::value::Type::I64
                                        | super::value::Type::U64
                                        | super::value::Type::F64
                                        | super::value::Type::Bool
                                        | super::value::Type::Char
                                ) {
                                    return Err(error(ErrorKind::Type));
                                }

                                let v = store.clone_outer(get(value)?)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::AssignBinary { out, op, lhs, rhs } => {
                                // D21: borrow both operand slots (lhs then rhs, the same
                                // InvalidCode order as `get`); two I64 values go to fast_i64
                                // using their borrowed payloads, every other pair is copied and
                                // takes the unchanged fallback. Both borrows end before `put`.
                                let base = frame.base;
                                let slots = &self.slots;
                                let left = slots
                                    .get(base + lhs as usize)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                let right = slots
                                    .get(base + rhs as usize)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                let fast = match (left, right) {
                                    (&Value::I64(a), &Value::I64(b)) => {
                                        fast_i64(op, Value::I64(a), Value::I64(b))
                                    }
                                    _ => None,
                                };
                                let v = if let Some(result) = fast {
                                    result?
                                } else {
                                    let (a, b) = (*left, *right);
                                    if op == arithmetic::Binary::Add
                                        && store.tag(a)? == super::value::Type::String
                                    {
                                        store.append_string(a, b)?;
                                        store.retain(a)?;
                                        a
                                    } else {
                                        binary(store, op, a, b)?
                                    }
                                };
                                self.put(store, base + out as usize, v)?;
                            }

                            Op::Literal { out, value } => {
                                self.put(store, frame.base + out as usize, value)?
                            }
                            Op::Copy { out, value } => {
                                let v = get(value)?;
                                store.retain(v)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Release { value } => {
                                self.put(store, frame.base + value as usize, Value::Unit)?
                            }
                            Op::Unary { out, op, value } => {
                                let v = arithmetic::unary(op, get(value)?)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Binary { out, op, lhs, rhs } => {
                                // D21: as AssignBinary, without the string append arm.
                                let base = frame.base;
                                let slots = &self.slots;
                                let left = slots
                                    .get(base + lhs as usize)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                let right = slots
                                    .get(base + rhs as usize)
                                    .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                                let fast = match (left, right) {
                                    (&Value::I64(a), &Value::I64(b)) => {
                                        fast_i64(op, Value::I64(a), Value::I64(b))
                                    }
                                    _ => None,
                                };
                                let v = if let Some(result) = fast {
                                    result?
                                } else {
                                    binary(store, op, *left, *right)?
                                };
                                self.put(store, base + out as usize, v)?;
                            }
                            Op::Cast { out, value, tag } => {
                                let v = arithmetic::cast(get(value)?, tag)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Is {
                                out,
                                value,
                                tag,
                                negate,
                            } => {
                                let operand = get(value)?;
                                let matches = match operand {
                                    Value::Function { .. } | Value::Type(_) => false,
                                    _ => store.tag(operand)? == tag,
                                };
                                let v = Value::Bool(matches ^ negate);
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Jump { target } => pc = target as usize,
                            Op::Branch { condition, yes, no } => {
                                let Value::Bool(condition) = get(condition)? else {
                                    return Err(error(ErrorKind::Type));
                                };
                                pc = if condition { yes as usize } else { no as usize };
                            }
                            Op::Call {
                                out,
                                function: callee,
                                args_start,
                                args_len,
                            } => call = Some((out, callee, args_start, args_len)),
                            Op::Return { value } => {
                                let value = get(value)?;
                                store.retain(value)?;
                                let out = frame.out;
                                self.commit_pc(frame_index, pc)?;
                                self.leave(store)?;
                                transferred = true;
                                if let Some(out) = out {
                                    self.put(store, out, value)?;
                                } else {
                                    return Ok(Some(value));
                                }
                            }
                            Op::Record {
                                out,
                                tag,
                                args_start,
                                args_len,
                            } => {
                                let descriptors = &function.args
                                    [args_start as usize..(args_start + args_len) as usize];
                                let mut fields = Vec::try_with_capacity(descriptors.len() / 2)?;
                                for pair in descriptors.chunks_exact(2) {
                                    fields.try_push((
                                        function.strings[pair[0] as usize].as_str(),
                                        get(pair[1])?,
                                    ))?;
                                }
                                let value = store.record(tag, &fields)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::Field { out, value, key } => {
                                let value = store
                                    .field(get(value)?, function.strings[key as usize].as_str())?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::SetField { value, key, new } => store.set_field(
                                get(value)?,
                                function.strings[key as usize].as_str(),
                                get(new)?,
                            )?,
                            Op::List {
                                out,
                                tag,
                                args_start,
                                args_len,
                            } => {
                                // No temporary proportional argument allocation: the slots themselves
                                // are not necessarily contiguous, so reserve only the bounded list.
                                let indices = &function.args
                                    [args_start as usize..args_start as usize + args_len as usize];
                                let mut values = Vec::try_with_capacity(indices.len())?;
                                for &i in indices {
                                    values.try_push(get(i)?)?;
                                }
                                let value = store.list(tag, &values)?;
                                self.put(store, frame.base + out as usize, value)?;
                            }
                            Op::TupleField { out, value, index } => {
                                if store.tag(get(value)?)? != super::value::Type::Tuple {
                                    return Err(error(ErrorKind::Type));
                                }
                                let v = store.get(get(value)?, index_value(get(index)?)?)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::SetTupleField { value, index, new } => {
                                if store.tag(get(value)?)? != super::value::Type::Tuple {
                                    return Err(error(ErrorKind::Type));
                                }
                                store.set(get(value)?, index_value(get(index)?)?, get(new)?)?;
                            }
                            Op::Get { out, value, index } => {
                                if !matches!(
                                    store.tag(get(value)?)?,
                                    super::value::Type::Vec | super::value::Type::Tuple
                                ) {
                                    return Err(error(ErrorKind::Type));
                                }

                                let i = index_value(get(index)?)?;
                                let v = store.get(get(value)?, i)?;
                                self.put(store, frame.base + out as usize, v)?;
                            }
                            Op::Set { value, index, new } => {
                                store.set(get(value)?, index_value(get(index)?)?, get(new)?)?
                            }
                            Op::Len { out, value } => {
                                if !matches!(
                                    store.tag(get(value)?)?,
                                    super::value::Type::Vec
                                        | super::value::Type::Tuple
                                        | super::value::Type::String
                                ) {
                                    return Err(error(ErrorKind::Type));
                                }

                                let n = u64::try_from(store.len(get(value)?)?)
                                    .map_err(|_| error(ErrorKind::Overflow))?;
                                self.put(store, frame.base + out as usize, Value::U64(n))?;
                            }
                            Op::Push { value, new } => store.push(get(value)?, get(new)?)?,
                            Op::Drop { value } => store.invalidate(get(value)?)?,
                        }
                        if let Some((out, callee, args_start, args_len)) = call {
                            self.commit_pc(frame_index, pc)?;
                            let callee_code = code
                                .function(callee)
                                .ok_or_else(|| error(ErrorKind::InvalidCode))?;
                            let source_args = &function.args
                                [args_start as usize..args_start as usize + args_len as usize];
                            let new_base = self.reserve_frame(callee_code.slots as usize)?;
                            for (i, &slot) in source_args.iter().enumerate() {
                                let value = self.slots[frame.base + slot as usize];
                                store.retain(value)?;
                                self.slots[new_base + i] = value;
                            }
                            self.frames.try_push(Frame {
                                function: callee,
                                pc: 0,
                                base: new_base,
                                out: Some(frame.base + out as usize),
                                // Lossless: source_pc < 1,048,576 by the Builder's bound.
                                callsite_pc: source_pc as u32,
                            })?;
                            transferred = true;
                        }
                        Ok(None)
                    })();
                    match result {
                        Ok(Some(v)) => return Ok(v),
                        Ok(None) if transferred => continue 'frames,
                        Ok(None) => (),
                        Err(e) => {
                            // A failed return can already have popped this frame.
                            // Never overwrite the parent's pc in that case. A failed
                            // call can have reserved slots without publishing a frame.
                            if let Some(current) = self.frames.get_mut(frame_index) {
                                current.pc = pc;
                            }
                            return Err(e.at(function.locations[source_pc]));
                        }
                    }
                }
            }
        })();
        match result {
            Ok(value) => Ok(value),
            Err(failure) => {
                let mut cleanup_error = None;
                // Innermost first. Only Op::Call sets a callsite, pushing the callee
                // directly above its caller; the entry frame has none, so pairing each
                // frame with the one below it visits exactly the frames that have one.
                for pair in self.frames.windows(2).rev() {
                    let (caller, frame) = (pair[0], pair[1]);
                    if frame.callsite_pc != NO_CALLSITE {
                        let location = code
                            .function(caller.function)
                            .expect("a published caller frame names a validated function")
                            .locations[frame.callsite_pc as usize];
                        if self.diagnostics.try_push(location).is_err() {
                            break;
                        }
                    }
                }
                // Includes a partially filled callee reserve if retaining an arg failed.
                while let Some(value) = self.slots.pop() {
                    if let Err(e) = store.release(value) {
                        cleanup_error = Some(e);
                    }
                }
                self.frames.clear();
                Err(Failure {
                    error: failure,
                    call_chain: core::mem::take(&mut self.diagnostics),
                    cleanup_error,
                })
            }
        }
    }
    /// Clean-state check for callers that repeat invocations: true when no
    /// frame, slot or pending diagnostic survives the last invocation.
    pub fn is_idle(&self) -> bool {
        self.frames.is_empty() && self.slots.is_empty() && self.diagnostics.is_empty()
    }
    fn reserve_frame(&mut self, slots: usize) -> Result<usize> {
        if self.frames.len() == MAX_FRAMES {
            return Err(error(ErrorKind::FrameCapacity));
        }
        let base = self.slots.len();
        let len = base
            .checked_add(slots)
            .ok_or_else(|| error(ErrorKind::Overflow))?;
        if len > MAX_SLOTS {
            return Err(error(ErrorKind::FrameCapacity));
        }
        self.slots.try_resize_with(len, || Value::Unit)?;
        Ok(base)
    }
    fn enter(
        &mut self,
        code: &Code,
        store: &mut Store,
        function: u32,
        args: Args<'_>,
        out: Option<usize>,
    ) -> Result<()> {
        let f = code
            .function(function)
            .ok_or_else(|| error(ErrorKind::InvalidCode))?;
        if args.len() != f.parameters as usize {
            return Err(error(ErrorKind::InvalidCode));
        }
        // Host roots: every one owner-checked before any retain, no allocation.
        if let Args::Roots(roots) = args {
            store.import_all(roots.iter().copied())?;
        }
        let base = self.reserve_frame(f.slots as usize)?;
        for i in 0..args.len() {
            let value = args.get(i);
            store.retain(value)?;
            self.slots[base + i] = value;
        }
        self.frames.try_push(Frame {
            function,
            pc: 0,
            base,
            out,
            callsite_pc: NO_CALLSITE,
        })?;
        Ok(())
    }
    /// Publish only into the frame whose local pc is being executed.
    fn commit_pc(&mut self, frame_index: usize, pc: usize) -> Result<()> {
        self.frames
            .get_mut(frame_index)
            .ok_or_else(|| error(ErrorKind::InvalidCode))?
            .pc = pc;
        Ok(())
    }
    #[inline(always)]
    fn put(&mut self, store: &mut Store, index: usize, value: Value) -> Result<()> {
        let slot = self
            .slots
            .get_mut(index)
            .ok_or_else(|| error(ErrorKind::InvalidCode))?;
        let old = core::mem::replace(slot, value);
        store.release_slot(old)
    }
    fn leave(&mut self, store: &mut Store) -> Result<()> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| error(ErrorKind::InvalidCode))?;
        // D13: an all-scalar window in an open store with no queued work would
        // release nothing slot by slot; drop it in one step. Otherwise (any
        // heap value, closed store, queued work, base beyond len) the D4 loop.
        if store.scalar_release_is_noop()
            && self
                .slots
                .get(frame.base..)
                .is_some_and(|window| window.iter().all(Store::is_slot_scalar))
        {
            #[cfg(test)]
            LEAVE_FAST_HITS.with(|c| c.set(c.get() + 1));
            self.slots.truncate(frame.base);
            return Ok(());
        }
        while self.slots.len() > frame.base {
            store.release_slot(
                self.slots
                    .pop()
                    .ok_or_else(|| error(ErrorKind::InvalidCode))?,
            )?;
        }
        Ok(())
    }
}
fn index_value(value: Value) -> Result<usize> {
    match value {
        Value::I64(n) => usize::try_from(n).map_err(|_| error(ErrorKind::IndexOutOfBounds)),
        Value::U64(n) => usize::try_from(n).map_err(|_| error(ErrorKind::IndexOutOfBounds)),
        _ => Err(error(ErrorKind::Type)),
    }
}

/// Exact signed-integer cases only; all other values/operations keep the
/// original string/coercion dispatch. No store access before result publication.
#[inline(always)]
fn fast_i64(op: arithmetic::Binary, a: Value, b: Value) -> Option<Result<Value>> {
    use arithmetic::Binary;
    let (Value::I64(a), Value::I64(b)) = (a, b) else {
        return None;
    };
    Some(match op {
        Binary::Add => a
            .checked_add(b)
            .map(Value::I64)
            .ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
        Binary::Sub => a
            .checked_sub(b)
            .map(Value::I64)
            .ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
        Binary::Mul => a
            .checked_mul(b)
            .map(Value::I64)
            .ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
        Binary::Div | Binary::Rem if b == 0 => {
            Err(Error::new(ErrorKind::DivisionByZero, "primitive operation"))
        }
        Binary::Div => a
            .checked_div(b)
            .map(Value::I64)
            .ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
        Binary::Rem => a
            .checked_rem(b)
            .map(Value::I64)
            .ok_or_else(|| Error::new(ErrorKind::Overflow, "primitive operation")),
        Binary::Eq => Ok(Value::Bool(a == b)),
        Binary::Ne => Ok(Value::Bool(a != b)),
        Binary::Lt => Ok(Value::Bool(a < b)),
        Binary::Le => Ok(Value::Bool(a <= b)),
        Binary::Gt => Ok(Value::Bool(a > b)),
        Binary::Ge => Ok(Value::Bool(a >= b)),
        Binary::BitAnd | Binary::BitOr | Binary::BitXor | Binary::Shl | Binary::Shr => return None,
    })
}

// D21-derived execution logic preserved during extraction: the string-pair branch keeps
// its heap-tag checks separate from the outer heap-pair test.
#[allow(clippy::collapsible_if)]
fn binary(store: &mut Store, op: arithmetic::Binary, a: Value, b: Value) -> Result<Value> {
    if matches!((a, b), (Value::Heap(_), Value::Heap(_))) {
        if store.tag(a)? == super::value::Type::String
            && store.tag(b)? == super::value::Type::String
        {
            if op == arithmetic::Binary::Add {
                return store.concat_string(a, b);
            }
            let left = store.text(a)?;
            let right = store.text(b)?;
            return arithmetic::compare(op, left == right, left < right, left > right);
        }
    }
    arithmetic::binary(op, a, b)
}
#[cfg(test)]
// `Failure` carries a located error and its call chain by value; it is the public error
// type of `invoke`, so boxing it would change the API.
#[allow(clippy::result_large_err)]
impl Executor {
    /// (frames, slots, diagnostics) capacities: reserved once at construction.
    pub(crate) fn capacities_for_test(&self) -> (usize, usize, usize) {
        (
            self.frames.capacity(),
            self.slots.capacity(),
            self.diagnostics.capacity(),
        )
    }
    /// Byte sizes of the three construction reservations, in reservation order.
    pub(crate) fn reservation_bytes_for_test() -> [usize; 3] {
        [
            MAX_FRAMES * core::mem::size_of::<Frame>(),
            MAX_SLOTS * core::mem::size_of::<Value>(),
            MAX_FRAMES * core::mem::size_of::<Location>(),
        ]
    }
    /// Owner-free single-shot entry for the crate's own tests.
    pub(crate) fn run_values(
        mut self,
        code: &Code,
        store: &mut Store,
        function: u32,
        args: &[Value],
        fuel: usize,
    ) -> core::result::Result<Value, Failure> {
        self.invoke_args(code, store, function, Args::Values(args), fuel)
    }
}

#[cfg(test)]
mod scalar_release_tests;

#[cfg(test)]
mod i64_fast_tests;

#[cfg(test)]
mod unit_retain_tests;

#[cfg(test)]
std::thread_local! {
    /// D13: per-thread count of leaves that took the bulk scalar fast path.
    static LEAVE_FAST_HITS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}
#[cfg(test)]
pub(crate) fn leave_fast_hits_for_test() -> usize {
    LEAVE_FAST_HITS.with(|c| c.get())
}

#[cfg(test)]
std::thread_local! {
    /// D14 test-only: request cancellation at the Nth charged instruction of
    /// this thread's next invocation(s), so a nested charged frame can be
    /// cancelled; there is no other way to cancel mid-invocation.
    static CANCEL_AT_CHARGE: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn cancel_at_charge_for_test(n: Option<usize>) {
    CANCEL_AT_CHARGE.with(|c| c.set(n));
}
#[cfg(test)]
fn cancel_tick_for_test() -> bool {
    CANCEL_AT_CHARGE.with(|c| match c.get() {
        Some(0) => {
            c.set(None);
            true
        }
        Some(n) => {
            c.set(Some(n - 1));
            false
        }
        None => false,
    })
}

#[cfg(test)]
mod leave_fast_tests;

#[cfg(test)]
mod lazy_callsite_tests;

#[cfg(test)]
mod template_tests;

#[cfg(test)]
mod call_value_tests;

#[cfg(test)]
mod binary_peek_tests;
