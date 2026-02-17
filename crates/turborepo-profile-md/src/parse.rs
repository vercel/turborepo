use serde::Deserialize;
use serde_json::Value;

/// A single event from a Chromium Trace Event Format JSON array.
///
/// The tracing-chrome crate outputs these with:
/// - `ph`: phase type ("b"/"e" for async begin/end, "i" for instant, "M" for
///   metadata)
/// - `ts`: timestamp in microseconds
/// - `name`: span or event name
/// - `cat`: category (tracing target, e.g. "turborepo_lib::run")
/// - `tid`: thread ID
/// - `id`: async span correlation ID (only for "b"/"e" phases)
/// - `.file` / `.line`: source location (when include_locations is true)
/// - `args`: recorded span fields (when include_args is true)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TraceEvent {
    pub ph: String,
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub ts: Option<f64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cat: Option<String>,
    #[serde(default)]
    pub tid: Option<u64>,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default, rename = ".file")]
    pub file: Option<String>,
    #[serde(default, rename = ".line")]
    pub line: Option<u64>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub s: Option<String>,
}

pub fn parse_trace(contents: &str) -> Result<Vec<TraceEvent>, serde_json::Error> {
    serde_json::from_str(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_async_trace() {
        let json = r#"[
            {"ph":"M","pid":1,"name":"thread_name","tid":0,"args":{"name":"main"}},
            {"ph":"b","pid":1,"ts":100.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1,".file":"src/run/mod.rs",".line":42},
            {"ph":"b","pid":1,"ts":200.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2,".file":"src/lib.rs",".line":10},
            {"ph":"e","pid":1,"ts":350.0,"name":"hash","cat":"turborepo_task_hash","tid":0,"id":2},
            {"ph":"e","pid":1,"ts":500.0,"name":"run","cat":"turborepo_lib::run","tid":0,"id":1}
        ]"#;

        let events = parse_trace(json).unwrap();
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].ph, "M");
        assert_eq!(events[1].ph, "b");
        assert_eq!(events[1].name.as_deref(), Some("run"));
        assert_eq!(events[1].file.as_deref(), Some("src/run/mod.rs"));
        assert_eq!(events[1].line, Some(42));
    }
}
