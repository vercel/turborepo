use anyhow::Result;
use serde_json::json;
use turbo_tasks_fs::File;
use turbopack_core::{asset::AssetContentVc, source_map::SourceMapVc};

/// An individual stack frame, as parsed by the stacktrace-parser npm module.
///
/// Line and column can be None if the frame is anonymous.
#[turbo_tasks::value(shared)]
#[derive(Debug)]
pub struct StackFrame {
    pub file: String,
    #[serde(rename = "lineNumber")]
    pub line: Option<usize>,
    pub column: Option<usize>,
    #[serde(rename = "methodName")]
    pub name: Option<String>,
}

impl StackFrame {
    pub fn get_pos(&self) -> Option<(usize, usize)> {
        match (self.line, self.column) {
            (Some(l), Some(c)) => Some((l, c)),
            _ => None,
        }
    }
}

/// Source Map Trace implmements the actual source map tracing logic, by parsing
/// the source map and calling the appropriate methods.
#[turbo_tasks::value(shared)]
#[derive(Debug)]
pub struct SourceMapTrace {
    map: SourceMapVc,
    line: usize,
    column: usize,
    name: Option<String>,
}

/// The result of performing a source map trace.
#[turbo_tasks::value(shared)]
#[derive(Debug)]
pub enum TraceResult {
    NotFound,
    Found(StackFrame),
}

#[turbo_tasks::value_impl]
#[turbo_tasks::function]
impl SourceMapTraceVc {
    #[turbo_tasks::function]
    pub async fn new(map: SourceMapVc, line: usize, column: usize, name: Option<String>) -> Self {
        SourceMapTrace {
            map,
            line,
            column,
            name,
        }
        .cell()
    }

    /// Traces the line/column through the source map into its original
    /// position.
    ///
    /// This method is god-awful slow. We're getting the content
    /// of a .map file, which means we're serializing all of the individual
    /// sections into a string and concatenating, taking that and
    /// deserializing into a DecodedMap, and then querying it. Besides being a
    /// memory hog, it'd be so much faster if we could just directly access
    /// the individual sections of the JS file's map without the
    /// serialization.
    #[turbo_tasks::function]
    pub async fn trace(self) -> Result<TraceResultVc> {
        let this = self.await?;

        let token = this
            .map
            .lookup_token(this.line.saturating_sub(1), this.column)
            .await?;
        let token = match &*token {
            Some(t) if t.has_source() => t,
            _ => return Ok(TraceResult::NotFound.cell()),
        };

        Ok(TraceResult::Found(StackFrame {
            file: token
                .get_source()
                .expect("token was unwraped already")
                .to_string(),
            line: token.get_source_line().map(|l| l.saturating_add(1)),
            column: token.get_source_column(),
            name: token
                .get_name()
                .map(|s| s.to_string())
                .or_else(|| this.name.clone()),
        })
        .cell())
    }

    /// Takes the trace and generates a (possibly valid) JSON asset content.
    #[turbo_tasks::function]
    pub async fn content(self) -> Result<AssetContentVc> {
        let trace = self.trace().await?;
        let result = match &*trace {
            // purposefully invalid JSON (it can't be empty), so that the catch handler will default
            // to the generated stack frame.
            TraceResult::NotFound => "".to_string(),
            TraceResult::Found(frame) => json!({
                "originalStackFrame": frame,
                // TODO
                "originalCodeFrame": null,
            })
            .to_string(),
        };
        Ok(File::from(result).into())
    }
}
