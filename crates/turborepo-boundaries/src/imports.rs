use std::collections::{BTreeMap, HashMap, HashSet};

use camino::Utf8Path;
use itertools::Itertools;
use miette::{NamedSource, SourceSpan};
use oxc_ast::ast::Comment;
use oxc_span::Span;
use tracing::debug;
use turbo_trace::ImportType;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, PathRelation, RelativeUnixPath};
use turborepo_errors::Spanned;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};
use unrs_resolver::{ResolveError, Resolver};

use crate::{BoundariesChecker, BoundariesDiagnostic, BoundariesResult, Error};

/// All the places a dependency can be declared
#[derive(Clone, Copy)]
pub struct DependencyLocations<'a> {
    // The containing package's name. We allow a package to import itself per JavaScript convention
    pub(crate) package: &'a PackageName,
    pub(crate) internal_dependencies: &'a HashSet<&'a PackageNode>,
    pub(crate) package_json: &'a PackageJson,
    pub(crate) unresolved_external_dependencies: Option<&'a BTreeMap<String, String>>,
    pub(crate) implicit_dependencies: &'a HashMap<String, Spanned<()>>,
    pub(crate) global_implicit_dependencies: &'a HashMap<String, Spanned<()>>,
}

impl<'a> DependencyLocations<'a> {
    /// Go through all the possible places a package could be declared to see if
    /// it's a valid import. We don't use `unrs_resolver` because there are some
    /// cases where you can resolve a package that isn't declared properly.
    fn is_dependency(&self, package_name: &PackageNode) -> bool {
        // The containing package's name. We allow a package to import itself per
        // JavaScript convention
        self.package == package_name.as_package_name()
            || self.internal_dependencies.contains(package_name)
            || self
                .unresolved_external_dependencies
                .is_some_and(|external_dependencies| {
                    external_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || self
                .package_json
                .dependencies
                .as_ref()
                .is_some_and(|dependencies| {
                    dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || self
                .package_json
                .dev_dependencies
                .as_ref()
                .is_some_and(|dev_dependencies| {
                    dev_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || self
                .package_json
                .peer_dependencies
                .as_ref()
                .is_some_and(|peer_dependencies| {
                    peer_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || self
                .package_json
                .optional_dependencies
                .as_ref()
                .is_some_and(|optional_dependencies| {
                    optional_dependencies.contains_key(package_name.as_package_name().as_str())
                })
            || self
                .implicit_dependencies
                .contains_key(package_name.as_package_name().as_str())
            || self
                .global_implicit_dependencies
                .contains_key(package_name.as_package_name().as_str())
    }
}

/// Checks if the given import can be resolved as a tsconfig path alias via the
/// resolver, e.g. `@/types/foo` -> `./src/foo` or `features/foo` ->
/// `./src/features/foo`, and if so, checks the resolved path against package
/// boundaries.
///
/// Called for all non-relative imports in `check_import`. This allows tsconfig
/// `paths` entries — whether they shadow package-name-shaped specifiers or use
/// non-package-name patterns like `!` or `@/foo` — to be recognised as local
/// imports instead of being incorrectly flagged as undeclared dependencies.
///
/// Returns `Ok(true)` if the import was resolved as a tsconfig path alias
/// (local or cross-package — the latter produces an `ImportLeavesPackage`
/// diagnostic via [`check_file_import`]).
///
/// Returns `Ok(false)` if the resolved path goes through `node_modules` (a
/// real npm package) or if the resolver could not resolve the import. The
/// caller should then fall through to `check_package_import`.
#[allow(clippy::too_many_arguments)]
fn check_import_as_tsconfig_path_alias(
    resolver: &Resolver,
    package_name: &PackageName,
    package_root: &AbsoluteSystemPath,
    span: SourceSpan,
    file_path: &AbsoluteSystemPath,
    file_content: &str,
    import: &str,
    result: &mut BoundariesResult,
) -> Result<bool, Error> {
    // Safety guard — relative imports are resolved as file imports elsewhere.
    if import.starts_with('.') {
        return Ok(false);
    }

    let dir = file_path.parent().expect("file_path must have a parent");

    match resolver.resolve(dir, import) {
        Ok(resolution) => {
            // If the resolved path goes through node_modules, the import
            // resolved to a real npm package rather than a tsconfig path alias
            // pointing to a local file.  Return false so the caller falls
            // through to `check_package_import`.
            let path = resolution.path();
            if path.components().any(|c| c.as_os_str() == "node_modules") {
                return Ok(false);
            }
            // Workspace packages are symlinked in node_modules, so the
            // resolved path won't contain `node_modules` after symlink
            // resolution. Detect these by checking if the resolution's
            // package.json name matches the import's package name — if so,
            // the resolver found the actual package, not a tsconfig alias.
            if BoundariesChecker::is_potential_package_name(import) {
                let import_pkg_name = get_package_name(import);
                if let Some(pkg_json) = resolution.package_json()
                    && pkg_json.name() == Some(import_pkg_name.as_str())
                {
                    return Ok(false);
                }
            }
            let Some(utf8_path) = Utf8Path::from_path(path) else {
                result.diagnostics.push(BoundariesDiagnostic::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                });
                return Ok(true);
            };
            let resolved_import_path = AbsoluteSystemPath::new(utf8_path)?;
            result.diagnostics.extend(check_file_import(
                file_path,
                package_root,
                package_name,
                import,
                resolved_import_path,
                span,
                file_content,
            )?);
            Ok(true)
        }
        // Expected resolution failures — the import isn't a tsconfig alias.
        Err(
            ResolveError::NotFound(_)
            | ResolveError::MatchedAliasNotFound(_, _)
            | ResolveError::Builtin { .. }
            | ResolveError::Ignored(_)
            | ResolveError::Specifier(_),
        ) => Ok(false),
        // Unexpected errors (I/O, broken tsconfig, etc.) — log for debugging
        // but still fall through to check_package_import.
        Err(e) => {
            debug!(
                import = %import,
                error = %e,
                "tsconfig path alias resolution failed unexpectedly, \
                 falling through to package import check"
            );
            Ok(false)
        }
    }
}

/// Validates a single import statement against package boundaries.
///
/// Dispatches to one of three paths:
/// 1. Relative imports (`./`, `../`) — validates the resolved path stays within
///    the package via [`check_file_import`].
/// 2. Non-relative imports — first tries
///    [`check_import_as_tsconfig_path_alias`] to resolve tsconfig `paths`
///    entries, then for package-name-shaped imports falls through to
///    [`check_package_import`] (validates the import is a declared dependency).
/// 3. Non-relative, non-package-name imports that don't resolve as tsconfig
///    aliases — skipped (no diagnostic).
///
/// Respects `@boundaries-ignore` comments placed above the import statement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_import(
    comments: &[Comment],
    source_text: &str,
    result: &mut BoundariesResult,
    package_name: &PackageName,
    package_root: &AbsoluteSystemPath,
    import: &str,
    import_type: &ImportType,
    span: &Span,
    statement_span: &Span,
    file_path: &AbsoluteSystemPath,
    file_content: &str,
    dependency_locations: DependencyLocations<'_>,
    resolver: &Resolver,
) -> Result<(), Error> {
    // If the import is prefixed with `@boundaries-ignore`, we ignore it, but print
    // a warning
    match BoundariesChecker::get_ignored_comment(comments, source_text, *statement_span) {
        Some(reason) if reason.is_empty() => {
            result.warnings.push(
                "@boundaries-ignore requires a reason, e.g. `// @boundaries-ignore implicit \
                 dependency`"
                    .to_string(),
            );
        }
        Some(_) => {
            let line = source_text[..span.start as usize]
                .chars()
                .filter(|&c| c == '\n')
                .count();
            result
                .warnings
                .push(format!("ignoring import on line {line} in {file_path}"));

            return Ok(());
        }
        None => {}
    }

    let start = span.start as usize;
    let end = span.end as usize;

    let span = SourceSpan::new(start.into(), end - start);

    let check_result = if import.starts_with(".") {
        // Relative file import
        let import_path = RelativeUnixPath::new(import)?;
        let dir_path = file_path
            .parent()
            .ok_or_else(|| Error::NoParentDir(file_path.to_owned()))?;
        let resolved_import_path = dir_path.join_unix_path(import_path).clean()?;
        check_file_import(
            file_path,
            package_root,
            package_name,
            import,
            &resolved_import_path,
            span,
            file_content,
        )?
    } else {
        // Non-relative import: try tsconfig alias resolution first. This
        // handles both package-name-shaped imports (where the alias may
        // shadow a package name) and non-package-name imports (like `!` or
        // `@/foo`) that can only be tsconfig aliases.
        if check_import_as_tsconfig_path_alias(
            resolver,
            package_name,
            package_root,
            span,
            file_path,
            file_content,
            import,
            result,
        )? {
            return Ok(());
        }
        if BoundariesChecker::is_potential_package_name(import) {
            check_package_import(
                import,
                *import_type,
                span,
                file_path,
                file_content,
                dependency_locations,
                resolver,
            )
        } else {
            None
        }
    };

    result.diagnostics.extend(check_result);

    Ok(())
}

/// Checks whether a resolved file import stays within the package boundary.
///
/// Returns `Some(BoundariesDiagnostic::ImportLeavesPackage)` if the resolved
/// path falls outside `package_path`, `None` otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_file_import(
    file_path: &AbsoluteSystemPath,
    package_path: &AbsoluteSystemPath,
    package_name: &PackageName,
    import: &str,
    resolved_import_path: &AbsoluteSystemPath,
    source_span: SourceSpan,
    file_content: &str,
) -> Result<Option<BoundariesDiagnostic>, Error> {
    // We have to check for this case because `relation_to_path` returns `Parent` if
    // the paths are equal and there's nothing wrong with importing the
    // package you're in.
    if resolved_import_path.as_str() == package_path.as_str() {
        return Ok(None);
    }
    // We use `relation_to_path` and not `contains` because `contains`
    // panics on invalid paths with too many `..` components
    if !matches!(
        package_path.relation_to_path(resolved_import_path),
        PathRelation::Parent
    ) {
        let resolved_import_path =
            AnchoredSystemPathBuf::relative_path_between(package_path, resolved_import_path)
                .to_string();

        Ok(Some(BoundariesDiagnostic::ImportLeavesPackage {
            path: file_path.to_owned(),
            import: import.to_string(),
            resolved_import_path,
            package_name: package_name.to_owned(),
            span: source_span,
            text: NamedSource::new(file_path.as_str(), file_content.to_string()),
        }))
    } else {
        Ok(None)
    }
}

/// Returns true if the import specifier refers to a Bun runtime builtin module.
///
/// Bun provides its own built-in modules (`bun`, `bun:test`, `bun:sqlite`,
/// etc.) that are available at runtime but are not Node.js builtins. Without
/// this check, a project with `@types/bun` in devDependencies would incorrectly
/// flag `import { $ } from "bun"` as a type-only import.
fn is_bun_builtin(import: &str) -> bool {
    import == "bun" || import.starts_with("bun:")
}

/// Extracts the npm package name from an import specifier.
///
/// For scoped packages (`@scope/name/path`), returns `@scope/name`.
/// For unscoped packages (`name/path`), returns `name`.
/// For bare imports without subpaths, returns the import as-is.
pub(crate) fn get_package_name(import: &str) -> String {
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
pub(crate) fn check_package_import(
    import: &str,
    import_type: ImportType,
    span: SourceSpan,
    file_path: &AbsoluteSystemPath,
    file_content: &str,
    dependency_locations: DependencyLocations<'_>,
    resolver: &Resolver,
) -> Option<BoundariesDiagnostic> {
    let package_name = get_package_name(import);

    if package_name.starts_with("@types/") && matches!(import_type, ImportType::Value) {
        return Some(BoundariesDiagnostic::NotTypeOnlyImport {
            path: file_path.to_owned(),
            import: import.to_string(),
            span,
            text: NamedSource::new(file_path.as_str(), file_content.to_string()),
        });
    }
    let package_name = PackageNode::Workspace(PackageName::Other(package_name));
    let folder = file_path.parent().expect("file_path should have a parent");
    let is_valid_dependency = dependency_locations.is_dependency(&package_name);

    if !is_valid_dependency
        && !is_bun_builtin(import)
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
        let is_types_dependency = dependency_locations.is_dependency(&types_package_name);

        if is_types_dependency {
            return match import_type {
                ImportType::Type => None,
                ImportType::Value => Some(BoundariesDiagnostic::NotTypeOnlyImport {
                    path: file_path.to_owned(),
                    import: import.to_string(),
                    span,
                    text: NamedSource::new(file_path.as_str(), file_content.to_string()),
                }),
            };
        }

        return Some(BoundariesDiagnostic::PackageNotFound {
            path: file_path.to_owned(),
            name: package_name.to_string(),
            span,
            text: NamedSource::new(file_path.as_str(), file_content.to_string()),
        });
    }

    None
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbo_trace::Tracer;

    use super::*;

    #[test_case("bun", true ; "bun bare import")]
    #[test_case("bun:test", true ; "bun test module")]
    #[test_case("bun:sqlite", true ; "bun sqlite module")]
    #[test_case("bun:ffi", true ; "bun ffi module")]
    #[test_case("bun:jsc", true ; "bun jsc module")]
    #[test_case("bunny", false ; "package starting with bun")]
    #[test_case("bun-framework", false ; "package with bun prefix")]
    #[test_case("@types/bun", false ; "types for bun")]
    #[test_case("react", false ; "unrelated package")]
    fn test_is_bun_builtin(import: &str, expected: bool) {
        assert_eq!(is_bun_builtin(import), expected);
    }

    #[test_case("", ""; "empty")]
    #[test_case("ship", "ship"; "basic")]
    #[test_case("@types/ship", "@types/ship"; "types")]
    #[test_case("@scope/ship", "@scope/ship"; "scoped")]
    #[test_case("@scope/foo/bar", "@scope/foo"; "scoped with path")]
    #[test_case("foo/bar", "foo"; "regular with path")]
    #[test_case("foo/", "foo"; "trailing slash")]
    #[test_case("foo/bar/baz", "foo"; "multiple slashes")]
    fn test_get_package_name(import: &str, expected: &str) {
        assert_eq!(get_package_name(import), expected);
    }

    fn make_tsconfig_alias_test_args(
        import: &str,
    ) -> (Resolver, PackageName, SourceSpan, String, BoundariesResult) {
        let resolver = Tracer::create_resolver(None);
        let package_name = PackageName::from("test-pkg");
        let span = SourceSpan::new(0.into(), 0);
        let file_content = format!("import {{ x }} from \"{import}\";");
        let result = BoundariesResult::default();
        (resolver, package_name, span, file_content, result)
    }

    // Package-name-shaped imports that have no matching tsconfig alias and no
    // corresponding file on disk should still return `false` so that the caller
    // can fall through to `check_package_import`.
    #[test_case("react" ; "bare package name")]
    #[test_case("lodash" ; "bare package name lodash")]
    #[test_case("@scope/package" ; "scoped package name")]
    #[test_case("@types/node" ; "types package name")]
    #[test_case("lodash/fp" ; "subpath import")]
    #[test_case("@scope/package/sub" ; "scoped subpath import")]
    #[test_case("@scope/package/deeply/nested" ; "scoped deeply nested subpath import")]
    fn tsconfig_alias_check_returns_false_for_unresolvable_package_imports(import: &str) {
        let (resolver, package_name, span, file_content, mut result) =
            make_tsconfig_alias_test_args(import);
        let tmp = tempfile::tempdir().unwrap();
        let package_root = AbsoluteSystemPath::new(tmp.path().to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        std::fs::write(file_path.as_std_path(), &file_content).unwrap();

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            &file_content,
            import,
            &mut result,
        )
        .unwrap();

        assert!(
            !resolved,
            "package import {import:?} with no tsconfig alias should not be resolved"
        );
        assert!(result.diagnostics.is_empty());
    }

    #[test_case("./foo" ; "relative current dir")]
    #[test_case("../bar" ; "relative parent dir")]
    #[test_case("./deeply/nested/module" ; "relative deeply nested")]
    fn tsconfig_alias_check_skips_relative_imports(import: &str) {
        let (resolver, package_name, span, file_content, mut result) =
            make_tsconfig_alias_test_args(import);
        let tmp = tempfile::tempdir().unwrap();
        let package_root = AbsoluteSystemPath::new(tmp.path().to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        std::fs::write(file_path.as_std_path(), &file_content).unwrap();

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            &file_content,
            import,
            &mut result,
        )
        .unwrap();

        assert!(
            !resolved,
            "relative import {import:?} should not be resolved as tsconfig alias"
        );
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn bun_import_not_flagged_as_type_only_when_types_bun_is_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::new(tmp.path().to_str().unwrap()).unwrap();
        let file_path = root.join_component("index.ts");
        let file_content = "import { $, which } from \"bun\";";
        std::fs::write(file_path.as_std_path(), file_content).unwrap();

        let resolver = Tracer::create_resolver(None);
        let package_name = PackageName::from("my-app");

        // `@types/bun` is listed as a devDependency, but `bun` itself is not
        let mut dev_deps = BTreeMap::new();
        dev_deps.insert("@types/bun".to_string(), "latest".to_string());
        let package_json = PackageJson {
            dev_dependencies: Some(dev_deps),
            ..Default::default()
        };

        let internal_deps = HashSet::new();
        let implicit_deps = HashMap::new();
        let global_implicit_deps = HashMap::new();

        let dependency_locations = DependencyLocations {
            package: &package_name,
            internal_dependencies: &internal_deps,
            package_json: &package_json,
            unresolved_external_dependencies: None,
            implicit_dependencies: &implicit_deps,
            global_implicit_dependencies: &global_implicit_deps,
        };

        let span = SourceSpan::new(0.into(), file_content.len());
        let result = check_package_import(
            "bun",
            ImportType::Value,
            span,
            &file_path,
            file_content,
            dependency_locations,
            &resolver,
        );

        assert!(
            result.is_none(),
            "import from 'bun' should not be flagged even when @types/bun is a devDependency"
        );
    }

    #[test]
    fn types_only_package_still_flagged_for_non_bun() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::new(tmp.path().to_str().unwrap()).unwrap();
        let file_path = root.join_component("index.ts");
        let file_content = "import { Ship } from \"ship\";";
        std::fs::write(file_path.as_std_path(), file_content).unwrap();

        let resolver = Tracer::create_resolver(None);
        let package_name = PackageName::from("my-app");

        // Only @types/ship exists, not ship itself
        let mut dev_deps = BTreeMap::new();
        dev_deps.insert("@types/ship".to_string(), "*".to_string());
        let package_json = PackageJson {
            dev_dependencies: Some(dev_deps),
            ..Default::default()
        };

        let internal_deps = HashSet::new();
        let implicit_deps = HashMap::new();
        let global_implicit_deps = HashMap::new();

        let dependency_locations = DependencyLocations {
            package: &package_name,
            internal_dependencies: &internal_deps,
            package_json: &package_json,
            unresolved_external_dependencies: None,
            implicit_dependencies: &implicit_deps,
            global_implicit_dependencies: &global_implicit_deps,
        };

        let span = SourceSpan::new(0.into(), file_content.len());
        let result = check_package_import(
            "ship",
            ImportType::Value,
            span,
            &file_path,
            file_content,
            dependency_locations,
            &resolver,
        );

        assert!(
            result.is_some(),
            "import from 'ship' should still be flagged when only @types/ship is a dependency"
        );
        assert!(matches!(
            result.unwrap(),
            BoundariesDiagnostic::NotTypeOnlyImport { .. }
        ));
    }

    #[test]
    fn tsconfig_alias_resolves_path_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create a tsconfig with a path alias
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "@/*": ["./*"] } } }"#,
        )
        .unwrap();

        // Create the target file the alias should resolve to
        std::fs::create_dir_all(root.join("utils")).unwrap();
        std::fs::write(root.join("utils").join("helper.ts"), "export const x = 1;").unwrap();

        // Create the source file
        let file_content = "import { x } from \"@/utils/helper\";";
        std::fs::write(root.join("index.ts"), file_content).unwrap();

        let package_root = AbsoluteSystemPath::new(root.to_str().unwrap()).unwrap();
        let tsconfig_path = AbsoluteSystemPath::new(tsconfig.to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        let package_name = PackageName::from("test-pkg");
        let span = SourceSpan::new(0.into(), 0);
        let mut result = BoundariesResult::default();

        let resolver = Tracer::create_resolver(Some(tsconfig_path));

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            file_content,
            "@/utils/helper",
            &mut result,
        )
        .unwrap();

        assert!(
            resolved,
            "@/utils/helper should be resolved as a tsconfig path alias"
        );
    }

    /// Regression test: an import whose specifier looks like a bare package
    /// name (e.g. `features/feature-a`) but is actually a tsconfig `paths`
    /// alias pointing to a local source file must be resolved as a tsconfig
    /// alias (returning `true`) rather than being incorrectly forwarded to
    /// `check_package_import` and flagged as an undeclared dependency.
    ///
    /// See: <https://github.com/vercel/turborepo/issues/11906>
    #[test]
    fn tsconfig_alias_resolves_package_name_shaped_path_alias() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize to match the resolver's symlink-resolved paths
        // (e.g. /tmp → /private/tmp on macOS). Uses dunce to avoid
        // \\?\ prefix on Windows which breaks path comparison.
        let root = dunce::canonicalize(tmp.path()).unwrap();

        // Mimic a tsconfig that maps `*` to `./src/*`, turning bare specifiers
        // like `features/feature-a` into local imports.
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "*": ["./src/*"] } } }"#,
        )
        .unwrap();

        // Create the target file the alias should resolve to
        std::fs::create_dir_all(root.join("src").join("features")).unwrap();
        std::fs::write(
            root.join("src").join("features").join("feature-a.ts"),
            "export const featureA = true;",
        )
        .unwrap();

        // Create the source file that imports via the alias
        let file_content = "import { featureA } from \"features/feature-a\";";
        std::fs::write(root.join("index.ts"), file_content).unwrap();

        let package_root = AbsoluteSystemPath::new(root.to_str().unwrap()).unwrap();
        let tsconfig_path = AbsoluteSystemPath::new(tsconfig.to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        let package_name = PackageName::from("test-pkg");
        let span = SourceSpan::new(0.into(), 0);
        let mut result = BoundariesResult::default();

        let resolver = Tracer::create_resolver(Some(tsconfig_path));

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            file_content,
            "features/feature-a",
            &mut result,
        )
        .unwrap();

        assert!(
            resolved,
            "features/feature-a with a tsconfig `*` alias should be resolved as a local import"
        );
        assert!(
            result.diagnostics.is_empty(),
            "expected no boundary violations for a locally-aliased import"
        );
        assert!(result.warnings.is_empty(), "expected no warnings");
    }

    /// Regression test: a tsconfig path alias must still be resolved as a local
    /// import even when the package root contains a `package.json` file.
    ///
    /// Previously, using `resolution.package_json().is_some()` caused the check
    /// to incorrectly treat tsconfig aliases as npm packages in any real
    /// project that has a `package.json` in its root directory.
    #[test]
    fn tsconfig_alias_resolves_with_package_json_present() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize to match the resolver's symlink-resolved paths
        // (e.g. /tmp → /private/tmp on macOS). Uses dunce to avoid
        // \\?\ prefix on Windows which breaks path comparison.
        let root = dunce::canonicalize(tmp.path()).unwrap();

        // Create a package.json so unrs_resolver can find it during resolution
        std::fs::write(
            root.join("package.json"),
            r#"{ "name": "test-pkg", "version": "1.0.0" }"#,
        )
        .unwrap();

        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "@/*": ["./*"] } } }"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("utils")).unwrap();
        std::fs::write(root.join("utils").join("helper.ts"), "export const x = 1;").unwrap();

        let file_content = "import { x } from \"@/utils/helper\";";
        std::fs::write(root.join("index.ts"), file_content).unwrap();

        let package_root = AbsoluteSystemPath::new(root.to_str().unwrap()).unwrap();
        let tsconfig_path = AbsoluteSystemPath::new(tsconfig.to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        let package_name = PackageName::from("test-pkg");
        let span = SourceSpan::new(0.into(), 0);
        let mut result = BoundariesResult::default();

        let resolver = Tracer::create_resolver(Some(tsconfig_path));

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            file_content,
            "@/utils/helper",
            &mut result,
        )
        .unwrap();

        assert!(
            resolved,
            "@/utils/helper should be resolved as a tsconfig path alias even when package.json is \
             present"
        );
        assert!(
            result.diagnostics.is_empty(),
            "expected no boundary violations for a locally-aliased import"
        );
        assert!(result.warnings.is_empty(), "expected no warnings");
    }

    /// When the resolver resolves an import to a path inside `node_modules`,
    /// the function must return `false` so the caller falls through to
    /// `check_package_import` for dependency-declaration validation.
    #[test]
    fn tsconfig_alias_check_returns_false_for_node_modules_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Wildcard alias that could match anything
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "*": ["./src/*"] } } }"#,
        )
        .unwrap();

        // Create a real node_modules package so the resolver can find it
        std::fs::create_dir_all(root.join("node_modules").join("some-pkg")).unwrap();
        std::fs::write(
            root.join("node_modules").join("some-pkg").join("index.js"),
            "module.exports = {};",
        )
        .unwrap();
        std::fs::write(
            root.join("node_modules")
                .join("some-pkg")
                .join("package.json"),
            r#"{ "name": "some-pkg", "main": "index.js" }"#,
        )
        .unwrap();

        let file_content = r#"import { x } from "some-pkg";"#;
        std::fs::write(root.join("index.ts"), file_content).unwrap();

        let package_root = AbsoluteSystemPath::new(root.to_str().unwrap()).unwrap();
        let tsconfig_path = AbsoluteSystemPath::new(tsconfig.to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        let package_name = PackageName::from("test-pkg");
        let span = SourceSpan::new(0.into(), 0);
        let mut result = BoundariesResult::default();

        let resolver = Tracer::create_resolver(Some(tsconfig_path));

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            file_content,
            "some-pkg",
            &mut result,
        )
        .unwrap();

        assert!(
            !resolved,
            "import resolving to node_modules must not be treated as a tsconfig alias"
        );
        assert!(result.diagnostics.is_empty());
    }

    /// A tsconfig alias that resolves to a file outside the package root should
    /// still be treated as a resolved alias (returns `true`), but produce an
    /// `ImportLeavesPackage` diagnostic.
    #[test]
    fn tsconfig_alias_flags_boundary_violation_for_out_of_package_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Shared file outside the package root
        let shared_dir = root.join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();
        std::fs::write(shared_dir.join("utils.ts"), "export const x = 1;").unwrap();

        // Package directory with a tsconfig alias pointing outside
        let pkg_dir = root.join("packages").join("my-app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let tsconfig = pkg_dir.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "@shared/*": ["../../shared/*"] } } }"#,
        )
        .unwrap();

        let file_content = r#"import { x } from "@shared/utils";"#;
        std::fs::write(pkg_dir.join("index.ts"), file_content).unwrap();

        let package_root = AbsoluteSystemPath::new(pkg_dir.to_str().unwrap()).unwrap();
        let tsconfig_path = AbsoluteSystemPath::new(tsconfig.to_str().unwrap()).unwrap();
        let file_path = package_root.join_component("index.ts");
        let package_name = PackageName::from("my-app");
        let span = SourceSpan::new(0.into(), 0);
        let mut result = BoundariesResult::default();

        let resolver = Tracer::create_resolver(Some(tsconfig_path));

        let resolved = check_import_as_tsconfig_path_alias(
            &resolver,
            &package_name,
            package_root,
            span,
            &file_path,
            file_content,
            "@shared/utils",
            &mut result,
        )
        .unwrap();

        assert!(
            resolved,
            "@shared/utils should be resolved as a tsconfig path alias"
        );
        assert!(
            !result.diagnostics.is_empty(),
            "expected an ImportLeavesPackage diagnostic for an out-of-package alias"
        );
    }
}
