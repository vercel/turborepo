use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// A raw trace line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ty")]
pub enum TraceRow<'a> {
    #[serde(rename = "B")]
    Start {
        ts: u64,
        id: u64,
        #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        #[serde(rename = "n")]
        name: &'a str,
        #[serde(rename = "t")]
        target: &'a str,
        #[serde(rename = "v", default, skip_serializing_if = "Vec::is_empty")]
        values: Vec<(String, TraceValue<'a>)>,
    },
    #[serde(rename = "E")]
    End { ts: u64, id: u64 },
    #[serde(rename = "b")]
    Enter {
        ts: u64,
        id: u64,
        #[serde(rename = "t")]
        thread_id: u64,
    },
    #[serde(rename = "e")]
    Exit { ts: u64, id: u64 },
    #[serde(rename = "i")]
    Event {
        ts: u64,
        #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        #[serde(rename = "v", default, skip_serializing_if = "Vec::is_empty")]
        values: Vec<(String, TraceValue<'a>)>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TraceValue<'a> {
    String(#[serde(borrow)] Cow<'a, str>),
    Bool(bool),
    UInt(u64),
    Int(i64),
    Float(f64),
}
