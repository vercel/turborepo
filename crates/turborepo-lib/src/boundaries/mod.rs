mod config;

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
};

pub use config::{BoundariesConfig, Permissions};
use git2::Repository;
use globwalk::Settings;
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use oxc_resolver::{ResolveError, Resolver};
use regex::Regex;
use swc_common::{
    comments::SingleThreadedComments, errors::Handler, input::StringInput, FileName, SourceMap,
};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use tracing::log::warn;
use turbo_trace::{ImportFinder, ImportType, Tracer};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathRelation, RelativeUnixPath};
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};
use turborepo_ui::{color, ColorConfig, BOLD_GREEN, BOLD_RED};

use crate::{run::Run, turbo_json::TurboJson};

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum BoundariesDiagnostic {
    #[error("Package `{package_name}` found without any tag listed in allowlist")]
    NotAllowedTag {
        package_name: PackageName,
        #[label("tag not found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: Arc<NamedSource>,
    },
    #[error("Package `{package_name}` found with tag listed in denylist: `{tag}`")]
    DeniedTag {
        package_name: PackageName,
        tag: String,
        #[label("tag found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: Arc<NamedSource>,
    },
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
        text: NamedSource<String>,
    },
    #[error("cannot import package `{name}` because it is not a dependency")]
    PackageNotFound {
        name: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("cannot import file `{import}` because it leaves the package")]
    ImportLeavesPackage {
        import: String,
        #[label("file imported here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("failed to parse file {0}")]
    ParseError(AbsoluteSystemPathBuf, swc_ecma_parser::error::Error),
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] crate::config::Error),
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
    files_checked: usize,
    packages_checked: usize,
    pub source_map: Arc<SourceMap>,
    pub diagnostics: Vec<BoundariesDiagnostic>,
}

impl BoundariesResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn emit(&self, color_config: ColorConfig) {
        let swc_color_config = if color_config.should_strip_ansi {
            swc_common::errors::ColorConfig::Never
        } else {
            swc_common::errors::ColorConfig::Always
        };

        let handler =
            Handler::with_tty_emitter(swc_color_config, true, false, Some(self.source_map.clone()));

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
        let result_message = if self.diagnostics.is_empty() {
            color!(color_config, BOLD_GREEN, "no issues found")
        } else {
            color!(
                color_config,
                BOLD_RED,
                "{} issues found",
                self.diagnostics.len()
            )
        };

        println!(
            "Checked {} files in {} packages, {}",
            self.files_checked, self.packages_checked, result_message
        );
    }
}

impl Run {
    pub async fn check_boundaries(&mut self) -> Result<BoundariesResult, Error> {
        let packages = self.pkg_dep_graph().packages();
        let repo = Repository::discover(self.repo_root()).ok().map(Mutex::new);
        let mut diagnostics = vec![];
        let source_map = SourceMap::default();
        let mut total_files_checked = 0;
        for (package_name, package_info) in packages {
            if !self.filtered_pkgs().contains(package_name)
                || matches!(package_name, PackageName::Root)
            {
                continue;
            }

            let turbo_json = self.turbo_json_loader().uncached_load(package_name)?;
            let package_root = self.repo_root().resolve(package_info.package_path());
            let internal_dependencies = self
                .pkg_dep_graph()
                .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
                .unwrap_or_default();
            let unresolved_external_dependencies =
                package_info.unresolved_external_dependencies.as_ref();

            let (files_checked, package_diagnostics) = self
                .check_package(
                    &repo,
                    &package_root,
                    &package_info.package_json,
                    &turbo_json,
                    internal_dependencies,
                    unresolved_external_dependencies,
                    &source_map,
                )
                .await?;

            total_files_checked += files_checked;
            diagnostics.extend(package_diagnostics);
        }

        Ok(BoundariesResult {
            files_checked: total_files_checked,
            // Subtract 1 for the root package
            packages_checked: self.pkg_dep_graph().len() - 1,
            source_map: Arc::new(source_map),
            diagnostics,
        })
    }

    fn is_potential_package_name(import: &str) -> bool {
        PACKAGE_NAME_REGEX.is_match(import)
    }

    fn validate_relation(
        &mut self,
        dependency: &PackageName,
        allow_list: &Option<HashSet<String>>,
        deny_list: &Option<HashSet<String>>,
    ) -> Result<Vec<BoundariesDiagnostic>, Error> {
        let dep_turbo_json = match self.turbo_json_loader().load(dependency) {
            Ok(turbo_json) => turbo_json,
            // If there is no turbo.json, then there are no rules to check
            Err(crate::config::Error::NoTurboJSON) => &TurboJson::default(),
            Err(e) => return Err(e.into()),
        };

        let dep_tags = dep_turbo_json
            .boundaries
            .as_ref()
            .and_then(|b| b.tags.as_ref())
            .unwrap_or_default();

        // If there is no allow list, then we allow all tags
        let mut is_allowed = allow_list.is_none();
        let mut diagnostics = Vec::new();
        for tag in dep_tags.as_inner() {
            if let Some(allow_list) = allow_list {
                if allow_list.contains(tag.as_inner()) {
                    is_allowed = true;
                }
            }

            if let Some(deny_list) = deny_list {
                if deny_list.contains(tag.as_inner()) {
                    let (span, text) = tag.span_and_text("turbo.json");
                    diagnostics.push(BoundariesDiagnostic::DeniedTag {
                        package_name: dependency.clone(),
                        tag: tag.as_inner().to_string(),
                        span,
                        text: Arc::new(text),
                    });
                }
            }
        }

        if !is_allowed {
            let (span, text) = dep_tags.span_and_text("turbo.json");
            diagnostics.push(BoundariesDiagnostic::NotAllowedTag {
                package_name: dependency.clone(),
                span,
                text: Arc::new(text),
            });
        }

        Ok(diagnostics)
    }

    async fn check_package_tags(
        &mut self,
        repo: &Option<Mutex<Repository>>,
        package_name: &PackageName,
        package_root: &AbsoluteSystemPath,
        package_json: &PackageJson,
        boundaries_config: &BoundariesConfig,
    ) -> Result<Vec<BoundariesDiagnostic>, Error> {
        let (dependency_allow_list, dependency_deny_list) =
            if let Some(dependency_rules) = boundaries_config.dependencies.as_ref() {
                // If list is `None`, we allow all dependencies. If it's `Some`, we allow only
                // the listed dependencies.
                let allow_list = dependency_rules
                    .allow
                    .as_ref()
                    .map(|allow| allow.iter().flatten().collect::<HashSet<_>>());

                let deny_list = dependency_rules
                    .deny
                    .as_ref()
                    .map(|deny| deny.iter().flatten().collect::<HashSet<_>>());

                (allow_list, deny_list)
            } else {
                (None, None)
            };

        let (dependent_allow_list, dependent_deny_list) =
            if let Some(dependent_rules) = boundaries_config.dependents.as_ref() {
                // If list is `None`, we allow all dependentes. If it's `Some`, we allow only
                // the listed dependentes.
                let allow_list = dependent_rules
                    .allow
                    .as_ref()
                    .map(|allow| allow.iter().flatten().collect::<HashSet<String>>());

                let deny_list = dependent_rules
                    .deny
                    .as_ref()
                    .map(|deny| deny.iter().flatten().collect::<HashSet<String>>());

                (allow_list, deny_list)
            } else {
                (None, None)
            };

        for dependency in self.pkg_dep_graph().dependencies(package_name) {
            let diagnostics =
                self.validate_relation(&dependency, &dependency_allow_list, &dependency_deny_list);
        }
    }

    /// Either returns a list of errors and number of files checked or a single,
    /// fatal error
    async fn check_package(
        &self,
        repo: &Option<Mutex<Repository>>,
        package_root: &AbsoluteSystemPath,
        package_json: &PackageJson,
        turbo_json: &TurboJson,
        internal_dependencies: HashSet<&PackageNode>,
        unresolved_external_dependencies: Option<&BTreeMap<String, String>>,
        source_map: &SourceMap,
    ) -> Result<(usize, Vec<BoundariesDiagnostic>), Error> {
        let files = globwalk::globwalk_with_settings(
            package_root,
            &[
                "**/*.js".parse().unwrap(),
                "**/*.jsx".parse().unwrap(),
                "**/*.ts".parse().unwrap(),
                "**/*.tsx".parse().unwrap(),
                "**/*.cjs".parse().unwrap(),
                "**/*.mjs".parse().unwrap(),
                "**/*.svelte".parse().unwrap(),
                "**/*.vue".parse().unwrap(),
            ],
            &["**/node_modules/**".parse().unwrap()],
            globwalk::WalkType::Files,
            Settings::default().ignore_nested_packages(),
        )?;

        let files_checked = files.len();

        let mut diagnostics: Vec<BoundariesDiagnostic> = Vec::new();
        // We assume the tsconfig.json is at the root of the package
        let tsconfig_path = package_root.join_component("tsconfig.json");

        let resolver =
            Tracer::create_resolver(tsconfig_path.exists().then(|| tsconfig_path.as_ref()));

        let mut not_supported_extensions = HashSet::new();
        for file_path in files {
            if let Some(ext @ ("svelte" | "vue")) = file_path.extension() {
                not_supported_extensions.insert(ext.to_string());
                continue;
            }

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
                let span = SourceSpan::new(start.into(), end - start);

                // We have a file import
                let check_result = if import.starts_with(".") {
                    self.check_file_import(&file_path, package_root, import, span, &file_content)?
                } else if Self::is_potential_package_name(import) {
                    self.check_package_import(
                        import,
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

        for ext in &not_supported_extensions {
            warn!(
                "{} files are currently not supported, boundaries checks will not apply to them",
                ext
            );
        }
        if !not_supported_extensions.is_empty() {
            println!();
        }

        Ok((files_checked, diagnostics))
    }

    fn check_file_import(
        &self,
        file_path: &AbsoluteSystemPath,
        package_path: &AbsoluteSystemPath,
        import: &str,
        source_span: SourceSpan,
        file_content: &str,
    ) -> Result<Option<BoundariesDiagnostic>, Error> {
        let import_path = RelativeUnixPath::new(import)?;
        let dir_path = file_path
            .parent()
            .ok_or_else(|| Error::NoParentDir(file_path.to_owned()))?;
        let resolved_import_path = dir_path.join_unix_path(import_path).clean()?;
        // We have to check for this case because `relation_to_path` returns `Parent` if
        // the paths are equal and there's nothing wrong with importing the
        // package you're in.
        if resolved_import_path.as_str() == package_path.as_str() {
            return Ok(None);
        }
        // We use `relation_to_path` and not `contains` because `contains`
        // panics on invalid paths with too many `..` components
        if !matches!(
            package_path.relation_to_path(&resolved_import_path),
            PathRelation::Parent
        ) {
            Ok(Some(BoundariesDiagnostic::ImportLeavesPackage {
                import: import.to_string(),
                span: source_span,
                text: NamedSource::new(file_path.as_str(), file_content.to_string()),
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
            || unresolved_external_dependencies.is_some_and(|external_dependencies| {
                external_dependencies.contains_key(package_name.as_package_name().as_str())
            })
            || package_json
                .dependencies
                .as_ref()
                .is_some_and(|dependencies| {
                    dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .dev_dependencies
                .as_ref()
                .is_some_and(|dev_dependencies| {
                    dev_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .peer_dependencies
                .as_ref()
                .is_some_and(|peer_dependencies| {
                    peer_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || package_json
                .optional_dependencies
                .as_ref()
                .is_some_and(|optional_dependencies| {
                    optional_dependencies.contains_key(package_name.as_package_name().as_str())
                })
    }

    fn get_package_name(import: &str) -> String {
        if import.starts_with("@") {
            import.split('/').take(2).join("/")
        } else {
            import
                .split_once("/")
                .map(|(import, _)| import)
                .unwrap_or(import)
                .to_string()
        }
    }

    #[allow(clippy::too_many_arguments)]
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
        let package_name = Self::get_package_name(import);

        if package_name.starts_with("@types/") && matches!(import_type, ImportType::Value) {
            return Some(BoundariesDiagnostic::NotTypeOnlyImport {
                import: import.to_string(),
                span,
                text: NamedSource::new(file_path.as_str(), file_content.to_string()),
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
                        text: NamedSource::new(file_path.as_str(), file_content.to_string()),
                    }),
                };
            }

            return Some(BoundariesDiagnostic::PackageNotFound {
                name: package_name.to_string(),
                span,
                text: NamedSource::new(file_path.as_str(), file_content.to_string()),
            });
        }

        None
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test_case("", ""; "empty")]
    #[test_case("ship", "ship"; "basic")]
    #[test_case("@types/ship", "@types/ship"; "types")]
    #[test_case("@scope/ship", "@scope/ship"; "scoped")]
    #[test_case("@scope/foo/bar", "@scope/foo"; "scoped with path")]
    #[test_case("foo/bar", "foo"; "regular with path")]
    #[test_case("foo/", "foo"; "trailing slash")]
    #[test_case("foo/bar/baz", "foo"; "multiple slashes")]
    fn test_get_package_name(import: &str, expected: &str) {
        assert_eq!(Run::get_package_name(import), expected);
    }
}
