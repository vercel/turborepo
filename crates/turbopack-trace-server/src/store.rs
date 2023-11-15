use std::{
    cmp::max,
    collections::HashSet,
    sync::{Arc, OnceLock},
};

use crate::span::{Span, SpanEvent, SpanGraph, SpanGraphEvent, SpanId};

pub struct Store {
    spans: Vec<Span>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            spans: vec![Span {
                id: SpanId::MAX,
                parent: None,
                start: 0,
                end: u64::MAX,
                category: "".into(),
                name: "(root)".into(),
                args: vec![],
                events: vec![],
                max_depth: OnceLock::new(),
                graph: OnceLock::new(),
                self_time: 0,
                total_time: OnceLock::new(),
                corrected_self_time: OnceLock::new(),
                corrected_total_time: OnceLock::new(),
            }],
        }
    }

    pub fn reset(&mut self) {
        self.spans.clear();
    }

    pub fn add_span(
        &mut self,
        parent: Option<SpanId>,
        start: u64,
        category: String,
        name: String,
        args: Vec<(String, String)>,
        outdated_spans: &mut HashSet<SpanId>,
    ) -> SpanId {
        let id = SpanId::new(self.spans.len()).unwrap();
        self.spans.push(Span {
            id,
            parent,
            start,
            end: start,
            category,
            name,
            args,
            events: vec![],
            max_depth: OnceLock::new(),
            graph: OnceLock::new(),
            self_time: 0,
            total_time: OnceLock::new(),
            corrected_self_time: OnceLock::new(),
            corrected_total_time: OnceLock::new(),
        });
        let parent = if let Some(parent) = parent {
            outdated_spans.insert(parent);
            &mut self.spans[parent.get()]
        } else {
            &mut self.spans[0]
        };
        parent.events.push(SpanEvent::Child { id });
        id
    }

    pub fn add_self_time(
        &mut self,
        span: SpanId,
        start: u64,
        end: u64,
        outdated_spans: &mut HashSet<SpanId>,
    ) {
        outdated_spans.insert(span);
        let span = &mut self.spans[span.get()];
        span.self_time += end - start;
        span.events.push(SpanEvent::SelfTime { start, end });
        span.end = max(span.end, end);
    }

    pub fn invalidate_outdated_spans(&mut self, outdated_spans: &HashSet<SpanId>) {
        for id in outdated_spans.iter() {
            let mut span = &mut self.spans[id.get()];
            loop {
                span.total_time.take();
                span.corrected_self_time.take();
                span.corrected_total_time.take();
                span.graph.take();
                let Some(parent) = span.parent else {
                    break;
                };
                if outdated_spans.contains(&parent) {
                    break;
                }
                span = &mut self.spans[parent.get()];
            }
        }
    }

    pub fn root_spans(&self) -> impl Iterator<Item = SpanRef<'_>> {
        self.spans[0].events.iter().filter_map(|event| match event {
            &SpanEvent::Child { id } => Some(SpanRef {
                span: &self.spans[id.get()],
                store: self,
            }),
            _ => None,
        })
    }
}

pub struct SpanRef<'a> {
    span: &'a Span,
    store: &'a Store,
}

impl<'a> SpanRef<'a> {
    pub fn id(&self) -> SpanId {
        self.span.id
    }

    pub fn parent(&self) -> Option<SpanRef<'_>> {
        self.span.parent.map(|id| SpanRef {
            span: &self.store.spans[id.get()],
            store: self.store,
        })
    }

    pub fn start(&self) -> u64 {
        self.span.start
    }

    pub fn end(&self) -> u64 {
        self.span.end
    }

    pub fn category(&self) -> &str {
        &self.span.category
    }

    pub fn name(&self) -> &str {
        &self.span.name
    }

    pub fn args(&self) -> impl Iterator<Item = (&str, &str)> {
        self.span.args.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn self_time(&self) -> u64 {
        self.span.self_time
    }

    pub fn events(&self) -> impl Iterator<Item = SpanEventRef<'a>> {
        self.span.events.iter().map(|event| match event {
            SpanEvent::SelfTime { .. } => SpanEventRef::SelfTime {
                start: self.span.start,
                end: self.span.end,
            },
            SpanEvent::Child { id } => SpanEventRef::Child {
                span: SpanRef {
                    span: &self.store.spans[id.get()],
                    store: self.store,
                },
            },
        })
    }

    pub fn children(&self) -> impl Iterator<Item = SpanRef<'a>> + DoubleEndedIterator + '_ {
        self.span.events.iter().filter_map(|event| match event {
            SpanEvent::SelfTime { .. } => None,
            SpanEvent::Child { id } => Some(SpanRef {
                span: &self.store.spans[id.get()],
                store: self.store,
            }),
        })
    }

    pub fn total_time(&self) -> u64 {
        *self.span.total_time.get_or_init(|| {
            self.children()
                .map(|child| child.total_time())
                .reduce(|a, b| a + b)
                .unwrap_or_default()
                + self.self_time()
        })
    }

    pub fn corrected_self_time(&self) -> u64 {
        *self
            .span
            .corrected_self_time
            .get_or_init(|| self.self_time())
    }

    pub fn corrected_total_time(&self) -> u64 {
        *self
            .span
            .corrected_total_time
            .get_or_init(|| self.total_time())
    }

    pub fn max_depth(&self) -> u32 {
        *self.span.max_depth.get_or_init(|| {
            self.children()
                .map(|child| child.max_depth() + 1)
                .max()
                .unwrap_or_default()
        })
    }

    pub fn graph(&self) -> impl Iterator<Item = SpanGraphEventRef<'a>> {
        self.span
            .graph
            .get_or_init(|| todo!())
            .iter()
            .map(|event| match event {
                SpanGraphEvent::SelfTime { duration } => SpanGraphEventRef::SelfTime {
                    duration: *duration,
                },
                SpanGraphEvent::Child { child } => SpanGraphEventRef::Child {
                    span: SpanGraphRef {
                        span: child.clone(),
                        store: self.store,
                    },
                },
            })
    }
}

pub enum SpanEventRef<'a> {
    SelfTime { start: u64, end: u64 },
    Child { span: SpanRef<'a> },
}

pub enum SpanGraphEventRef<'a> {
    SelfTime { duration: u64 },
    Child { span: SpanGraphRef<'a> },
}

pub struct SpanGraphRef<'a> {
    span: Arc<SpanGraph>,
    store: &'a Store,
}
