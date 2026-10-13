//! OS and architecture values shared by Turborepo crates.
//!
//! [`Platform::current`] describes the compiled binary's target. Explicit
//! values describe other targets without cross-compiling. Vendor naming and
//! artifact support belong to consumers; this crate does not detect runtime
//! system libraries.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Macos,
    Linux,
    /// A target outside the operating systems currently used by Turborepo.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X64,
    Arm64,
    /// A target outside the architectures currently used by Turborepo.
    Unknown,
}

/// OS and architecture, independent of any vendor's artifact support.
///
/// ```
/// use turborepo_platform::{Architecture, OperatingSystem, Platform};
///
/// const TARGET: Platform = Platform::new(OperatingSystem::Linux, Architecture::Arm64);
/// assert_eq!(TARGET.arch(), Architecture::Arm64);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    os: OperatingSystem,
    arch: Architecture,
}

impl Platform {
    pub const fn new(os: OperatingSystem, arch: Architecture) -> Self {
        Self { os, arch }
    }

    /// The compiled binary's OS and architecture, not runtime compatibility
    /// detection or a guarantee that a vendor publishes a matching artifact.
    pub const fn current() -> Self {
        let os = if cfg!(target_os = "windows") {
            OperatingSystem::Windows
        } else if cfg!(target_os = "macos") {
            OperatingSystem::Macos
        } else if cfg!(target_os = "linux") {
            OperatingSystem::Linux
        } else {
            OperatingSystem::Unknown
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Architecture::X64
        } else if cfg!(target_arch = "aarch64") {
            Architecture::Arm64
        } else {
            Architecture::Unknown
        };
        Self::new(os, arch)
    }

    pub const fn os(self) -> OperatingSystem {
        self.os
    }

    pub const fn arch(self) -> Architecture {
        self.arch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_targets_preserve_os_and_architecture() {
        for os in [
            OperatingSystem::Windows,
            OperatingSystem::Macos,
            OperatingSystem::Linux,
            OperatingSystem::Unknown,
        ] {
            for arch in [
                Architecture::X64,
                Architecture::Arm64,
                Architecture::Unknown,
            ] {
                let platform = Platform::new(os, arch);
                assert_eq!(platform.os(), os);
                assert_eq!(platform.arch(), arch);
            }
        }
    }

    #[test]
    fn current_is_const_and_matches_rust_target_constants() {
        const CURRENT: Platform = Platform::current();
        let os = match std::env::consts::OS {
            "windows" => OperatingSystem::Windows,
            "macos" => OperatingSystem::Macos,
            "linux" => OperatingSystem::Linux,
            _ => OperatingSystem::Unknown,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Architecture::X64,
            "aarch64" => Architecture::Arm64,
            _ => Architecture::Unknown,
        };
        assert_eq!(CURRENT, Platform::new(os, arch));
    }
}
