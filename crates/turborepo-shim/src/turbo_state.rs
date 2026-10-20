use const_format::formatcp;
use turborepo_platform::{Architecture, OperatingSystem, Platform};

const fn package_os(os: OperatingSystem) -> &'static str {
    match os {
        OperatingSystem::Windows => "windows",
        OperatingSystem::Macos => "darwin",
        OperatingSystem::Linux => "linux",
        OperatingSystem::Unknown => "unknown",
    }
}

const fn package_arch(arch: Architecture) -> &'static str {
    match arch {
        Architecture::X64 => "64",
        Architecture::Arm64 => "arm64",
        Architecture::Unknown => "unknown",
    }
}

/// Struct containing helper methods for querying information about the
/// currently running turbo binary.
#[derive(Debug)]
pub struct TurboState;

impl TurboState {
    pub const fn platform_name() -> &'static str {
        const TARGET: Platform = Platform::current();
        const ARCH: &str = package_arch(TARGET.arch());
        const OS: &str = package_os(TARGET.os());

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
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_package_spellings_for_explicit_targets() {
        for (os, os_name) in [
            (OperatingSystem::Windows, "windows"),
            (OperatingSystem::Macos, "darwin"),
            (OperatingSystem::Linux, "linux"),
            (OperatingSystem::Unknown, "unknown"),
        ] {
            for (arch, arch_name) in [
                (Architecture::X64, "64"),
                (Architecture::Arm64, "arm64"),
                (Architecture::Unknown, "unknown"),
            ] {
                let platform = Platform::new(os, arch);
                assert_eq!(
                    format!(
                        "{}-{}",
                        package_os(platform.os()),
                        package_arch(platform.arch())
                    ),
                    format!("{os_name}-{arch_name}")
                );
            }
        }
    }

    #[test]
    fn test_current_package_names_preserve_legacy_spellings() {
        const PLATFORM: &str = TurboState::platform_name();
        const PACKAGE: &str = TurboState::platform_package_name();
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "windows",
            "linux" => "linux",
            _ => "unknown",
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "64",
            "aarch64" => "arm64",
            _ => "unknown",
        };
        let expected = format!("{os}-{arch}");
        assert_eq!(PLATFORM, expected);
        assert_eq!(PACKAGE, format!("turbo-{expected}"));
        assert_eq!(TurboState::scoped_platform_package_scope(), "@turbo");
        assert_eq!(TurboState::scoped_platform_package_dir(), expected);
        assert_eq!(
            TurboState::binary_name(),
            if cfg!(windows) { "turbo.exe" } else { "turbo" }
        );
    }

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
