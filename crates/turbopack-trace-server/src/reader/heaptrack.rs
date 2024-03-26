use std::{collections::HashSet, str::from_utf8, sync::Arc};

use anyhow::{bail, Context, Result};
use rustc_demangle::demangle;

use super::TraceFormat;
use crate::{span::SpanIndex, store_container::StoreContainer};

#[derive(Debug)]
struct TraceNode {
    ip_index: usize,
    parent_index: usize,
}

impl TraceNode {
    pub fn read(s: &mut &[u8]) -> Result<Self> {
        Ok(Self {
            ip_index: read_hex_index(s)?,
            parent_index: read_hex_index(s)?,
        })
    }
}

#[derive(Debug)]
struct InstructionPointer {
    module_index: usize,
    frames: Vec<Frame>,
}

impl InstructionPointer {
    pub fn read(s: &mut &[u8]) -> Result<Self> {
        let _ip = read_hex(s)?;
        Ok(Self {
            module_index: read_hex_index(s)?,
            frames: read_all(s, |s| Frame::read(s))?,
        })
    }
}

#[derive(Debug)]
struct Frame {
    function_index: usize,
    file_index: usize,
    line: u64,
}

impl Frame {
    pub fn read(s: &mut &[u8]) -> Result<Self> {
        Ok(Self {
            function_index: read_hex_index(s)?,
            file_index: read_hex_index(s)?,
            line: read_hex(s)?,
        })
    }
}

#[derive(Debug)]
struct AllocationInfo {
    size: u64,
    trace_index: usize,
}

impl AllocationInfo {
    pub fn read(s: &mut &[u8]) -> Result<Self> {
        Ok(Self {
            size: read_hex(s)?,
            trace_index: read_hex_index(s)?,
        })
    }
}

pub struct HeaptrackFormat {
    store: Arc<StoreContainer>,
    version: u32,
    last_timestamp: u64,
    strings: Vec<String>,
    traces: Vec<SpanIndex>,
    instruction_pointers: Vec<InstructionPointer>,
    allocations: Vec<AllocationInfo>,
}

impl HeaptrackFormat {
    pub fn new(store: Arc<StoreContainer>) -> Self {
        Self {
            store,
            version: 0,
            last_timestamp: 0,
            strings: vec!["".to_string()],
            traces: vec![SpanIndex::new(usize::MAX).unwrap()],
            instruction_pointers: vec![InstructionPointer {
                module_index: 0,
                frames: Vec::new(),
            }],
            allocations: vec![AllocationInfo {
                size: 0,
                trace_index: 0,
            }],
        }
    }
}

impl TraceFormat for HeaptrackFormat {
    fn read(&mut self, mut buffer: &[u8]) -> anyhow::Result<usize> {
        let mut bytes_read = 0;
        let mut outdated_spans = HashSet::new();
        let mut store = self.store.write();
        loop {
            let Some(line_end) = buffer.iter().position(|b| *b == b'\n') else {
                break;
            };
            let full_line = &buffer[..line_end];
            buffer = &buffer[line_end + 1..];
            bytes_read += full_line.len() + 1;

            if full_line.is_empty() {
                continue;
            }
            let ty = full_line[0];
            let mut line = &full_line[2..];

            // For format see https://github.com/KDE/heaptrack/blob/b000a73e0bf0a275ec41eef0fe34701a0942cdd8/src/analyze/accumulatedtracedata.cpp#L151
            match ty {
                b'v' => {
                    let _ = read_hex(&mut line)?;
                    self.version = read_hex(&mut line)? as u32;
                    if self.version != 2 && self.version != 3 {
                        bail!("Unsupported version: {} (expected 2 or 3)", self.version);
                    }
                }
                b's' => {
                    let string = if self.version == 2 {
                        String::from_utf8(line.to_vec())?
                    } else {
                        read_sized_string(&mut line)?
                    };
                    self.strings.push(string);
                }
                b't' => {
                    let TraceNode {
                        ip_index,
                        parent_index,
                    } = TraceNode::read(&mut line)?;
                    let parent = if parent_index > 0 {
                        Some(*self.traces.get(parent_index).context("parent not found")?)
                    } else {
                        None
                    };
                    let InstructionPointer {
                        module_index,
                        frames,
                    } = self
                        .instruction_pointers
                        .get(ip_index)
                        .context("ip not found")?;
                    let module = self
                        .strings
                        .get(*module_index)
                        .context("module not found")?;
                    let name = if let Some(first_frame) = frames.first() {
                        let file = self
                            .strings
                            .get(first_frame.file_index)
                            .context("file not found")?;
                        let function = self
                            .strings
                            .get(first_frame.function_index)
                            .context("function not found")?;
                        format!("{} @ {file}:{}", demangle(function), first_frame.line)
                    } else {
                        "unknown".to_string()
                    };
                    let mut args = Vec::new();
                    for Frame {
                        function_index,
                        file_index,
                        line,
                    } in frames.iter()
                    {
                        let file = self.strings.get(*file_index).context("file not found")?;
                        let function = self
                            .strings
                            .get(*function_index)
                            .context("function not found")?;
                        args.push((
                            "location".to_string(),
                            format!("{} @ {file}:{line}", demangle(function)),
                        ));
                    }
                    let span_index = store.add_span(
                        parent,
                        self.last_timestamp,
                        module.to_string(),
                        name,
                        args,
                        &mut outdated_spans,
                    );
                    self.traces.push(span_index);
                }
                b'i' => {
                    let ip = InstructionPointer::read(&mut line)?;
                    self.instruction_pointers.push(ip);
                }
                b'#' => {
                    // comment
                }
                b'X' => {
                    let line = from_utf8(line)?;
                    println!("Debuggee: {line}");
                }
                b'c' => {
                    // timestamp
                    let timestamp = read_hex(&mut line)?;
                    self.last_timestamp = timestamp;
                }
                b'a' => {
                    // allocation info
                    let info = AllocationInfo::read(&mut line)?;
                    self.allocations.push(info);
                }
                b'+' => {
                    // allocation
                    let index = read_hex_index(&mut line)?;
                    let AllocationInfo { size, trace_index } = self
                        .allocations
                        .get(index)
                        .context("allocation not found")?;
                    if *trace_index > 0 {
                        let span_index =
                            self.traces.get(*trace_index).context("trace not found")?;
                        store.add_allocation(*span_index, *size, 1, &mut outdated_spans);
                    }
                }
                b'-' => {
                    // deallocation
                    let index = read_hex_index(&mut line)?;
                    let AllocationInfo { size, trace_index } = self
                        .allocations
                        .get(index)
                        .context("allocation not found")?;
                    if *trace_index > 0 {
                        let span_index =
                            self.traces.get(*trace_index).context("trace not found")?;
                        store.add_deallocation(*span_index, *size, 1, &mut outdated_spans);
                    }
                }
                b'R' => {
                    // RSS timestamp
                }
                b'A' => {
                    // attached
                    // ignore
                }
                b'S' => {
                    // embedded suppression
                    // ignore
                }
                b'I' => {
                    // System info
                    // ignore
                }
                _ => {
                    let line = from_utf8(line)?;
                    println!("{} {line}", ty as char)
                }
            }
        }
        store.invalidate_outdated_spans(&outdated_spans);
        Ok(bytes_read)
    }
}

fn read_hex_index(s: &mut &[u8]) -> anyhow::Result<usize> {
    Ok(read_hex(s)? as usize)
}

fn read_hex(s: &mut &[u8]) -> anyhow::Result<u64> {
    let mut n: u64 = 0;
    loop {
        if let Some(c) = s.get(0) {
            match c {
                b'0'..=b'9' => {
                    n *= 16;
                    n += (*c - b'0') as u64;
                }
                b'a'..=b'f' => {
                    n *= 16;
                    n += (*c - b'a' + 10) as u64;
                }
                b' ' => {
                    *s = &s[1..];
                    return Ok(n);
                }
                _ => {
                    bail!("Expected hex char");
                }
            }
            *s = &s[1..];
        } else {
            return Ok(n);
        }
    }
}

fn read_sized_string(s: &mut &[u8]) -> anyhow::Result<String> {
    let size = read_hex(s)? as usize;
    let str = &s[..size];
    *s = &s[size..];
    Ok(String::from_utf8(str.to_vec())?)
}

fn read_all<T>(
    s: &mut &[u8],
    f: impl Fn(&mut &[u8]) -> anyhow::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let mut res = Vec::new();
    while !s.is_empty() {
        res.push(f(s)?);
    }
    Ok(res)
}
