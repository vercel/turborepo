use std::{
    num::NonZeroUsize,
    sync::{Arc, OnceLock},
};

pub type SpanId = NonZeroUsize;

pub struct Span {
    // These values won't change after creation:
    pub id: SpanId,
    pub parent: Option<SpanId>,
    pub start: u64,
    pub category: String,
    pub name: String,
    pub args: Vec<(String, String)>,

    // This might change during writing:
    pub events: Vec<SpanEvent>,

    // These values are computed automatically:
    pub end: u64,
    pub self_time: u64,

    // These values are computed when accessed (and maybe deleted during writing):
    pub max_depth: OnceLock<u32>,
    pub total_time: OnceLock<u64>,
    pub corrected_self_time: OnceLock<u64>,
    pub corrected_total_time: OnceLock<u64>,
    pub graph: OnceLock<Vec<SpanGraphEvent>>,
}

pub enum SpanEvent {
    SelfTime { start: u64, end: u64 },
    Child { id: SpanId },
}

pub enum SpanGraphEvent {
    SelfTime { duration: u64 },
    Child { child: Arc<SpanGraph> },
}

pub struct SpanGraph {
    // These values won't change after creation:
    pub id: u64,
    pub spans: Vec<SpanId>,

    // These values are computed when accessed:
    pub events: Option<Vec<SpanGraphEvent>>,
}
