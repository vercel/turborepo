use std::collections::VecDeque;

use serde::Deserialize;

use super::{PackageEntry, PackageInfo};
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
// tarball     -> [ "name@tarball", INFO ]
// root        -> [ "name@root:", { bin, binDir } ]
// git         -> [ "name@git+repo", INFO, .bun-tag string (TODO: remove this) ]
// github      -> [ "name@github:user/repo", INFO, .bun-tag string (TODO: remove
// this) ]
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
                    Vals::Info(info) => {
                        serde_json::to_value(info.other).expect("failed to convert info to value")
                    }
                    _ => return None,
                })
                .ok()
            });
            return Ok(Self {
                ident: key,
                info,
                registry,
                checksum: None,
                root,
            });
        }

        // The second value can be either registry (string) or info (object)
        // Note: GitHub/git packages should never have a registry field
        let is_git_package = key.contains("@git+") || key.contains("@github:");
        if let Some(val) = vals.pop_front() {
            match val {
                Vals::Str(reg) => {
                    // Only set registry for npm packages, not git/github packages
                    if !is_git_package {
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

        // Checksum is last
        let checksum = vals.pop_front().and_then(|val| match val {
            Vals::Str(sha) => Some(sha),
            Vals::Info(_) => None,
        });

        Ok(Self {
            ident: key,
            info,
            registry,
            checksum,
            root: None,
        })
    }
}

#[cfg(test)]
mod test {
    use std::{str::FromStr, sync::OnceLock};

    use serde_json::json;
    use test_case::test_case;

    use super::*;
    use crate::{BunLockfile, Lockfile, bun::WorkspaceEntry};

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
            root: None,
        }
    );

    // GitHub package with CORRUPTED input (has empty string at position 1)
    // This tests that the deserializer fix correctly ignores registry for github packages
    fixture!(
        github_pkg_corrupted_input,
        PackageEntry,
        PackageEntry {
            ident: "@tanstack/react-store@github:TanStack/store#24a971c".into(),
            registry: None, // Even with corrupted 4-element input, github packages must NOT have registry
            info: Some(PackageInfo {
                dependencies: Some(("@tanstack/store".into(), "0.7.0".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("24a971c".into()),
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
    fn test_deserialization<T: for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(
        input: serde_json::Value,
        expected: &T,
    ) {
        let actual: T = serde_json::from_value(input).unwrap();
        assert_eq!(&actual, expected);
    }

    #[test]
    fn test_full_parse() {
        let contents = include_str!("../../fixtures/basic-bun-v0.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn test_patch() {
        let contents = include_str!("../../fixtures/bun-patch-v0.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn test_v1_create_turbo() {
        let contents = include_str!("../../fixtures/bun-v1-create-turbo.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());

        let lockfile = result.unwrap();

        // Test transitive closure calculation to ensure all dependencies can be
        // resolved
        for (workspace_path, workspace_entry) in &lockfile.data.workspaces {
            let mut unresolved_deps = std::collections::HashMap::new();

            if let Some(deps) = &workspace_entry.dependencies {
                unresolved_deps.extend(deps.clone());
            }
            if let Some(dev_deps) = &workspace_entry.dev_dependencies {
                unresolved_deps.extend(dev_deps.clone());
            }

            if !unresolved_deps.is_empty() {
                let closure =
                    crate::transitive_closure(&lockfile, workspace_path, unresolved_deps, false);
                assert!(
                    closure.is_ok(),
                    "Transitive closure failed for workspace '{}': {}",
                    workspace_path,
                    closure.unwrap_err()
                );
            }
        }
    }

    #[test]
    fn test_v1_issue_10410() {
        let contents = include_str!("../../fixtures/bun-v1-issue-10410.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());

        let lockfile = result.unwrap();

        let result = lockfile.all_dependencies("@tailwindcss/oxide-wasm32-wasi@4.1.13");
        assert!(
            result.is_ok(),
            "Failed to get dependencies for @tailwindcss/oxide-wasm32-wasi: {}",
            result.unwrap_err()
        );

        // Test full transitive closure for each workspace
        for (workspace_path, workspace_entry) in &lockfile.data.workspaces {
            let mut unresolved_deps = std::collections::HashMap::new();

            if let Some(deps) = &workspace_entry.dependencies {
                unresolved_deps.extend(deps.clone());
            }
            if let Some(dev_deps) = &workspace_entry.dev_dependencies {
                unresolved_deps.extend(dev_deps.clone());
            }

            if !unresolved_deps.is_empty() {
                let closure =
                    crate::transitive_closure(&lockfile, workspace_path, unresolved_deps, false);
                assert!(
                    closure.is_ok(),
                    "Transitive closure failed for workspace '{}': {}. This likely means a \
                     package entry is missing or bundled dependencies are not being resolved \
                     correctly.",
                    workspace_path,
                    closure.unwrap_err()
                );

                // Verify we got some packages in the closure
                let closure = closure.unwrap();
                assert!(
                    !closure.is_empty(),
                    "Expected non-empty transitive closure for workspace '{workspace_path}'"
                );
            }
        }
    }
}
