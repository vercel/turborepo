use std::{collections::HashMap, sync::Arc};

use camino::Utf8PathBuf;
use globwalk::WalkType;
use miette::{Diagnostic, NamedSource, SourceSpan};
use oxc_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};
use swc_common::{comments::SingleThreadedComments, input::StringInput, FileName, SourceMap};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use tokio::task::JoinSet;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};

use crate::import_finder::ImportFinder;

#[derive(Debug, Default)]
pub struct SeenFile {
    pub ast: Option<swc_ecma_ast::Module>,
}

pub struct Tracer {
    files: Vec<(AbsoluteSystemPathBuf, usize)>,
    ts_config: Option<AbsoluteSystemPathBuf>,
    source_map: Arc<SourceMap>,
    cwd: AbsoluteSystemPathBuf,
    errors: Vec<TraceError>,
    import_type: ImportType,
}

#[derive(Debug, Error, Diagnostic)]
pub enum TraceError {
    #[error("failed to parse file {}: {:?}", .0, .1)]
    ParseError(AbsoluteSystemPathBuf, swc_ecma_parser::error::Error),
    #[error("failed to read file: {0}")]
    FileNotFound(AbsoluteSystemPathBuf),
    #[error(transparent)]
    PathEncoding(PathError),
    #[error("tracing a root file `{0}`, no parent found")]
    RootFile(AbsoluteSystemPathBuf),
    #[error("failed to resolve import to `{path}`")]
    Resolve {
        path: String,
        #[label("import here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource,
    },
    #[error("failed to walk files")]
    GlobError(#[from] globwalk::WalkError),
}

pub struct TraceResult {
    pub errors: Vec<TraceError>,
    pub files: HashMap<AbsoluteSystemPathBuf, SeenFile>,
}

/// The type of imports to trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportType {
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
            import_type: ImportType::All,
            errors: Vec::new(),
            source_map: Arc::new(SourceMap::default()),
        }
    }

    pub fn set_import_type(&mut self, import_type: ImportType) {
        self.import_type = import_type;
    }

    #[tracing::instrument(skip(resolver, source_map))]
    pub async fn get_imports_from_file(
        source_map: &SourceMap,
        errors: &mut Vec<TraceError>,
        resolver: &Resolver,
        file_path: &AbsoluteSystemPath,
        import_type: ImportType,
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

        let syntax = if file_path.extension() == Some("ts") || file_path.extension() == Some("tsx")
        {
            Syntax::Typescript(TsSyntax {
                tsx: file_path.extension() == Some("tsx"),
                decorators: true,
                ..Default::default()
            })
        } else {
            Syntax::Es(EsSyntax {
                jsx: true,
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
        for (import, span) in finder.imports() {
            debug!("processing {} in {}", import, file_path);
            let Some(file_dir) = file_path.parent() else {
                errors.push(TraceError::RootFile(file_path.to_owned()));
                continue;
            };
            match resolver.resolve(file_dir, import) {
                Ok(resolved) => {
                    debug!("resolved {:?}", resolved);
                    match resolved.into_path_buf().try_into() {
                        Ok(path) => files.push(path),
                        Err(err) => {
                            errors.push(TraceError::PathEncoding(err));
                            continue;
                        }
                    }
                }
                Err(err @ ResolveError::Builtin { .. }) => {
                    debug!("built in: {:?}", err);
                }
                Err(err) => {
                    debug!("failed to resolve: {:?}", err);
                    let (start, end) = source_map.span_to_char_offset(&source_file, *span);

                    errors.push(TraceError::Resolve {
                        path: import.to_string(),
                        span: (start as usize, end as usize).into(),
                        text: NamedSource::new(file_path.to_string(), file_content.clone()),
                    });
                    continue;
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
        let resolver = root
            .ancestors()
            .skip(1)
            .find(|p| p.join_component("tsconfig.json").exists())
            .map(|ts_config_dir| {
                let mut options = existing_resolver.options().clone();
                options.tsconfig = Some(TsconfigOptions {
                    config_file: ts_config_dir
                        .join_component("tsconfig.json")
                        .as_std_path()
                        .into(),
                    references: TsconfigReferences::Auto,
                });

                existing_resolver.clone_with_options(options)
            });

        root.ancestors()
            .skip(1)
            .find(|p| p.join_component("node_modules").exists())
            .map(|node_modules_dir| {
                let node_modules = node_modules_dir.join_component("node_modules");
                let resolver = resolver.as_ref().unwrap_or(existing_resolver);
                let options = resolver
                    .options()
                    .clone()
                    .with_module(node_modules.to_string());

                resolver.clone_with_options(options)
            })
    }

    pub fn create_resolver(&mut self) -> Resolver {
        let mut options = ResolveOptions::default()
            .with_builtin_modules(true)
            .with_force_extension(EnforceExtension::Disabled)
            .with_extension(".ts")
            .with_extension(".tsx")
            .with_condition_names(&["import", "require"]);

        if let Some(ts_config) = self.ts_config.take() {
            options.tsconfig = Some(TsconfigOptions {
                config_file: ts_config.into(),
                references: TsconfigReferences::Auto,
            });
        }

        Resolver::new(options)
    }

    pub async fn trace(mut self, max_depth: Option<usize>) -> TraceResult {
        let mut seen: HashMap<AbsoluteSystemPathBuf, SeenFile> = HashMap::new();
        let resolver = self.create_resolver();

        while let Some((file_path, file_depth)) = self.files.pop() {
            if let Some(max_depth) = max_depth {
                if file_depth > max_depth {
                    continue;
                }
            }
            self.trace_file(&resolver, file_path, file_depth, &mut seen)
                .await;
        }

        TraceResult {
            files: seen,
            errors: self.errors,
        }
    }

    pub async fn reverse_trace(mut self) -> TraceResult {
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
                    errors: vec![e.into()],
                }
            }
        };

        let mut futures = JoinSet::new();

        let resolver = Arc::new(self.create_resolver());
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

                for import in imported_files {
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
