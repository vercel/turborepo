use std::collections::{HashMap, HashSet};

use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};

use super::{
    ChangedPackagesError, ExternalDependencyChange, PackageGraph, PackageName,
    dep_splitter::{DependencySplitter, WorkspaceNameIndex, WorkspacePathIndex},
    javascript::lockfile_identity_resolution,
};
use crate::{package_json::PackageJson, relationships::DependencyKind};

fn configuration_matches(path: &str, historical: Option<&[u8]>, local: Option<&[u8]>) -> bool {
    match path {
        ".npmrc" => {
            use crate::package_manager::npmrc::NpmRc;
            matches!((NpmRc::from_reader(historical.unwrap_or_default()),
                      NpmRc::from_reader(local.unwrap_or_default())), (Ok(a), Ok(b)) if a == b)
        }
        ".yarnrc.yml" => {
            use crate::package_manager::yarnrc::YarnRc;
            let parse = |contents: Option<&[u8]>| match contents {
                Some(bytes) => YarnRc::from_reader(bytes),
                None => Ok(YarnRc::default()),
            };
            matches!((parse(historical), parse(local)), (Ok(a), Ok(b)) if a == b)
        }
        "pnpm-workspace.yaml" => match (historical, local) {
            (Some(historical), Some(local)) => {
                match (std::str::from_utf8(historical), std::str::from_utf8(local)) {
                    // Git can check out CRLF while the historical blob contains LF.
                    (Ok(historical), Ok(local)) => {
                        historical.replace("\r\n", "\n") == local.replace("\r\n", "\n")
                    }
                    _ => historical == local,
                }
            }
            _ => historical == local,
        },
        _ => historical == local,
    }
}

impl PackageGraph {
    /// Resolve a ref-to-ref comparison independently of the checkout's lockfile
    /// and dependency declarations. The execution graph itself remains
    /// unchanged.
    pub fn changed_packages_from_lockfile_snapshots(
        &self,
        path: &AnchoredSystemPath,
        previous_contents: &[u8],
        current_contents: &[u8],
        read: impl Fn(&AbsoluteSystemPath) -> Option<Vec<u8>>,
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let unavailable = || ChangedPackagesError::ResolutionUnavailable;
        let pm = self.package_manager().ok_or_else(unavailable)?;
        // Native resolvers need their own historical metadata adapters. Never
        // silently compare a historical native lockfile with the working tree.
        if path.as_str() != pm.lockfile_name() {
            return Err(unavailable());
        }

        // The existing splitter reads package-manager configuration from disk.
        // Reuse it only if that configuration is identical at the requested ref.
        let pnpm = matches!(
            pm.lockfile_manager(),
            crate::package_manager::PackageManager::Pnpm
                | crate::package_manager::PackageManager::Pnpm6
                | crate::package_manager::PackageManager::Pnpm9
        );
        let configuration: HashSet<_> = [
            pnpm.then_some(".npmrc"),
            pnpm.then_some("pnpm-workspace.yaml"),
            matches!(pm, crate::package_manager::PackageManager::Berry).then_some(".yarnrc.yml"),
            pm.workspace_configuration_path(),
        ]
        .into_iter()
        .flatten()
        .collect();
        for relative in configuration {
            let absolute = self.repo_root().join_component(relative);
            let local = match std::fs::read(&absolute) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err(unavailable()),
            };
            if !configuration_matches(relative, read(&absolute).as_deref(), local.as_deref()) {
                return Err(unavailable());
            }
        }

        let mut manifests = HashMap::new();
        for (identity, directory) in self.knowledge.package_json_packages() {
            let absolute = self
                .repo_root()
                .resolve(directory)
                .join_component("package.json");
            let contents = read(&absolute).ok_or_else(unavailable)?;
            let contents = std::str::from_utf8(&contents)
                .map_err(|_| ChangedPackagesError::NonUtf8Lockfile)?;
            let manifest = PackageJson::load_from_str(contents, absolute.as_str())
                .map_err(|error| ChangedPackagesError::Resolution(error.to_string()))?;
            let name = PackageName::from(identity);
            if name != PackageName::Root
                && manifest.name.as_ref().map(|name| name.as_str()) != Some(identity)
            {
                return Err(unavailable());
            }
            manifests.insert(name, manifest);
        }
        let root = manifests.get(&PackageName::Root).ok_or_else(unavailable)?;
        if root.package_manager.as_ref().map(|value| value.as_str())
            != self
                .root_package_json
                .as_ref()
                .and_then(|p| p.package_manager.as_ref())
                .map(|value| value.as_str())
        {
            return Err(unavailable());
        }
        let yarnrc = matches!(pm, crate::package_manager::PackageManager::Berry)
            .then(|| crate::package_manager::yarnrc::YarnRc::from_file(self.repo_root()))
            .transpose()?;
        let previous = pm.parse_lockfile(root, previous_contents, yarnrc.clone())?;
        let current = pm.parse_lockfile(root, current_contents, yarnrc)?;
        let link_workspaces = pm.link_workspace_packages(self.repo_root());
        let catalogs = pm.read_catalogs(self.repo_root());
        let paths = WorkspacePathIndex::from_knowledge(&self.knowledge);
        let names = WorkspaceNameIndex::from_workspaces(&manifests);
        let mut dependencies = HashMap::new();
        for (identity, directory) in self.knowledge.package_json_packages() {
            let manifest = &manifests[&PackageName::from(identity)];
            let absolute = self.repo_root().resolve(directory);
            let splitter = DependencySplitter::new(
                self.repo_root(),
                &absolute,
                &manifests,
                link_workspaces,
                &paths,
                &names,
                catalogs.as_ref(),
            );
            let mut seen = HashSet::new();
            let external = manifest
                .dependencies_with_kind()
                .filter(|(name, version, kind)| {
                    seen.insert(*name)
                        && !matches!(kind, DependencyKind::Peer { .. })
                        && splitter.is_internal(name, version).is_none()
                })
                .map(|(name, version, _)| (name.clone(), version.clone()))
                .collect();
            dependencies.insert(directory.to_unix().to_string(), external);
        }
        turborepo_rayon_compat::block_in_place(|| {
            let previous_resolution = lockfile_identity_resolution(
                &self.knowledge,
                previous.as_ref(),
                dependencies.clone(),
            )
            .map_err(ChangedPackagesError::Resolution)?;
            let current_resolution =
                lockfile_identity_resolution(&self.knowledge, current.as_ref(), dependencies)
                    .map_err(ChangedPackagesError::Resolution)?;
            self.compare_lockfile_resolutions(
                &current_resolution,
                &previous_resolution,
                current.global_change(previous.as_ref()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use turbopath::AbsoluteSystemPath;

    use super::*;

    #[test]
    fn configuration_comparison_only_checks_dependency_settings() {
        assert!(configuration_matches(
            ".npmrc",
            None,
            Some(b"script-shell=bash\n")
        ));
        assert!(!configuration_matches(
            ".npmrc",
            None,
            Some(b"link-workspace-packages=true\n")
        ));
        assert!(configuration_matches(
            ".yarnrc.yml",
            None,
            Some(b"cacheFolder: .cache\n")
        ));
        assert!(!configuration_matches(
            ".yarnrc.yml",
            None,
            Some(b"enableTransparentWorkspaces: false\n")
        ));
        assert!(!configuration_matches(
            ".yarnrc.yml",
            None,
            Some(b"invalid: [\n")
        ));
    }

    #[test]
    fn workspace_configuration_tolerates_checkout_line_endings() {
        let lf = b"packages:\n  - packages/*\n".as_slice();
        let crlf = b"packages:\r\n  - packages/*\r\n".as_slice();
        for (historical, local, expected) in [
            (lf, lf, true),
            (lf, crlf, true),
            (crlf, lf, true),
            (lf, b"packages:\r\n  - packages/*\n", true),
            (lf, b"packages:\r\n  - apps/*\r\n", false),
            (
                lf,
                b"packages:\r\n  - packages/*\r\nlinkWorkspacePackages: true\r\n",
                false,
            ),
            (
                b"catalog:\n  foo: 1.0.0\n",
                b"catalog:\r\n  foo: 2.0.0\r\n",
                false,
            ),
            (b"value: a\rb\n", b"value: ab\n", false),
            (b"\xff\n", b"\xfe\r\n", false),
        ] {
            assert_eq!(
                configuration_matches("pnpm-workspace.yaml", Some(historical), Some(local)),
                expected,
                "historical={historical:?}, local={local:?}"
            );
        }
        assert!(!configuration_matches(
            "pnpm-workspace.yaml",
            None,
            Some(lf)
        ));
        assert!(!configuration_matches(
            "pnpm-workspace.yaml",
            Some(lf),
            None
        ));
        assert!(!configuration_matches(
            "pnpm-workspace.yaml",
            None,
            Some(b"")
        ));
        assert!(configuration_matches("pnpm-workspace.yaml", None, None));
        assert!(!configuration_matches("other.yaml", Some(lf), Some(crlf)));
    }

    fn lockfile(version: &str, later_dependency: bool) -> Vec<u8> {
        let (b, package, snapshot) = if later_dependency {
            (
                "    dependencies:\n      bar:\n        specifier: 1.0.0\n        version: 1.0.0\n",
                "  bar@1.0.0:\n    resolution: {integrity: sha512-bar}\n",
                "  bar@1.0.0: {}\n",
            )
        } else {
            ("    {}\n", "", "")
        };
        let specifier = if version == "2.0.0" {
            "^2.0.0"
        } else {
            "^1.0.0"
        };
        format!("lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      foo:\n        specifier: '{specifier}'\n        version: {version}\n  packages/b:\n{b}packages:\n  foo@{version}:\n    resolution: {{integrity: sha512-foo}}\n{package}snapshots:\n  foo@{version}: {{}}\n{snapshot}").into_bytes()
    }

    #[tokio::test]
    async fn historical_lockfiles_use_head_manifests_not_later_checkout_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        fs::create_dir_all(dir.path().join("packages/a")).unwrap();
        fs::create_dir_all(dir.path().join("packages/b")).unwrap();
        let root_json = br#"{"name":"root","private":true,"packageManager":"pnpm@11.22.0"}"#;
        let workspace = b"packages:\n  - packages/*\n";
        let checkout_workspace = b"packages:\r\n  - packages/*\r\n";
        let a_head = br#"{"name":"a","dependencies":{"foo":"^1.0.0"}}"#;
        let b_head = br#"{"name":"b"}"#;
        for (path, bytes) in [
            ("package.json", root_json.as_slice()),
            ("pnpm-workspace.yaml", checkout_workspace.as_slice()),
            (
                "packages/a/package.json",
                br#"{"name":"a","dependencies":{"foo":"^2.0.0"}}"#.as_slice(),
            ),
            (
                "packages/b/package.json",
                br#"{"name":"b","dependencies":{"bar":"1.0.0"}}"#.as_slice(),
            ),
            ("pnpm-lock.yaml", lockfile("2.0.0", true).as_slice()),
        ] {
            fs::write(dir.path().join(path), bytes).unwrap();
        }
        let graph = PackageGraph::builder(
            root,
            PackageJson::load(&root.join_component("package.json")).unwrap(),
        )
        .build()
        .await
        .unwrap();
        let head_files: HashMap<_, _> = [
            ("package.json", root_json.to_vec()),
            ("pnpm-workspace.yaml", workspace.to_vec()),
            ("packages/a/package.json", a_head.to_vec()),
            ("packages/b/package.json", b_head.to_vec()),
        ]
        .into_iter()
        .map(|(path, data)| {
            (
                root.join_unix_path(turbopath::RelativeUnixPath::new(path).unwrap()),
                data,
            )
        })
        .collect();
        let read = |path: &AbsoluteSystemPath| head_files.get(path).cloned();
        let path = AnchoredSystemPath::new("pnpm-lock.yaml").unwrap();
        let previous = lockfile("1.0.0", false);
        let head = lockfile("1.1.0", false);
        let changes = graph
            .changed_packages_from_lockfile_snapshots(path, &previous, &head, read)
            .unwrap();
        let names: Vec<_> = changes
            .iter()
            .map(|change| change.package.name.as_str())
            .collect();
        assert_eq!(names, ["a"]);
        assert!(changes[0].added.iter().any(|p| p.version == "1.1.0"));
        assert!(changes[0].removed.iter().any(|p| p.version == "1.0.0"));
        assert!(
            graph
                .changed_packages_from_lockfile_snapshots(path, &previous, &previous, read)
                .unwrap()
                .is_empty()
        );
        let working = graph
            .changed_packages_from_lockfile_contents(path, &previous)
            .unwrap();
        assert!(
            working
                .iter()
                .any(|change| change.package.name.as_str() == "b")
        );
        assert!(
            graph
                .changed_packages_from_lockfile_snapshots(
                    path,
                    &previous,
                    b"invalid lockfile",
                    read
                )
                .is_err()
        );
        assert!(
            graph
                .changed_packages_from_lockfile_snapshots(path, &previous, &head, |_| None)
                .is_err()
        );
        let changed_config = |path: &AbsoluteSystemPath| {
            if path.file_name() == Some("pnpm-workspace.yaml") {
                Some(b"packages: []\nlinkWorkspacePackages: true\n".to_vec())
            } else {
                read(path)
            }
        };
        assert!(
            graph
                .changed_packages_from_lockfile_snapshots(path, &previous, &head, changed_config)
                .is_err()
        );
    }
}
