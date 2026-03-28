mod analyze;
mod format;
mod parse;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read trace file: {0}")]
    ReadFile(#[from] std::io::Error),
    #[error("failed to parse trace JSON: {0}")]
    ParseJson(#[from] serde_json::Error),
}

/// Reads a Chromium Trace Event Format JSON file produced by `turbo run
/// --profile` and writes an LLM-friendly markdown summary alongside it.
///
/// The output markdown contains:
/// - Summary table (duration, span count, unique functions)
/// - Top N hottest functions
/// - Hot Functions table sorted by self-time
/// - Call Tree table sorted by total-time
/// - Function Details with caller/callee relationships
pub fn trace_to_markdown(trace_path: &Path, output_path: &Path) -> Result<(), Error> {
    let contents = std::fs::read_to_string(trace_path)?;
    let md = trace_contents_to_markdown(&contents)?;
    std::fs::write(output_path, md)?;
    Ok(())
}

/// Same as `trace_to_markdown` but operates on the trace file contents
/// directly, returning the markdown string rather than writing to a file.
pub fn trace_contents_to_markdown(contents: &str) -> Result<String, Error> {
    let events = parse::parse_trace(contents)?;
    let analysis = analyze::analyze(&events);
    Ok(format::format_markdown(&analysis))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end() {
        let json = r#"[
            {"ph":"M","pid":1,"name":"thread_name","tid":0,"args":{"name":"main"}},
            {"ph":"b","pid":1,"ts":0.0,"name":"build","cat":"turborepo_lib::run","tid":0,"id":1,".file":"crates/turborepo-lib/src/run/builder.rs",".line":194},
            {"ph":"b","pid":1,"ts":50.0,"name":"resolve_packages","cat":"turborepo_lib::run","tid":0,"id":2,".file":"crates/turborepo-lib/src/run/mod.rs",".line":189},
            {"ph":"e","pid":1,"ts":150.0,"name":"resolve_packages","cat":"turborepo_lib::run","tid":0,"id":2},
            {"ph":"b","pid":1,"ts":200.0,"name":"calculate_hashes","cat":"turborepo_task_hash","tid":0,"id":3,".file":"crates/turborepo-task-hash/src/lib.rs",".line":104},
            {"ph":"b","pid":1,"ts":220.0,"name":"calculate_file_hash","cat":"turborepo_task_hash","tid":0,"id":4,".file":"crates/turborepo-task-hash/src/lib.rs",".line":320},
            {"ph":"e","pid":1,"ts":350.0,"name":"calculate_file_hash","cat":"turborepo_task_hash","tid":0,"id":4},
            {"ph":"e","pid":1,"ts":400.0,"name":"calculate_hashes","cat":"turborepo_task_hash","tid":0,"id":3},
            {"ph":"b","pid":1,"ts":410.0,"name":"execute_task","cat":"turborepo_task_executor::exec","tid":0,"id":5,".file":"crates/turborepo-task-executor/src/exec.rs",".line":272,"args":{"task":"web#build"}},
            {"ph":"e","pid":1,"ts":900.0,"name":"execute_task","cat":"turborepo_task_executor::exec","tid":0,"id":5},
            {"ph":"e","pid":1,"ts":1000.0,"name":"build","cat":"turborepo_lib::run","tid":0,"id":1}
        ]"#;

        let md = trace_contents_to_markdown(json).unwrap();

        // Verify structure
        assert!(md.contains("# CPU Profile"));
        assert!(md.contains("| Duration | Spans | Functions |"));
        assert!(md.contains("1.0ms")); // total duration ~1000us
        assert!(md.contains("## Hot Functions (Self Time)"));
        assert!(md.contains("## Call Tree (Total Time)"));
        assert!(md.contains("`execute_task`"));
        assert!(md.contains("`build`"));
    }

    #[test]
    fn file_round_trip() {
        let json = r#"[
            {"ph":"b","pid":1,"ts":0.0,"name":"run","cat":"test","tid":0,"id":1},
            {"ph":"e","pid":1,"ts":1000.0,"name":"run","cat":"test","tid":0,"id":1}
        ]"#;

        let dir = tempfile::tempdir().unwrap();
        let trace_path = dir.path().join("test.trace");
        let md_path = dir.path().join("test.trace.md");

        std::fs::write(&trace_path, json).unwrap();
        trace_to_markdown(&trace_path, &md_path).unwrap();

        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("# CPU Profile"));
        assert!(md.contains("`run`"));
    }
}
