use std::collections::{BTreeMap, HashSet};

use itertools::Itertools;
use miette::{NamedSource, SourceSpan};
use oxc_resolver::{ResolveError, Resolver};
use turbo_trace::ImportType;
use turbopath::{AbsoluteSystemPath, PathRelation, RelativeUnixPath};
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};

use crate::{
    boundaries::{BoundariesDiagnostic, Error},
    run::Run,
};

impl Run {
    pub(crate) fn check_file_import(
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
    pub(crate) fn check_package_import(
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
