use std::{cmp::max, collections::HashMap};

use serde::{Deserialize, Serialize};

use crate::{
    server::ViewRect,
    store::{SpanGraphEventRef, SpanGraphRef, SpanId, SpanRef, Store},
};

const EXTRA_WIDTH_PERCENTAGE: u64 = 20;
const EXTRA_HEIGHT: u64 = 5;

#[derive(Default)]
pub struct Viewer {
    span_options: HashMap<SpanId, SpanOptions>,
}

pub enum ExpandedState {
    Expanded,
    AllExpanded,
    Collapsed,
}

#[derive(Default)]
struct SpanOptions {
    expanded: Option<ExpandedState>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ViewLineUpdate {
    y: u64,
    spans: Vec<ViewSpan>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ViewSpan {
    id: u64,
    #[serde(rename = "x")]
    start: u64,
    #[serde(rename = "w")]
    width: u64,
    #[serde(rename = "cat")]
    category: String,
    #[serde(rename = "t")]
    text: String,
    #[serde(rename = "c")]
    count: u64,
}

enum QueueItem<'a> {
    Span(SpanRef<'a>),
    SpanGraph(SpanGraphRef<'a>),
}

impl<'a> QueueItem<'a> {
    fn corrected_total_time(&self) -> u64 {
        match self {
            QueueItem::Span(span) => span.corrected_total_time(),
            QueueItem::SpanGraph(span_graph) => span_graph.corrected_total_time(),
        }
    }

    fn max_depth(&self) -> u32 {
        match self {
            QueueItem::Span(span) => span.max_depth(),
            QueueItem::SpanGraph(span_graph) => span_graph.max_depth(),
        }
    }
}

struct QueueItemWithState<'a> {
    item: QueueItem<'a>,
    line_index: usize,
    start: u64,
    placeholder: bool,
    expanded: bool,
}

impl Viewer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_expanded_state(&mut self, id: SpanId, expanded: Option<ExpandedState>) {
        self.span_options.entry(id).or_default().expanded = expanded;
    }

    pub fn compute_update(&mut self, store: &Store, view_rect: &ViewRect) -> Vec<ViewLineUpdate> {
        let mut queue = Vec::new();

        let mut current = 0;
        let mut root_spans = store
            .root_spans()
            .map(|span| {
                let start = span.start();
                let end = span.end();
                let width = span.corrected_total_time();
                (span, start, end, width)
            })
            .collect::<Vec<_>>();
        root_spans.sort_by_key(|(_, _, end, _)| *end);
        for (span, start, _, width) in root_spans {
            current = max(current, start);
            queue.push(QueueItemWithState {
                item: QueueItem::Span(span),
                line_index: 0,
                start: current,
                placeholder: false,
                expanded: false,
            });
            current += width;
        }
        queue.reverse();

        let mut lines: Vec<Vec<LineEntry<'_>>> = vec![];

        while let Some(QueueItemWithState {
            item: span,
            line_index,
            start,
            placeholder,
            expanded,
        }) = queue.pop()
        {
            // filter by view rect (vertical)
            if line_index > (view_rect.y + view_rect.height + EXTRA_HEIGHT) as usize {
                continue;
            }

            // offset by last entry if needed
            let line = get_line(&mut lines, line_index);
            let width = span.corrected_total_time();

            if line_index > 0 {
                // filter by view rect (horizontal)
                if start > view_rect.x + view_rect.width * (100 + EXTRA_WIDTH_PERCENTAGE) / 100 {
                    continue;
                }
                if start + width
                    < view_rect
                        .x
                        .saturating_sub(view_rect.width * EXTRA_WIDTH_PERCENTAGE / 100)
                {
                    continue;
                }
            }

            // compute children
            let mut children = Vec::new();
            let mut current = start;
            fn handle_child<'a>(
                children: &mut Vec<(QueueItemWithState<'a>, u32, (u64, u64))>,
                current: &mut u64,
                view_rect: &ViewRect,
                line_index: usize,
                expanded: bool,
                child: QueueItem<'a>,
            ) {
                let child_width = child.corrected_total_time();
                let max_depth = child.max_depth();
                let pixel1 = *current * view_rect.horizontal_pixels / view_rect.width;
                let pixel2 =
                    ((*current + child_width) * view_rect.horizontal_pixels + view_rect.width - 1)
                        / view_rect.width;
                children.push((
                    QueueItemWithState {
                        item: child,
                        line_index: line_index + 1,
                        start: *current,
                        placeholder: false,
                        expanded,
                    },
                    max_depth,
                    (pixel1, pixel2),
                ));
                *current += child_width;
            }
            match &span {
                QueueItem::Span(span) => {
                    let (show_children, expanded) = match self
                        .span_options
                        .get(&span.id())
                        .and_then(|o| o.expanded.as_ref())
                    {
                        None => (expanded, expanded),
                        Some(ExpandedState::Expanded) => (true, expanded),
                        Some(ExpandedState::AllExpanded) => (true, true),
                        Some(ExpandedState::Collapsed) => (false, false),
                    };
                    if show_children {
                        for child in span.children() {
                            handle_child(
                                &mut children,
                                &mut current,
                                view_rect,
                                line_index,
                                expanded,
                                QueueItem::Span(child),
                            );
                        }
                    } else {
                        for event in span.graph() {
                            match event {
                                SpanGraphEventRef::SelfTime { duration: _ } => {}
                                SpanGraphEventRef::Child { graph } => {
                                    handle_child(
                                        &mut children,
                                        &mut current,
                                        view_rect,
                                        line_index,
                                        expanded,
                                        QueueItem::SpanGraph(graph),
                                    );
                                }
                            }
                        }
                    }
                }
                QueueItem::SpanGraph(span_graph) => {
                    let (show_spans, expanded) = match self
                        .span_options
                        .get(&span_graph.id())
                        .and_then(|o| o.expanded.as_ref())
                    {
                        None => (expanded, expanded),
                        Some(ExpandedState::Expanded) => (true, expanded),
                        Some(ExpandedState::AllExpanded) => (true, true),
                        Some(ExpandedState::Collapsed) => (false, false),
                    };
                    if show_spans && span_graph.count() > 1 {
                        for child in span_graph.root_spans() {
                            handle_child(
                                &mut children,
                                &mut current,
                                view_rect,
                                line_index,
                                expanded,
                                QueueItem::Span(child),
                            );
                        }
                    } else {
                        for child in span_graph.children() {
                            handle_child(
                                &mut children,
                                &mut current,
                                view_rect,
                                line_index,
                                expanded,
                                QueueItem::SpanGraph(child),
                            );
                        }
                    }
                }
            }

            const MIN_VISIBLE_PIXEL_SIZE: u64 = 3;

            // When span size is smaller than a pixel, we only show the deepest child.
            if placeholder {
                if let Some((mut entry, _, _)) =
                    children.into_iter().max_by_key(|(_, depth, _)| *depth)
                {
                    entry.placeholder = true;
                    queue.push(entry);
                }

                // add span to line
                line.push(LineEntry {
                    start,
                    width,
                    ty: LineEntryType::Placeholder,
                });
            } else {
                // add children to queue
                children.reverse();
                let mut last_pixel = u64::MAX;
                let mut last_max_depth = 0;
                for (mut entry, max_depth, (pixel1, pixel2)) in children {
                    if last_pixel <= pixel1 + MIN_VISIBLE_PIXEL_SIZE {
                        if last_max_depth < max_depth {
                            queue.pop();
                            entry.placeholder = true;
                        } else {
                            if let Some(entry) = queue.last_mut() {
                                entry.placeholder = true;
                            }
                            continue;
                        }
                    };
                    queue.push(entry);
                    last_max_depth = max_depth;
                    last_pixel = pixel2;
                }

                // add span to line
                line.push(LineEntry {
                    start,
                    width,
                    ty: match span {
                        QueueItem::Span(span) => LineEntryType::Span(span),
                        QueueItem::SpanGraph(span_graph) => LineEntryType::SpanGraph(span_graph),
                    },
                });
            }
        }

        lines
            .into_iter()
            .enumerate()
            .map(|(y, line)| ViewLineUpdate {
                y: y as u64,
                spans: line
                    .into_iter()
                    .map(|entry| match entry.ty {
                        LineEntryType::Placeholder => ViewSpan {
                            id: 0,
                            start: entry.start,
                            width: entry.width,
                            category: String::new(),
                            text: String::new(),
                            count: 1,
                        },
                        LineEntryType::Span(span) => {
                            let (category, text) = span.nice_name();
                            ViewSpan {
                                id: span.id().get() as u64,
                                start: entry.start,
                                width: entry.width,
                                category: category.to_string(),
                                text: text.to_string(),
                                count: 1,
                            }
                        }
                        LineEntryType::SpanGraph(graph) => {
                            let (category, text) = graph.nice_name();
                            ViewSpan {
                                id: graph.id().get() as u64,
                                start: entry.start,
                                width: entry.width,
                                category: category.to_string(),
                                text: text.to_string(),
                                count: graph.count() as u64,
                            }
                        }
                    })
                    .collect(),
            })
            .collect()
    }
}

fn get_line<T: Default>(lines: &mut Vec<T>, i: usize) -> &mut T {
    if i >= lines.len() {
        lines.resize_with(i + 1, || Default::default());
    }
    &mut lines[i]
}

struct LineEntry<'a> {
    start: u64,
    width: u64,
    ty: LineEntryType<'a>,
}

enum LineEntryType<'a> {
    Placeholder,
    Span(SpanRef<'a>),
    SpanGraph(SpanGraphRef<'a>),
}

fn nice_name(span: &SpanRef<'_>) -> (String, String) {
    if let Some(name) = span
        .args()
        .find(|&(k, _)| k == "name")
        .map(|(_, v)| v.to_string())
    {
        (format!("{} {}", span.name(), span.category()), name)
    } else {
        (span.category().to_string(), span.name().to_string())
    }
}
