use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use camino::Utf8Path;
use itertools::Itertools;
use miette::{NamedSource, SourceSpan};
use oxc_resolver::{ResolveError, Resolver};
use swc_common::{SourceFile, Span, comments::SingleThreadedComments};
use turbo_trace::ImportType;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, PathRelation, RelativeUnixPath};
use turborepo_errors::Spanned;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};

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
    /// it's a valid import. We don't use `oxc_resolver` because there are some
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
/// resolver, e.g. `@/types/foo` -> `./src/foo`, and if so, checks the resolved
/// path against package boundaries.
///
/// Only attempts resolution for imports that are not relative paths and not
/// bare package names, so that bare specifiers like `react` still go through
/// `check_package_import` for dependency declaration validation.
///
/// Returns true if the import was resolved as a tsconfig path alias.
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
    // Skip relative imports and bare package names — those are handled elsewhere.
    // We only want to resolve tsconfig path aliases here.
    if import.starts_with('.') || BoundariesChecker::is_potential_package_name(import) {
        return Ok(false);
    }

    let dir = file_path.parent().expect("file_path must have a parent");

    match resolver.resolve(dir, import) {
        Ok(resolution) => {
            let path = resolution.path();
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
        Err(_) => Ok(false),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check_import(
    comments: &SingleThreadedComments,
    result: &mut BoundariesResult,
    source_file: &Arc<SourceFile>,
    package_name: &PackageName,
    package_root: &AbsoluteSystemPath,
    import: &str,
    import_type: &ImportType,
    span: &Span,
    file_path: &AbsoluteSystemPath,
    file_content: &str,
    dependency_locations: DependencyLocations<'_>,
    resolver: &Resolver,
) -> Result<(), Error> {
    // If the import is prefixed with `@boundaries-ignore`, we ignore it, but print
    // a warning
    match BoundariesChecker::get_ignored_comment(comments, *span) {
        Some(reason) if reason.is_empty() => {
            result.warnings.push(
                "@boundaries-ignore requires a reason, e.g. `// @boundaries-ignore implicit \
                 dependency`"
                    .to_string(),
            );
        }
        Some(_) => {
            // Try to get the line number for warning
            let line = result.source_map.lookup_line(span.lo()).map(|l| l.line);
            if let Ok(line) = line {
                result
                    .warnings
                    .push(format!("ignoring import on line {line} in {file_path}"));
            } else {
                result
                    .warnings
                    .push(format!("ignoring import in {file_path}"));
            }

            return Ok(());
        }
        None => {}
    }

    let (start, end) = result.source_map.span_to_char_offset(source_file, *span);
    let start = start as usize;
    let end = end as usize;

    let span = SourceSpan::new(start.into(), end - start);

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

    // We have a file import
    let check_result = if import.starts_with(".") {
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
    } else if BoundariesChecker::is_potential_package_name(import) {
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
    };

    result.diagnostics.extend(check_result);

    Ok(())
}

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

    #[test_case("react" ; "bare package name")]
    #[test_case("lodash" ; "bare package name lodash")]
    #[test_case("@scope/package" ; "scoped package name")]
    #[test_case("@types/node" ; "types package name")]
    fn tsconfig_alias_check_skips_bare_package_names(import: &str) {
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
            "bare package name {import:?} should not be resolved as tsconfig alias"
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
}
