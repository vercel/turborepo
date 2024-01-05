use std::collections::HashMap;

use crate::{
    span::{SpanBottomUp, SpanIndex},
    span_ref::SpanRef,
};

pub fn build_bottom_up_graph(span: SpanRef<'_>) -> Vec<SpanBottomUp> {
    let mut roots = HashMap::new();
    let mut current_iterators = vec![span.children()];
    let mut current_path: Vec<(&'_ str, SpanIndex)> = vec![];
    while let Some(mut iter) = current_iterators.pop() {
        if let Some(child) = iter.next() {
            current_iterators.push(iter);

            let name = child.group_name();
            let (_, mut bottom_up) = roots
                .raw_entry_mut()
                .from_key(name)
                .or_insert_with(|| (name.to_string(), SpanBottomUp::new(child.index())));
            bottom_up.self_spans.push(child.index());
            let mut prev = None;
            for &(name, example_span) in current_path.iter().rev() {
                if prev == Some(name) {
                    continue;
                }
                let (_, child_bottom_up) = bottom_up
                    .children
                    .raw_entry_mut()
                    .from_key(name)
                    .or_insert_with(|| (name.to_string(), SpanBottomUp::new(example_span)));
                child_bottom_up.self_spans.push(child.index());
                bottom_up = child_bottom_up;
                prev = Some(name);
            }

            current_path.push((child.group_name(), child.index()));
            current_iterators.push(child.children());
        } else {
            current_path.pop();
        }
    }
    roots.into_values().collect()
}
