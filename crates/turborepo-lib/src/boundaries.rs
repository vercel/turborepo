use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
};

use git2::Repository;
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use oxc_resolver::{ResolveError, Resolver};
use regex::Regex;
use swc_common::{
    comments::SingleThreadedComments,
    errors::{ColorConfig, Handler},
    input::StringInput,
    FileName, SourceMap,
};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use turbo_trace::{ImportFinder, ImportType, Tracer};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, PathRelation};
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};

use crate::run::Run;

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum BoundariesDiagnostic {
    #[error(
        "importing from a type declaration package, but import is not declared as a type-only \
         import"
    )]
    #[help("add `type` to the import declaration")]
    NotTypeOnlyImport {
        import: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: Arc<NamedSource>,
    },
    #[error("cannot import package `{name}` because it is not a dependency")]
    PackageNotFound {
        name: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: Arc<NamedSource>,
    },
    #[error("cannot import file `{import}` because it leaves the package")]
    ImportLeavesPackage {
        import: String,
        #[label("file imported here")]
        span: SourceSpan,
        #[source_code]
        text: Arc<NamedSource>,
    },
    #[error("failed to parse file {0}")]
    ParseError(AbsoluteSystemPathBuf, swc_ecma_parser::error::Error),
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("file `{0}` does not have a parent directory")]
    NoParentDir(AbsoluteSystemPathBuf),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Lockfiles(#[from] turborepo_lockfiles::Error),
    #[error(transparent)]
    GlobWalk(#[from] globwalk::WalkError),
    #[error("failed to read file: {0}")]
    FileNotFound(AbsoluteSystemPathBuf),
}

static PACKAGE_NAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(@[a-z0-9-~][a-z0-9-._~]*\/)?[a-z0-9-~][a-z0-9-._~]*$").unwrap()
});

pub struct BoundariesResult {
    pub source_map: Arc<SourceMap>,
    pub diagnostics: Vec<BoundariesDiagnostic>,
}

impl BoundariesResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn emit(&self) {
        let handler = Handler::with_tty_emitter(
            ColorConfig::Auto,
            true,
            false,
            Some(self.source_map.clone()),
        );

        for diagnostic in self.diagnostics.clone() {
            match diagnostic {
                BoundariesDiagnostic::ParseError(_, e) => {
                    e.clone().into_diagnostic(&handler).emit();
                }
                e => {
                    eprintln!("{:?}", Report::new(e.to_owned()));
                }
            }
        }
    }
}

impl Run {
    pub async fn check_boundaries(&self) -> Result<BoundariesResult, Error> {
        let packages = self.pkg_dep_graph().packages();
        let repo = Repository::discover(&self.repo_root()).ok().map(Mutex::new);
        let mut diagnostics = vec![];
        let source_map = SourceMap::default();
        for (package_name, package_info) in packages {
            if !self.filtered_pkgs().contains(package_name)
                || matches!(package_name, PackageName::Root)
            {
                continue;
            }

            let package_root = self.repo_root().resolve(package_info.package_path());

            let internal_dependencies = self
                .pkg_dep_graph()
                .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
                .unwrap_or_default();
            let unresolved_external_dependencies =
                package_info.unresolved_external_dependencies.as_ref();

            let package_diagnostics = self
                .check_package(
                    &repo,
                    &package_root,
                    &package_info.package_json,
                    internal_dependencies,
                    unresolved_external_dependencies,
                    &source_map,
                )
                .await?;

            diagnostics.extend(package_diagnostics);
        }

        Ok(BoundariesResult {
            source_map: Arc::new(source_map),
            diagnostics,
        })
    }

    fn is_potential_package_name(import: &str) -> bool {
        PACKAGE_NAME_REGEX.is_match(import)
    }

    /// Either returns a list of errors or a single, fatal error
    async fn check_package(
        &self,
        repo: &Option<Mutex<Repository>>,
        package_root: &AbsoluteSystemPath,
        package_json: &PackageJson,
        internal_dependencies: HashSet<&PackageNode>,
        unresolved_external_dependencies: Option<&BTreeMap<String, String>>,
        source_map: &SourceMap,
    ) -> Result<Vec<BoundariesDiagnostic>, Error> {
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

        let mut diagnostics: Vec<BoundariesDiagnostic> = Vec::new();
        // We assume the tsconfig.json is at the root of the package
        let tsconfig_path = package_root.join_component("tsconfig.json");

        // TODO: Load tsconfig.json
        let resolver =
            Tracer::create_resolver(tsconfig_path.exists().then(|| tsconfig_path.as_ref()));

        for file_path in files {
            if let Some(repo) = repo {
                let repo = repo.lock().expect("lock poisoned");
                if matches!(repo.status_should_ignore(file_path.as_std_path()), Ok(true)) {
                    continue;
                }
            };
            // Read the file content
            let Ok(file_content) = tokio::fs::read_to_string(&file_path).await else {
                return Err(Error::FileNotFound(file_path.to_owned()));
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
                    diagnostics.push(BoundariesDiagnostic::ParseError(file_path.to_owned(), err));
                    continue;
                }
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder::default();
            module.visit_with(&mut finder);
            for (import, span, import_type) in finder.imports() {
                let (start, end) = source_map.span_to_char_offset(&source_file, *span);
                let start = start as usize;
                let end = end as usize;
                let span = SourceSpan::new(start.into(), (end - start).into());

                // We have a file import
                let check_result = if import.starts_with(".") {
                    self.check_file_import(&file_path, &package_root, &import, span, &file_content)?
                } else if Self::is_potential_package_name(&import) {
                    self.check_package_import(
                        &import,
                        *import_type,
                        span,
                        &file_path,
                        &file_content,
                        package_json,
                        &internal_dependencies,
                        unresolved_external_dependencies,
                        &resolver,
                    )
                } else {
                    None
                };

                if let Some(diagnostic) = check_result {
                    diagnostics.push(diagnostic);
                }
            }
        }

        Ok(diagnostics)
    }

    fn check_file_import(
        &self,
        file_path: &AbsoluteSystemPath,
        package_path: &AbsoluteSystemPath,
        import: &str,
        source_span: SourceSpan,
        file_content: &str,
    ) -> Result<Option<BoundariesDiagnostic>, Error> {
        let import_path = AnchoredSystemPath::new(import)?;
        let dir_path = file_path
            .parent()
            .ok_or_else(|| Error::NoParentDir(file_path.to_owned()))?;
        let resolved_import_path = dir_path.resolve(&import_path).clean()?;
        // We have to check for this case because `relation_to_path` returns `Parent` if
        // the paths are equal and there's nothing wrong with importing the
        // package you're in.
        if resolved_import_path.as_str() == package_path.as_str() {
            return Ok(None);
        }
        // We use `relation_to_path` and not `contains` because `contains`
        // panics on invalid paths with too many `..` components
        if matches!(
            package_path.relation_to_path(&resolved_import_path),
            PathRelation::Divergent | PathRelation::Child
        ) {
            Ok(Some(BoundariesDiagnostic::ImportLeavesPackage {
                import: import.to_string(),
                span: source_span,
                text: Arc::new(NamedSource::new(
                    file_path.as_str(),
                    file_content.to_string(),
                )),
            }))
        } else {
            Ok(None)
        }
    }

    /// Go through all the possible places a package could be declared to see if
    /// it's a valid import. We don't use `oxc_resolver` because there are some
    /// cases where you can resolve a package that isn't declared properly.
    fn is_dependency(
        internal_dependencies: &HashSet<&PackageNode>,
        package_json: &PackageJson,
        unresolved_external_dependencies: Option<&BTreeMap<String, String>>,
        package_name: &PackageNode,
    ) -> bool {
        internal_dependencies.contains(&package_name)
            || unresolved_external_dependencies.map_or(false, |external_dependencies| {
                external_dependencies.contains_key(package_name.as_package_name().as_str())
            })
            || package_json
                .dependencies
                .as_ref()
                .map_or(false, |dependencies| {
                    dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .dev_dependencies
                .as_ref()
                .map_or(false, |dev_dependencies| {
                    dev_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .peer_dependencies
                .as_ref()
                .map_or(false, |peer_dependencies| {
                    peer_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .optional_dependencies
                .as_ref()
                .map_or(false, |optional_dependencies| {
                    optional_dependencies.contains_key(package_name.as_package_name().as_str())
                })
    }

    fn check_package_import(
        &self,
        import: &str,
        import_type: ImportType,
        span: SourceSpan,
        file_path: &AbsoluteSystemPath,
        file_content: &str,
        package_json: &PackageJson,
        internal_dependencies: &HashSet<&PackageNode>,
        unresolved_external_dependencies: Option<&BTreeMap<String, String>>,
        resolver: &Resolver,
    ) -> Option<BoundariesDiagnostic> {
        let package_name = if import.starts_with("@") {
            import.split('/').take(2).join("/")
        } else {
            import
                .split_once("/")
                .map(|(import, _)| import)
                .unwrap_or(import)
                .to_string()
        };

        if package_name.starts_with("@types/") && matches!(import_type, ImportType::Value) {
            return Some(BoundariesDiagnostic::NotTypeOnlyImport {
                import: import.to_string(),
                span,
                text: Arc::new(NamedSource::new(
                    file_path.as_str(),
                    file_content.to_string(),
                )),
            });
        }
        let package_name = PackageNode::Workspace(PackageName::Other(package_name));
        let folder = file_path.parent().expect("file_path should have a parent");
        let is_valid_dependency = Self::is_dependency(
            internal_dependencies,
            package_json,
            unresolved_external_dependencies,
            &package_name,
        );

        if !is_valid_dependency
            && !matches!(
                resolver.resolve(folder, import),
                Err(ResolveError::Builtin { .. })
            )
        {
            // Check the @types package
            let types_package_name = PackageNode::Workspace(PackageName::Other(format!(
                "@types/{}",
                package_name.as_package_name().as_str()
            )));
            let is_types_dependency = Self::is_dependency(
                internal_dependencies,
                package_json,
                unresolved_external_dependencies,
                &types_package_name,
            );

            if is_types_dependency {
                return match import_type {
                    ImportType::Type => None,
                    ImportType::Value => Some(BoundariesDiagnostic::NotTypeOnlyImport {
                        import: import.to_string(),
                        span,
                        text: Arc::new(NamedSource::new(
                            file_path.as_str(),
                            file_content.to_string(),
                        )),
                    }),
                };
            }

            return Some(BoundariesDiagnostic::PackageNotFound {
                name: package_name.to_string(),
                span,
                text: Arc::new(NamedSource::new(
                    file_path.as_str(),
                    file_content.to_string(),
                )),
            });
        }

        None
    }
}
