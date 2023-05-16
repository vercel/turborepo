use std::{
    cmp::{max, min, Reverse},
    collections::{hash_map::Entry, HashMap, HashSet},
    eprintln,
    ops::Range,
};

use intervaltree::{Element, IntervalTree};
use turbopack_cli_utils::tracing::{FullTraceRow, TraceRow};

macro_rules! pjson {
    ($($tt:tt)*) => {
        println!(",");
        print!($($tt)*);
    }
}

fn main() {
    // Read first argument from argv
    let mut args: HashSet<String> = std::env::args().skip(1).collect();
    let mut single = args.remove("--single");
    let mut merged = args.remove("--merged");
    let mut threads = args.remove("--threads");
    if !single && !merged && !threads {
        single = true;
        merged = true;
        threads = true;
    }
    let arg = args
        .iter()
        .next()
        .map_or(".turbopack/trace.log", String::as_str);

    eprint!("Reading trace from {}...", arg);

    // Read file to string
    let file = std::fs::read_to_string(arg).unwrap();
    eprintln!(" done ({} MiB)", file.len() / 1024 / 1024);

    eprint!("Analysing trace into span tree...");

    // Parse trace rows
    let trace_rows = file.lines().enumerate().filter_map(|(i, line)| {
        match serde_json::from_str::<FullTraceRow<'_>>(&line) {
            Ok(row) => Some(row),
            Err(err) => {
                eprintln!("Error parsing trace line {}:\n> {}\n{}", i, line, err);
                None
            }
        }
    });

    let mut spans = Vec::new();
    spans.push(Span {
        id: 0,
        parent: 0,
        name: "",
        start: 0,
        end: 0,
        self_start: None,
        items: Vec::new(),
        values: serde_json::Map::new(),
    });

    let mut active_ids = HashMap::new();

    fn ensure_span(active_ids: &mut HashMap<u64, usize>, spans: &mut Vec<Span>, id: u64) -> usize {
        match active_ids.entry(id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let internal_id = spans.len();
                entry.insert(internal_id);
                let span = Span {
                    id,
                    parent: 0,
                    name: "",
                    start: 0,
                    end: 0,
                    self_start: None,
                    items: Vec::new(),
                    values: serde_json::Map::new(),
                };
                spans.push(span);
                internal_id
            }
        }
    }

    let mut all_self_times = Vec::new();
    let mut name_counts: HashMap<&str, usize> = HashMap::new();

    for FullTraceRow { ts, data } in trace_rows {
        match data {
            TraceRow::Start {
                id,
                parent,
                name,
                values,
            } => {
                let internal_id = ensure_span(&mut active_ids, &mut spans, id);
                spans[internal_id].name = name;
                spans[internal_id].start = ts;
                spans[internal_id].end = ts;
                spans[internal_id].values = values;
                let internal_parent =
                    parent.map_or(0, |id| ensure_span(&mut active_ids, &mut spans, id));
                spans[internal_id].parent = internal_parent;
                let parent = &mut spans[internal_parent];
                parent.items.push(SpanItem::Child(internal_id));
                *name_counts.entry(name).or_default() += 1;
            }
            TraceRow::End { id } => {
                // id might be reused
                if let Some(internal_id) = active_ids.remove(&id) {
                    let span = &mut spans[internal_id];
                    span.end = ts;
                }
            }
            TraceRow::Enter { id, thread_id: _ } => {
                let internal_id = ensure_span(&mut active_ids, &mut spans, id);
                let span = &mut spans[internal_id];
                span.self_start = Some(SelfTimeStarted { ts });
            }
            TraceRow::Exit { id } => {
                let internal_id = ensure_span(&mut active_ids, &mut spans, id);
                let span = &mut spans[internal_id];
                if let Some(SelfTimeStarted { ts: ts_start }) = span.self_start {
                    let (start, end) = if ts_start > ts {
                        (ts, ts_start)
                    } else {
                        (ts_start, ts)
                    };
                    if end > start {
                        span.items.push(SpanItem::SelfTime {
                            start,
                            duration: end.saturating_sub(start),
                        });
                        all_self_times.push(Element {
                            range: start..end,
                            value: (internal_id, span.items.len() - 1),
                        })
                    }
                }
            }
            TraceRow::Event {
                parent,
                name,
                values,
            } => {
                // TODO
            }
        }
    }

    eprintln!(" done ({} spans)", spans.len());

    let mut name_counts: Vec<(&str, usize)> = name_counts.into_iter().collect();
    name_counts.sort_by_key(|(_, count)| Reverse(*count));

    eprintln!("Top 10 span names:");
    for (name, count) in name_counts.into_iter().take(10) {
        eprintln!("{}x {}", count, name);
    }

    println!("[");
    print!(r#"{{"ph":"M","pid":1,"name":"thread_name","tid":0,"args":{{"name":"Single CPU"}}}}"#);
    pjson!(r#"{{"ph":"M","pid":2,"name":"thread_name","tid":0,"args":{{"name":"Scaling CPU"}}}}"#);

    let busy = all_self_times.into_iter().collect::<IntervalTree<_, _>>();

    if threads {
        eprint!("Distributing time into virtual threads...");
        let mut virtual_threads = Vec::new();

        let find_thread = |virtual_threads: &mut Vec<VirtualThread>,
                           stack: &[usize],
                           start: u64| {
            let idle_threads = virtual_threads
                .iter()
                .enumerate()
                .filter(|(_, thread)| thread.ts <= start)
                .collect::<Vec<_>>();
            for (index, id) in stack.iter().enumerate() {
                for &(i, thread) in &idle_threads {
                    if thread.stack.len() > index && thread.stack[index] == *id {
                        return i;
                    }
                }
            }
            if let Some((index, _)) = idle_threads.into_iter().max_by_key(|(_, thread)| thread.ts) {
                return index;
            }
            virtual_threads.push(VirtualThread {
                stack: Vec::new(),
                ts: 0,
            });
            let index = virtual_threads.len() - 1;
            pjson!(
                r#"{{"ph":"M","pid":3,"name":"thread_name","tid":{index},"args":{{"name":"Virtual Thread"}}}}"#
            );
            index
        };

        let get_stack = |mut id: usize| {
            let mut stack = Vec::new();
            while id != 0 {
                let span = &spans[id];
                stack.push(id);
                id = span.parent;
            }
            stack.reverse();
            stack
        };

        for &Element {
            range: Range { start, .. },
            value: (id, index),
        } in busy.iter_sorted()
        {
            let span = &spans[id];
            let SpanItem::SelfTime { start: _, duration } = &span.items[index] else {
                panic!("Expected index to self time");
            };
            let stack = get_stack(id);
            let thread = find_thread(&mut virtual_threads, &stack, start);

            let virtual_thread = &mut virtual_threads[thread];
            let ts = virtual_thread.ts;
            let thread_stack = &mut virtual_thread.stack;

            // Leave old spans on that thread
            while !thread_stack.is_empty()
                && thread_stack.last() != stack.get(thread_stack.len() - 1)
            {
                let id = thread_stack.pop().unwrap();
                let span = &spans[id];
                pjson!(
                    r#"{{"ph":"E","pid":3,"ts":{ts},"name":{},"cat":"TODO","tid":{thread}}}"#,
                    serde_json::to_string(&span.name).unwrap()
                );
            }

            // Advance thread time to start
            if virtual_thread.ts < start {
                if !thread_stack.is_empty() {
                    pjson!(
                        r#"{{"ph":"B","pid":3,"ts":{ts},"name":"idle","cat":"TODO","tid":{thread}}}"#,
                    );
                    pjson!(
                        r#"{{"ph":"E","pid":3,"ts":{start},"name":"idle","cat":"TODO","tid":{thread}}}"#,
                    );
                }
                virtual_thread.ts = start;
            }

            // Enter new spans on that thread
            for id in stack[thread_stack.len()..].iter() {
                thread_stack.push(*id);
                let span = &spans[*id];
                pjson!(
                    r#"{{"ph":"B","pid":3,"ts":{start},"name":{},"cat":"TODO","tid":{thread}}}"#,
                    serde_json::to_string(&span.name).unwrap(),
                );
            }

            virtual_thread.ts += duration;
        }

        // Leave all threads
        for (i, VirtualThread { ts, mut stack }) in virtual_threads.into_iter().enumerate() {
            while let Some(id) = stack.pop() {
                let span = &spans[id];
                pjson!(
                    r#"{{"ph":"E","pid":3,"ts":{ts},"name":{},"cat":"TODO","tid":{i}}}"#,
                    serde_json::to_string(&span.name).unwrap()
                );
            }
        }
        eprintln!(" done");
    }

    if single || merged {
        eprint!("Emitting span tree...");

        let get_concurrency = |range: Range<u64>| {
            let mut sum = 0;
            for interval in busy.query(range.clone()) {
                let start = max(interval.range.start, range.start);
                let end = min(interval.range.end, range.end);
                sum += end - start;
            }
            100 * sum / (range.end - range.start)
        };

        let target_concurrency = 200;
        let warn_concurrency = 400;

        enum Task {
            Enter {
                id: usize,
                root: bool,
            },
            Exit {
                name_json: String,
                start: u64,
                start_scaled: u64,
            },
            SelfTime {
                duration: u64,
                concurrency: u64,
            },
        }
        let mut ts = 0;
        let mut tts = 0;
        let mut merged_ts = 0;
        let mut merged_tts = 0;
        let mut stack = spans
            .iter()
            .enumerate()
            .skip(1)
            .rev()
            .filter_map(|(id, span)| {
                if span.parent == 0 {
                    Some(Task::Enter { id, root: true })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        while let Some(task) = stack.pop() {
            match task {
                Task::Enter { id, root } => {
                    let span = &mut spans[id];
                    if root {
                        if ts < span.start {
                            ts = span.start;
                        }
                        if tts < span.start {
                            tts = span.start;
                        }
                        if merged_ts < span.start {
                            merged_ts = span.start;
                        }
                        if merged_tts < span.start {
                            merged_tts = span.start;
                        }
                    }
                    let name_json = if let Some(name_value) = span.values.get("name") {
                        serde_json::to_string(&if let serde_json::Value::String(s) = name_value {
                            format!("{} {s}", span.name)
                        } else {
                            format!("{} {name_value}", span.name)
                        })
                        .unwrap()
                    } else {
                        serde_json::to_string(&span.name).unwrap()
                    };
                    let args_json = serde_json::to_string(&span.values).unwrap();
                    if single {
                        pjson!(
                            r#"{{"ph":"B","pid":1,"ts":{ts},"tts":{tts},"name":{name_json},"cat":"TODO","tid":0,"args":{args_json}}}"#,
                        );
                    }
                    if merged {
                        pjson!(
                            r#"{{"ph":"B","pid":2,"ts":{merged_ts},"tts":{merged_tts},"name":{name_json},"cat":"TODO","tid":0,"args":{args_json}}}"#,
                        );
                    }
                    stack.push(Task::Exit {
                        name_json,
                        start: ts,
                        start_scaled: tts,
                    });
                    for item in span.items.iter().rev() {
                        match item {
                            SpanItem::SelfTime {
                                start, duration, ..
                            } => {
                                let range = *start..(start + duration);
                                let new_concurrency = get_concurrency(range);
                                let new_duration = *duration;
                                if let Some(Task::SelfTime {
                                    duration,
                                    concurrency,
                                }) = stack.last_mut()
                                {
                                    *concurrency = ((*concurrency) * (*duration)
                                        + new_concurrency * new_duration)
                                        / (*duration + new_duration);
                                    *duration += new_duration;
                                } else {
                                    stack.push(Task::SelfTime {
                                        duration: new_duration,
                                        concurrency: new_concurrency,
                                    });
                                }
                            }
                            SpanItem::Child(id) => {
                                stack.push(Task::Enter {
                                    id: *id,
                                    root: false,
                                });
                            }
                        }
                    }
                }
                Task::Exit {
                    name_json,
                    start,
                    start_scaled,
                } => {
                    if ts > start && tts > start_scaled {
                        let concurrency = (ts - start) * target_concurrency / (tts - start_scaled);
                        if single {
                            pjson!(
                                r#"{{"ph":"E","pid":1,"ts":{ts},"tts":{tts},"name":{name_json},"cat":"TODO","tid":0,"args":{{"concurrency":{}}}}}"#,
                                concurrency as f64 / 100.0,
                            );
                        }
                        if merged {
                            pjson!(
                                r#"{{"ph":"E","pid":2,"ts":{merged_ts},"tts":{merged_tts},"name":{name_json},"cat":"TODO","tid":0,"args":{{"concurrency":{}}}}}"#,
                                concurrency as f64 / 100.0,
                            );
                        }
                    } else {
                        if single {
                            pjson!(
                                r#"{{"ph":"E","pid":1,"ts":{ts},"tts":{tts},"name":{name_json},"cat":"TODO","tid":0}}"#,
                            );
                        }
                        if merged {
                            pjson!(
                                r#"{{"ph":"E","pid":2,"ts":{merged_ts},"tts":{merged_tts},"name":{name_json},"cat":"TODO","tid":0}}"#,
                            );
                        }
                    }
                }
                Task::SelfTime {
                    duration,
                    concurrency,
                } => {
                    let scaled_duration =
                        (duration * target_concurrency + concurrency - 1) / concurrency;
                    let merged_duration = (duration * 100 + concurrency - 1) / concurrency;
                    let merged_scaled_duration =
                        (merged_duration * target_concurrency + concurrency - 1) / concurrency;
                    let target_duration = duration * concurrency / warn_concurrency;
                    let merged_target_duration = merged_duration * concurrency / warn_concurrency;
                    if concurrency <= warn_concurrency {
                        let target = ts + target_duration;
                        let merged_target = merged_ts + merged_target_duration;
                        if single {
                            pjson!(
                                r#"{{"ph":"B","pid":1,"ts":{target},"tts":{tts},"name":"idle cpus","cat":"low concurrency","tid":0,"args":{{"concurrency":{}}}}}"#,
                                concurrency as f64 / 100.0,
                            );
                        }
                        if merged {
                            pjson!(
                                r#"{{"ph":"B","pid":2,"ts":{merged_target},"tts":{merged_tts},"name":"idle cpus","cat":"low concurrency","tid":0,"args":{{"concurrency":{}}}}}"#,
                                concurrency as f64 / 100.0,
                            );
                        }
                    }
                    ts += duration;
                    tts += scaled_duration;
                    merged_ts += merged_duration;
                    merged_tts += merged_scaled_duration;
                    if concurrency <= warn_concurrency {
                        if single {
                            pjson!(
                                r#"{{"ph":"E","pid":1,"ts":{ts},"tts":{tts},"name":"idle cpus","cat":"low concurrency","tid":0}}"#,
                            );
                        }
                        if merged {
                            pjson!(
                                r#"{{"ph":"E","pid":2,"ts":{merged_ts},"tts":{merged_tts},"name":"idle cpus","cat":"low concurrency","tid":0}}"#,
                            );
                        }
                    }
                }
            }
        }
        eprintln!(" done");
    }
    println!();
    println!("]");
}

#[derive(Debug)]
struct SelfTimeStarted {
    ts: u64,
}

#[derive(Debug, Default)]
struct Span<'a> {
    id: u64,
    parent: usize,
    name: &'a str,
    start: u64,
    end: u64,
    self_start: Option<SelfTimeStarted>,
    items: Vec<SpanItem>,
    values: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug)]
enum SpanItem {
    SelfTime { start: u64, duration: u64 },
    Child(usize),
}

#[derive(Debug)]
struct VirtualThread {
    ts: u64,
    stack: Vec<usize>,
}
