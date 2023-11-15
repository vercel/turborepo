use std::{cmp::max, collections::HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    server::ViewRect,
    span::SpanId,
    store::{SpanEventRef, SpanGraphRef, SpanRef, Store},
};

const EXTRA_WIDTH_PERCENTAGE: u64 = 20;
const EXTRA_HEIGHT: u64 = 5;

pub struct Viewer {
    known_lines: Vec<Option<KnownLine>>,
    expanded_spans: HashSet<SpanId>,
}

struct KnownLine {
    start: u64,
    end: u64,
    min_duration: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ViewLineUpdate {
    x: u64,
    y: u64,
    width: u64,
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

impl Viewer {
    pub fn new() -> Self {
        Self {
            known_lines: vec![],
            expanded_spans: HashSet::new(),
        }
    }

    pub fn compute_update(&mut self, store: &Store, view_rect: &ViewRect) -> Vec<ViewLineUpdate> {
        let mut queue = Vec::new();

        let mut current = 0;
        for span in store.root_spans() {
            let start = span.start();
            current = max(current, start);
            let width = span.corrected_total_time();
            queue.push((span, 0, current));
            current += width;
        }
        queue.reverse();

        let mut lines: Vec<Vec<LineEntry<'_>>> = vec![];

        while let Some((span, line_index, start)) = queue.pop() {
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

            let pixel_width =
                (width * view_rect.horizontal_pixels + view_rect.width) / view_rect.width;

            // compute children
            let mut children = Vec::new();
            let mut current = start;
            for child in span.children() {
                let child_width = child.corrected_total_time();
                let max_depth = child.max_depth();
                children.push(((child, line_index + 1, current), max_depth));
                current += child_width;
            }

            // When span size is smaller than a pixel, we only show the deepest child.
            if pixel_width <= 100 {
                if let Some((entry, _)) = children.into_iter().max_by_key(|(_, depth)| *depth) {
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
                for (entry, _) in children {
                    queue.push(entry);
                }

                // add span to line
                line.push(LineEntry {
                    start,
                    width,
                    ty: LineEntryType::Span(span),
                });
            }
        }

        lines
            .into_iter()
            .enumerate()
            .map(|(y, line)| ViewLineUpdate {
                x: 0,
                y: y as u64,
                width: u64::MAX,
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
                            let (category, text) = nice_name(&span);
                            ViewSpan {
                                id: span.id().get() as u64,
                                start: entry.start,
                                width: entry.width,
                                category,
                                text,
                                count: 1,
                            }
                        }
                        LineEntryType::SpanGraph(_) => todo!(),
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
