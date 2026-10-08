//! Node-specific artifact names, formats, and URLs. Platform identity is
//! shared; these vendor spellings and availability rules are not.

use semver::Version;
use thiserror::Error;
use turborepo_platform::{Architecture, OperatingSystem, Platform};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NodeArtifactError {
    #[error("no supported standard Node artifact for this OS or architecture")]
    UnsupportedPlatform,
    #[error("Node artifact filename is {length} bytes; maximum is 255")]
    FilenameTooLong { length: usize },
}

/// Node's standard distribution layout, not a claim that every version exists
/// or that an archive is compatible with the runtime's system libraries.
///
/// Filenames are bounded to 255 bytes. Semver's validated syntax excludes path
/// separators and traversal components in the version.
#[derive(Debug, Clone)]
pub struct NodeArtifact {
    filename: String,
    directory: String,
}

impl NodeArtifact {
    pub fn for_platform(version: &Version, platform: Platform) -> Result<Self, NodeArtifactError> {
        let (os, extension) = match platform.os() {
            OperatingSystem::Windows => ("win", "zip"),
            OperatingSystem::Macos => ("darwin", "tar.gz"),
            OperatingSystem::Linux => ("linux", "tar.gz"),
            OperatingSystem::Unknown => return Err(NodeArtifactError::UnsupportedPlatform),
        };
        let arch = match platform.arch() {
            Architecture::X64 => "x64",
            Architecture::Arm64 => "arm64",
            Architecture::Unknown => return Err(NodeArtifactError::UnsupportedPlatform),
        };
        let filename = format!("node-v{version}-{os}-{arch}.{extension}");
        if filename.len() > 255 {
            return Err(NodeArtifactError::FilenameTooLong {
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

// Shared lock-to-vendor mapping for selection and provisioning. GNU artifact
// identity is not runtime libc detection; there are no official musl archives.
pub(crate) fn lock_target(platform: crate::lock::Platform) -> Option<(Platform, &'static str)> {
    use Architecture::{Arm64, X64};
    use OperatingSystem::{Linux, Macos, Windows};

    use crate::lock::Platform as Locked;
    let (os, arch, spelling) = match platform {
        Locked::MacosX64 => (Macos, X64, "macos-x64"),
        Locked::MacosArm64 => (Macos, Arm64, "macos-arm64"),
        Locked::LinuxX64Gnu => (Linux, X64, "linux-x64-gnu"),
        Locked::LinuxArm64Gnu => (Linux, Arm64, "linux-arm64-gnu"),
        Locked::WindowsX64 => (Windows, X64, "windows-x64"),
        Locked::WindowsArm64 => (Windows, Arm64, "windows-arm64"),
        _ => return None,
    };
    Some((Platform::new(os, arch), spelling))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS_ARTIFACTS: [(OperatingSystem, &str, &str); 3] = [
        (OperatingSystem::Windows, "win", "zip"),
        (OperatingSystem::Macos, "darwin", "tar.gz"),
        (OperatingSystem::Linux, "linux", "tar.gz"),
    ];
    const ARCH_ARTIFACTS: [(Architecture, &str); 2] =
        [(Architecture::X64, "x64"), (Architecture::Arm64, "arm64")];

    #[test]
    fn standard_node_artifacts_have_exact_names_and_urls() -> Result<(), Box<dyn std::error::Error>>
    {
        for version in ["22.14.0", "23.0.0-rc.1+build.42"] {
            let version = Version::parse(version)?;
            for (os, vendor_os, extension) in OS_ARTIFACTS {
                for (arch, vendor_arch) in ARCH_ARTIFACTS {
                    let platform = Platform::new(os, arch);
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
    fn unknown_targets_are_rejected() {
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
                let result =
                    NodeArtifact::for_platform(&Version::new(22, 14, 0), Platform::new(os, arch));
                assert_eq!(
                    result.is_err(),
                    os == OperatingSystem::Unknown || arch == Architecture::Unknown
                );
            }
        }
    }

    #[test]
    fn artifact_filename_limit_is_exact() -> Result<(), Box<dyn std::error::Error>> {
        for (os, _, _) in OS_ARTIFACTS {
            for (arch, _) in ARCH_ARTIFACTS {
                let platform = Platform::new(os, arch);
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
                        Err(NodeArtifactError::FilenameTooLong { length: 256 })
                    ));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn semver_rejects_path_and_url_delimiters() {
        for suffix in [
            "../escape",
            "..\\escape",
            "%2e%2e%2fescape",
            "a:b",
            "a?b",
            "a#b",
            "a\0b",
            "é",
        ] {
            for separator in ["-", "+"] {
                assert!(Version::parse(&format!("1.2.3{separator}{suffix}")).is_err());
            }
        }
    }
}
