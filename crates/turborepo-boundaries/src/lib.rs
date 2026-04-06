#![allow(clippy::sliced_string_as_bytes)]
// miette's derive macro causes false positives for these lints
#![allow(unused_assignments)]

mod config;
mod imports;
mod tags;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    sync::LazyLock,
};

pub use config::{BoundariesConfig, Permissions, Rule, RulesMap};
use globwalk::Settings;
use indicatif::ProgressBar;
use miette::{Diagnostic, NamedSource, Report, SourceSpan};
use oxc_ast::ast::Comment;
use rayon::prelude::*;
use regex::Regex;
pub use tags::{ProcessedPermissions, ProcessedRule, ProcessedRulesMap};
use thiserror::Error;
use tracing::{debug_span, info_span};
use turbo_trace::{ImportTraceType, Tracer, find_imports};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_log::Subsystem;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName, PackageNode};
use turborepo_ui::{BOLD_GREEN, BOLD_RED, ColorConfig, color};
use unrs_resolver::Resolver;

use crate::imports::DependencyLocations;

pub trait PackageGraphProvider: Send + Sync {
    fn packages(&self) -> Box<dyn Iterator<Item = (&PackageName, &PackageInfo)> + '_>;
    fn immediate_dependencies(&self, node: &PackageNode) -> Option<HashSet<&PackageNode>>;
    fn dependencies(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_>;
    fn ancestors(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_>;
    /// Returns strongly connected components with more than one member,
    /// representing circular dependency chains in the package graph.
    /// Each inner Vec is ordered to form a cycle path.
    fn find_cycles(&self) -> Vec<Vec<PackageName>>;
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

    fn find_cycles(&self) -> Vec<Vec<PackageName>> {
        self.find_cycles()
    }
}

pub trait TurboJsonProvider: Send + Sync {
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
    #[error("failed to parse file {0}: {1}")]
    ParseError(AbsoluteSystemPathBuf, String),
    #[error("Circular package dependency detected: {cycle_path}")]
    CircularDependency { cycle_path: String },
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
            Self::CircularDependency { .. } => None,
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
    pub diagnostics: Vec<BoundariesDiagnostic>,
}

impl BoundariesResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    fn merge(&mut self, other: BoundariesResult) {
        self.files_checked += other.files_checked;
        self.packages_checked += other.packages_checked;
        self.warnings.extend(other.warnings);
        self.diagnostics.extend(other.diagnostics);
    }

    pub fn emit(&self, color_config: ColorConfig) {
        for diagnostic in &self.diagnostics {
            eprintln!("{:?}", Report::new(diagnostic.clone()));
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
            turborepo_log::warn(
                turborepo_log::Source::turbo(Subsystem::Boundaries),
                warning.to_string(),
            )
            .emit();
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

/// Parse a file with oxc, returning both imports and comments.
///
/// We parse directly here (rather than using `turbo_trace::parse_file`) because
/// we need access to the comment list for `@boundaries-ignore` detection.
fn parse_with_comments(
    file_path: &AbsoluteSystemPath,
    source: &str,
) -> Option<(Vec<turbo_trace::ImportResult>, Vec<Comment>)> {
    let _span = debug_span!("parse_file", path = %file_path).entered();
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::from_path(file_path.as_std_path()).unwrap_or_default();
    let ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    if ret.panicked {
        return None;
    }
    let imports = find_imports(&ret.module_record, &ret.program.body, ImportTraceType::All);
    let comments: Vec<Comment> = ret.program.comments.iter().copied().collect();
    Some((imports, comments))
}

pub struct BoundariesChecker;

impl BoundariesChecker {
    /// Returns the underlying reason if an import has been marked as ignored.
    ///
    /// Searches for the nearest comment that ends before the import span and
    /// checks if it contains `@boundaries-ignore`.
    pub(crate) fn get_ignored_comment(
        comments: &[Comment],
        source_text: &str,
        import_span: oxc_span::Span,
    ) -> Option<String> {
        // Walk backwards through comments that end before the import. We check
        // multiple because there may be stacked comments before an import:
        //   // @boundaries-ignore reason
        //   // @ts-ignore
        //   import { foo } from "bar";
        //
        // To detect blank lines we check the gap between each comment and the
        // *next* item in the chain (initially the import, then the previous
        // comment we visited). A blank line means >1 newline in that gap.
        let leading = comments.iter().filter(|c| c.span.end <= import_span.start);

        let mut next_start = import_span.start;

        for comment in leading.rev() {
            let between = &source_text[comment.span.end as usize..next_start as usize];
            if between.chars().filter(|&c| c == '\n').count() > 1 {
                break;
            }

            let content_span = comment.content_span();
            let text = &source_text[content_span.start as usize..content_span.end as usize];
            if let Some(reason) = text.trim().strip_prefix("@boundaries-ignore") {
                return Some(reason.to_string());
            }

            next_start = comment.span.start;
        }
        None
    }

    /// Returns `true` if the import specifier looks like it could be an npm
    /// package name (e.g. `react`, `@scope/pkg`, `lodash/fp`).
    ///
    /// Used in [`imports::check_import`] to decide whether a non-relative
    /// import that didn't resolve as a tsconfig alias should be checked
    /// against declared dependencies.
    fn is_potential_package_name(import: &str) -> bool {
        let base = imports::get_package_name(import);
        PACKAGE_NAME_REGEX.is_match(base)
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
    pub fn check_boundaries<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        show_progress: bool,
    ) -> Result<BoundariesResult, Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let _span = info_span!("check_boundaries").entered();
        let rules_map = Self::get_processed_rules_map(ctx.root_boundaries_config);
        let packages: Vec<_> = ctx.pkg_dep_graph.packages().collect();
        let mut result = BoundariesResult::default();

        {
            let _span = info_span!("find_cycles").entered();
            for cycle in ctx.pkg_dep_graph.find_cycles() {
                let cycle_path = cycle
                    .iter()
                    .map(|name| name.to_string())
                    .chain(std::iter::once(cycle[0].to_string()))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                result
                    .diagnostics
                    .push(BoundariesDiagnostic::CircularDependency { cycle_path });
            }
        }

        let global_implicit_dependencies = ctx
            .turbo_json_provider
            .implicit_dependencies(&PackageName::Root);

        let packages_to_check: Vec<_> = packages
            .into_iter()
            .filter(|(name, _)| {
                ctx.filtered_pkgs.contains(name) && !matches!(name, PackageName::Root)
            })
            .collect();

        let progress = if show_progress {
            println!("Checking packages...");
            ProgressBar::new(packages_to_check.len() as u64)
        } else {
            ProgressBar::hidden()
        };

        let package_results: Vec<Result<BoundariesResult, Error>> = {
            let _span = info_span!("check_all_packages", count = packages_to_check.len()).entered();
            turborepo_rayon_compat::block_in_place(|| {
                packages_to_check
                    .par_iter()
                    .map(|(package_name, package_info)| {
                        let pkg_result = Self::check_package(
                            ctx,
                            package_name,
                            package_info,
                            &rules_map,
                            &global_implicit_dependencies,
                        );
                        progress.inc(1);
                        pkg_result
                    })
                    .collect()
            })
        };

        for pkg_result in package_results {
            result.merge(pkg_result?);
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

    fn check_package<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        tag_rules: &Option<ProcessedRulesMap>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
    ) -> Result<BoundariesResult, Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let _span = info_span!("check_package", package = %package_name).entered();
        let mut result = BoundariesResult::default();

        let implicit_dependencies = ctx.turbo_json_provider.implicit_dependencies(package_name);
        let file_result = Self::check_package_files(
            ctx,
            package_name,
            package_info,
            &implicit_dependencies,
            global_implicit_dependencies,
        )?;
        result.merge(file_result);

        // Only check package tags if turbo.json exists for this package
        if ctx.turbo_json_provider.has_turbo_json(package_name) {
            let _span = info_span!("check_package_tags", package = %package_name).entered();
            let package_tags = ctx.turbo_json_provider.package_tags(package_name);
            result.diagnostics.extend(tags::check_package_tags(
                ctx,
                PackageNode::Workspace(package_name.clone()),
                &package_info.package_json,
                package_tags,
                tag_rules.as_ref(),
            )?);
        }

        result.packages_checked = 1;

        Ok(result)
    }

    fn check_package_files<G, T>(
        ctx: &BoundariesContext<'_, G, T>,
        package_name: &PackageName,
        package_info: &PackageInfo,
        implicit_dependencies: &HashMap<String, Spanned<()>>,
        global_implicit_dependencies: &HashMap<String, Spanned<()>>,
    ) -> Result<BoundariesResult, Error>
    where
        G: PackageGraphProvider,
        T: TurboJsonProvider,
    {
        let _span = info_span!("check_package_files", package = %package_name).entered();
        let package_root = ctx.repo_root.resolve(package_info.package_path());
        let internal_dependencies = ctx
            .pkg_dep_graph
            .immediate_dependencies(&PackageNode::Workspace(package_name.to_owned()))
            .unwrap_or_default();

        let files = {
            let _span = info_span!("globwalk", package = %package_name).entered();
            globwalk::globwalk_with_settings(
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
            )?
        };

        // We assume the tsconfig.json is at the root of the package
        let tsconfig_path = package_root.join_component("tsconfig.json");
        let resolver =
            Tracer::create_resolver(tsconfig_path.exists().then(|| tsconfig_path.as_ref()));

        let mut not_supported_extensions = HashSet::new();

        let (js_ts_files, other_files): (Vec<_>, Vec<_>) = files
            .iter()
            .partition(|f| !matches!(f.extension(), Some("svelte" | "vue")));

        for file_path in &other_files {
            if let Some(ext @ ("svelte" | "vue")) = file_path.extension() {
                not_supported_extensions.insert(ext.to_string());
            }
        }

        let dependency_locations = DependencyLocations {
            package: package_name,
            internal_dependencies: &internal_dependencies,
            package_json: &package_info.package_json,
            implicit_dependencies,
            global_implicit_dependencies,
            unresolved_external_dependencies: package_info
                .unresolved_external_dependencies
                .as_ref(),
        };

        type FileResult = Result<(Vec<BoundariesDiagnostic>, Vec<String>), Error>;
        let file_results: Vec<FileResult> = {
            let _span = info_span!(
                "process_files",
                package = %package_name,
                count = js_ts_files.len()
            )
            .entered();
            js_ts_files
                .par_iter()
                .map(|file_path| {
                    Self::process_file(
                        package_name,
                        &package_root,
                        file_path,
                        dependency_locations,
                        &resolver,
                    )
                })
                .collect()
        };

        let mut result = BoundariesResult::default();
        for file_result in file_results {
            let (diagnostics, warnings) = file_result?;
            result.diagnostics.extend(diagnostics);
            result.warnings.extend(warnings);
        }

        for ext in &not_supported_extensions {
            result.warnings.push(format!(
                "{ext} files are currently not supported, boundaries checks will not apply to them"
            ));
        }

        result.files_checked = files.len();

        Ok(result)
    }

    fn process_file(
        package_name: &PackageName,
        package_root: &AbsoluteSystemPath,
        file_path: &AbsoluteSystemPath,
        dependency_locations: DependencyLocations<'_>,
        resolver: &Resolver,
    ) -> Result<(Vec<BoundariesDiagnostic>, Vec<String>), Error> {
        let file_content = file_path
            .read_to_string()
            .map_err(|_| Error::FileNotFound(file_path.to_owned()))?;

        let (imports, comments) = match parse_with_comments(file_path, &file_content) {
            Some(result) => result,
            None => {
                return Ok((
                    vec![BoundariesDiagnostic::ParseError(
                        file_path.to_owned(),
                        "parser panicked".to_string(),
                    )],
                    Vec::new(),
                ));
            }
        };

        let mut diagnostics = Vec::new();
        let mut warnings = Vec::new();

        for import_result in &imports {
            imports::check_import(
                &comments,
                &file_content,
                &mut diagnostics,
                &mut warnings,
                package_name,
                package_root,
                &import_result.specifier,
                &import_result.import_type,
                &import_result.span,
                &import_result.statement_span,
                file_path,
                &file_content,
                dependency_locations,
                resolver,
            )?;
        }

        Ok((diagnostics, warnings))
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
        assert!(BoundariesChecker::is_potential_package_name("lodash/fp"));
        assert!(BoundariesChecker::is_potential_package_name(
            "@scope/package/sub"
        ));
        assert!(BoundariesChecker::is_potential_package_name(
            "@scope/package/deeply/nested"
        ));
        assert!(!BoundariesChecker::is_potential_package_name("./relative"));
        assert!(!BoundariesChecker::is_potential_package_name("../parent"));
        assert!(!BoundariesChecker::is_potential_package_name("/absolute"));
    }

    #[test]
    fn merge_accumulates_all_fields() {
        let mut a = BoundariesResult {
            files_checked: 5,
            packages_checked: 2,
            warnings: vec!["warn-a".into()],
            diagnostics: vec![BoundariesDiagnostic::CircularDependency {
                cycle_path: "a -> b -> a".into(),
            }],
        };
        let b = BoundariesResult {
            files_checked: 3,
            packages_checked: 1,
            warnings: vec!["warn-b1".into(), "warn-b2".into()],
            diagnostics: vec![BoundariesDiagnostic::InvalidPath {
                path: "/bad".into(),
            }],
        };

        a.merge(b);

        assert_eq!(a.files_checked, 8);
        assert_eq!(a.packages_checked, 3);
        assert_eq!(a.warnings.len(), 3);
        assert_eq!(a.diagnostics.len(), 2);
    }

    #[test]
    fn merge_with_empty_is_identity() {
        let mut result = BoundariesResult {
            files_checked: 10,
            packages_checked: 4,
            warnings: vec!["w".into()],
            diagnostics: vec![BoundariesDiagnostic::CircularDependency {
                cycle_path: "x -> y -> x".into(),
            }],
        };

        result.merge(BoundariesResult::default());

        assert_eq!(result.files_checked, 10);
        assert_eq!(result.packages_checked, 4);
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.diagnostics.len(), 1);
    }

    // Minimal mock providers for integration tests
    struct MockGraph {
        packages: Vec<(PackageName, PackageInfo)>,
    }

    impl MockGraph {
        fn new(packages: Vec<(PackageName, PackageInfo)>) -> Self {
            Self { packages }
        }
    }

    impl PackageGraphProvider for MockGraph {
        fn packages(&self) -> Box<dyn Iterator<Item = (&PackageName, &PackageInfo)> + '_> {
            Box::new(self.packages.iter().map(|(n, i)| (n, i)))
        }

        fn immediate_dependencies(&self, _: &PackageNode) -> Option<HashSet<&PackageNode>> {
            Some(HashSet::new())
        }

        fn dependencies(&self, _: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
            Box::new(std::iter::empty())
        }

        fn ancestors(&self, _: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
            Box::new(std::iter::empty())
        }

        fn find_cycles(&self) -> Vec<Vec<PackageName>> {
            Vec::new()
        }
    }

    struct MockTurboJson;

    impl TurboJsonProvider for MockTurboJson {
        fn has_turbo_json(&self, _: &PackageName) -> bool {
            false
        }

        fn boundaries_config(&self, _: &PackageName) -> Option<&BoundariesConfig> {
            None
        }

        fn package_tags(&self, _: &PackageName) -> Option<&Spanned<Vec<Spanned<String>>>> {
            None
        }

        fn implicit_dependencies(&self, _: &PackageName) -> HashMap<String, Spanned<()>> {
            HashMap::new()
        }
    }

    #[test]
    fn check_boundaries_runs_through_rayon() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::new(tmp.path().to_str().unwrap()).unwrap();

        // Create two packages, each with one JS file containing a local import
        for pkg in &["pkg-a", "pkg-b"] {
            let pkg_dir = repo_root.join_components(&["packages", pkg]);
            std::fs::create_dir_all(pkg_dir.as_std_path()).unwrap();
            std::fs::write(
                pkg_dir.join_component("package.json").as_std_path(),
                format!(r#"{{"name": "{pkg}"}}"#),
            )
            .unwrap();
            std::fs::write(
                pkg_dir.join_component("index.ts").as_std_path(),
                "import './local';\n",
            )
            .unwrap();
        }

        let packages = vec![
            (
                PackageName::Other("pkg-a".into()),
                PackageInfo {
                    package_json: Default::default(),
                    package_json_path: turbopath::AnchoredSystemPathBuf::from_raw(
                        "packages/pkg-a/package.json",
                    )
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            ),
            (
                PackageName::Other("pkg-b".into()),
                PackageInfo {
                    package_json: Default::default(),
                    package_json_path: turbopath::AnchoredSystemPathBuf::from_raw(
                        "packages/pkg-b/package.json",
                    )
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            ),
        ];

        let graph = MockGraph::new(packages);
        let turbo_json = MockTurboJson;
        let filtered: HashSet<PackageName> = ["pkg-a", "pkg-b"]
            .iter()
            .map(|&n| PackageName::Other(n.into()))
            .collect();

        let ctx = BoundariesContext {
            repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };

        let result = BoundariesChecker::check_boundaries(&ctx, false).unwrap();

        assert_eq!(result.packages_checked, 2);
        assert_eq!(result.files_checked, 2);
        // Local imports (./local) should not produce diagnostics
        assert!(
            result.diagnostics.is_empty(),
            "local imports should not produce diagnostics, got: {:?}",
            result.diagnostics.len()
        );
    }

    #[test]
    fn merge_preserves_ordering() {
        let mut a = BoundariesResult {
            diagnostics: vec![BoundariesDiagnostic::CircularDependency {
                cycle_path: "first".into(),
            }],
            ..Default::default()
        };
        let b = BoundariesResult {
            diagnostics: vec![BoundariesDiagnostic::CircularDependency {
                cycle_path: "second".into(),
            }],
            ..Default::default()
        };

        a.merge(b);

        match (&a.diagnostics[0], &a.diagnostics[1]) {
            (
                BoundariesDiagnostic::CircularDependency { cycle_path: first },
                BoundariesDiagnostic::CircularDependency { cycle_path: second },
            ) => {
                assert_eq!(first, "first");
                assert_eq!(second, "second");
            }
            _ => panic!("expected CircularDependency variants"),
        }
    }
}
