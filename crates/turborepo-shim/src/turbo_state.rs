use const_format::formatcp;

/// Struct containing helper methods for querying information about the
/// currently running turbo binary.
#[derive(Debug)]
pub struct TurboState;

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

    /// Scope segment for `@turbo/{platform}` packages. Split from dir to
    /// avoid `/` in a single `join_components` segment (which debug-asserts).
    pub const fn scoped_platform_package_scope() -> &'static str {
        "@turbo"
    }

    /// Directory segment under the scope (e.g. `"linux-64"`).
    pub const fn scoped_platform_package_dir() -> &'static str {
        TurboState::platform_name()
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

    pub fn version() -> &'static str {
        include_str!("../../../version.txt")
            .lines()
            .next()
            .expect("Failed to read version from version.txt")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_scoped_package_path_segments_have_no_separators() {
        let scope = TurboState::scoped_platform_package_scope();
        let dir = TurboState::scoped_platform_package_dir();
        assert!(
            scope.starts_with('@'),
            "scope must start with '@' for npm scoped packages"
        );
        assert!(
            !scope.contains('/') && !scope.contains('\\'),
            "scope segment must not contain path separators (join_components constraint)"
        );
        assert!(
            !dir.contains('/') && !dir.contains('\\'),
            "dir segment must not contain path separators (join_components constraint)"
        );
    }
}
