use std::{
    env, fs,
    path::{Path, PathBuf},
};

use camino::Utf8PathBuf;
use dunce::canonicalize as fs_canonicalize;
use semver::Version;
use serde::Deserialize;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::package_json::PackageJson;

use crate::TurboState;

/// Structure that holds information on an existing local turbo install
#[derive(Debug)]
pub struct LocalTurboState {
    bin_path: PathBuf,
    version: String,
}

impl LocalTurboState {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn binary(&self) -> &Path {
        &self.bin_path
    }
    // Hoisted strategy:
    // - `bun install`
    // - `npm install`
    // - `yarn`
    // - `yarn install --flat`
    // - berry (nodeLinker: "node-modules")
    //
    // This also supports people directly depending upon the platform version.
    fn generate_hoisted_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        Some(root_path.join_component("node_modules"))
    }

    // Nested strategy:
    // - `npm install --install-strategy=shallow` (`npm install --global-style`)
    // - `npm install --install-strategy=nested` (`npm install --legacy-bundling`)
    // - berry (nodeLinker: "pnpm")
    //
    // Returns `node_modules/turbo/node_modules` — the caller appends
    // package-specific segments. This works for both legacy
    // (`turbo-{platform}`) and scoped (`@turbo/{platform}`) packages since
    // `join_components` handles multi-segment paths correctly.
    fn generate_nested_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        Some(root_path.join_components(&["node_modules", "turbo", "node_modules"]))
    }

    // Linked strategy:
    // - `pnpm install`
    // - `npm install --install-strategy=linked`
    fn generate_linked_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        // root_path/node_modules/turbo is a symlink. Canonicalize the symlink to what
        // it points to. We do this _before_ traversing up to the parent,
        // because on Windows, if you canonicalize a path that ends with `/..`
        // it traverses to the parent directory before it follows the symlink,
        // leading to the wrong place. We could separate the Windows
        // implementation, but this workaround works for other platforms as
        // well.
        let turbo_path = root_path.as_path().join("node_modules").join("turbo");

        match fs_canonicalize(&turbo_path) {
            Ok(canonical_path) => match canonical_path.parent() {
                Some(parent) => AbsoluteSystemPathBuf::try_from(parent).ok(),
                None => None,
            },
            Err(_) => {
                // On Windows, canonicalize can fail with permission errors even when
                // the symlink is valid. Try using read_link instead.
                #[cfg(target_os = "windows")]
                {
                    match fs::read_link(turbo_path.as_std_path()) {
                        Ok(link_target) => {
                            // The link target is relative to the symlink location
                            // e.g., ".pnpm/turbo@1.0.0/node_modules/turbo"
                            // We need to resolve it relative to node_modules directory
                            let node_modules =
                                PathBuf::from(root_path.as_path().as_str()).join("node_modules");
                            let resolved = node_modules.join(&link_target);

                            // Get the parent directory (should be .pnpm/turbo@1.0.0/node_modules)
                            resolved
                                .parent()
                                .and_then(|parent| AbsoluteSystemPathBuf::try_from(parent).ok())
                        }
                        Err(_) => None,
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    None
                }
            }
        }
    }

    // The unplugged directory doesn't have a fixed path.
    fn get_unplugged_base_path(root_path: &AbsoluteSystemPath) -> Utf8PathBuf {
        let yarn_rc_filename =
            env::var("YARN_RC_FILENAME").unwrap_or_else(|_| String::from(".yarnrc.yml"));
        let yarn_rc_filepath = root_path.as_path().join(yarn_rc_filename);

        let yarn_rc_yaml_string = fs::read_to_string(yarn_rc_filepath).unwrap_or_default();
        let yarn_rc: YarnRc = serde_yaml_ng::from_str(&yarn_rc_yaml_string).unwrap_or_default();

        root_path.as_path().join(yarn_rc.pnp_unplugged_folder)
    }

    // Unplugged strategy (Berry 2.1+): Berry encodes the package identity in
    // the unplugged directory name. For scoped `@turbo/linux-64` the dir is
    // `@turbo-linux-64-npm-{version}-{hash}`, for legacy `turbo-linux-64` it
    // is `turbo-linux-64-npm-{version}-{hash}`.
    fn find_in_unplugged(
        unplugged_base_path: &Utf8PathBuf,
        package_prefix: &str,
    ) -> Option<AbsoluteSystemPathBuf> {
        unplugged_base_path
            .read_dir_utf8()
            .ok()
            .and_then(|mut read_dir| {
                read_dir.find_map(|item| match item {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        if file_name.starts_with(package_prefix) {
                            AbsoluteSystemPathBuf::new(
                                unplugged_base_path.join(file_name).join("node_modules"),
                            )
                            .ok()
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                })
            })
    }

    /// Try to resolve a local turbo binary at a specific root + package path.
    /// Returns `None` if the binary doesn't exist or the package metadata is
    /// unreadable — the caller should try the next candidate.
    fn try_probe_binary(
        root: &AbsoluteSystemPath,
        package_path: &[&str],
        binary_name: &str,
    ) -> Option<Self> {
        let mut bin_components: Vec<&str> = Vec::with_capacity(package_path.len() + 2);
        bin_components.extend_from_slice(package_path);
        bin_components.extend_from_slice(&["bin", binary_name]);

        let bin_path = root.join_components(&bin_components);
        let bin_path = match fs_canonicalize(&bin_path) {
            Ok(p) => p,
            Err(_) => {
                debug!("No local turbo binary found at: {}", bin_path);
                return None;
            }
        };

        let mut json_components: Vec<&str> = Vec::with_capacity(package_path.len() + 1);
        json_components.extend_from_slice(package_path);
        json_components.push("package.json");
        let resolved_package_json_path = root.join_components(&json_components);

        let Some(platform_package_json) = PackageJson::load(&resolved_package_json_path).ok()
        else {
            debug!(
                "Failed to load package.json at: {}",
                resolved_package_json_path
            );
            return None;
        };
        let Some(local_version) = platform_package_json.version else {
            debug!(
                "No version field in package.json at: {}",
                resolved_package_json_path
            );
            return None;
        };

        debug!("Local turbo path: {}", bin_path.display());
        debug!("Local turbo version: {}", &local_version);
        Some(Self {
            bin_path,
            version: local_version,
        })
    }

    // We support twelve per-platform packages (six scoped `@turbo/{platform}`
    // and six legacy `turbo-{platform}`) and one `turbo` package which handles
    // indirection. We identify the per-platform package and execute the
    // appropriate binary directly. We can choose to operate this aggressively
    // because the _worst_ outcome is that we run global `turbo`.
    //
    // In spite of that, the only known unsupported local invocation is
    // Yarn/Berry < 2.1 PnP
    pub fn infer(root_path: &AbsoluteSystemPath) -> Option<Self> {
        let binary_name = TurboState::binary_name();

        // Prefer scoped `@turbo/{platform}` over legacy `turbo-{platform}`.
        // Scoped packages are the canonical format going forward; legacy is
        // retained for backward compatibility.
        let scoped_path: &[&str] = &[
            TurboState::scoped_platform_package_scope(),
            TurboState::scoped_platform_package_dir(),
        ];
        let legacy_path: &[&str] = &[TurboState::platform_package_name()];
        let package_paths: &[&[&str]] = &[scoped_path, legacy_path];

        // Ordered cheap-to-expensive: hoisted/nested are pure path joins,
        // linked requires symlink resolution, unplugged requires directory
        // scanning. Detecting the package manager is more expensive than
        // exhaustive search.
        let search_functions = [
            Self::generate_hoisted_path,
            Self::generate_nested_path,
            Self::generate_linked_path,
        ];

        // For each root, try all package formats before moving to the next
        // (more expensive) strategy. This avoids redundant filesystem work
        // compared to exhausting all roots per format.
        for root in search_functions
            .iter()
            .filter_map(|search_function| search_function(root_path))
        {
            for package_path in package_paths {
                if let Some(state) = Self::try_probe_binary(&root, package_path, binary_name) {
                    return Some(state);
                }
            }
        }

        // Unplugged strategy (Berry 2.1+): directory scanning is
        // package-name-aware because Berry encodes the identity in the
        // directory name. Read the unplugged base path once to avoid
        // re-parsing .yarnrc.yml.
        let unplugged_base_path = Self::get_unplugged_base_path(root_path);
        for package_path in package_paths {
            // Berry unplugged dirs use `{name}-npm-{version}-{hash}`.
            // For scoped `@turbo/linux-64` this becomes `@turbo-linux-64-npm-...`.
            let unplugged_prefix = package_path.join("-");
            if let Some(root) = Self::find_in_unplugged(&unplugged_base_path, &unplugged_prefix)
                && let Some(state) = Self::try_probe_binary(&root, package_path, binary_name)
            {
                return Some(state);
            }
        }

        None
    }

    /// Check to see if the detected local executable is the one currently
    /// running.
    pub fn local_is_self(&self) -> bool {
        std::env::current_exe().is_ok_and(|current_exe| {
            fs_canonicalize(current_exe)
                .is_ok_and(|canonical_current_exe| canonical_current_exe == self.bin_path)
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YarnRc {
    pnp_unplugged_folder: Utf8PathBuf,
}

impl Default for YarnRc {
    fn default() -> Self {
        Self {
            pnp_unplugged_folder: [".yarn", "unplugged"].iter().collect(),
        }
    }
}

pub fn turbo_version_has_shim(version: &str) -> bool {
    if let Ok(version) = Version::parse(version) {
        // only need to check major and minor (this will include canaries)
        if version.major == 1 {
            return version.minor >= 7;
        }
        version.major > 1
    } else {
        // In the case that we don't get passed a valid semver we should avoid a panic.
        // We shouldn't hit this we introduce back inferring package version from schema
        // or package.json.
        true
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;

    #[test_case("1.7.0-canary.0", true; "canary")]
    #[test_case("1.7.0-canary.1", true; "newer_canary")]
    #[test_case("1.7.1-canary.6", true; "newer_minor_canary")]
    #[test_case("1.7.0", true; "release")]
    #[test_case("1.6.3", false; "old")]
    #[test_case("1.6.2-canary.1", false; "old_canary")]
    #[test_case("1.8.0", true; "new")]
    #[test_case("2.1.0", true; "new major")]
    #[test_case("*", true; "star")]
    #[test_case("2.0", true; "version 2 0")]
    #[test_case("latest", true; "latest")]
    #[test_case("canary", true; "canary tag")]
    fn test_skip_infer_version_constraint(version: &str, expected: bool) {
        assert_eq!(turbo_version_has_shim(version), expected);
    }

    fn create_mock_turbo_install(root: &AbsoluteSystemPath, package_path: &[&str], version: &str) {
        let binary_name = TurboState::binary_name();

        let mut bin_components: Vec<&str> = package_path.to_vec();
        bin_components.extend_from_slice(&["bin", binary_name]);
        let bin_file = root.join_components(&bin_components);
        bin_file.ensure_dir().unwrap();
        bin_file.create_with_contents("").unwrap();

        let mut json_components: Vec<&str> = package_path.to_vec();
        json_components.push("package.json");
        let json_file = root.join_components(&json_components);
        json_file.ensure_dir().unwrap();
        json_file
            .create_with_contents(format!(
                r#"{{"name": "test-turbo", "version": "{}"}}"#,
                version,
            ))
            .unwrap();
    }

    fn scoped_path() -> Vec<&'static str> {
        vec![
            TurboState::scoped_platform_package_scope(),
            TurboState::scoped_platform_package_dir(),
        ]
    }

    fn legacy_path() -> Vec<&'static str> {
        vec![TurboState::platform_package_name()]
    }

    #[test]
    fn test_infer_hoisted_scoped() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let nm = root.join_component("node_modules");

        create_mock_turbo_install(&nm, &scoped_path(), "2.0.0");

        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "2.0.0");
    }

    #[test]
    fn test_infer_hoisted_legacy_fallback() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let nm = root.join_component("node_modules");

        create_mock_turbo_install(&nm, &legacy_path(), "1.9.0");

        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "1.9.0");
    }

    #[test]
    fn test_infer_scoped_preferred_over_legacy() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let nm = root.join_component("node_modules");

        create_mock_turbo_install(&nm, &scoped_path(), "3.0.0");
        create_mock_turbo_install(&nm, &legacy_path(), "2.0.0");

        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "3.0.0");
    }

    #[test]
    fn test_infer_empty_dir_returns_none() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        assert!(LocalTurboState::infer(root).is_none());
    }

    #[test]
    fn test_infer_malformed_package_json_continues_search() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let nm = root.join_component("node_modules");

        let scoped = scoped_path();
        let binary_name = TurboState::binary_name();

        // Create scoped binary but with invalid package.json
        let mut bin_components: Vec<&str> = scoped.clone();
        bin_components.extend_from_slice(&["bin", binary_name]);
        let bin_file = nm.join_components(&bin_components);
        bin_file.ensure_dir().unwrap();
        bin_file.create_with_contents("").unwrap();

        let mut json_components: Vec<&str> = scoped.clone();
        json_components.push("package.json");
        let json_file = nm.join_components(&json_components);
        json_file.ensure_dir().unwrap();
        json_file.create_with_contents("not valid json").unwrap();

        // Create valid legacy install
        create_mock_turbo_install(&nm, &legacy_path(), "1.8.0");

        // Should fall through to legacy despite scoped binary existing
        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "1.8.0");
    }

    #[test]
    fn test_infer_unplugged_scoped() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();

        // Berry unplugged dirs use `{identity}-npm-{version}-{hash}`.
        // For @turbo/linux-64 → @turbo-linux-64-npm-2.1.0-abc123
        let scoped_dir = TurboState::scoped_platform_package_dir();
        let unplugged_dir_name = format!("@turbo-{}-npm-2.1.0-abc123", scoped_dir);

        let unplugged_nm =
            root.join_components(&[".yarn", "unplugged", &unplugged_dir_name, "node_modules"]);
        create_mock_turbo_install(&unplugged_nm, &scoped_path(), "2.1.0");

        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "2.1.0");
    }

    #[test]
    fn test_infer_unplugged_legacy() {
        let tmpdir = TempDir::with_prefix("turbo_infer").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();

        let platform_package_name = TurboState::platform_package_name();
        let unplugged_dir_name = format!("{}-npm-1.9.0-def456", platform_package_name);

        let unplugged_nm =
            root.join_components(&[".yarn", "unplugged", &unplugged_dir_name, "node_modules"]);
        create_mock_turbo_install(&unplugged_nm, &legacy_path(), "1.9.0");

        let result = LocalTurboState::infer(root).unwrap();
        assert_eq!(result.version(), "1.9.0");
    }
}
