use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use camino::Utf8Path;
use itertools::Itertools;
use miette::{NamedSource, SourceSpan};
use oxc_resolver::{ResolveError, Resolver, TsConfig};
use swc_common::{comments::SingleThreadedComments, SourceFile, Span};
use turbo_trace::ImportType;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, PathRelation, RelativeUnixPath};
use turborepo_errors::Spanned;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};

use crate::{
    boundaries::{tsconfig::TsConfigLoader, BoundariesDiagnostic, BoundariesResult, Error},
    run::Run,
};

/// All the places a dependency can be declared
#[derive(Clone, Copy)]
pub struct DependencyLocations<'a> {
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
        self.internal_dependencies.contains(package_name)
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

impl Run {
    /// Checks if the given import can be resolved as a tsconfig path alias,
    /// e.g. `@/types/foo` -> `./src/foo`, and if so, checks the resolved paths.
    ///
    /// Returns true if the import was resolved as a tsconfig path alias.
    #[allow(clippy::too_many_arguments)]
    fn check_import_as_tsconfig_path_alias(
        &self,
        tsconfig_loader: &mut TsConfigLoader,
        package_name: &PackageName,
        package_root: &AbsoluteSystemPath,
        span: SourceSpan,
        file_path: &AbsoluteSystemPath,
        file_content: &str,
        import: &str,
        result: &mut BoundariesResult,
    ) -> Result<bool, Error> {
        let dir = file_path.parent().expect("file_path must have a parent");
        let Some(tsconfig) = tsconfig_loader.load(dir, result) else {
            return Ok(false);
        };

        let resolved_paths = tsconfig.resolve(dir.as_std_path(), import);
        for path in &resolved_paths {
            let Some(utf8_path) = Utf8Path::from_path(path) else {
                result.diagnostics.push(BoundariesDiagnostic::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                });
                continue;
            };
            let resolved_import_path = AbsoluteSystemPath::new(utf8_path)?;
            result.diagnostics.extend(self.check_file_import(
                file_path,
                package_root,
                package_name,
                import,
                resolved_import_path,
                span,
                file_content,
            )?);
        }

        Ok(!resolved_paths.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_import(
        &self,
        comments: &SingleThreadedComments,
        tsconfig_loader: &mut TsConfigLoader,
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
        match Self::get_ignored_comment(comments, *span) {
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

        if self.check_import_as_tsconfig_path_alias(
            tsconfig_loader,
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
            self.check_file_import(
                file_path,
                package_root,
                package_name,
                import,
                &resolved_import_path,
                span,
                file_content,
            )?
        } else if Self::is_potential_package_name(import) {
            self.check_package_import(
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
        &self,
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
        &self,
        import: &str,
        import_type: ImportType,
        span: SourceSpan,
        file_path: &AbsoluteSystemPath,
        file_content: &str,
        dependency_locations: DependencyLocations<'_>,
        resolver: &Resolver,
    ) -> Option<BoundariesDiagnostic> {
        let package_name = Self::get_package_name(import);

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
