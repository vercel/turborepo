use std::collections::VecDeque;

use serde::Deserialize;

use super::{PackageEntry, PackageInfo, is_git_or_github_package};
use crate::bun::RootInfo;
// Comment explaining entry schemas taken from bun.lock.zig
// first index is resolution for each type of package
// npm         -> [
//                "name@version",
//                registry (TODO: remove if default),
//                INFO,
//                integrity
//                ]
// symlink     -> [ "name@link:path", INFO ]
// folder      -> [ "name@file:path", INFO ]
// workspace   -> [ "name@workspace:path", INFO ]
// local tarball -> [ "name@./path.tgz", INFO, integrity? ]
// remote tarball -> [ "name@https://host/path.tgz", INFO, integrity? ]
// root        -> [ "name@root:", { bin, binDir } ]
// git         -> [ "name@git+repo", INFO, .bun-tag string, integrity? ]
// github      -> [ "name@github:user/repo", INFO, .bun-tag string, integrity? ]
impl<'de> Deserialize<'de> for PackageEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Vals {
            Str(String),
            Info(Box<PackageInfo>),
        }
        let mut vals = VecDeque::<Vals>::deserialize(deserializer)?;

        // First value is always the package key
        let key = vals
            .pop_front()
            .ok_or_else(|| de::Error::custom("expected package entry to not be empty"))?;
        let Vals::Str(key) = key else {
            return Err(de::Error::custom(
                "expected first element in package to be string",
            ));
        };
        let val_to_info = |val| match val {
            Vals::Str(_) => None,
            Vals::Info(package_info) => Some(*package_info),
        };

        let mut registry = None;
        let mut info = None;

        // Special case: root packages have a unique second value, so we handle it here
        if key.ends_with("@root:") {
            let root = vals.pop_front().and_then(|val| {
                serde_json::from_value::<RootInfo>(match val {
                    Vals::Info(info) => serde_json::to_value(info.other).ok()?,
                    _ => return None,
                })
                .ok()
            });
            return Ok(Self {
                ident: key,
                info,
                registry,
                checksum: None,
                integrity: None,
                root,
            });
        }

        // The second value can be either registry (string) or info (object)
        // Note: GitHub/git packages should never have a registry field
        if let Some(val) = vals.pop_front() {
            match val {
                Vals::Str(reg) => {
                    // Only set registry for npm packages, not git/github packages
                    if !is_git_or_github_package(&key) {
                        registry = Some(reg);
                    }
                }
                Vals::Info(package_info) => info = Some(*package_info),
            }
        };

        // Info will be next if we haven't already found it
        if info.is_none() {
            info = vals.pop_front().and_then(val_to_info);
        }

        let mut next_string = || {
            vals.pop_front().and_then(|val| match val {
                Vals::Str(s) => Some(s),
                Vals::Info(_) => None,
            })
        };
        let checksum = next_string();
        // Only git/github entries carry a further element after the checksum.
        let integrity = if is_git_or_github_package(&key) {
            next_string()
        } else {
            None
        };

        Ok(Self {
            ident: key,
            info,
            registry,
            checksum,
            integrity,
            root: None,
        })
    }
}

#[cfg(test)]
mod test {
    use std::sync::OnceLock;

    use serde_json::json;
    use test_case::test_case;

    use super::*;
    use crate::bun::WorkspaceEntry;

    macro_rules! fixture {
        ($name:ident, $kind:ty, $cons:expr) => {
            fn $name() -> &'static $kind {
                static ONCE: OnceLock<$kind> = OnceLock::new();
                ONCE.get_or_init(|| $cons)
            }
        };
    }

    fixture!(
        basic_workspace,
        WorkspaceEntry,
        WorkspaceEntry {
            name: "bun-test".into(),
            dev_dependencies: Some(
                Some(("turbo".to_string(), "^2.3.3".to_string()))
                    .into_iter()
                    .collect()
            ),
            ..Default::default()
        }
    );

    fixture!(
        workspace_with_version,
        WorkspaceEntry,
        WorkspaceEntry {
            name: "docs".into(),
            version: Some("0.1.0".into()),
            ..Default::default()
        }
    );

    fixture!(
        registry_pkg,
        PackageEntry,
        PackageEntry {
            ident: "is-odd@3.0.1".into(),
            registry: Some("".into()),
            info: Some(PackageInfo {
                dependencies: Some(("is-number".into(), "^6.0.0".into()))
                    .into_iter()
                    .collect(),
                dev_dependencies: Some(("is-bigint".into(), "1.1.0".into()))
                    .into_iter()
                    .collect(),
                peer_dependencies: Some(("is-even".into(), "1.0.0".into()))
                    .into_iter()
                    .collect(),
                optional_peers: Some("is-even".into()).into_iter().collect(),
                optional_dependencies: Some(("is-regexp".into(), "1.0.0".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("sha".into()),
            integrity: None,
            root: None,
        }
    );

    fixture!(
        workspace_pkg,
        PackageEntry,
        PackageEntry {
            ident: "docs".into(),
            info: Some(PackageInfo {
                dependencies: Some(("is-odd".into(), "3.0.1".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            registry: None,
            checksum: None,
            integrity: None,
            root: None,
        }
    );

    fixture!(
        root_pkg,
        PackageEntry,
        PackageEntry {
            ident: "some-package@root:".into(),
            root: Some(RootInfo {
                bin: Some("bin".into()),
                bin_dir: Some("binDir".into()),
            }),
            info: None,
            registry: None,
            checksum: None,
            integrity: None,
        }
    );

    // GitHub package fixture - should never have a registry field
    fixture!(
        github_pkg,
        PackageEntry,
        PackageEntry {
            ident: "@tanstack/react-store@github:TanStack/store#24a971c".into(),
            registry: None, // GitHub packages must NOT have registry
            info: Some(PackageInfo {
                dependencies: Some(("@tanstack/store".into(), "0.7.0".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("24a971c".into()),
            integrity: None,
            root: None,
        }
    );

    // GitHub package with CORRUPTED input (has empty string at position 1)
    // This tests that the deserializer fix correctly ignores registry for github
    // packages
    fixture!(
        github_pkg_corrupted_input,
        PackageEntry,
        PackageEntry {
            ident: "@tanstack/react-store@github:TanStack/store#24a971c".into(),
            registry: None, /* Even with corrupted 4-element input, github packages must NOT have
                             * registry */
            info: Some(PackageInfo {
                dependencies: Some(("@tanstack/store".into(), "0.7.0".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("24a971c".into()),
            integrity: None,
            root: None,
        }
    );

    // Git package fixture - should never have a registry field
    fixture!(
        git_pkg,
        PackageEntry,
        PackageEntry {
            ident: "my-package@git+https://github.com/user/repo#abc123".into(),
            registry: None, // Git packages must NOT have registry
            info: Some(PackageInfo {
                dependencies: Some(("lodash".into(), "4.17.21".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("abc123".into()),
            integrity: None,
            root: None,
        }
    );

    // File package fixture - 2-element array: [ident, info]
    fixture!(
        file_pkg,
        PackageEntry,
        PackageEntry {
            ident: "@api/sdk@file:apps/api/.api/apis/sdk".into(),
            registry: None,
            info: Some(PackageInfo {
                dependencies: Some(("is-odd".into(), "^3.0.1".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: None,
            integrity: None,
            root: None,
        }
    );

    // Link package fixture - 2-element array: [ident, info]
    fixture!(
        link_pkg,
        PackageEntry,
        PackageEntry {
            ident: "my-pkg@link:../../local-pkg".into(),
            registry: None,
            info: Some(PackageInfo::default()),
            checksum: None,
            integrity: None,
            root: None,
        }
    );

    fixture!(
        workspace_with_bin,
        WorkspaceEntry,
        WorkspaceEntry {
            name: "cli".into(),
            bin: Some(json!({"cli": "bin/cli.js"})),
            ..Default::default()
        }
    );

    fixture!(
        workspace_with_bin_dir,
        WorkspaceEntry,
        WorkspaceEntry {
            name: "scripts".into(),
            bin_dir: Some("bin".into()),
            ..Default::default()
        }
    );

    fixture!(
        root_workspace_without_name,
        WorkspaceEntry,
        WorkspaceEntry {
            dependencies: Some(
                Some(("is-odd".to_string(), "3.0.1".to_string()))
                    .into_iter()
                    .collect()
            ),
            ..Default::default()
        }
    );

    fixture!(
        root_pkg_with_object_bin,
        PackageEntry,
        PackageEntry {
            ident: "some-package@root:".into(),
            root: Some(RootInfo {
                bin: Some(json!({"some-package": "cli.js"})),
                bin_dir: None,
            }),
            info: None,
            registry: None,
            checksum: None,
            integrity: None,
        }
    );

    fixture!(
        root_pkg_without_bins,
        PackageEntry,
        PackageEntry {
            ident: "some-package@root:".into(),
            root: Some(RootInfo {
                bin: None,
                bin_dir: None,
            }),
            info: None,
            registry: None,
            checksum: None,
            integrity: None,
        }
    );

    fixture!(
        git_pkg_with_integrity,
        PackageEntry,
        PackageEntry {
            ident: "my-package@git+https://github.com/user/repo#abc123".into(),
            registry: None,
            info: Some(PackageInfo::default()),
            checksum: Some("abc123".into()),
            integrity: Some("sha512-integrity".into()),
            root: None,
        }
    );

    fixture!(
        local_tarball_pkg,
        PackageEntry,
        PackageEntry {
            ident: "bar@./vendor/bar-0.0.2.tgz".into(),
            registry: None,
            info: Some(PackageInfo::default()),
            checksum: Some("sha512-bar".into()),
            integrity: None,
            root: None,
        }
    );

    #[test_case(json!({"name": "bun-test", "devDependencies": {"turbo": "^2.3.3"}}), basic_workspace() ; "basic")]
    #[test_case(json!({"name": "docs", "version": "0.1.0"}), workspace_with_version() ; "with version")]
    #[test_case(json!(["is-odd@3.0.1", "", {"dependencies": {"is-number": "^6.0.0"}, "devDependencies": {"is-bigint": "1.1.0"}, "peerDependencies": {"is-even": "1.0.0"}, "optionalDependencies": {"is-regexp": "1.0.0"}, "optionalPeers": ["is-even"]}, "sha"]), registry_pkg() ; "registry package")]
    #[test_case(json!(["docs", {"dependencies": {"is-odd": "3.0.1"}}]), workspace_pkg() ; "workspace package")]
    #[test_case(json!(["some-package@root:", {"bin": "bin", "binDir": "binDir"}]), root_pkg() ; "root package")]
    #[test_case(json!(["@tanstack/react-store@github:TanStack/store#24a971c", {"dependencies": {"@tanstack/store": "0.7.0"}}, "24a971c"]), github_pkg() ; "github package")]
    #[test_case(json!(["my-package@git+https://github.com/user/repo#abc123", {"dependencies": {"lodash": "4.17.21"}}, "abc123"]), git_pkg() ; "git package")]
    #[test_case(json!(["@tanstack/react-store@github:TanStack/store#24a971c", "", {"dependencies": {"@tanstack/store": "0.7.0"}}, "24a971c"]), github_pkg_corrupted_input() ; "github package with corrupted 4-element input")]
    #[test_case(json!(["@api/sdk@file:apps/api/.api/apis/sdk", {"dependencies": {"is-odd": "^3.0.1"}}]), file_pkg() ; "file package")]
    #[test_case(json!(["my-pkg@link:../../local-pkg", {}]), link_pkg() ; "link package")]
    #[test_case(json!({"name": "cli", "bin": {"cli": "bin/cli.js"}}), workspace_with_bin() ; "workspace entry with bin")]
    #[test_case(json!({"name": "scripts", "binDir": "bin"}), workspace_with_bin_dir() ; "workspace entry with binDir")]
    #[test_case(json!({"dependencies": {"is-odd": "3.0.1"}}), root_workspace_without_name() ; "root workspace entry without name")]
    #[test_case(json!(["some-package@root:", {"bin": {"some-package": "cli.js"}}]), root_pkg_with_object_bin() ; "root package with object bin")]
    #[test_case(json!(["some-package@root:", {}]), root_pkg_without_bins() ; "root package without bins")]
    #[test_case(json!(["my-package@git+https://github.com/user/repo#abc123", {}, "abc123", "sha512-integrity"]), git_pkg_with_integrity() ; "git package with integrity")]
    #[test_case(json!(["bar@./vendor/bar-0.0.2.tgz", {}, "sha512-bar"]), local_tarball_pkg() ; "local tarball package")]
    fn test_deserialization<T: for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(
        input: serde_json::Value,
        expected: &T,
    ) {
        let actual: T = serde_json::from_value(input).unwrap();
        assert_eq!(&actual, expected);
    }
}
