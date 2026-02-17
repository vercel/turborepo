use std::fmt::Write;

use crate::analyze::{FunctionId, ProfileAnalysis};

/// Minimum self-time percentage to appear in the Hot Functions table.
const HOT_FUNCTION_THRESHOLD_PERCENT: f64 = 0.4;
/// Maximum number of functions in the "Top N" summary line.
const TOP_N: usize = 10;
/// Minimum total-time percentage to appear in the Call Tree table.
const CALL_TREE_THRESHOLD_PERCENT: f64 = 0.4;
/// Minimum self-time percentage to get a Function Details section.
const DETAIL_THRESHOLD_PERCENT: f64 = 0.5;

pub fn format_markdown(analysis: &ProfileAnalysis) -> String {
    let mut out = String::with_capacity(8192);

    write_header(&mut out, analysis);
    write_top_n(&mut out, analysis);
    write_hot_functions(&mut out, analysis);
    write_call_tree(&mut out, analysis);
    write_function_details(&mut out, analysis);

    out
}

fn write_header(out: &mut String, analysis: &ProfileAnalysis) {
    let duration = format_duration_us(analysis.total_duration_us);
    let unique_fns = analysis.functions.len();

    let _ = writeln!(out, "# CPU Profile");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Duration | Spans | Functions |");
    let _ = writeln!(out, "|----------|-------|-----------|");
    let _ = writeln!(
        out,
        "| {} | {} | {} |",
        duration, analysis.span_count, unique_fns
    );
    let _ = writeln!(out);
}

fn write_top_n(out: &mut String, analysis: &ProfileAnalysis) {
    if analysis.functions.is_empty() {
        return;
    }

    let total = analysis.total_duration_us;
    if total <= 0.0 {
        return;
    }

    let entries: Vec<String> = analysis
        .functions
        .iter()
        .take(TOP_N)
        .filter(|f| f.self_time_us > 0.0)
        .map(|f| {
            let pct = (f.self_time_us / total) * 100.0;
            format!("`{}` {:.1}%", f.id.name, pct)
        })
        .collect();

    let _ = writeln!(out, "**Top {}:** {}", entries.len(), entries.join(", "));
    let _ = writeln!(out);
}

fn write_hot_functions(out: &mut String, analysis: &ProfileAnalysis) {
    let total = analysis.total_duration_us;
    if total <= 0.0 {
        return;
    }

    let _ = writeln!(out, "## Hot Functions (Self Time)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| Self% | Self | Total% | Total | Function | Location |"
    );
    let _ = writeln!(
        out,
        "|------:|-----:|-------:|------:|----------|----------|"
    );

    for func in &analysis.functions {
        let self_pct = (func.self_time_us / total) * 100.0;
        if self_pct < HOT_FUNCTION_THRESHOLD_PERCENT {
            break;
        }
        let total_pct = (func.total_time_us / total) * 100.0;

        let _ = writeln!(
            out,
            "| {:.1}% | {} | {:.1}% | {} | `{}` | `{}` |",
            self_pct,
            format_duration_us(func.self_time_us),
            total_pct,
            format_duration_us(func.total_time_us),
            func.id.name,
            func.id.location(),
        );
    }

    let _ = writeln!(out);
}

fn write_call_tree(out: &mut String, analysis: &ProfileAnalysis) {
    let total = analysis.total_duration_us;
    if total <= 0.0 {
        return;
    }

    // Sort by total time descending for call tree view
    let mut by_total: Vec<_> = analysis.functions.iter().collect();
    by_total.sort_by(|a, b| {
        b.total_time_us
            .partial_cmp(&a.total_time_us)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let _ = writeln!(out, "## Call Tree (Total Time)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| Total% | Total | Self% | Self | Function | Location |"
    );
    let _ = writeln!(
        out,
        "|-------:|------:|------:|-----:|----------|----------|"
    );

    for func in &by_total {
        let total_pct = (func.total_time_us / total) * 100.0;
        if total_pct < CALL_TREE_THRESHOLD_PERCENT {
            break;
        }
        let self_pct = (func.self_time_us / total) * 100.0;

        let _ = writeln!(
            out,
            "| {:.1}% | {} | {:.1}% | {} | `{}` | `{}` |",
            total_pct,
            format_duration_us(func.total_time_us),
            self_pct,
            format_duration_us(func.self_time_us),
            func.id.name,
            func.id.location(),
        );
    }

    let _ = writeln!(out);
}

fn write_function_details(out: &mut String, analysis: &ProfileAnalysis) {
    let total = analysis.total_duration_us;
    if total <= 0.0 {
        return;
    }

    let _ = writeln!(out, "## Function Details");
    let _ = writeln!(out);

    for func in &analysis.functions {
        let self_pct = (func.self_time_us / total) * 100.0;
        if self_pct < DETAIL_THRESHOLD_PERCENT {
            continue;
        }

        let _ = writeln!(out, "### `{}`", func.id.name);
        let _ = writeln!(
            out,
            "`{}` | Self: {:.1}% ({}) | Total: {:.1}% ({}) | Calls: {}",
            func.id.location(),
            self_pct,
            format_duration_us(func.self_time_us),
            (func.total_time_us / total) * 100.0,
            format_duration_us(func.total_time_us),
            func.call_count,
        );
        let _ = writeln!(out);

        // Callers (who calls this function)
        let callers: Vec<(&FunctionId, u64)> = analysis
            .call_edges
            .iter()
            .filter(|((_, callee), _)| callee == &func.id)
            .map(|((caller, _), count)| (caller, *count))
            .collect();

        if !callers.is_empty() {
            let _ = writeln!(out, "**Called by:**");
            for (caller, count) in &callers {
                let _ = writeln!(out, "- `{}` ({})", caller.name, count);
            }
            let _ = writeln!(out);
        }

        // Callees (what this function calls)
        let callees: Vec<(&FunctionId, u64)> = analysis
            .call_edges
            .iter()
            .filter(|((caller, _), _)| caller == &func.id)
            .map(|((_, callee), count)| (callee, *count))
            .collect();

        if !callees.is_empty() {
            let _ = writeln!(out, "**Calls:**");
            for (callee, count) in &callees {
                let _ = writeln!(out, "- `{}` ({})", callee.name, count);
            }
            let _ = writeln!(out);
        }
    }
}

fn format_duration_us(us: f64) -> String {
    if us >= 1_000_000.0 {
        format!("{:.1}s", us / 1_000_000.0)
    } else if us >= 1_000.0 {
        format!("{:.1}ms", us / 1_000.0)
    } else {
        format!("{:.0}us", us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze::analyze, parse::parse_trace};

    #[test]
    fn format_basic_profile() {
        let json = r#"[
            {"ph":"b","pid":1,"ts":0.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1,".file":"src/run.rs",".line":10},
            {"ph":"b","pid":1,"ts":100.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2,".file":"src/hash.rs",".line":20},
            {"ph":"e","pid":1,"ts":300.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2},
            {"ph":"b","pid":1,"ts":310.0,"name":"execute","cat":"turborepo_task_executor","tid":0,"id":3,".file":"src/exec.rs",".line":30},
            {"ph":"e","pid":1,"ts":490.0,"name":"execute","cat":"turborepo_task_executor","tid":0,"id":3},
            {"ph":"e","pid":1,"ts":500.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1}
        ]"#;

        let events = parse_trace(json).unwrap();
        let analysis = analyze(&events);
        let md = format_markdown(&analysis);

        assert!(md.contains("# CPU Profile"));
        assert!(md.contains("## Hot Functions (Self Time)"));
        assert!(md.contains("## Call Tree (Total Time)"));
        assert!(md.contains("`hash`"));
        assert!(md.contains("`run`"));
        assert!(md.contains("`execute`"));
    }
}
