use std::{collections::HashMap, fmt};

use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};

use super::{PackageInfo, PackageName};

pub struct DependencySplitter<'a> {
    repo_root: &'a AbsoluteSystemPath,
    workspace_dir: &'a AbsoluteSystemPath,
    workspaces: &'a HashMap<PackageName, PackageInfo>,
}

impl<'a> DependencySplitter<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        workspace_dir: &'a AbsoluteSystemPath,
        workspaces: &'a HashMap<PackageName, PackageInfo>,
    ) -> Self {
        Self {
            repo_root,
            workspace_dir,
            workspaces,
        }
    }

    pub fn is_internal(&self, name: &str, version: &str) -> Option<PackageName> {
        // TODO implement borrowing for workspaces to allow for zero copy queries
        let workspace_name = PackageName::Other(
            version
                .strip_prefix("workspace:")
                .and_then(|version| version.rsplit_once('@'))
                .filter(|(_, version)| *version == "*" || *version == "^" || *version == "~")
                .map_or(name, |(actual_name, _)| actual_name)
                .to_string(),
        );
        let is_internal = self
            .workspaces
            .get(&workspace_name)
            // This is the current Go behavior, in the future we might not want to paper over a
            // missing version
            .map(|e| e.package_json.version.as_deref().unwrap_or_default())
            .map_or(false, |workspace_version| {
                DependencyVersion::new(version).matches_workspace_package(
                    workspace_version,
                    self.workspace_dir,
                    self.repo_root,
                )
            });
        match is_internal {
            true => Some(workspace_name),
            false => None,
        }
    }
}

struct DependencyVersion<'a> {
    protocol: Option<&'a str>,
    version: &'a str,
}

impl<'a> DependencyVersion<'a> {
    fn new(qualified_version: &'a str) -> Self {
        qualified_version.split_once(':').map_or(
            Self {
                protocol: None,
                version: qualified_version,
            },
            |(protocol, version)| Self {
                protocol: Some(protocol),
                version,
            },
        )
    }

    fn is_external(&self) -> bool {
        // The npm protocol for yarn by default still uses the workspace package if the
        // workspace version is in a compatible semver range. See https://github.com/yarnpkg/berry/discussions/4015
        // For now, we will just assume if the npm protocol is being used and the
        // version matches its an internal dependency which matches the existing
        // behavior before this additional logic was added.

        // TODO: extend this to support the `enableTransparentWorkspaces` yarn option
        self.protocol.map_or(false, |p| p != "npm")
    }

    fn matches_workspace_package(
        &self,
        package_version: &str,
        cwd: &AbsoluteSystemPath,
        root: &AbsoluteSystemPath,
    ) -> bool {
        match self.protocol {
            Some("workspace") => {
                // TODO: Since support at the moment is non-existent for workspaces that contain
                // multiple versions of the same package name, just assume its a
                // match and don't check the range for an exact match.
                true
            }
            Some("file") | Some("link") => {
                // Default to internal if we have the package but somehow cannot get the path
                RelativeUnixPathBuf::new(self.version)
                    .and_then(|file_path| cwd.join_unix_path(file_path))
                    .map_or(true, |dep_path| root.contains(&dep_path))
            }
            Some(_) if self.is_external() => {
                // Other protocols are assumed to be external references ("github:", etc)
                false
            }
            _ if self.version == "*" => true,
            _ => {
                // If we got this far, then we need to check the workspace package version to
                // see it satisfies the dependencies range to determin whether
                // or not its an internal or external dependency.
                let constraint = node_semver::Range::parse(self.version);
                let version = node_semver::Version::parse(package_version);

                // For backwards compatibility with existing behavior, if we can't parse the
                // version then we treat the dependency as an internal package
                // reference and swallow the error.

                // TODO: some package managers also support tags like "latest". Does extra
                // handling need to be added for this corner-case
                constraint
                    .ok()
                    .zip(version.ok())
                    .map_or(true, |(constraint, version)| constraint.satisfies(&version))
            }
        }
    }
}

impl<'a> fmt::Display for DependencyVersion<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.protocol {
            Some(protocol) => f.write_fmt(format_args!("{}:{}", protocol, self.version)),
            None => f.write_str(self.version),
        }
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use super::*;
    use crate::package_json::PackageJson;

    #[test_case("1.2.3", None, "1.2.3", Some("@scope/foo") ; "handles exact match")]
    #[test_case("1.2.3", None, "^1.0.0", Some("@scope/foo") ; "handles semver range satisfied")]
    #[test_case("2.3.4", None, "^1.0.0", None ; "handles semver range not satisfied")]
    #[test_case("1.2.3", None, "workspace:1.2.3", Some("@scope/foo") ; "handles workspace protocol with version")]
    #[test_case("1.2.3", None, "workspace:*", Some("@scope/foo") ; "handles workspace protocol with no version")]
    #[test_case("1.2.3", Some("bar"), "workspace:../bar/", Some("bar") ; "handles workspace protocol with relative path")]
    #[test_case("1.2.3", None, "workspace:../@scope/foo", Some("@scope/foo") ; "handles workspace protocol with scoped relative path")]
    #[test_case("1.2.3", None, "npm:^1.2.3", Some("@scope/foo") ; "handles npm protocol with satisfied semver range")]
    #[test_case("2.3.4", None, "npm:^1.2.3", None ; "handles npm protocol with not satisfied semver range")]
    #[test_case("1.2.3", None, "1.2.2-alpha-123abcd.0", None ; "handles pre-release versions")]
    // for backwards compatability with the code before versions were verified
    #[test_case("sometag", None, "1.2.3", Some("@scope/foo") ; "handles non-semver package version")]
    // for backwards compatability with the code before versions were verified
    #[test_case("1.2.3", None, "sometag", Some("@scope/foo") ; "handles non-semver dependency version")]
    #[test_case("1.2.3", None, "file:../libB", Some("@scope/foo") ; "handles file:.. inside repo")]
    #[test_case("1.2.3", None, "file:../../../otherproject", None ; "handles file:.. outside repo")]
    #[test_case("1.2.3", None, "link:../libB", Some("@scope/foo") ; "handles link:.. inside repo")]
    #[test_case("1.2.3", None, "link:../../../otherproject", None ; "handles link:.. outside repo")]
    #[test_case("0.0.0-development", None, "*", Some("@scope/foo") ; "handles development versions")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@*", Some("@scope/foo") ; "handles pnpm alias star")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@~", Some("@scope/foo") ; "handles pnpm alias tilda")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@^", Some("@scope/foo") ; "handles pnpm alias caret")]
    fn test_matches_workspace_package(
        package_version: &str,
        dependency_name: Option<&str>,
        range: &str,
        expected: Option<&str>,
    ) {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            "C:\\some\\repo"
        } else {
            "/some/repo"
        })
        .unwrap();
        let pkg_dir = root.join_components(&["packages", "libA"]);
        let workspaces = {
            let mut map = HashMap::new();
            map.insert(
                PackageName::Other("@scope/foo".to_string()),
                PackageInfo {
                    package_json: PackageJson {
                        version: Some(package_version.to_string()),
                        ..Default::default()
                    },
                    package_json_path: AnchoredSystemPathBuf::from_raw(
                        ["packages", "@scope", "foo"].join(std::path::MAIN_SEPARATOR_STR),
                    )
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            );
            map.insert(
                PackageName::Other("bar".to_string()),
                PackageInfo {
                    package_json: PackageJson {
                        version: Some("1.0.0".to_string()),
                        ..Default::default()
                    },
                    package_json_path: AnchoredSystemPathBuf::from_raw(
                        ["packages", "bar"].join(std::path::MAIN_SEPARATOR_STR),
                    )
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            );
            map.insert(
                PackageName::Other("baz".to_string()),
                PackageInfo {
                    package_json: PackageJson {
                        version: Some("1.0.0".to_string()),
                        ..Default::default()
                    },
                    package_json_path: AnchoredSystemPathBuf::from_raw(
                        ["packages", "baz"].join(std::path::MAIN_SEPARATOR_STR),
                    )
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            );
            map
        };

        let splitter = DependencySplitter {
            repo_root: &root,
            workspace_dir: &pkg_dir,
            workspaces: &workspaces,
        };

        assert_eq!(
            splitter.is_internal(dependency_name.unwrap_or("@scope/foo"), range),
            expected.map(PackageName::from)
        );
    }
}
