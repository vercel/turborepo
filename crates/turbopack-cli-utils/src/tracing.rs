use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FullTraceRow<'a> {
    pub ts: u64,
    #[serde(flatten, borrow)]
    pub data: TraceRow<'a>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ty")]
pub enum TraceRow<'a> {
    #[serde(rename = "B")]
    Start {
        id: u64,
        #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        #[serde(rename = "n")]
        name: &'a str,
    },
    #[serde(rename = "E")]
    End { id: u64 },
    #[serde(rename = "b")]
    Enter {
        id: u64,
        #[serde(rename = "t")]
        thread_id: u64,
    },
    #[serde(rename = "e")]
    Exit { id: u64 },
    #[serde(rename = "i")]
    Event {
        #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        #[serde(rename = "n")]
        name: &'a str,
    },
}
