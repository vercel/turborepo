mod config;
mod imports;
mod tags;
mod tsconfig;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
};

pub use config::{Permissions, RootBoundariesConfig, Rule};
use git2::Repository;
use globwalk::Settings;
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use regex::Regex;
use swc_common::{
    comments::{Comments, SingleThreadedComments},
    errors::Handler,
    input::StringInput,
    FileName, SourceMap, Span,
};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, Syntax, TsSyntax};
use swc_ecma_visit::VisitWith;
use thiserror::Error;
use tracing::log::warn;
use turbo_trace::{ImportFinder, Tracer};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::{PackageInfo, PackageName, PackageNode};
use turborepo_ui::{color, ColorConfig, BOLD_GREEN, BOLD_RED};

use crate::{
    boundaries::{tags::ProcessedRulesMap, tsconfig::TsConfigLoader},
    run::Run,
};

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum SecondaryDiagnostic {
    #[error("consider adding one of the following tags listed here")]
    Allowlist {
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("denylist defined here")]
    Denylist {
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
}

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum BoundariesDiagnostic {
    #[error("Path `{path}` is not valid UTF-8. Turborepo only supports UTF-8 paths.")]
    InvalidPath { path: String },
    #[error(
        "Package `{package_name}` found without any tag listed in allowlist for \
         `{source_package_name}`"
    )]
    NoTagInAllowlist {
        // The package that is declaring the allowlist
        source_package_name: PackageName,
        // The package that is either a dependency or dependent of the source package
        package_name: PackageName,
        #[label("tag not found here")]
        span: Option<SourceSpan>,
        #[help]
        help: Option<String>,
        #[source_code]
        text: NamedSource<String>,
        #[related]
        secondary: [SecondaryDiagnostic; 1],
    },
    #[error(
        "Package `{package_name}` found with tag listed in denylist for `{source_package_name}`: \
         `{tag}`"
    )]
    DeniedTag {
        source_package_name: PackageName,
        package_name: PackageName,
        tag: String,
        #[label("tag found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
        #[related]
        secondary: [SecondaryDiagnostic; 1],
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

/// Maximum number of warnings to show
const MAX_WARNINGS: usize = 16;

#[derive(Default)]
pub struct BoundariesResult {
    files_checked: usize,
    packages_checked: usize,
    warnings: Vec<String>,
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

        for warning in self.warnings.iter().take(MAX_WARNINGS) {
            warn!("{}", warning);
        }
        if !self.warnings.is_empty() {
            eprintln!();
        }

        println!(
            "Checked {} files in {} packages, {}",
            self.files_checked, self.packages_checked, result_message
        );
    }
}

impl Run {
    pub async fn check_boundaries(&self) -> Result<BoundariesResult, Error> {
        let package_tags = self.get_package_tags();
        let rules_map = self.get_processed_rules_map();
        let packages: Vec<_> = self.pkg_dep_graph().packages().collect();
        let repo = Repository::discover(self.repo_root()).ok().map(Mutex::new);
        let mut result = BoundariesResult::default();

        for (package_name, package_info) in packages {
            if !self.filtered_pkgs().contains(package_name)
                || matches!(package_name, PackageName::Root)
            {
                continue;
            }

            self.check_package(
                &repo,
                package_name,
                package_info,
                &package_tags,
                &rules_map,
                &mut result,
            )
            .await?;
        }

        Ok(result)
    }

    /// Returns the underlying reason if an import has been marked as ignored
    fn get_ignored_comment(comments: &SingleThreadedComments, span: Span) -> Option<String> {
        if let Some(import_comments) = comments.get_leading(span.lo()) {
            for comment in import_comments {
                if let Some(reason) = comment.text.trim().strip_prefix("@boundaries-ignore") {
                    return Some(reason.to_string());
                }
            }
        }

        None
    }

    /// Either returns a list of errors and number of files checked or a single,
    /// fatal error
    async fn check_package(
        &self,
        repo: &Option<Mutex<Repository>>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        all_package_tags: &HashMap<PackageName, Spanned<Vec<Spanned<String>>>>,
        tag_rules: &Option<ProcessedRulesMap>,
        result: &mut BoundariesResult,
    ) -> Result<(), Error> {
        self.check_package_files(repo, package_name, package_info, result)
            .await?;

        if let Some(current_package_tags) = all_package_tags.get(package_name) {
            if let Some(tag_rules) = tag_rules {
                result.diagnostics.extend(self.check_package_tags(
                    PackageNode::Workspace(package_name.clone()),
                    current_package_tags,
                    all_package_tags,
                    tag_rules,
                )?);
            } else {
                // NOTE: if we use tags for something other than boundaries, we should remove
                // this warning
                warn!(
                    "No boundaries rules found, but package {} has tags",
                    package_name
                );
            }
        }

        result.packages_checked += 1;

        Ok(())
    }

    fn is_potential_package_name(import: &str) -> bool {
        PACKAGE_NAME_REGEX.is_match(import)
    }

    async fn check_package_files(
        &self,
        repo: &Option<Mutex<Repository>>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        result: &mut BoundariesResult,
    ) -> Result<(), Error> {
        let package_root = self.repo_root().resolve(package_info.package_path());
        let internal_dependencies = self
            .pkg_dep_graph()
            .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
            .unwrap_or_default();
        let unresolved_external_dependencies =
            package_info.unresolved_external_dependencies.as_ref();

        let files = globwalk::globwalk_with_settings(
            &package_root,
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

        // We assume the tsconfig.json is at the root of the package
        let tsconfig_path = package_root.join_component("tsconfig.json");

        let resolver =
            Tracer::create_resolver(tsconfig_path.exists().then(|| tsconfig_path.as_ref()));

        let mut not_supported_extensions = HashSet::new();
        let mut tsconfig_loader = TsConfigLoader::new(&resolver);

        for file_path in &files {
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

            let source_file = result.source_map.new_source_file(
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
                    result
                        .diagnostics
                        .push(BoundariesDiagnostic::ParseError(file_path.to_owned(), err));
                    continue;
                }
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder::default();
            module.visit_with(&mut finder);
            for (import, span, import_type) in finder.imports() {
                self.check_import(
                    &comments,
                    &mut tsconfig_loader,
                    result,
                    &source_file,
                    &package_root,
                    import,
                    import_type,
                    span,
                    file_path,
                    &file_content,
                    &package_info,
                    &internal_dependencies,
                    unresolved_external_dependencies,
                    &resolver,
                )?;
            }
        }

        for ext in &not_supported_extensions {
            result.warnings.push(format!(
                "{} files are currently not supported, boundaries checks will not apply to them",
                ext
            ));
        }

        result.files_checked += files.len();

        Ok(())
    }
}
