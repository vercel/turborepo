use serde::{Deserialize, Serialize};
use serde_json::Map;

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
        #[serde(rename = "t")]
        target: &'a str,
        #[serde(rename = "v", default, skip_serializing_if = "Map::is_empty")]
        values: Map<String, serde_json::Value>,
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
        #[serde(rename = "v", default, skip_serializing_if = "Map::is_empty")]
        values: Map<String, serde_json::Value>,
    },
}
