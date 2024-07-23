use std::{collections::HashMap, fmt};

use turbopath::{
    AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPath,
    RelativeUnixPathBuf,
};

use super::{npmrc::NpmRc, PackageInfo, PackageName};
use crate::package_manager::PackageManager;

pub struct DependencySplitter<'a> {
    repo_root: &'a AbsoluteSystemPath,
    workspace_dir: &'a AbsoluteSystemPath,
    workspaces: &'a HashMap<PackageName, PackageInfo>,
    link_workspace_packages: bool,
}

impl<'a> DependencySplitter<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        workspace_dir: &'a AbsoluteSystemPath,
        workspaces: &'a HashMap<PackageName, PackageInfo>,
        package_manager: PackageManager,
        npmrc: Option<&'a NpmRc>,
    ) -> Self {
        Self {
            repo_root,
            workspace_dir,
            workspaces,
            link_workspace_packages: npmrc
                .and_then(|npmrc| npmrc.link_workspace_packages)
                .unwrap_or(!matches!(package_manager, PackageManager::Pnpm9)),
        }
    }

    pub fn is_internal(&self, name: &str, version: &str) -> Option<PackageName> {
        // If link_workspace_packages isn't set any version wihtout workspace protocol
        // is considered external.
        if !self.link_workspace_packages && !version.starts_with("workspace:") {
            return None;
        }
        let workspace_specifier = WorkspacePackageSpecifier::new(version)
            .unwrap_or(WorkspacePackageSpecifier::Alias(name));
        let (workspace_name, info) = self.find_package(workspace_specifier)?;
        let is_internal = DependencyVersion::new(version).matches_workspace_package(
            // This is the current Go behavior, in the future we might not want to paper over a
            // missing version
            info.package_json.version.as_deref().unwrap_or_default(),
            self.workspace_dir,
            self.repo_root,
        );

        match is_internal {
            true => Some(workspace_name),
            false => None,
        }
    }

    // Find a package in workspace from a specifier
    fn find_package(
        &self,
        specifier: WorkspacePackageSpecifier,
    ) -> Option<(PackageName, &PackageInfo)> {
        match specifier {
            WorkspacePackageSpecifier::Alias(name) => {
                // TODO implement borrowing for workspaces to allow for zero copy queries
                let package_name = PackageName::Other(name.to_string());
                let info = self.workspaces.get(&package_name)?;
                Some((package_name, info))
            }
            WorkspacePackageSpecifier::Path(path) => {
                let package_path = self.workspace_dir.join_unix_path(path);
                // There's a chance that the user provided path could escape the root, in which
                // case we don't support packages outside of the workspace.
                // Pnpm also doesn't support this so we defer to them to provide the error
                // message.
                let package_path = AnchoredSystemPathBuf::new(self.repo_root, package_path).ok()?;
                let (name, info) = self.workspace(&package_path).or_else(|| {
                    // Yarn4 allows for workspace root relative paths
                    let package_path = self.repo_root.join_unix_path(path);
                    let package_path =
                        AnchoredSystemPathBuf::new(self.repo_root, package_path).ok()?;
                    self.workspace(&package_path)
                })?;
                Some((name.clone(), info))
            }
        }
    }

    fn workspace(&self, path: &AnchoredSystemPath) -> Option<(&PackageName, &PackageInfo)> {
        self.workspaces
            .iter()
            .find(|(_, info)| info.package_path() == path)
    }
}

// A parsed variant of a package dependency that uses the workspace protocol
// The specifier can either be a package name or a relative path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspacePackageSpecifier<'a> {
    Alias(&'a str),
    Path(&'a RelativeUnixPath),
}

impl<'a> WorkspacePackageSpecifier<'a> {
    fn new(version: &'a str) -> Option<Self> {
        let version = version.strip_prefix("workspace:")?;
        match version.rsplit_once('@') {
            Some((name, "*")) | Some((name, "^")) | Some((name, "~")) => Some(Self::Alias(name)),
            // No indication of different name for the package
            // We want to capture specifiers that have type "directory" by npa which boils down to
            // checking for slashes: https://github.com/pnpm/npm-package-arg/blob/main/npa.js#L79
            Some(_) | None if version.contains('/') => {
                RelativeUnixPath::new(version).ok().map(Self::Path)
            }
            Some(_) | None => None,
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
                    .map(|file_path| cwd.join_unix_path(file_path))
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
                // or not it's an internal or external dependency.
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
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::package_json::PackageJson;

    #[test_case("1.2.3", None, "1.2.3", Some("@scope/foo"), true ; "handles exact match")]
    #[test_case("1.2.3", None, "^1.0.0", Some("@scope/foo"), true ; "handles semver range satisfied")]
    #[test_case("2.3.4", None, "^1.0.0", None, true ; "handles semver range not satisfied")]
    #[test_case("1.2.3", None, "workspace:1.2.3", Some("@scope/foo"), true ; "handles workspace protocol with version")]
    #[test_case("1.2.3", None, "workspace:*", Some("@scope/foo"), true ; "handles workspace protocol with no version")]
    #[test_case("1.2.3", None, "workspace:../@scope/foo", Some("@scope/foo"), true ; "handles workspace protocol with scoped relative path")]
    #[test_case("1.2.3", None, "workspace:packages/@scope/foo", Some("@scope/foo"), true ; "handles workspace protocol with root relative path")]
    #[test_case("1.2.3", Some("bar"), "workspace:../baz", Some("baz"), true ; "handles workspace protocol with path to differing package")]
    #[test_case("1.2.3", None, "npm:^1.2.3", Some("@scope/foo"), true ; "handles npm protocol with satisfied semver range")]
    #[test_case("2.3.4", None, "npm:^1.2.3", None, true ; "handles npm protocol with not satisfied semver range")]
    #[test_case("1.2.3", None, "1.2.2-alpha-123abcd.0", None, true ; "handles pre-release versions")]
    // for backwards compatability with the code before versions were verified
    #[test_case("sometag", None, "1.2.3", Some("@scope/foo"), true ; "handles non-semver package version")]
    // for backwards compatability with the code before versions were verified
    #[test_case("1.2.3", None, "sometag", Some("@scope/foo"), true ; "handles non-semver dependency version")]
    #[test_case("1.2.3", None, "file:../libB", Some("@scope/foo"), true ; "handles file:.. inside repo")]
    #[test_case("1.2.3", None, "file:../../../otherproject", None, true ; "handles file:.. outside repo")]
    #[test_case("1.2.3", None, "link:../libB", Some("@scope/foo"), true ; "handles link:.. inside repo")]
    #[test_case("1.2.3", None, "link:../../../otherproject", None, true ; "handles link:.. outside repo")]
    #[test_case("0.0.0-development", None, "*", Some("@scope/foo"), true ; "handles development versions")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@*", Some("@scope/foo"), true ; "handles pnpm alias star")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@~", Some("@scope/foo"), true ; "handles pnpm alias tilda")]
    #[test_case("1.2.3", Some("foo"), "workspace:@scope/foo@^", Some("@scope/foo"), true ; "handles pnpm alias caret")]
    #[test_case("1.2.3", None, "1.2.3", None, false ; "no workspace linking")]
    #[test_case("1.2.3", None, "workspace:1.2.3", Some("@scope/foo"), false ; "no workspace linking with protocol")]
    fn test_matches_workspace_package(
        package_version: &str,
        dependency_name: Option<&str>,
        range: &str,
        expected: Option<&str>,
        link_workspace_packages: bool,
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
                        ["packages", "@scope", "foo", "package.json"]
                            .join(std::path::MAIN_SEPARATOR_STR),
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
                        ["packages", "bar", "package.json"].join(std::path::MAIN_SEPARATOR_STR),
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
                        ["packages", "baz", "package.json"].join(std::path::MAIN_SEPARATOR_STR),
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
            link_workspace_packages,
        };

        assert_eq!(
            splitter.is_internal(dependency_name.unwrap_or("@scope/foo"), range),
            expected.map(PackageName::from)
        );
    }

    #[test_case("1.2.3", None ; "non-workspace")]
    #[test_case("workspace:1.2.3", None ; "workspace version")]
    #[test_case("workspace:*", None ; "workspace any")]
    #[test_case("workspace:foo@*", Some(WorkspacePackageSpecifier::Alias("foo")) ; "star")]
    #[test_case("workspace:foo@~", Some(WorkspacePackageSpecifier::Alias("foo")) ; "tilde")]
    #[test_case("workspace:foo@^", Some(WorkspacePackageSpecifier::Alias("foo")) ; "caret")]
    #[test_case("workspace:@scope/foo@*", Some(WorkspacePackageSpecifier::Alias("@scope/foo")) ; "package with scope")]
    #[test_case("workspace:../bar", Some(WorkspacePackageSpecifier::Path(RelativeUnixPath::new("../bar").unwrap())) ; "package with path")]
    #[test_case("workspace:notpath", None ; "package with not a path")]
    #[test_case("workspace:../@scope/foo", Some(WorkspacePackageSpecifier::Path(RelativeUnixPath::new("../@scope/foo").unwrap())) ; "scope in path")]
    fn test_workspace_specifier(input: &str, expected: Option<WorkspacePackageSpecifier>) {
        assert_eq!(WorkspacePackageSpecifier::new(input), expected);
    }
}
