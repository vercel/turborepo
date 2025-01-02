use std::sync::Arc;

use async_graphql::{Enum, Object, SimpleObject};
use camino::Utf8PathBuf;
use itertools::Itertools;
use miette::SourceCode;
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{EsSyntax, Syntax, TsSyntax};
use turbo_trace::Tracer;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    query::{Array, Error},
    run::Run,
};

pub struct File {
    run: Arc<Run>,
    path: AbsoluteSystemPathBuf,
    ast: Option<swc_ecma_ast::Module>,
}

impl File {
    pub fn new(run: Arc<Run>, path: AbsoluteSystemPathBuf) -> Result<Self, Error> {
        #[cfg(windows)]
        let path = path.to_realpath()?;

        Ok(Self {
            run,
            path,
            ast: None,
        })
    }

    pub fn with_ast(mut self, ast: Option<swc_ecma_ast::Module>) -> Self {
        self.ast = ast;

        self
    }

    fn parse_file(&self) -> Result<swc_ecma_ast::Module, Error> {
        let contents = self.path.read_to_string()?;
        let source_map = swc_common::SourceMap::default();
        let file = source_map.new_source_file(
            swc_common::FileName::Custom(self.path.to_string()).into(),
            contents.clone(),
        );
        let syntax = if self.path.extension() == Some("ts") || self.path.extension() == Some("tsx")
        {
            Syntax::Typescript(TsSyntax {
                tsx: self.path.extension() == Some("tsx"),
                decorators: true,
                ..Default::default()
            })
        } else {
            Syntax::Es(EsSyntax {
                jsx: self.path.ends_with(".jsx"),
                ..Default::default()
            })
        };
        let comments = swc_common::comments::SingleThreadedComments::default();
        let mut errors = Vec::new();
        let module = swc_ecma_parser::parse_file_as_module(
            &file,
            syntax,
            EsVersion::EsNext,
            Some(&comments),
            &mut errors,
        )
        .map_err(Error::Parse)?;

        Ok(module)
    }
}

#[derive(SimpleObject, Debug, Default)]
pub struct TraceError {
    message: String,
    reason: String,
    path: Option<String>,
    import: Option<String>,
    start: Option<usize>,
    end: Option<usize>,
}

impl From<turbo_trace::TraceError> for TraceError {
    fn from(error: turbo_trace::TraceError) -> Self {
        let message = error.to_string();
        match error {
            turbo_trace::TraceError::FileNotFound(file) => TraceError {
                message,
                path: Some(file.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::PathEncoding(_) => TraceError {
                message,
                ..Default::default()
            },
            turbo_trace::TraceError::RootFile(path) => TraceError {
                message,
                path: Some(path.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::ParseError(path, e) => TraceError {
                message: format!("failed to parse file: {:?}", e),
                path: Some(path.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::GlobError(err) => TraceError {
                message: format!("failed to glob files: {}", err),
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

                TraceError {
                    message,
                    import,
                    reason,
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
    errors: Array<TraceError>,
}

impl TraceResult {
    fn new(result: turbo_trace::TraceResult, run: Arc<Run>) -> Result<Self, Error> {
        Ok(Self {
            files: result
                .files
                .into_iter()
                .sorted_by(|a, b| a.0.cmp(&b.0))
                .map(|(path, file)| Ok(File::new(run.clone(), path)?.with_ast(file.ast)))
                .collect::<Result<_, Error>>()?,
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

impl From<ImportType> for turbo_trace::ImportType {
    fn from(import_type: ImportType) -> Self {
        match import_type {
            ImportType::All => turbo_trace::ImportType::All,
            ImportType::Types => turbo_trace::ImportType::Types,
            ImportType::Values => turbo_trace::ImportType::Values,
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
        // Remove the file itself from the result
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
        // Remove the file itself from the result
        result.files.remove(&self.path);
        TraceResult::new(result, self.run.clone())
    }

    async fn ast(&self) -> Option<serde_json::Value> {
        if let Some(ast) = &self.ast {
            serde_json::to_value(ast).ok()
        } else {
            serde_json::to_value(&self.parse_file().ok()?).ok()
        }
    }
}
