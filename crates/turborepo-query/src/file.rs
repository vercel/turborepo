use std::sync::Arc;

use async_graphql::{Enum, Object, SimpleObject};
use camino::Utf8PathBuf;
use miette::SourceCode;
use turbo_trace::Tracer;
use turbopath::AbsoluteSystemPathBuf;

use crate::{Array, Diagnostic, Error, QueryRun};

pub struct File {
    run: Arc<dyn QueryRun>,
    path: AbsoluteSystemPathBuf,
    ast: Option<serde_json::Value>,
}

impl File {
    pub fn new(run: Arc<dyn QueryRun>, path: AbsoluteSystemPathBuf) -> Result<Self, Error> {
        #[cfg(windows)]
        let path = path.to_realpath()?;

        Ok(Self {
            run,
            path,
            ast: None,
        })
    }

    pub fn with_ast(mut self, ast: Option<serde_json::Value>) -> Self {
        self.ast = ast;
        self
    }

    fn parse_file(&self) -> Result<serde_json::Value, Error> {
        let contents = self.path.read_to_string()?;
        let (_, ast_json) = turbo_trace::parse_file(
            &self.path,
            &contents,
            turbo_trace::ImportTraceType::All,
            true,
        )
        .map_err(Error::Parse)?;
        ast_json.ok_or_else(|| Error::Parse("AST serialization failed".to_string()))
    }
}

impl From<turbo_trace::TraceError> for Diagnostic {
    fn from(error: turbo_trace::TraceError) -> Self {
        let message = error.to_string();
        match error {
            turbo_trace::TraceError::FileNotFound(file) => Diagnostic {
                message,
                path: Some(file.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::PathError(_) => Diagnostic {
                message,
                ..Default::default()
            },
            turbo_trace::TraceError::RootFile(path) => Diagnostic {
                message,
                path: Some(path.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::ParseError(path, msg) => Diagnostic {
                message: format!("failed to parse file: {msg}"),
                path: Some(path.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::GlobError(err) => Diagnostic {
                message: format!("failed to glob files: {err}"),
                ..Default::default()
            },
            turbo_trace::TraceError::Resolve {
                span,
                text,
                file_path,
                reason,
                ..
            } => {
                let import = text
                    .read_span(&span, 1, 1)
                    .ok()
                    .map(|s| String::from_utf8_lossy(s.data()).to_string());

                Diagnostic {
                    message,
                    import,
                    reason: Some(reason.to_string()),
                    path: Some(file_path),
                    start: Some(span.offset()),
                    end: Some(span.offset() + span.len()),
                }
            }
        }
    }
}

#[derive(SimpleObject)]
struct TraceResult {
    files: Array<File>,
    errors: Array<Diagnostic>,
}

impl TraceResult {
    fn new(result: turbo_trace::TraceResult, run: Arc<dyn QueryRun>) -> Result<Self, Error> {
        let mut files = result
            .files
            .into_iter()
            .map(|(path, file)| Ok(File::new(run.clone(), path)?.with_ast(file.ast)))
            .collect::<Result<Vec<_>, Error>>()?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            files: Array::from(files),
            errors: result.errors.into_iter().map(|e| e.into()).collect(),
        })
    }
}

/// The type of imports to trace.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Enum)]
pub enum ImportType {
    /// Trace all imports.
    All,
    /// Trace only `import type` imports
    Types,
    /// Trace only `import` imports and not `import type` imports
    Values,
}

impl From<ImportType> for turbo_trace::ImportTraceType {
    fn from(import_type: ImportType) -> Self {
        match import_type {
            ImportType::All => turbo_trace::ImportTraceType::All,
            ImportType::Types => turbo_trace::ImportTraceType::Types,
            ImportType::Values => turbo_trace::ImportTraceType::Values,
        }
    }
}

#[Object]
impl File {
    async fn contents(&self) -> Result<String, Error> {
        let contents = self.path.read_to_string()?;
        Ok(contents)
    }

    async fn path(&self) -> Result<String, Error> {
        Ok(self
            .run
            .repo_root()
            .anchor(&self.path)
            .map(|path| path.to_string())?)
    }

    async fn absolute_path(&self) -> Result<String, Error> {
        Ok(self.path.to_string())
    }

    async fn dependencies(
        &self,
        depth: Option<usize>,
        ts_config: Option<String>,
        import_type: Option<ImportType>,
        emit_errors: Option<bool>,
    ) -> Result<TraceResult, Error> {
        let mut tracer = Tracer::new(
            self.run.repo_root().to_owned(),
            vec![self.path.clone()],
            ts_config.map(Utf8PathBuf::from),
        );

        if let Some(import_type) = import_type {
            tracer.set_import_type(import_type.into());
        }

        let mut result = tracer.trace(depth).await;
        if emit_errors.unwrap_or(true) {
            result.emit_errors();
        }
        result.files.remove(&self.path);
        TraceResult::new(result, self.run.clone())
    }

    async fn dependents(
        &self,
        ts_config: Option<String>,
        import_type: Option<ImportType>,
    ) -> Result<TraceResult, Error> {
        let mut tracer = Tracer::new(
            self.run.repo_root().to_owned(),
            vec![self.path.clone()],
            ts_config.map(Utf8PathBuf::from),
        );

        if let Some(import_type) = import_type {
            tracer.set_import_type(import_type.into());
        }

        let mut result = tracer.reverse_trace().await;
        result.emit_errors();
        result.files.remove(&self.path);
        TraceResult::new(result, self.run.clone())
    }

    async fn ast(&self) -> Option<serde_json::Value> {
        if let Some(ast) = &self.ast {
            Some(ast.clone())
        } else {
            self.parse_file().ok()
        }
    }
}
