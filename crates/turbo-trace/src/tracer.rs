use std::{collections::HashMap, fmt, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use globwalk::WalkType;
use miette::{Diagnostic, Report, SourceSpan};
use oxc_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};
use swc_common::{
    FileName, SourceMap,
    comments::SingleThreadedComments,
    errors::{ColorConfig, Handler},
    input::StringInput,
};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{Capturing, EsSyntax, Parser, Syntax, TsSyntax, lexer::Lexer};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use tokio::task::JoinSet;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};

use crate::import_finder::ImportFinder;

#[derive(Debug, Default)]
pub struct SeenFile {
    // We have to add these because of a Rust bug where dead code analysis
    // doesn't work properly in multi-target crates
    // (i.e. crates with both a binary and library)
    // https://github.com/rust-lang/rust/issues/95513
    #[allow(dead_code)]
    pub ast: Option<swc_ecma_ast::Module>,
}

pub struct Tracer {
    files: Vec<(AbsoluteSystemPathBuf, usize)>,
    ts_config: Option<AbsoluteSystemPathBuf>,
    source_map: Arc<SourceMap>,
    cwd: AbsoluteSystemPathBuf,
    errors: Vec<TraceError>,
    import_type: ImportTraceType,
}

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum TraceError {
    #[error("failed to parse file {}: {:?}", .0, .1)]
    ParseError(AbsoluteSystemPathBuf, swc_ecma_parser::error::Error),
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
        let handler = Handler::with_tty_emitter(
            ColorConfig::Auto,
            true,
            false,
            Some(self.source_map.clone()),
        );
        for error in &self.errors {
            match error {
                TraceError::ParseError(_, e) => {
                    e.clone().into_diagnostic(&handler).emit();
                }
                e => {
                    eprintln!("{:?}", Report::new(e.clone()));
                }
            }
        }
    }
}

pub struct TraceResult {
    #[allow(dead_code)]
    source_map: Arc<SourceMap>,
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
            source_map: Arc::new(SourceMap::default()),
        }
    }

    #[allow(dead_code)]
    pub fn set_import_type(&mut self, import_type: ImportTraceType) {
        self.import_type = import_type;
    }

    #[tracing::instrument(skip(resolver, source_map))]
    pub async fn get_imports_from_file(
        source_map: &SourceMap,
        errors: &mut Vec<TraceError>,
        resolver: &Resolver,
        file_path: &AbsoluteSystemPath,
        import_type: ImportTraceType,
    ) -> Option<(Vec<AbsoluteSystemPathBuf>, SeenFile)> {
        // Read the file content
        let Ok(file_content) = tokio::fs::read_to_string(&file_path).await else {
            errors.push(TraceError::FileNotFound(file_path.to_owned()));
            return None;
        };

        let comments = SingleThreadedComments::default();

        let source_file = source_map.new_source_file(
            FileName::Custom(file_path.to_string()).into(),
            file_content.clone(),
        );

        let syntax = if matches!(file_path.extension(), Some("ts") | Some("tsx")) {
            Syntax::Typescript(TsSyntax {
                tsx: file_path.extension() == Some("tsx"),
                decorators: true,
                ..Default::default()
            })
        } else {
            Syntax::Es(EsSyntax {
                jsx: true,
                import_attributes: true,
                ..Default::default()
            })
        };

        let lexer = Lexer::new(
            syntax,
            EsVersion::EsNext,
            StringInput::from(&*source_file),
            Some(&comments),
        );

        let mut parser = Parser::new_from(Capturing::new(lexer));

        // Parse the file as a module
        let module = match parser.parse_module() {
            Ok(module) => module,
            Err(err) => {
                errors.push(TraceError::ParseError(file_path.to_owned(), err));
                return None;
            }
        };

        // Visit the AST and find imports
        let mut finder = ImportFinder::new(import_type);
        module.visit_with(&mut finder);
        // Convert found imports/requires to absolute paths and add them to files to
        // visit
        let mut files = Vec::new();
        for (import, span, _) in finder.imports() {
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
                        // Try to resolve the import as a type import via `@/types/<import>`
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

                    // Also try without the extension just in case the wrong extension is used
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
                    let (start, end) = source_map.span_to_char_offset(&source_file, *span);
                    let start = start as usize;
                    let end = end as usize;

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

        Some((files, SeenFile { ast: Some(module) }))
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

        let Some((imports, seen_file)) = Self::get_imports_from_file(
            &self.source_map,
            &mut self.errors,
            resolver,
            &file_path,
            self.import_type,
        )
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

        // Resolves the closest `node_modules` directory. This is to work with monorepos
        // where both the package and the monorepo have a `node_modules`
        // directory.
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
            // Some packages export a `module` field instead of `main`. This is non-standard,
            // but was a proposal at some point.
            .with_main_field("module")
            .with_main_field("types")
            // Condition names are used to determine which export to use when importing a module.
            // We add a bunch so oxc_resolver can resolve all kinds of imports.
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
            source_map: self.source_map.clone(),
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
                    source_map: self.source_map.clone(),
                    files: HashMap::new(),
                    errors: vec![TraceError::GlobError(Arc::new(e))],
                };
            }
        };

        let mut futures = JoinSet::new();

        let resolver = Arc::new(Self::create_resolver(self.ts_config.as_deref()));
        let source_map = self.source_map.clone();
        let shared_self = Arc::new(self);

        for file in files {
            let shared_self = shared_self.clone();
            let resolver = resolver.clone();
            futures.spawn(async move {
                let file_resolver = Self::infer_resolver_with_ts_config(&file, &resolver);
                let resolver = file_resolver.as_ref().unwrap_or(&resolver);
                let mut errors = Vec::new();

                let Some((imported_files, seen_file)) = Self::get_imports_from_file(
                    &shared_self.source_map,
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
                    // Windows has this annoying habit of abbreviating paths
                    // like `C:\Users\Admini~1` instead of `C:\Users\Administrator`
                    // We canonicalize to get the proper, full length path
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
            source_map,
            files: usages,
            errors,
        }
    }
}
