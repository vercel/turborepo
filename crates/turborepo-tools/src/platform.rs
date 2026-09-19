//! Host platform detection and the naming schemes upstream distributions use.

use std::fmt;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl Platform {
    /// The platform turbo is running on, or an error when no upstream ships
    /// binaries for it.
    pub fn current() -> Result<Self, Error> {
        Self::from_consts(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn from_consts(os: &str, arch: &str) -> Result<Self, Error> {
        let unsupported = || Error::UnsupportedPlatform {
            os: os.to_string(),
            arch: arch.to_string(),
        };
        let os_kind = match os {
            "linux" => Os::Linux,
            "macos" => Os::MacOs,
            "windows" => Os::Windows,
            _ => return Err(unsupported()),
        };
        let arch_kind = match arch {
            "x86_64" => Arch::X64,
            "aarch64" => Arch::Arm64,
            _ => return Err(unsupported()),
        };
        Ok(Self {
            os: os_kind,
            arch: arch_kind,
        })
    }

    pub fn is_windows(&self) -> bool {
        self.os == Os::Windows
    }

    /// `foo` or `foo.exe`.
    pub fn exe(&self, name: &str) -> String {
        if self.is_windows() {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    /// Rust target triple, as used by rustup and uv release assets.
    pub fn rust_triple(&self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Linux, Arch::X64) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Arch::Arm64) => "aarch64-unknown-linux-gnu",
            (Os::MacOs, Arch::X64) => "x86_64-apple-darwin",
            (Os::MacOs, Arch::Arm64) => "aarch64-apple-darwin",
            (Os::Windows, Arch::X64) => "x86_64-pc-windows-msvc",
            (Os::Windows, Arch::Arm64) => "aarch64-pc-windows-msvc",
        }
    }

    /// `node-v22.1.0-<this>.tar.gz` naming on nodejs.org.
    pub fn node_suffix(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "win",
        };
        let arch = match self.arch {
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
        };
        format!("{os}-{arch}")
    }

    /// `bun-<this>.zip` naming on GitHub releases.
    pub fn bun_suffix(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "windows",
        };
        let arch = match self.arch {
            Arch::X64 => "x64",
            Arch::Arm64 => "aarch64",
        };
        format!("{os}-{arch}")
    }

    /// `go1.22.3.<this>.tar.gz` naming on go.dev.
    pub fn go_suffix(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "windows",
        };
        let arch = match self.arch {
            Arch::X64 => "amd64",
            Arch::Arm64 => "arm64",
        };
        format!("{os}-{arch}")
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.rust_triple())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_platforms() {
        let linux = Platform::from_consts("linux", "x86_64").unwrap();
        assert_eq!(linux.node_suffix(), "linux-x64");
        assert_eq!(linux.go_suffix(), "linux-amd64");
        assert_eq!(linux.bun_suffix(), "linux-x64");
        assert_eq!(linux.rust_triple(), "x86_64-unknown-linux-gnu");
        assert_eq!(linux.exe("go"), "go");

        let win = Platform::from_consts("windows", "aarch64").unwrap();
        assert_eq!(win.node_suffix(), "win-arm64");
        assert_eq!(win.exe("go"), "go.exe");
        assert_eq!(win.bun_suffix(), "windows-aarch64");
    }

    #[test]
    fn rejects_unknown_platforms() {
        assert!(matches!(
            Platform::from_consts("freebsd", "x86_64"),
            Err(Error::UnsupportedPlatform { .. })
        ));
        assert!(matches!(
            Platform::from_consts("linux", "riscv64"),
            Err(Error::UnsupportedPlatform { .. })
        ));
    }
}
