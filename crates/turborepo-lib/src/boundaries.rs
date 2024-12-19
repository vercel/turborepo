use std::collections::{BTreeMap, HashSet};

use git2::Repository;
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use oxc_resolver::{ResolveError, Resolver};
use swc_common::{comments::SingleThreadedComments, input::StringInput, FileName, SourceMap};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use turbo_trace::{ImportFinder, Tracer};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::run::Run;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("cannot import package `{name}` because it is not a dependency")]
    PackageNotFound {
        name: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource,
    },
    #[error(transparent)]
    GlobWalk(#[from] globwalk::WalkError),
    #[error("failed to read file: {0}")]
    FileNotFound(AbsoluteSystemPathBuf),
    #[error("failed to parse file {0}")]
    ParseError(AbsoluteSystemPathBuf, swc_ecma_parser::error::Error),
}

impl Run {
    pub async fn check_boundaries(&self) -> Result<(), Error> {
        let packages = self.pkg_dep_graph().packages();
        let repo = Repository::discover(&self.repo_root()).ok();
        println!(
            "{:?}",
            self.pkg_dep_graph()
                .package_info(&PackageName::Other("@turbo/gen".to_string()))
                .unwrap()
                .unresolved_external_dependencies
        );
        for (package_name, package_info) in packages {
            let package_root = self.repo_root().resolve(package_info.package_path());
            let internal_dependencies = self
                .pkg_dep_graph()
                .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
                .unwrap_or_default();
            let external_dependencies = package_info.unresolved_external_dependencies.as_ref();
            self.check_package(
                &repo,
                &package_root,
                internal_dependencies,
                external_dependencies,
            )
            .await?;
        }
        Ok(())
    }
    async fn check_package(
        &self,
        repo: &Option<Repository>,
        package_root: &AbsoluteSystemPath,
        internal_dependencies: HashSet<&PackageNode>,
        external_dependencies: Option<&BTreeMap<String, String>>,
    ) -> Result<(), Error> {
        let files = globwalk::globwalk(
            package_root,
            &[
                "**/*.js".parse().unwrap(),
                "**/*.jsx".parse().unwrap(),
                "**/*.ts".parse().unwrap(),
                "**/*.tsx".parse().unwrap(),
                "**/*.vue".parse().unwrap(),
                "**/*.svelte".parse().unwrap(),
            ],
            &[
                "**/node_modules/**".parse().unwrap(),
                "**/examples/**".parse().unwrap(),
            ],
            globwalk::WalkType::Files,
        )?;

        let source_map = SourceMap::default();
        let mut errors: Vec<Error> = Vec::new();
        // TODO: Load tsconfig.json
        let resolver = Tracer::create_resolver(None);

        for file_path in files {
            if let Some(repo) = repo {
                if matches!(repo.status_should_ignore(file_path.as_std_path()), Ok(true)) {
                    continue;
                }
            };
            // Read the file content
            let Ok(file_content) = tokio::fs::read_to_string(&file_path).await else {
                errors.push(Error::FileNotFound(file_path.to_owned()));
                continue;
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
                    errors.push(Error::ParseError(file_path.to_owned(), err));
                    continue;
                }
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder::default();
            module.visit_with(&mut finder);
            for (import, span) in finder.imports() {
                let (start, end) = source_map.span_to_char_offset(&source_file, *span);
                let start = start as usize;
                let end = end as usize;
                let span = SourceSpan::new(start.into(), (end - start).into());

                // We have a file import
                let check_result = if import.starts_with(".") {
                    self.check_file_import(&import)
                } else {
                    self.check_package_import(
                        &import,
                        span,
                        &file_path,
                        &file_content,
                        &internal_dependencies,
                        external_dependencies,
                        &resolver,
                    )
                };

                if let Err(err) = check_result {
                    errors.push(err);
                }
            }
        }

        for error in errors {
            println!("{:?}", Report::new(error));
        }

        Ok(())
    }

    fn check_file_import(&self, import: &str) -> Result<(), Error> {
        Ok(())
    }

    fn check_package_import(
        &self,
        import: &str,
        span: SourceSpan,
        file_path: &AbsoluteSystemPath,
        file_content: &str,
        internal_dependencies: &HashSet<&PackageNode>,
        external_dependencies: Option<&BTreeMap<String, String>>,
        resolver: &Resolver,
    ) -> Result<(), Error> {
        let package_name = if import.starts_with("@") {
            import.split('/').take(2).join("/")
        } else {
            import
                .split_once("/")
                .map(|(import, _)| import)
                .unwrap_or(import)
                .to_string()
        };
        let package_name = PackageNode::Workspace(PackageName::Other(package_name));
        let folder = file_path.parent().expect("file_path should have a parent");
        let contained_in_dependencies = internal_dependencies.contains(&package_name)
            || external_dependencies.map_or(false, |external_dependencies| {
                external_dependencies.contains_key(&package_name.to_string())
            });

        if !contained_in_dependencies
            && !matches!(
                resolver.resolve(folder, import),
                Err(ResolveError::Builtin { .. })
            )
        {
            return Err(Error::PackageNotFound {
                name: package_name.to_string(),
                span,
                text: NamedSource::new(file_path.as_str(), file_content.to_string()),
            });
        }

        Ok(())
    }
}
