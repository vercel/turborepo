//! Internal setup platform primitives. Normalization and vendor artifact
//! selection are separate. Runtime host detection, version resolution, network
//! transfers, digest verification, and installation are deliberately out of
//! scope.

use semver::Version;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X64,
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    Gnu,
    Musl,
}

/// A normalized OS/architecture/libc tuple, independent of vendor artifact
/// support.
///
/// Callers cannot mutate the fields into an invalid combination:
///
/// ```compile_fail
/// fn change_os(platform: &mut turborepo_setup::Platform) {
///     platform.os = turborepo_setup::OperatingSystem::Windows;
/// }
/// ```
///
/// ```compile_fail
/// fn change_arch(platform: &mut turborepo_setup::Platform) {
///     platform.arch = turborepo_setup::Architecture::X64;
/// }
/// ```
///
/// ```compile_fail
/// fn change_libc(platform: &mut turborepo_setup::Platform) {
///     platform.libc = None;
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    os: OperatingSystem,
    arch: Architecture,
    libc: Option<Libc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlatformError {
    #[error("unsupported OS, architecture, or libc combination")]
    UnsupportedPlatform,
    #[error("no supported standard Node artifact for this platform (Linux requires glibc)")]
    UnsupportedNodeArtifact,
    #[error("Node artifact filename is {length} bytes; maximum is 255")]
    NodeArtifactFilenameTooLong { length: usize },
}

impl Platform {
    /// Accept Rust and vendor spellings without inferring a Linux libc.
    pub fn normalize(os: &str, arch: &str, libc: Option<&str>) -> Result<Self, PlatformError> {
        let os = match os {
            "windows" | "win32" => OperatingSystem::Windows,
            "macos" | "darwin" => OperatingSystem::Macos,
            "linux" => OperatingSystem::Linux,
            _ => return Err(PlatformError::UnsupportedPlatform),
        };
        let arch = match arch {
            "x86_64" | "x64" | "amd64" => Architecture::X64,
            "aarch64" | "arm64" => Architecture::Arm64,
            _ => return Err(PlatformError::UnsupportedPlatform),
        };
        let libc = match (os, libc) {
            (OperatingSystem::Linux, Some("gnu" | "glibc")) => Some(Libc::Gnu),
            (OperatingSystem::Linux, Some("musl")) => Some(Libc::Musl),
            (OperatingSystem::Windows | OperatingSystem::Macos, None) => None,
            _ => return Err(PlatformError::UnsupportedPlatform),
        };
        Ok(Self { os, arch, libc })
    }

    pub const fn os(&self) -> OperatingSystem {
        self.os
    }

    pub const fn arch(&self) -> Architecture {
        self.arch
    }

    pub const fn libc(&self) -> Option<Libc> {
        self.libc
    }
}

const MAX_ARTIFACT_FILENAME_BYTES: usize = 255;

/// Node's standard distribution layout, not a claim that every version exists.
///
/// Filenames are bounded to 255 bytes. Semver's validated syntax excludes path
/// separators and traversal components in the version.
#[derive(Debug, Clone)]
pub struct NodeArtifact {
    filename: String,
    directory: String,
}

impl NodeArtifact {
    /// Select a standard Node artifact for an explicitly supplied platform.
    ///
    /// Valid but unusually long semver prerelease/build metadata can exceed the
    /// filename's 255-byte limit and is rejected rather than truncated.
    pub fn for_platform(version: &Version, platform: Platform) -> Result<Self, PlatformError> {
        let (os, extension) = match (platform.os(), platform.libc()) {
            (OperatingSystem::Windows, None) => ("win", "zip"),
            (OperatingSystem::Macos, None) => ("darwin", "tar.gz"),
            (OperatingSystem::Linux, Some(Libc::Gnu)) => ("linux", "tar.gz"),
            _ => return Err(PlatformError::UnsupportedNodeArtifact),
        };
        let arch = match platform.arch() {
            Architecture::X64 => "x64",
            Architecture::Arm64 => "arm64",
        };
        let filename = format!("node-v{version}-{os}-{arch}.{extension}");
        if filename.len() > MAX_ARTIFACT_FILENAME_BYTES {
            return Err(PlatformError::NodeArtifactFilenameTooLong {
                length: filename.len(),
            });
        }
        Ok(Self {
            filename,
            directory: format!("https://nodejs.org/dist/v{version}"),
        })
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn url(&self) -> String {
        format!("{}/{}", self.directory, self.filename)
    }

    pub fn checksums_url(&self) -> String {
        format!("{}/SHASUMS256.txt", self.directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_aliases_map_to_exact_enums() -> Result<(), PlatformError> {
        for (os, expected_os) in [
            ("windows", OperatingSystem::Windows),
            ("win32", OperatingSystem::Windows),
            ("macos", OperatingSystem::Macos),
            ("darwin", OperatingSystem::Macos),
            ("linux", OperatingSystem::Linux),
        ] {
            let libcs: &[(Option<&str>, Option<Libc>)] = match expected_os {
                OperatingSystem::Linux => &[
                    (Some("gnu"), Some(Libc::Gnu)),
                    (Some("glibc"), Some(Libc::Gnu)),
                    (Some("musl"), Some(Libc::Musl)),
                ],
                _ => &[(None, None)],
            };
            for (arch, expected_arch) in [
                ("x86_64", Architecture::X64),
                ("x64", Architecture::X64),
                ("amd64", Architecture::X64),
                ("aarch64", Architecture::Arm64),
                ("arm64", Architecture::Arm64),
            ] {
                for &(libc, expected_libc) in libcs {
                    let platform = Platform::normalize(os, arch, libc)?;
                    assert_eq!(platform.os(), expected_os, "{os}/{arch}/{libc:?}");
                    assert_eq!(platform.arch(), expected_arch, "{os}/{arch}/{libc:?}");
                    assert_eq!(platform.libc(), expected_libc, "{os}/{arch}/{libc:?}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn unsupported_and_incomplete_tuples_are_rejected() {
        for (os, arch, libc) in [
            ("freebsd", "x64", None),
            ("", "x64", None),
            ("Windows", "x64", None),
            ("windows", "ia32", None),
            ("windows", "X64", None),
            ("macos", "", None),
            ("linux", "armv7l", Some("gnu")),
            ("linux", "riscv64", Some("gnu")),
            ("linux", "arm64", None),
            ("linux", "x64", None),
            ("linux", "x64", Some("")),
            ("linux", "x64", Some("unknown")),
            ("linux", "x64", Some("GNU")),
        ] {
            assert_eq!(
                Platform::normalize(os, arch, libc),
                Err(PlatformError::UnsupportedPlatform),
                "{os}/{arch}/{libc:?}"
            );
        }
    }

    #[test]
    fn non_linux_platforms_reject_every_libc() {
        for os in ["windows", "win32", "macos", "darwin"] {
            for arch in ["x64", "arm64"] {
                for libc in ["gnu", "glibc", "musl", "unknown", ""] {
                    assert_eq!(
                        Platform::normalize(os, arch, Some(libc)),
                        Err(PlatformError::UnsupportedPlatform),
                        "{os}/{arch}/{libc}"
                    );
                }
            }
        }
    }

    #[test]
    fn standard_node_artifacts_have_exact_names_and_urls() -> Result<(), Box<dyn std::error::Error>>
    {
        for version in ["22.14.0", "23.0.0-rc.1+build.42"] {
            let version = Version::parse(version)?;
            for (os, vendor_os, libc, extension) in [
                ("windows", "win", None, "zip"),
                ("macos", "darwin", None, "tar.gz"),
                ("linux", "linux", Some("gnu"), "tar.gz"),
            ] {
                for (arch, vendor_arch) in [("x86_64", "x64"), ("aarch64", "arm64")] {
                    let platform = Platform::normalize(os, arch, libc)?;
                    let artifact = NodeArtifact::for_platform(&version, platform)?;
                    let filename = format!("node-v{version}-{vendor_os}-{vendor_arch}.{extension}");
                    assert_eq!(artifact.filename(), filename);
                    assert_eq!(
                        artifact.url(),
                        format!("https://nodejs.org/dist/v{version}/{filename}")
                    );
                    assert_eq!(
                        artifact.checksums_url(),
                        format!("https://nodejs.org/dist/v{version}/SHASUMS256.txt")
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn normalized_musl_is_not_a_standard_node_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let version = Version::parse("22.14.0")?;
        for (arch, expected_arch) in [("x64", Architecture::X64), ("arm64", Architecture::Arm64)] {
            let musl = Platform::normalize("linux", arch, Some("musl"))?;
            assert_eq!(musl.os(), OperatingSystem::Linux);
            assert_eq!(musl.arch(), expected_arch);
            assert_eq!(musl.libc(), Some(Libc::Musl));
            assert!(matches!(
                NodeArtifact::for_platform(&version, musl),
                Err(PlatformError::UnsupportedNodeArtifact)
            ));
        }
        Ok(())
    }

    #[test]
    fn artifact_filename_limit_is_exact_for_all_platforms() -> Result<(), Box<dyn std::error::Error>>
    {
        for (os, libc) in [("windows", None), ("macos", None), ("linux", Some("gnu"))] {
            for arch in ["x64", "arm64"] {
                let platform = Platform::normalize(os, arch, libc)?;
                for separator in ["-", "+"] {
                    let short_version = Version::parse(&format!("1.2.3{separator}a"))?;
                    let short_artifact = NodeArtifact::for_platform(&short_version, platform)?;
                    let identifier_length = 255 - short_artifact.filename().len() + 1;
                    let version = Version::parse(&format!(
                        "1.2.3{separator}{}",
                        "a".repeat(identifier_length)
                    ))?;
                    let artifact = NodeArtifact::for_platform(&version, platform)?;
                    assert_eq!(artifact.filename().len(), 255);
                    assert!(artifact.filename().contains(&version.to_string()));

                    let too_long = Version::parse(&format!("{version}a"))?;
                    assert!(matches!(
                        NodeArtifact::for_platform(&too_long, platform),
                        Err(PlatformError::NodeArtifactFilenameTooLong { length: 256 })
                    ));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn semver_rejects_traversal_and_url_delimiters() -> Result<(), Box<dyn std::error::Error>> {
        for separator in ["-", "+"] {
            for identifier in [
                ".",
                "..",
                "../escape",
                "..\\escape",
                "/absolute",
                "\\absolute",
                "a..b",
                "a/b",
                "a\\b",
                "%2e%2e%2fescape",
                "a:b",
                "a?b",
                "a#b",
                "a\0b",
                "é",
            ] {
                let version = format!("1.2.3{separator}{identifier}");
                assert!(Version::parse(&version).is_err(), "{version:?}");
            }
        }
        let platform = Platform::normalize("linux", "x64", Some("gnu"))?;
        let version = Version::parse("1.2.3-alpha.1+build.42")?;
        let artifact = NodeArtifact::for_platform(&version, platform)?;
        assert_eq!(
            artifact.filename(),
            "node-v1.2.3-alpha.1+build.42-linux-x64.tar.gz"
        );
        Ok(())
    }
}
