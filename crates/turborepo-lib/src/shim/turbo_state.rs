use std::{env, path::PathBuf};

use const_format::formatcp;
use turborepo_repository::inference::RepoState;

use crate::get_version;

#[derive(Debug)]
pub struct TurboState {
    bin_path: Option<PathBuf>,
    version: &'static str,
    repo_state: Option<RepoState>,
}

impl Default for TurboState {
    fn default() -> Self {
        Self {
            bin_path: env::current_exe().ok(),
            version: get_version(),
            repo_state: None,
        }
    }
}

impl TurboState {
    pub const fn platform_name() -> &'static str {
        const ARCH: &str = {
            #[cfg(target_arch = "x86_64")]
            {
                "64"
            }
            #[cfg(target_arch = "aarch64")]
            {
                "arm64"
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                "unknown"
            }
        };

        const OS: &str = {
            #[cfg(target_os = "macos")]
            {
                "darwin"
            }
            #[cfg(target_os = "windows")]
            {
                "windows"
            }
            #[cfg(target_os = "linux")]
            {
                "linux"
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                "unknown"
            }
        };

        formatcp!("{}-{}", OS, ARCH)
    }

    pub const fn platform_package_name() -> &'static str {
        formatcp!("turbo-{}", TurboState::platform_name())
    }

    pub const fn binary_name() -> &'static str {
        {
            #[cfg(windows)]
            {
                "turbo.exe"
            }
            #[cfg(not(windows))]
            {
                "turbo"
            }
        }
    }

    #[allow(dead_code)]
    pub fn version() -> &'static str {
        include_str!("../../../../version.txt")
            .lines()
            .next()
            .expect("Failed to read version from version.txt")
    }
}
