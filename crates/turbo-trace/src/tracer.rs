use std::{collections::HashSet, fs, sync::Arc};

use camino::Utf8PathBuf;
use miette::{miette, LabeledSpan, NamedSource};
use oxc_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};
use swc_common::{comments::SingleThreadedComments, input::StringInput, FileName, SourceMap};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use turbopath::AbsoluteSystemPathBuf;

use crate::{import_finder::ImportFinder, Error};

pub struct Tracer {
    cwd: AbsoluteSystemPathBuf,
    files: Vec<AbsoluteSystemPathBuf>,
    seen: HashSet<AbsoluteSystemPathBuf>,
    ts_config: Option<AbsoluteSystemPathBuf>,
    source_map: Arc<SourceMap>,
}

pub struct TraceResult {
    pub errors: Vec<String>,
    pub files: HashSet<AbsoluteSystemPathBuf>,
}

impl Tracer {
    pub fn new(
        files: Vec<Utf8PathBuf>,
        cwd: Option<Utf8PathBuf>,
        ts_config: Option<Utf8PathBuf>,
    ) -> Result<Self, Error> {
        let abs_cwd = if let Some(cwd) = cwd {
            AbsoluteSystemPathBuf::from_cwd(cwd)?
        } else {
            AbsoluteSystemPathBuf::cwd()?
        };

        let files = files
            .into_iter()
            .map(|f| AbsoluteSystemPathBuf::from_unknown(&abs_cwd, f))
            .collect();

        let ts_config =
            ts_config.map(|ts_config| AbsoluteSystemPathBuf::from_unknown(&abs_cwd, ts_config));

        let seen = HashSet::new();

        Ok(Self {
            cwd: abs_cwd,
            files,
            seen,
            ts_config,
            source_map: Arc::new(SourceMap::default()),
        })
    }
    pub fn trace(mut self) -> TraceResult {
        let mut options = ResolveOptions::default()
            .with_builtin_modules(true)
            .with_force_extension(EnforceExtension::Disabled)
            .with_extension(".ts");
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
            let file_content = match fs::read_to_string(&file_path) {
                Ok(content) => content,
                Err(err) => {
                    errors.push(format!("failed to read file {}: {}", file_path, err));
                    continue;
                }
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
            let module = match parser.parse_module() {
                Ok(module) => module,
                Err(err) => {
                    errors.push(format!("failed to parse module {}: {:?}", file_path, err));
                    continue;
                }
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder::default();
            module.visit_with(&mut finder);

            // Convert found imports/requires to absolute paths and add them to files to
            // visit
            for (import, span) in finder.imports() {
                let file_dir = file_path.parent().unwrap_or(&self.cwd);
                match resolver.resolve(&file_dir, &import) {
                    Ok(resolved) => match resolved.into_path_buf().try_into() {
                        Ok(path) => self.files.push(path),
                        Err(err) => {
                            errors.push(err.to_string());
                        }
                    },
                    Err(ResolveError::Builtin(_)) => {}
                    Err(err) => {
                        let (start, end) = self.source_map.span_to_char_offset(&source_file, *span);

                        let report = miette!(
                            labels = [LabeledSpan::at(
                                (start as usize)..(end as usize),
                                "import here"
                            )],
                            "failed to resolve import",
                        )
                        .with_source_code(NamedSource::new(
                            file_path.to_string(),
                            file_content.clone(),
                        ));

                        println!("{:?}", report);

                        errors.push(err.to_string());
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
