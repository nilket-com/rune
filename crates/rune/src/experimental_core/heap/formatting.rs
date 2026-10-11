//! Independent bounded value rendering. No legacy values or formatting protocols.
//! Count and validate first, then allocate one bounded output buffer. The shared
//! borrow of the store prevents mutation between the two passes.
use super::{Payload, Store};
use crate::alloc::String;
use crate::experimental_core::{
    error::{Error, ErrorKind, Result},
    value::{Type, Value},
};
use core::fmt::{self, Write};

const MAX_BYTES: usize = 64 << 10;
const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 4096;
struct Sink {
    text: Option<String>,
    bytes: usize,
    nodes: usize,
    failure: Option<Error>,
}
impl Sink {
    fn push(&mut self, text: &str) -> Result<()> {
        let bytes = self.bytes.checked_add(text.len()).ok_or_else(limit)?;
        if bytes > MAX_BYTES {
            return Err(limit());
        }
        if let Some(output) = &mut self.text {
            output.try_push_str(text)?;
        }
        self.bytes = bytes;
        Ok(())
    }
    fn formatted(&mut self, value: fmt::Arguments<'_>) -> Result<()> {
        self.write_fmt(value).map_err(|_| {
            self.failure
                .unwrap_or_else(|| Error::new(ErrorKind::Type, "format value"))
        })
    }
}
impl Write for Sink {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push(text).map_err(|error| {
            self.failure = Some(error);
            fmt::Error
        })
    }
}
fn limit() -> Error {
    Error::new(ErrorKind::FormatLimit, "format bytes/depth/nodes")
}
fn unsupported() -> Error {
    Error::new(ErrorKind::Unsupported, "value formatting form")
}
impl Store {
    /// Display/debug on the admitted slice. Nominal struct names are not owned
    /// yet, so nominal debug is explicitly refused rather than guessed.
    pub(crate) fn format_value(&self, value: Value, debug: bool) -> Result<String> {
        self.format_parts(core::iter::once(Ok((value, debug))))
    }
    /// The descriptor iterator is immutable and repeatable; no proportional
    /// metadata copy is needed. All source operands were evaluated before this.
    pub(crate) fn format_parts<I>(&self, parts: I) -> Result<String>
    where
        I: Iterator<Item = Result<(Value, bool)>> + Clone,
    {
        if self.is_closed() {
            return Err(Error::new(ErrorKind::ClosedStore, "format value"));
        }
        let mut sink = Sink {
            text: None,
            bytes: 0,
            nodes: 0,
            failure: None,
        };
        for part in parts.clone() {
            let (value, debug) = part?;
            self.render(value, debug, 0, &mut sink)?;
        }
        let size = sink.bytes;
        let output = String::try_with_capacity(size)?;
        // Allocator rounding is also subject to the per-value temporary cap.
        if output.capacity() > MAX_BYTES {
            return Err(limit());
        }
        sink = Sink {
            text: Some(output),
            bytes: 0,
            nodes: 0,
            failure: None,
        };
        for part in parts {
            let (value, debug) = part?;
            self.render(value, debug, 0, &mut sink)?;
        }
        if sink.bytes != size {
            return Err(Error::new(ErrorKind::InvalidCode, "format two-pass size"));
        }
        sink.text.ok_or_else(unsupported)
    }
    fn render(&self, value: Value, debug: bool, depth: usize, sink: &mut Sink) -> Result<()> {
        if depth > MAX_DEPTH || sink.nodes == MAX_NODES {
            return Err(limit());
        }
        sink.nodes += 1;
        match value {
            Value::Unit if debug => sink.push("()"),
            Value::Unit => Err(Error::new(ErrorKind::Type, "missing display form")),
            Value::Bool(n) => sink.push(if n { "true" } else { "false" }),
            Value::I64(n) => sink.formatted(format_args!("{n}")),
            Value::U64(n) => sink.formatted(format_args!("{n}")),
            Value::F64(n) => {
                // The pinned reference uses this same scalar-only, stack-buffer
                // algorithm. No old Value or runtime formatter is involved.
                let mut buffer = ryu::Buffer::new();
                sink.push(buffer.format(n))
            }
            Value::Char(c) if debug => sink.formatted(format_args!("{c:?}")),
            Value::Char(c) => {
                let mut bytes = [0; 4];
                sink.push(c.encode_utf8(&mut bytes))
            }
            Value::Heap(handle) => match self.payload(handle)? {
                Payload::String(text) if debug => {
                    sink.formatted(format_args!("{:?}", text.as_str()))
                }
                Payload::String(text) => sink.push(text),
                Payload::List { tag, values } if debug => {
                    let (open, close) = match tag {
                        Type::Vec => ("[", "]"),
                        Type::Tuple => ("(", ")"),
                        _ => return Err(unsupported()),
                    };
                    sink.push(open)?;
                    for (i, value) in values.iter().enumerate() {
                        if i != 0 {
                            sink.push(", ")?;
                        }
                        self.render(*value, true, depth + 1, sink)?;
                    }
                    // Rune's one-element tuple has no trailing comma.
                    sink.push(close)
                }
                Payload::Record {
                    tag: Type::Object,
                    fields,
                } if debug => {
                    sink.push("{")?;
                    for (i, (key, value)) in fields.iter().enumerate() {
                        if i != 0 {
                            sink.push(", ")?;
                        }
                        sink.formatted(format_args!("{:?}: ", key.as_str()))?;
                        self.render(*value, true, depth + 1, sink)?;
                    }
                    sink.push("}")
                }
                Payload::Variant {
                    tag,
                    success,
                    payload,
                } if debug => {
                    let name = match (tag, success) {
                        (Type::Option, true) => "Some",
                        (Type::Option, false) => "None",
                        (Type::Result, true) => "Ok",
                        (Type::Result, false) => "Err",
                        _ => return Err(unsupported()),
                    };
                    sink.push(name)?;
                    if let Some(value) = payload {
                        sink.push("(")?;
                        self.render(*value, true, depth + 1, sink)?;
                        sink.push(")")?;
                    }
                    Ok(())
                }
                Payload::List {
                    tag: Type::Vec | Type::Tuple,
                    ..
                }
                | Payload::Record {
                    tag: Type::Object, ..
                }
                | Payload::Variant {
                    tag: Type::Option | Type::Result,
                    ..
                } if !debug => Err(Error::new(ErrorKind::Type, "missing display form")),
                _ => Err(unsupported()),
            },
            _ => Err(unsupported()),
        }
    }
}
