use std::collections::HashMap;

use crate::parse::TraceEvent;

/// Uniquely identifies a function by its name and source location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionId {
    pub name: String,
    pub category: String,
    pub file: String,
    pub line: u64,
}

impl FunctionId {
    pub fn location(&self) -> String {
        if self.file.is_empty() {
            self.category.clone()
        } else if self.line > 0 {
            format!("{}:{}", self.file, self.line)
        } else {
            self.file.clone()
        }
    }
}

/// Aggregated timing data for a single function.
#[derive(Debug, Clone)]
pub struct FunctionStats {
    pub id: FunctionId,
    /// Total wall time spent in this function (including children).
    pub total_time_us: f64,
    /// Self time (total minus time in child spans).
    pub self_time_us: f64,
    /// Number of times this function was entered.
    pub call_count: u64,
}

/// A resolved span with begin/end timestamps and parent relationship.
#[derive(Debug, Clone)]
struct ResolvedSpan {
    func_id: FunctionId,
    start_us: f64,
    end_us: f64,
    tid: u64,
    parent_idx: Option<usize>,
}

/// Full analysis result.
#[derive(Debug)]
pub struct ProfileAnalysis {
    pub total_duration_us: f64,
    pub span_count: u64,
    pub functions: Vec<FunctionStats>,
    /// caller -> callee -> count
    pub call_edges: HashMap<(FunctionId, FunctionId), u64>,
}

pub fn analyze(events: &[TraceEvent]) -> ProfileAnalysis {
    let spans = resolve_spans(events);
    let total_duration_us = compute_total_duration(&spans);
    let span_count = spans.len() as u64;

    let spans_with_parents = assign_parents(&spans);
    let (functions, call_edges) = compute_function_stats(&spans_with_parents);

    ProfileAnalysis {
        total_duration_us,
        span_count,
        functions,
        call_edges,
    }
}

/// Match async "b"/"e" pairs into resolved spans.
fn resolve_spans(events: &[TraceEvent]) -> Vec<ResolvedSpan> {
    // Group begin events by (tid, id, name) to match with end events.
    // For async spans, the `id` field is the correlation key.
    let mut pending: HashMap<(u64, u64, String), PendingSpan> = HashMap::new();
    let mut resolved = Vec::new();

    for event in events {
        let name = match &event.name {
            Some(n) => n.clone(),
            None => continue,
        };

        match event.ph.as_str() {
            "b" | "B" => {
                let tid = event.tid.unwrap_or(0);
                let id = event.id.unwrap_or(0);
                let ts = match event.ts {
                    Some(ts) => ts,
                    None => continue,
                };

                let func_id = FunctionId {
                    name: name.clone(),
                    category: event.cat.clone().unwrap_or_default(),
                    file: event.file.clone().unwrap_or_default(),
                    line: event.line.unwrap_or(0),
                };

                pending.insert(
                    (tid, id, name),
                    PendingSpan {
                        func_id,
                        start_us: ts,
                        tid,
                    },
                );
            }
            "e" | "E" => {
                let tid = event.tid.unwrap_or(0);
                let id = event.id.unwrap_or(0);
                let ts = match event.ts {
                    Some(ts) => ts,
                    None => continue,
                };

                let key = (tid, id, name);
                if let Some(begin) = pending.remove(&key) {
                    resolved.push(ResolvedSpan {
                        func_id: begin.func_id,
                        start_us: begin.start_us,
                        end_us: ts,
                        tid: begin.tid,
                        parent_idx: None,
                    });
                }
            }
            // "i" (instant), "M" (metadata) -- skip for span analysis
            _ => {}
        }
    }

    // Sort by start time for parent assignment
    resolved.sort_by(|a, b| {
        a.start_us
            .partial_cmp(&b.start_us)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    resolved
}

struct PendingSpan {
    func_id: FunctionId,
    start_us: f64,
    tid: u64,
}

fn compute_total_duration(spans: &[ResolvedSpan]) -> f64 {
    if spans.is_empty() {
        return 0.0;
    }
    let min_start = spans
        .iter()
        .map(|s| s.start_us)
        .fold(f64::INFINITY, f64::min);
    let max_end = spans
        .iter()
        .map(|s| s.end_us)
        .fold(f64::NEG_INFINITY, f64::max);
    max_end - min_start
}

/// Assign parent spans using a stack-based approach per thread.
/// For async traces, we use timestamp containment: a span is a child of
/// the most recent span on the same thread that fully contains it.
fn assign_parents(spans: &[ResolvedSpan]) -> Vec<ResolvedSpan> {
    let mut result = spans.to_vec();

    // Group spans by tid
    let mut by_tid: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, span) in result.iter().enumerate() {
        by_tid.entry(span.tid).or_default().push(i);
    }

    for indices in by_tid.values() {
        // Within each thread, use a stack to track nesting.
        // Spans are already sorted by start time globally.
        let mut stack: Vec<usize> = Vec::new();

        for &idx in indices {
            // Pop spans from the stack that have ended before this span starts
            while let Some(&top_idx) = stack.last() {
                if result[top_idx].end_us <= result[idx].start_us {
                    stack.pop();
                } else {
                    break;
                }
            }

            // The top of the stack (if any) is our parent
            if let Some(&parent_idx) = stack.last() {
                // Only set parent if this span is fully contained
                if result[parent_idx].start_us <= result[idx].start_us
                    && result[idx].end_us <= result[parent_idx].end_us
                {
                    result[idx].parent_idx = Some(parent_idx);
                }
            }

            stack.push(idx);
        }
    }

    result
}

fn compute_function_stats(
    spans: &[ResolvedSpan],
) -> (Vec<FunctionStats>, HashMap<(FunctionId, FunctionId), u64>) {
    let mut stats_map: HashMap<FunctionId, (f64, f64, u64)> = HashMap::new();
    let mut call_edges: HashMap<(FunctionId, FunctionId), u64> = HashMap::new();

    // First pass: accumulate total time and call count
    for span in spans {
        let duration = span.end_us - span.start_us;
        let entry = stats_map
            .entry(span.func_id.clone())
            .or_insert((0.0, 0.0, 0));
        entry.0 += duration; // total_time
        entry.1 += duration; // self_time (will subtract children below)
        entry.2 += 1; // call_count
    }

    // Second pass: subtract child time from parent's self time, record call edges
    for span in spans {
        if let Some(parent_idx) = span.parent_idx {
            let parent = &spans[parent_idx];
            let child_duration = span.end_us - span.start_us;

            if let Some(parent_stats) = stats_map.get_mut(&parent.func_id) {
                parent_stats.1 -= child_duration;
            }

            *call_edges
                .entry((parent.func_id.clone(), span.func_id.clone()))
                .or_insert(0) += 1;
        }
    }

    // Clamp negative self-times (can happen with overlapping async spans)
    for stats in stats_map.values_mut() {
        if stats.1 < 0.0 {
            stats.1 = 0.0;
        }
    }

    let mut functions: Vec<FunctionStats> = stats_map
        .into_iter()
        .map(|(id, (total, self_time, count))| FunctionStats {
            id,
            total_time_us: total,
            self_time_us: self_time,
            call_count: count,
        })
        .collect();

    // Sort by self time descending
    functions.sort_by(|a, b| {
        b.self_time_us
            .partial_cmp(&a.self_time_us)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (functions, call_edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_trace;

    #[test]
    fn basic_analysis() {
        let json = r#"[
            {"ph":"M","pid":1,"name":"thread_name","tid":0,"args":{"name":"main"}},
            {"ph":"b","pid":1,"ts":0.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1,".file":"src/run.rs",".line":10},
            {"ph":"b","pid":1,"ts":100.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2,".file":"src/hash.rs",".line":20},
            {"ph":"e","pid":1,"ts":300.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2},
            {"ph":"e","pid":1,"ts":500.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1}
        ]"#;

        let events = parse_trace(json).unwrap();
        let analysis = analyze(&events);

        assert_eq!(analysis.span_count, 2);
        assert!((analysis.total_duration_us - 500.0).abs() < 0.01);

        // "run" has total=500, self=500-200=300
        let run = analysis
            .functions
            .iter()
            .find(|f| f.id.name == "run")
            .unwrap();
        assert!((run.total_time_us - 500.0).abs() < 0.01);
        assert!((run.self_time_us - 300.0).abs() < 0.01);

        // "hash" has total=200, self=200
        let hash = analysis
            .functions
            .iter()
            .find(|f| f.id.name == "hash")
            .unwrap();
        assert!((hash.total_time_us - 200.0).abs() < 0.01);
        assert!((hash.self_time_us - 200.0).abs() < 0.01);
    }
}
