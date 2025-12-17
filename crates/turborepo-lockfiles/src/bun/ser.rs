use serde::{Serialize, ser::SerializeTuple};

use super::PackageEntry;

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
impl Serialize for PackageEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(4)?;

        // First value is always the package key
        tuple.serialize_element(&self.ident)?;

        // For root packages, only thing left to serialize is the root info
        if let Some(root) = &self.root {
            tuple.serialize_element(root)?;
            return tuple.end();
        }

        // npm packages have a registry (but not git/github packages)
        if let Some(registry) = &self.registry {
            // Defense-in-depth: skip registry for git/github packages even if incorrectly set
            let is_git_package = self.ident.contains("@git+") || self.ident.contains("@github:");
            if !is_git_package {
                tuple.serialize_element(registry)?;
            }
        }

        // All packages have info in the next slot
        if let Some(info) = &self.info {
            tuple.serialize_element(info)?;
        };

        // npm packages, git, and github have a checksum/integrity
        if let Some(checksum) = &self.checksum {
            tuple.serialize_element(checksum)?;
        }

        tuple.end()
    }
}

#[cfg(test)]
mod test {
    use std::sync::OnceLock;

    use serde_json::json;
    use test_case::test_case;

    use super::*;
    use crate::bun::{PackageInfo, RootInfo, WorkspaceEntry};

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

    // GitHub package fixture - should never have a registry field in output
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

    // Git package fixture - should never have a registry field in output
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

    // Test fixture to verify defense-in-depth: even if a github package
    // somehow has registry set (e.g., from corrupted input), serialization
    // should NOT output the registry field
    fixture!(
        github_pkg_with_corrupted_registry,
        PackageEntry,
        PackageEntry {
            ident: "@tanstack/react-store@github:TanStack/store#24a971c".into(),
            registry: Some("".into()), // Incorrectly set registry (corruption case)
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

    #[test_case(json!({"name": "bun-test", "devDependencies": {"turbo": "^2.3.3"}}), basic_workspace() ; "basic")]
    #[test_case(json!({"name": "docs", "version": "0.1.0"}), workspace_with_version() ; "with version")]
    #[test_case(json!(["is-odd@3.0.1", "", {"dependencies": {"is-number": "^6.0.0"}, "devDependencies": {"is-bigint": "1.1.0"}, "peerDependencies": {"is-even": "1.0.0"}, "optionalDependencies": {"is-regexp": "1.0.0"}, "optionalPeers": ["is-even"]}, "sha"]), registry_pkg() ; "registry package")]
    #[test_case(json!(["docs", {"dependencies": {"is-odd": "3.0.1"}}]), workspace_pkg() ; "workspace package")]
    #[test_case(json!(["some-package@root:", {"bin": "bin", "binDir": "binDir"}]), root_pkg() ; "root package")]
    #[test_case(json!(["@tanstack/react-store@github:TanStack/store#24a971c", {"dependencies": {"@tanstack/store": "0.7.0"}}, "24a971c"]), github_pkg() ; "github package")]
    #[test_case(json!(["my-package@git+https://github.com/user/repo#abc123", {"dependencies": {"lodash": "4.17.21"}}, "abc123"]), git_pkg() ; "git package")]
    // Defense-in-depth test: corrupted registry should be stripped from github packages during serialization
    #[test_case(json!(["@tanstack/react-store@github:TanStack/store#24a971c", {"dependencies": {"@tanstack/store": "0.7.0"}}, "24a971c"]), github_pkg_with_corrupted_registry() ; "github package with corrupted registry stripped")]
    fn test_serialization<T: Serialize + PartialEq + std::fmt::Debug>(
        expected: serde_json::Value,
        input: &T,
    ) {
        let actual = serde_json::to_value(input).unwrap();
        assert_eq!(actual, expected);
    }
}
