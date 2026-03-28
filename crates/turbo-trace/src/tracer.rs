use std::{collections::HashMap, fmt, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use globwalk::WalkType;
use miette::{Diagnostic, Report, SourceSpan};
use oxc_allocator::Allocator;
use oxc_estree::{CompactTSSerializer, ESTree};
use oxc_parser::Parser;
use oxc_span::SourceType;
use thiserror::Error;
use tokio::task::JoinSet;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};
use unrs_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};

use crate::import_finder::{self, ImportResult};

#[derive(Debug, Default)]
pub struct SeenFile {
    // We have to add these because of a Rust bug where dead code analysis
    // doesn't work properly in multi-target crates
    // (i.e. crates with both a binary and library)
    // https://github.com/rust-lang/rust/issues/95513
    #[allow(dead_code)]
    pub ast: Option<serde_json::Value>,
}

pub struct Tracer {
    files: Vec<(AbsoluteSystemPathBuf, usize)>,
    ts_config: Option<AbsoluteSystemPathBuf>,
    cwd: AbsoluteSystemPathBuf,
    errors: Vec<TraceError>,
    import_type: ImportTraceType,
}

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum TraceError {
    #[error("failed to parse file {0}: {1}")]
    ParseError(AbsoluteSystemPathBuf, String),
    #[error("failed to read file: {0}")]
    FileNotFound(AbsoluteSystemPathBuf),
    #[error(transparent)]
    PathError(Arc<PathError>),
    #[error("tracing a root file `{0}`, no parent found")]
    RootFile(AbsoluteSystemPathBuf),
    #[error("failed to resolve import to `{import}` in `{file_path}`")]
    Resolve {
        import: String,
        file_path: String,
        #[label("import here")]
        span: SourceSpan,
        #[source_code]
        text: String,
        #[help]
        reason: String,
    },
    #[error("failed to walk files")]
    GlobError(Arc<globwalk::WalkError>),
}

impl TraceResult {
    #[allow(dead_code)]
    pub fn emit_errors(&self) {
        for error in &self.errors {
            eprintln!("{:?}", Report::new(error.clone()));
        }
    }
}

pub struct TraceResult {
    pub errors: Vec<TraceError>,
    pub files: HashMap<AbsoluteSystemPathBuf, SeenFile>,
}

impl fmt::Debug for TraceResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceResult")
            .field("files", &self.files)
            .field("errors", &self.errors)
            .finish()
    }
}

/// The type of imports to trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ImportTraceType {
    /// Trace all imports.
    All,
    /// Trace only `import type` imports
    Types,
    /// Trace only `import` imports and not `import type` imports
    Values,
}

/// Parse a file with oxc and extract imports.
///
/// Returns the list of found imports and optionally the serialized AST (as
/// ESTree JSON). The AST is serialized eagerly while the oxc allocator is
/// alive, since the parsed `Program<'a>` cannot outlive it.
pub fn parse_file(
    file_path: &AbsoluteSystemPath,
    file_content: &str,
    import_type: ImportTraceType,
    include_ast: bool,
) -> Result<(Vec<ImportResult>, Option<serde_json::Value>), String> {
    let source_type = SourceType::from_path(file_path.as_std_path()).unwrap_or_default();

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, file_content, source_type).parse();

    if ret.panicked {
        let messages: Vec<String> = ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(messages.join(", "));
    }

    let imports = import_finder::find_imports(&ret.module_record, &ret.program.body, import_type);

    let ast_json = if include_ast {
        let mut serializer = CompactTSSerializer::new(false);
        ret.program.serialize(&mut serializer);
        let json_string = serializer.into_string();
        serde_json::from_str(&json_string).ok()
    } else {
        None
    };

    Ok((imports, ast_json))
}

impl Tracer {
    pub fn new(
        cwd: AbsoluteSystemPathBuf,
        files: Vec<AbsoluteSystemPathBuf>,
        ts_config: Option<Utf8PathBuf>,
    ) -> Self {
        let ts_config =
            ts_config.map(|ts_config| AbsoluteSystemPathBuf::from_unknown(&cwd, ts_config));

        let files = files.into_iter().map(|file| (file, 0)).collect::<Vec<_>>();

        Self {
            files,
            ts_config,
            cwd,
            import_type: ImportTraceType::All,
            errors: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn set_import_type(&mut self, import_type: ImportTraceType) {
        self.import_type = import_type;
    }

    #[tracing::instrument(skip(errors))]
    pub async fn get_imports_from_file(
        errors: &mut Vec<TraceError>,
        resolver: &Resolver,
        file_path: &AbsoluteSystemPath,
        import_type: ImportTraceType,
    ) -> Option<(Vec<AbsoluteSystemPathBuf>, SeenFile)> {
        let Ok(file_content) = tokio::fs::read_to_string(&file_path).await else {
            errors.push(TraceError::FileNotFound(file_path.to_owned()));
            return None;
        };

        let (imports, ast_json) = match parse_file(file_path, &file_content, import_type, true) {
            Ok(result) => result,
            Err(msg) => {
                errors.push(TraceError::ParseError(file_path.to_owned(), msg));
                return None;
            }
        };

        let mut files = Vec::new();
        for ImportResult {
            specifier: import,
            span,
            ..
        } in &imports
        {
            debug!("processing {} in {}", import, file_path);
            let Some(file_dir) = file_path.parent() else {
                errors.push(TraceError::RootFile(file_path.to_owned()));
                continue;
            };
            match resolver.resolve(file_dir, import) {
                Ok(resolved) => {
                    debug!("resolved {:?}", resolved);
                    match resolved.into_path_buf().try_into().map_err(Arc::new) {
                        Ok(path) => files.push(path),
                        Err(err) => {
                            errors.push(TraceError::PathError(err));
                            continue;
                        }
                    }
                }
                Err(err @ ResolveError::Builtin { .. }) => {
                    debug!("built in: {:?}", err);
                }
                Err(err) => {
                    if !import.starts_with(".") {
                        let type_package = format!("@types/{import}");
                        debug!("trying to resolve type import: {type_package}");
                        let resolved_type_import = resolver
                            .resolve(file_dir, type_package.as_str())
                            .ok()
                            .and_then(|resolved| resolved.into_path_buf().try_into().ok());

                        if let Some(resolved_type_import) = resolved_type_import {
                            debug!("resolved type import succeeded");
                            files.push(resolved_type_import);
                            continue;
                        }
                    }

                    let without_extension = Utf8Path::new(import).with_extension("");
                    debug!(
                        "trying to resolve extensionless import: {}",
                        without_extension
                    );
                    let resolved_extensionless_import = resolver
                        .resolve(file_dir, without_extension.as_str())
                        .ok()
                        .and_then(|resolved| resolved.into_path_buf().try_into().ok());

                    if let Some(resolved_extensionless_import) = resolved_extensionless_import {
                        debug!("resolved extensionless import succeeded");
                        files.push(resolved_extensionless_import);
                        continue;
                    }

                    debug!("failed to resolve: {:?}", err);
                    let start = span.start as usize;
                    let end = span.end as usize;

                    errors.push(TraceError::Resolve {
                        import: import.to_string(),
                        file_path: file_path.to_string(),
                        span: SourceSpan::new(start.into(), end - start),
                        text: file_content.clone(),
                        reason: err.to_string(),
                    });
                }
            }
        }

        Some((files, SeenFile { ast: ast_json }))
    }

    pub async fn trace_file(
        &mut self,
        resolver: &Resolver,
        file_path: AbsoluteSystemPathBuf,
        depth: usize,
        seen: &mut HashMap<AbsoluteSystemPathBuf, SeenFile>,
    ) {
        let file_resolver = Self::infer_resolver_with_ts_config(&file_path, resolver);
        let resolver = file_resolver.as_ref().unwrap_or(resolver);

        if seen.contains_key(&file_path) {
            return;
        }

        let entry = seen.entry(file_path.clone()).or_default();

        if matches!(file_path.extension(), Some("css") | Some("json")) {
            return;
        }

        let Some((imports, seen_file)) =
            Self::get_imports_from_file(&mut self.errors, resolver, &file_path, self.import_type)
                .await
        else {
            return;
        };

        *entry = seen_file;

        self.files
            .extend(imports.into_iter().map(|import| (import, depth + 1)));
    }

    /// Attempts to find the closest tsconfig and creates a resolver with it,
    /// so alias resolution, e.g. `@/foo/bar`, works.
    fn infer_resolver_with_ts_config(
        root: &AbsoluteSystemPath,
        existing_resolver: &Resolver,
    ) -> Option<Resolver> {
        let tsconfig_dir = root
            .ancestors()
            .skip(1)
            .find(|p| p.join_component("tsconfig.json").exists());

        let node_modules_dir = root
            .ancestors()
            .skip(1)
            .find(|p| p.join_component("node_modules").exists());

        if tsconfig_dir.is_some() || node_modules_dir.is_some() {
            let mut options = existing_resolver.options().clone();
            if let Some(tsconfig_dir) = tsconfig_dir {
                options.tsconfig = Some(TsconfigOptions {
                    config_file: tsconfig_dir
                        .join_component("tsconfig.json")
                        .as_std_path()
                        .into(),
                    references: TsconfigReferences::Auto,
                });
            }

            if let Some(node_modules_dir) = node_modules_dir {
                options = options
                    .with_module(node_modules_dir.join_component("node_modules").to_string());
            }

            Some(existing_resolver.clone_with_options(options))
        } else {
            None
        }
    }

    pub fn create_resolver(ts_config: Option<&AbsoluteSystemPath>) -> Resolver {
        let mut options = ResolveOptions::default()
            .with_builtin_modules(true)
            .with_force_extension(EnforceExtension::Disabled)
            .with_extension(".ts")
            .with_extension(".tsx")
            .with_extension(".jsx")
            .with_extension(".d.ts")
            .with_extension(".mjs")
            .with_extension(".cjs")
            .with_main_field("module")
            .with_main_field("types")
            .with_condition_names(&["import", "require", "node", "types", "default"]);

        if let Some(ts_config) = ts_config {
            options.tsconfig = Some(TsconfigOptions {
                config_file: ts_config.as_std_path().into(),
                references: TsconfigReferences::Auto,
            });
        }

        Resolver::new(options)
    }

    pub async fn trace(mut self, max_depth: Option<usize>) -> TraceResult {
        let mut seen: HashMap<AbsoluteSystemPathBuf, SeenFile> = HashMap::new();
        let resolver = Self::create_resolver(self.ts_config.as_deref());

        while let Some((file_path, file_depth)) = self.files.pop() {
            if let Some(max_depth) = max_depth
                && file_depth > max_depth
            {
                continue;
            }
            self.trace_file(&resolver, file_path, file_depth, &mut seen)
                .await;
        }

        TraceResult {
            files: seen,
            errors: self.errors,
        }
    }

    pub async fn reverse_trace(self) -> TraceResult {
        let files = match globwalk::globwalk(
            &self.cwd,
            &[
                "**/*.js".parse().expect("valid glob"),
                "**/*.jsx".parse().expect("valid glob"),
                "**/*.ts".parse().expect("valid glob"),
                "**/*.tsx".parse().expect("valid glob"),
            ],
            &[
                "**/node_modules/**".parse().expect("valid glob"),
                "**/.next/**".parse().expect("valid glob"),
            ],
            WalkType::Files,
        ) {
            Ok(files) => files,
            Err(e) => {
                return TraceResult {
                    files: HashMap::new(),
                    errors: vec![TraceError::GlobError(Arc::new(e))],
                };
            }
        };

        let mut futures = JoinSet::new();

        let resolver = Arc::new(Self::create_resolver(self.ts_config.as_deref()));
        let shared_self = Arc::new(self);

        for file in files {
            let shared_self = shared_self.clone();
            let resolver = resolver.clone();
            futures.spawn(async move {
                let file_resolver = Self::infer_resolver_with_ts_config(&file, &resolver);
                let resolver = file_resolver.as_ref().unwrap_or(&resolver);
                let mut errors = Vec::new();

                let Some((imported_files, seen_file)) = Self::get_imports_from_file(
                    &mut errors,
                    resolver,
                    &file,
                    shared_self.import_type,
                )
                .await
                else {
                    return (errors, None);
                };

                for mut import in imported_files {
                    if cfg!(windows) {
                        match import.to_realpath() {
                            Ok(path) => {
                                import = path;
                            }
                            Err(err) => {
                                errors.push(TraceError::PathError(Arc::new(err)));
                                return (errors, None);
                            }
                        }
                    }

                    if shared_self
                        .files
                        .iter()
                        .any(|(source, _)| import.as_path() == source.as_path())
                    {
                        return (errors, Some((file, seen_file)));
                    }
                }

                (errors, None)
            });
        }

        let mut usages = HashMap::new();
        let mut errors = Vec::new();

        while let Some(result) = futures.join_next().await {
            let (errs, file) = result.unwrap();
            errors.extend(errs);

            if let Some((path, seen_file)) = file {
                usages.insert(path, seen_file);
            }
        }

        TraceResult {
            files: usages,
            errors,
        }
    }
}
