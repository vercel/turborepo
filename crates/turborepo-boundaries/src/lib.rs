#![allow(clippy::sliced_string_as_bytes)]
// miette's derive macro causes false positives for these lints
#![allow(unused_assignments)]

mod config;
mod imports;
mod tags;
mod tsconfig;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    sync::{Arc, LazyLock, Mutex},
};

pub use config::{BoundariesConfig, Permissions, Rule, RulesMap};
#[cfg(feature = "git2")]
use git2::Repository;
use globwalk::Settings;
use indicatif::{ProgressBar, ProgressIterator};
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use oxc_resolver::Resolver;
use regex::Regex;
use swc_common::{
    FileName, SourceMap, Span,
    comments::{Comments, SingleThreadedComments},
    errors::Handler,
    input::StringInput,
};
use swc_ecma_ast::EsVersion;
use swc_ecma_parser::{EsSyntax, Parser, Syntax, TsSyntax, lexer::Lexer};
use swc_ecma_visit::VisitWith;
pub use tags::{ProcessedPermissions, ProcessedRule, ProcessedRulesMap};
use thiserror::Error;
use tracing::log::warn;
use turbo_trace::{ImportFinder, Tracer};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName, PackageNode};
use turborepo_ui::{BOLD_GREEN, BOLD_RED, ColorConfig, color};

use crate::{imports::DependencyLocations, tsconfig::TsConfigLoader};

pub trait PackageGraphProvider {
    fn packages(&self) -> Box<dyn Iterator<Item = (&PackageName, &PackageInfo)> + '_>;
    fn immediate_dependencies(&self, node: &PackageNode) -> Option<HashSet<&PackageNode>>;
    fn dependencies(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_>;
    fn ancestors(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_>;
}

impl PackageGraphProvider for PackageGraph {
    fn packages(&self) -> Box<dyn Iterator<Item = (&PackageName, &PackageInfo)> + '_> {
        Box::new(self.packages())
    }

    fn immediate_dependencies(&self, node: &PackageNode) -> Option<HashSet<&PackageNode>> {
        self.immediate_dependencies(node)
    }

    fn dependencies(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
        Box::new(self.dependencies(node).into_iter())
    }

    fn ancestors(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
        Box::new(self.ancestors(node).into_iter())
    }
}

pub trait TurboJsonProvider {
    /// Returns true if turbo.json exists and can be loaded for this package
    fn has_turbo_json(&self, pkg: &PackageName) -> bool;
    fn boundaries_config(&self, pkg: &PackageName) -> Option<&BoundariesConfig>;
    fn package_tags(&self, pkg: &PackageName) -> Option<&Spanned<Vec<Spanned<String>>>>;
    fn implicit_dependencies(&self, pkg: &PackageName) -> HashMap<String, Spanned<()>>;
}

pub struct BoundariesContext<'a, G: PackageGraphProvider, T: TurboJsonProvider> {
    pub repo_root: &'a AbsoluteSystemPath,
    pub pkg_dep_graph: &'a G,
    pub turbo_json_provider: &'a T,
    pub root_boundaries_config: Option<&'a BoundariesConfig>,
    pub filtered_pkgs: &'a HashSet<PackageName>,
}

#[derive(Clone, Debug, Error, Diagnostic)]
pub enum SecondaryDiagnostic {
    #[error("package `{package} is defined here")]
    PackageDefinedHere {
        package: String,
        #[label]
        package_span: Option<SourceSpan>,
        #[source_code]
        package_text: NamedSource<String>,
    },
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
    #[error("Package boundaries rules cannot have `tags` key")]
    PackageBoundariesHasTags {
        #[label("tags defined here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Tag `{tag}` cannot share the same name as package `{package}`")]
    TagSharesPackageName {
        tag: String,
        package: String,
        #[label("tag defined here")]
        tag_span: Option<SourceSpan>,
        #[source_code]
        tag_text: NamedSource<String>,
        #[related]
        secondary: [SecondaryDiagnostic; 1],
    },
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
        path: AbsoluteSystemPathBuf,
        import: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("cannot import package `{name}` because it is not a dependency")]
    PackageNotFound {
        path: AbsoluteSystemPathBuf,
        name: String,
        #[label("package imported here")]
        span: SourceSpan,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("import `{import}` leaves the package")]
    #[diagnostic(help(
        "`{import}` resolves to path `{resolved_import_path}` which is outside of `{package_name}`"
    ))]
    ImportLeavesPackage {
        path: AbsoluteSystemPathBuf,
        import: String,
        resolved_import_path: String,
        package_name: PackageName,
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
    #[error("failed to write to file: {0}")]
    FileWrite(AbsoluteSystemPathBuf),
}

impl BoundariesDiagnostic {
    pub fn path_and_span(&self) -> Option<(&AbsoluteSystemPath, SourceSpan)> {
        match self {
            Self::ImportLeavesPackage { path, span, .. } => Some((path, *span)),
            Self::PackageNotFound { path, span, .. } => Some((path, *span)),
            Self::NotTypeOnlyImport { path, span, .. } => Some((path, *span)),
            _ => None,
        }
    }
}

static PACKAGE_NAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(@[a-z0-9-~][a-z0-9-._~]*\/)?[a-z0-9-~][a-z0-9-._~]*$").unwrap()
});

/// Maximum number of warnings to show
const MAX_WARNINGS: usize = 16;

#[derive(Default)]
pub struct BoundariesResult {
    pub files_checked: usize,
    pub packages_checked: usize,
    pub warnings: Vec<String>,
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
        let result_message = match self.diagnostics.len() {
            0 => color!(color_config, BOLD_GREEN, "no issues found"),
            1 => color!(color_config, BOLD_RED, "1 issue found"),
            _ => color!(
                color_config,
                BOLD_RED,
                "{} issues found",
                self.diagnostics.len()
            ),
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

pub struct BoundariesChecker;

impl BoundariesChecker {
    /// Returns the underlying reason if an import has been marked as ignored
    pub(crate) fn get_ignored_comment(
        comments: &SingleThreadedComments,
        span: Span,
    ) -> Option<String> {
        if let Some(import_comments) = comments.get_leading(span.lo()) {
            for comment in import_comments {
                if let Some(reason) = comment.text.trim().strip_prefix("@boundaries-ignore") {
                    return Some(reason.to_string());
                }
            }
        }

        None
    }

    fn is_potential_package_name(import: &str) -> bool {
        PACKAGE_NAME_REGEX.is_match(import)
    }

    /// Patch a file with boundaries-ignore comments
    pub fn patch_file(
        file_path: &AbsoluteSystemPath,
        file_patches: Vec<(SourceSpan, String)>,
    ) -> Result<(), Error> {
        // Deduplicate and sort by offset
        let file_patches = file_patches
            .into_iter()
            .map(|(span, patch)| (span.offset(), patch))
            .collect::<BTreeMap<usize, String>>();

        let contents = file_path
            .read_to_string()
            .map_err(|_| Error::FileNotFound(file_path.to_owned()))?;

        let mut options = OpenOptions::new();
        options.read(true).write(true).truncate(true);
        let mut file = file_path
            .open_with_options(options)
            .map_err(|_| Error::FileNotFound(file_path.to_owned()))?;

        let mut last_idx = 0;
        for (idx, reason) in file_patches {
            let contents_before_span = &contents[last_idx..idx];

            // Find the last newline before the span (note this is the index into the slice,
            // not the full file)
            let newline_idx = contents_before_span.rfind('\n');

            // If newline exists, we write all the contents before newline
            if let Some(newline_idx) = newline_idx {
                file.write_all(contents[last_idx..(last_idx + newline_idx)].as_bytes())
                    .map_err(|_| Error::FileWrite(file_path.to_owned()))?;
                file.write_all(b"\n")
                    .map_err(|_| Error::FileWrite(file_path.to_owned()))?;
            }

            file.write_all(b"// @boundaries-ignore ")
                .map_err(|_| Error::FileWrite(file_path.to_owned()))?;
            file.write_all(reason.as_bytes())
                .map_err(|_| Error::FileWrite(file_path.to_owned()))?;
            file.write_all(b"\n")
                .map_err(|_| Error::FileWrite(file_path.to_owned()))?;

            last_idx = idx;
        }

        file.write_all(contents[last_idx..].as_bytes())
            .map_err(|_| Error::FileWrite(file_path.to_owned()))?;

        Ok(())
    }

    /// Check boundaries for all packages
    pub async fn check_boundaries<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        show_progress: bool,
    ) -> Result<BoundariesResult, Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let rules_map = Self::get_processed_rules_map(ctx.root_boundaries_config);
        let packages: Vec<_> = ctx.pkg_dep_graph.packages().collect();
        #[cfg(feature = "git2")]
        let repo = Repository::discover(ctx.repo_root).ok().map(Mutex::new);
        #[cfg(not(feature = "git2"))]
        let repo: Option<()> = None;
        let mut result = BoundariesResult::default();
        let global_implicit_dependencies = ctx
            .turbo_json_provider
            .implicit_dependencies(&PackageName::Root);

        let progress = if show_progress {
            println!("Checking packages...");
            ProgressBar::new(packages.len() as u64)
        } else {
            ProgressBar::hidden()
        };

        for (package_name, package_info) in packages.into_iter().progress_with(progress) {
            if !ctx.filtered_pkgs.contains(package_name)
                || matches!(package_name, PackageName::Root)
            {
                continue;
            }

            Self::check_package(
                ctx,
                &repo,
                package_name,
                package_info,
                &rules_map,
                &global_implicit_dependencies,
                &mut result,
            )
            .await?;
        }

        Ok(result)
    }

    fn get_processed_rules_map(
        root_boundaries_config: Option<&BoundariesConfig>,
    ) -> Option<ProcessedRulesMap> {
        root_boundaries_config
            .and_then(|boundaries| boundaries.tags.as_ref())
            .map(|tags| {
                tags.as_inner()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().into()))
                    .collect()
            })
    }

    #[cfg(feature = "git2")]
    async fn check_package<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        repo: &Option<Mutex<Repository>>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        tag_rules: &Option<ProcessedRulesMap>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
        result: &mut BoundariesResult,
    ) -> Result<(), Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let implicit_dependencies = ctx.turbo_json_provider.implicit_dependencies(package_name);
        Self::check_package_files(
            ctx,
            repo,
            package_name,
            package_info,
            implicit_dependencies,
            global_implicit_dependencies,
            result,
        )
        .await?;

        // Only check package tags if turbo.json exists for this package
        if ctx.turbo_json_provider.has_turbo_json(package_name) {
            let package_tags = ctx.turbo_json_provider.package_tags(package_name);
            result.diagnostics.extend(tags::check_package_tags(
                ctx,
                PackageNode::Workspace(package_name.clone()),
                &package_info.package_json,
                package_tags,
                tag_rules.as_ref(),
            )?);
        }

        result.packages_checked += 1;

        Ok(())
    }

    #[cfg(not(feature = "git2"))]
    async fn check_package<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        _repo: &Option<()>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        tag_rules: &Option<ProcessedRulesMap>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
        result: &mut BoundariesResult,
    ) -> Result<(), Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let implicit_dependencies = ctx.turbo_json_provider.implicit_dependencies(package_name);
        Self::check_package_files_no_git(
            ctx,
            package_name,
            package_info,
            implicit_dependencies,
            global_implicit_dependencies,
            result,
        )
        .await?;

        // Only check package tags if turbo.json exists for this package
        if ctx.turbo_json_provider.has_turbo_json(package_name) {
            let package_tags = ctx.turbo_json_provider.package_tags(package_name);
            result.diagnostics.extend(tags::check_package_tags(
                ctx,
                PackageNode::Workspace(package_name.clone()),
                &package_info.package_json,
                package_tags,
                tag_rules.as_ref(),
            )?);
        }

        result.packages_checked += 1;

        Ok(())
    }

    #[cfg(feature = "git2")]
    async fn check_package_files<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        repo: &Option<Mutex<Repository>>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        implicit_dependencies: HashMap<String, Spanned<()>>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
        result: &mut BoundariesResult,
    ) -> Result<(), Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let package_root = ctx.repo_root.resolve(package_info.package_path());
        let internal_dependencies = ctx
            .pkg_dep_graph
            .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
            .unwrap_or_default();
        let _unresolved_external_dependencies =
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
            Self::process_file(
                ctx,
                &mut tsconfig_loader,
                result,
                package_name,
                &package_root,
                file_path,
                &implicit_dependencies,
                global_implicit_dependencies,
                package_info,
                &internal_dependencies,
                &resolver,
            )
            .await?;
        }

        for ext in &not_supported_extensions {
            result.warnings.push(format!(
                "{ext} files are currently not supported, boundaries checks will not apply to them"
            ));
        }

        result.files_checked += files.len();

        Ok(())
    }

    #[cfg(not(feature = "git2"))]
    async fn check_package_files_no_git<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        implicit_dependencies: HashMap<String, Spanned<()>>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
        result: &mut BoundariesResult,
    ) -> Result<(), Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let package_root = ctx.repo_root.resolve(package_info.package_path());
        let internal_dependencies = ctx
            .pkg_dep_graph
            .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
            .unwrap_or_default();
        let _unresolved_external_dependencies =
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

            Self::process_file(
                ctx,
                &mut tsconfig_loader,
                result,
                package_name,
                &package_root,
                file_path,
                &implicit_dependencies,
                global_implicit_dependencies,
                package_info,
                &internal_dependencies,
                &resolver,
            )
            .await?;
        }

        for ext in &not_supported_extensions {
            result.warnings.push(format!(
                "{ext} files are currently not supported, boundaries checks will not apply to them"
            ));
        }

        result.files_checked += files.len();

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_file<G, T>(
        _ctx: &BoundariesContext<'_, G, T>,
        tsconfig_loader: &mut TsConfigLoader<'_>,
        result: &mut BoundariesResult,
        package_name: &PackageName,
        package_root: &AbsoluteSystemPath,
        file_path: &AbsoluteSystemPath,
        implicit_dependencies: &HashMap<String, Spanned<()>>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
        package_info: &PackageInfo,
        internal_dependencies: &HashSet<&PackageNode>,
        resolver: &Resolver,
    ) -> Result<(), Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
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

        let mut parser = Parser::new_from(lexer);

        // Parse the file as a module
        let module: swc_ecma_ast::Module = match parser.parse_module() {
            Ok(module) => module,
            Err(err) => {
                result
                    .diagnostics
                    .push(BoundariesDiagnostic::ParseError(file_path.to_owned(), err));
                return Ok(());
            }
        };

        // Visit the AST and find imports
        let mut finder = ImportFinder::default();
        module.visit_with(&mut finder);
        let dependency_locations = DependencyLocations {
            package: package_name,
            internal_dependencies,
            package_json: &package_info.package_json,
            implicit_dependencies,
            global_implicit_dependencies,
            unresolved_external_dependencies: package_info
                .unresolved_external_dependencies
                .as_ref(),
        };

        for (import, span, import_type) in finder.imports() {
            imports::check_import(
                &comments,
                tsconfig_loader,
                result,
                &source_file,
                package_name,
                package_root,
                import,
                import_type,
                span,
                file_path,
                &file_content,
                dependency_locations,
                resolver,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_name_regex() {
        assert!(BoundariesChecker::is_potential_package_name("lodash"));
        assert!(BoundariesChecker::is_potential_package_name(
            "@scope/package"
        ));
        assert!(BoundariesChecker::is_potential_package_name("my-package"));
        assert!(!BoundariesChecker::is_potential_package_name("./relative"));
        assert!(!BoundariesChecker::is_potential_package_name("../parent"));
        assert!(!BoundariesChecker::is_potential_package_name("/absolute"));
    }
}
