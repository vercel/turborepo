use std::{borrow::Cow, sync::Arc};

use indexmap::IndexMap;
use serde::Deserialize;

use super::TraceFormat;
use crate::store_container::StoreContainer;

pub struct NextJsFormat {
    store: Arc<StoreContainer>,
}

impl NextJsFormat {
    pub fn new(store: Arc<StoreContainer>) -> Self {
        Self { store }
    }
}

impl TraceFormat for NextJsFormat {
    fn read(&mut self, mut buffer: &[u8]) -> anyhow::Result<usize> {
        let mut bytes_read = 0;
        loop {
            let Some(line_end) = buffer.iter().position(|b| *b == b'\n') else {
                break;
            };
            let line = &buffer[..line_end];
            buffer = &buffer[line_end + 1..];
            bytes_read += line.len() + 1;

            let spans: Vec<NextJsSpan> = serde_json::from_slice(line)?;
            println!("Read {} spans", spans.len());
        }
        Ok(bytes_read)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]

enum TagValue<'a> {
    String(Cow<'a, str>),
    Number(f64),
    Bool(bool),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NextJsSpan<'a> {
    name: Cow<'a, str>,
    duration: u64,
    timestamp: u64,
    id: u64,
    parent_id: Option<u64>,
    tags: IndexMap<Cow<'a, str>, Option<TagValue<'a>>>,
    start_time: u64,
    trace_id: Cow<'a, str>,
}
