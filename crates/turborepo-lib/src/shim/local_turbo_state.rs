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

use super::TurboState;

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
        let canonical_path =
            fs_canonicalize(root_path.as_path().join("node_modules").join("turbo")).ok()?;

        AbsoluteSystemPathBuf::try_from(canonical_path.parent()?).ok()
    }

    // The unplugged directory doesn't have a fixed path.
    fn get_unplugged_base_path(root_path: &AbsoluteSystemPath) -> Utf8PathBuf {
        let yarn_rc_filename =
            env::var("YARN_RC_FILENAME").unwrap_or_else(|_| String::from(".yarnrc.yml"));
        let yarn_rc_filepath = root_path.as_path().join(yarn_rc_filename);

        let yarn_rc_yaml_string = fs::read_to_string(yarn_rc_filepath).unwrap_or_default();
        let yarn_rc: YarnRc = serde_yaml::from_str(&yarn_rc_yaml_string).unwrap_or_default();

        root_path.as_path().join(yarn_rc.pnp_unplugged_folder)
    }

    // Unplugged strategy:
    // - berry 2.1+
    fn generate_unplugged_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        let platform_package_name = TurboState::platform_package_name();
        let unplugged_base_path = Self::get_unplugged_base_path(root_path);

        unplugged_base_path
            .read_dir_utf8()
            .ok()
            .and_then(|mut read_dir| {
                // berry includes additional metadata in the filename.
                // We actually have to find the platform package.
                read_dir.find_map(|item| match item {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        if file_name.starts_with(platform_package_name) {
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

    // We support six per-platform packages and one `turbo` package which handles
    // indirection. We identify the per-platform package and execute the appropriate
    // binary directly. We can choose to operate this aggressively because the
    // _worst_ outcome is that we run global `turbo`.
    //
    // In spite of that, the only known unsupported local invocation is Yarn/Berry <
    // 2.1 PnP
    pub fn infer(root_path: &AbsoluteSystemPath) -> Option<Self> {
        let platform_package_name = TurboState::platform_package_name();
        let binary_name = TurboState::binary_name();

        let platform_package_json_path_components = [platform_package_name, "package.json"];
        let platform_package_executable_path_components =
            [platform_package_name, "bin", binary_name];

        // These are lazy because the last two are more expensive.
        let search_functions = [
            Self::generate_hoisted_path,
            Self::generate_nested_path,
            Self::generate_linked_path,
            Self::generate_unplugged_path,
        ];

        // Detecting the package manager is more expensive than just doing an exhaustive
        // search.
        for root in search_functions
            .iter()
            .filter_map(|search_function| search_function(root_path))
        {
            // Needs borrow because of the loop.
            #[allow(clippy::needless_borrow)]
            let bin_path = root.join_components(&platform_package_executable_path_components);
            match fs_canonicalize(&bin_path) {
                Ok(bin_path) => {
                    let resolved_package_json_path =
                        root.join_components(&platform_package_json_path_components);
                    let platform_package_json =
                        PackageJson::load(&resolved_package_json_path).ok()?;
                    let local_version = platform_package_json.version?;

                    debug!("Local turbo path: {}", bin_path.display());
                    debug!("Local turbo version: {}", &local_version);
                    return Some(Self {
                        bin_path,
                        version: local_version,
                    });
                }
                Err(_) => debug!("No local turbo binary found at: {}", bin_path),
            }
        }

        None
    }

    pub fn supports_skip_infer_and_single_package(&self) -> bool {
        turbo_version_has_shim(&self.version)
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

fn turbo_version_has_shim(version: &str) -> bool {
    let version = Version::parse(version).unwrap();
    // only need to check major and minor (this will include canaries)
    if version.major == 1 {
        return version.minor >= 7;
    }

    version.major > 1
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_skip_infer_version_constraint() {
        let canary = "1.7.0-canary.0";
        let newer_canary = "1.7.0-canary.1";
        let newer_minor_canary = "1.7.1-canary.6";
        let release = "1.7.0";
        let old = "1.6.3";
        let old_canary = "1.6.2-canary.1";
        let new = "1.8.0";
        let new_major = "2.1.0";

        assert!(turbo_version_has_shim(release));
        assert!(turbo_version_has_shim(canary));
        assert!(turbo_version_has_shim(newer_canary));
        assert!(turbo_version_has_shim(newer_minor_canary));
        assert!(turbo_version_has_shim(new));
        assert!(turbo_version_has_shim(new_major));
        assert!(!turbo_version_has_shim(old));
        assert!(!turbo_version_has_shim(old_canary));
    }
}
