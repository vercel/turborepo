use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use dunce::canonicalize as fs_canonicalize;
use log::debug;
use serde::{Deserialize, Serialize};

use super::turbo_state::TurboState;
use crate::files::{package_json, yarn_rc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTurboState {
    pub bin_path: PathBuf,
    pub version: String,
}

impl LocalTurboState {
    // Hoisted strategy:
    // - `npm install`
    // - `yarn`
    // - `yarn install --flat`
    // - berry (nodeLinker: "node-modules")
    //
    // This also supports people directly depending upon the platform version.
    fn generate_hoisted_path(root_path: &Path) -> Option<PathBuf> {
        Some(root_path.join("node_modules"))
    }

    // Nested strategy:
    // - `npm install --install-strategy=shallow` (`npm install --global-style`)
    // - `npm install --install-strategy=nested` (`npm install --legacy-bundling`)
    // - berry (nodeLinker: "pnpm")
    fn generate_nested_path(root_path: &Path) -> Option<PathBuf> {
        Some(
            root_path
                .join("node_modules")
                .join("turbo")
                .join("node_modules"),
        )
    }

    // Linked strategy:
    // - `pnpm install`
    // - `npm install --install-strategy=linked`
    fn generate_linked_path(root_path: &Path) -> Option<PathBuf> {
        fs_canonicalize(root_path.join("node_modules").join("turbo").join("..")).ok()
    }

    // The unplugged directory doesn't have a fixed path.
    fn get_unplugged_base_path(root_path: &Path) -> PathBuf {
        let yarn_rc_filename =
            env::var_os("YARN_RC_FILENAME").unwrap_or_else(|| OsString::from(".yarnrc.yml"));
        let yarn_rc_filepath = root_path.join(yarn_rc_filename);
        let yarn_rc = yarn_rc::read(&yarn_rc_filepath).unwrap_or_default();

        root_path.join(yarn_rc.pnp_unplugged_folder)
    }

    // Unplugged strategy:
    // - berry 2.1+
    fn generate_unplugged_path(root_path: &Path) -> Option<PathBuf> {
        let platform_package_name = TurboState::platform_package_name();
        let unplugged_base_path = Self::get_unplugged_base_path(root_path);

        unplugged_base_path
            .read_dir()
            .ok()
            .and_then(|mut read_dir| {
                // berry includes additional metadata in the filename.
                // We actually have to find the platform package.
                read_dir.find_map(|item| match item {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        if file_name
                            .to_string_lossy()
                            .starts_with(platform_package_name)
                        {
                            Some(unplugged_base_path.join(file_name).join("node_modules"))
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
    pub fn infer(root_path: &Path) -> Option<Self> {
        let platform_package_name = TurboState::platform_package_name();
        let binary_name = TurboState::binary_name();

        let platform_package_json_path: PathBuf =
            [platform_package_name, "package.json"].iter().collect();
        let platform_package_executable_path: PathBuf =
            [platform_package_name, "bin", binary_name].iter().collect();

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
            let bin_path = root.join(&platform_package_executable_path);
            match fs_canonicalize(&bin_path) {
                Ok(bin_path) => {
                    // This is done in a loop and Clippy's suggestion is wrong.
                    #[allow(clippy::needless_borrow)]
                    let resolved_package_json_path = root.join(&platform_package_json_path);
                    let platform_package_json =
                        package_json::read(&resolved_package_json_path).unwrap_or_default();

                    let version = match platform_package_json.version {
                        Some(version) => version,
                        None => continue,
                    };

                    debug!("Local turbo path: {}", bin_path.display());
                    debug!("Local turbo version: {}", version);
                    return Some(Self { bin_path, version });
                }
                Err(_) => debug!("No local turbo binary found at: {}", bin_path.display()),
            }
        }

        None
    }
}
