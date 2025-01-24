use std::collections::VecDeque;

use serde::Deserialize;

use super::{PackageEntry, PackageInfo};
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
        // For workspace packages deps are second element, rest have them as third
        // element
        let info = vals
            .pop_front()
            .and_then(val_to_info)
            .or_else(|| vals.pop_front().and_then(val_to_info));
        Ok(Self {
            key,
            info,
            // The rest are only necessary for serializing a lockfile and aren't needed until adding
            // `prune` support
            registry: None,
            checksum: None,
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
    use crate::{bun::WorkspaceEntry, BunLockfile};

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
            key: "is-odd@3.0.1".into(),
            registry: None,
            info: Some(PackageInfo {
                dependencies: Some(("is-number".into(), "^6.0.0".into()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: None,
            root: None,
        }
    );

    fixture!(
        workspace_pkg,
        PackageEntry,
        PackageEntry {
            key: "docs".into(),
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
    #[test_case(json!({"name": "bun-test", "devDependencies": {"turbo": "^2.3.3"}}), basic_workspace() ; "basic")]
    #[test_case(json!({"name": "docs", "version": "0.1.0"}), workspace_with_version() ; "with version")]
    #[test_case(json!(["is-odd@3.0.1", "", {"dependencies": {"is-number": "^6.0.0"}}, "sha"]), registry_pkg() ; "registry package")]
    #[test_case(json!(["docs", {"dependencies": {"is-odd": "3.0.1"}}]), workspace_pkg() ; "workspace package")]
    fn test_deserialization<T: for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(
        input: serde_json::Value,
        expected: &T,
    ) {
        let actual: T = serde_json::from_value(input).unwrap();
        assert_eq!(&actual, expected);
    }

    #[test]
    fn test_full_parse() {
        let contents = include_str!("../../fixtures/basic-bun.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn test_patch() {
        let contents = include_str!("../../fixtures/bun-patch.lock");
        let result = BunLockfile::from_str(contents);
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}
