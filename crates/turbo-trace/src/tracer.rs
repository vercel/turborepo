use std::{collections::HashSet, fs, rc::Rc};

use camino::Utf8PathBuf;
use miette::{Diagnostic, NamedSource, SourceSpan};
use oxc_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};
use swc_common::{comments::SingleThreadedComments, input::StringInput, FileName, SourceMap};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};

use crate::import_finder::ImportFinder;

pub struct Tracer {
    files: Vec<AbsoluteSystemPathBuf>,
    seen: HashSet<AbsoluteSystemPathBuf>,
    ts_config: Option<AbsoluteSystemPathBuf>,
    source_map: Rc<SourceMap>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum TraceError {
    #[error("failed to read file: {0}")]
    FileNotFound(AbsoluteSystemPathBuf),
    #[error(transparent)]
    PathEncoding(PathError),
    #[error("tracing a root file `{0}`, no parent found")]
    RootFile(AbsoluteSystemPathBuf),
    #[error("failed to resolve import")]
    Resolve {
        #[label("import here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource,
    },
}

pub struct TraceResult {
    pub errors: Vec<TraceError>,
    pub files: HashSet<AbsoluteSystemPathBuf>,
}

impl Tracer {
    pub fn new(
        cwd: AbsoluteSystemPathBuf,
        files: Vec<AbsoluteSystemPathBuf>,
        ts_config: Option<Utf8PathBuf>,
    ) -> Self {
        let ts_config =
            ts_config.map(|ts_config| AbsoluteSystemPathBuf::from_unknown(&cwd, ts_config));

        let seen = HashSet::new();

        Self {
            files,
            seen,
            ts_config,
            source_map: Rc::new(SourceMap::default()),
        }
    }

    pub fn trace(mut self) -> TraceResult {
        let mut options = ResolveOptions::default()
            .with_builtin_modules(true)
            .with_force_extension(EnforceExtension::Disabled)
            .with_extension(".ts")
            .with_extension(".tsx");
        if let Some(ts_config) = self.ts_config.take() {
            options.tsconfig = Some(TsconfigOptions {
                config_file: ts_config.into(),
                references: TsconfigReferences::Auto,
            });
        }

        let resolver = Resolver::new(options);
        let mut errors = vec![];

        while let Some(file_path) = self.files.pop() {
            if matches!(file_path.extension(), Some("json") | Some("css")) {
                continue;
            }

            if self.seen.contains(&file_path) {
                continue;
            }

            self.seen.insert(file_path.clone());

            // Read the file content
            let Ok(file_content) = fs::read_to_string(&file_path) else {
                errors.push(TraceError::FileNotFound(file_path.clone()));
                continue;
            };

            let comments = SingleThreadedComments::default();

            let source_file = self.source_map.new_source_file(
                FileName::Custom(file_path.to_string()).into(),
                file_content.clone(),
            );

            let syntax =
                if file_path.extension() == Some("ts") || file_path.extension() == Some("tsx") {
                    Syntax::Typescript(TsSyntax {
                        tsx: file_path.extension() == Some("tsx"),
                        decorators: true,
                        ..Default::default()
                    })
                } else {
                    Syntax::Es(EsSyntax {
                        jsx: file_path.ends_with(".jsx"),
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
            let Ok(module) = parser.parse_module() else {
                errors.push(TraceError::FileNotFound(file_path.to_owned()));
                continue;
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder::default();
            module.visit_with(&mut finder);

            // Convert found imports/requires to absolute paths and add them to files to
            // visit
            for (import, span) in finder.imports() {
                let Some(file_dir) = file_path.parent() else {
                    errors.push(TraceError::RootFile(file_path.to_owned()));
                    continue;
                };
                match resolver.resolve(file_dir, import) {
                    Ok(resolved) => match resolved.into_path_buf().try_into() {
                        Ok(path) => self.files.push(path),
                        Err(err) => {
                            errors.push(TraceError::PathEncoding(err));
                        }
                    },
                    Err(ResolveError::Builtin(_)) => {}
                    Err(_) => {
                        let (start, end) = self.source_map.span_to_char_offset(&source_file, *span);

                        errors.push(TraceError::Resolve {
                            span: (start as usize, end as usize).into(),
                            text: NamedSource::new(file_path.to_string(), file_content.clone()),
                        });
                    }
                }
            }
        }

        TraceResult {
            files: self.seen,
            errors,
        }
    }
}
