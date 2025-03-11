use std::sync::Arc;

use async_graphql::{Enum, Object, SimpleObject, Union};
use camino::Utf8PathBuf;
use miette::SourceCode;
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{EsSyntax, Syntax, TsSyntax};
use turbo_trace::Tracer;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{
    change_mapper::{ChangeMapper, GlobalDepsPackageChangeMapper},
    package_graph::PackageNode,
};

use crate::{
    query::{package::Package, Array, Diagnostic, Error, PackageChangeReason},
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
            turbo_trace::TraceError::ParseError(path, e) => Diagnostic {
                message: format!("failed to parse file: {:?}", e),
                path: Some(path.to_string()),
                ..Default::default()
            },
            turbo_trace::TraceError::GlobError(err) => Diagnostic {
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
    fn new(result: turbo_trace::TraceResult, run: Arc<Run>) -> Result<Self, Error> {
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

#[derive(SimpleObject)]
struct All {
    reason: PackageChangeReason,
    count: usize,
}

#[derive(Union)]
enum PackageMapping {
    All(All),
    Package(Package),
}

impl File {
    fn get_package(&self) -> Result<Option<PackageMapping>, Error> {
        let change_mapper = ChangeMapper::new(
            self.run.pkg_dep_graph(),
            vec![],
            GlobalDepsPackageChangeMapper::new(
                self.run.pkg_dep_graph(),
                self.run
                    .root_turbo_json()
                    .global_deps
                    .iter()
                    .map(|dep| dep.as_str()),
            )?,
        );

        // If the file is not in the repo, we can't get the package
        let Ok(anchored_path) = self.run.repo_root().anchor(&self.path) else {
            return Ok(None);
        };

        let package = change_mapper
            .package_detector()
            .detect_package(&anchored_path);

        match package {
            turborepo_repository::change_mapper::PackageMapping::All(reason) => {
                Ok(Some(PackageMapping::All(All {
                    reason: reason.into(),
                    count: self.run.pkg_dep_graph().len(),
                })))
            }
            turborepo_repository::change_mapper::PackageMapping::Package((package, _)) => Ok(Some(
                PackageMapping::Package(Package::new(self.run.clone(), package.name.clone())?),
            )),
            turborepo_repository::change_mapper::PackageMapping::None => Ok(None),
        }
    }
}

#[Object]
impl File {
    async fn contents(&self) -> Result<String, Error> {
        Ok(self.path.read_to_string()?)
    }

    // This is `Option` because the file may not be in the repo
    async fn path(&self) -> Option<String> {
        self.run
            .repo_root()
            .anchor(&self.path)
            .ok()
            .map(|path| path.to_string())
    }

    async fn absolute_path(&self) -> String {
        self.path.to_string()
    }

    async fn package(&self) -> Result<Option<PackageMapping>, Error> {
        self.get_package()
    }

    /// Gets the affected packages for the file, i.e. all packages that depend
    /// on the file.
    async fn affected_packages(&self) -> Result<Array<Package>, Error> {
        match self.get_package() {
            Ok(Some(PackageMapping::All(_))) => {
                let mut packages: Vec<_> = self
                    .run
                    .pkg_dep_graph()
                    .packages()
                    .map(|(name, _)| Package::new(self.run.clone(), name.clone()))
                    .collect::<Result<Vec<_>, Error>>()?;

                packages.sort_by(|a, b| a.get_name().cmp(b.get_name()));
                Ok(Array::from(packages))
            }
            Ok(Some(PackageMapping::Package(package))) => {
                let node: PackageNode = PackageNode::Workspace(package.get_name().clone());
                let mut ancestors = self
                    .run
                    .pkg_dep_graph()
                    .ancestors(&node)
                    .iter()
                    .map(|package| {
                        Package::new(self.run.clone(), package.as_package_name().clone())
                    })
                    // Add the package itself to the list
                    .chain(std::iter::once(Ok(package.clone())))
                    .collect::<Result<Vec<_>, Error>>()?;

                ancestors.sort_by(|a, b| a.get_name().cmp(b.get_name()));

                Ok(Array::from(ancestors))
            }
            Ok(None) => Ok(Array::default()),
            Err(e) => Err(e),
        }
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
