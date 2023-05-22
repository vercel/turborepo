use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
};

use serde::{Deserialize, Serialize};

/// A raw trace line.
#[derive(Debug, Serialize, Deserialize)]
pub enum TraceRow<'a> {
    /// A new span has been started, but not entered yet.
    Start {
        ts: u64,
        id: u64,
        parent: Option<u64>,
        name: &'a str,
        target: &'a str,
        #[serde(borrow)]
        values: Vec<(Cow<'a, str>, TraceValue<'a>)>,
    },
    /// A span has ended. The id might be reused in future.
    End { ts: u64, id: u64 },
    /// A span has been entered. This means it is spending CPU time now.
    Enter { ts: u64, id: u64, thread_id: u64 },
    /// A span has been exited. This means it is not spending CPU time anymore.
    Exit { ts: u64, id: u64 },
    /// A event has happened for some span.
    Event {
        ts: u64,
        parent: Option<u64>,
        values: Vec<(Cow<'a, str>, TraceValue<'a>)>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TraceValue<'a> {
    String(#[serde(borrow)] Cow<'a, str>),
    Bool(bool),
    UInt(u64),
    Int(i64),
    Float(f64),
}

impl Display for TraceValue<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceValue::String(s) => write!(f, "{}", s),
            TraceValue::Bool(b) => write!(f, "{}", b),
            TraceValue::UInt(u) => write!(f, "{}", u),
            TraceValue::Int(i) => write!(f, "{}", i),
            TraceValue::Float(fl) => write!(f, "{}", fl),
        }
    }
}

impl<'a> TraceValue<'a> {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            TraceValue::UInt(u) => Some(*u),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            TraceValue::String(s) => Some(s),
            _ => None,
        }
    }
}
