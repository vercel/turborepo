//! Official Astral uv release names and URLs. Lock platform identity is shared;
//! these target triples and archive layouts are uv's.

use std::collections::BTreeMap;

use semver::Version;

use crate::lock::{Artifact, Format, Platform};

/// Astral's release mirror serves metadata and archives without redirects.
pub(crate) const ORIGIN: &str = "https://releases.astral.sh";
pub(crate) const INDEX_PATH: &str = "/github/versions/main/v1/uv.ndjson";

/// Every native lock platform with an official uv archive. Promotion support
/// is decided by the installer, not by recording a portable selection.
pub(crate) const PLATFORMS: [Platform; 8] = [
    Platform::MacosX64,
    Platform::MacosArm64,
    Platform::LinuxX64Gnu,
    Platform::LinuxArm64Gnu,
    Platform::LinuxX64Musl,
    Platform::LinuxArm64Musl,
    Platform::WindowsX64,
    Platform::WindowsArm64,
];

/// Canonical uv release identity: `X.Y.Z` without prerelease or build data.
pub(crate) fn exact_release(value: &str) -> Option<Version> {
    Version::parse(value)
        .ok()
        .filter(|v| v.pre.is_empty() && v.build.is_empty() && v.to_string() == value)
}

pub(crate) fn release_path(version: &Version) -> String {
    format!("/github/uv/releases/download/{version}")
}

/// The standard archive for one lock platform, not a claim that it exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvArtifact {
    target: &'static str,
    version: Version,
}

impl UvArtifact {
    pub fn for_platform(version: &Version, platform: Platform) -> Option<Self> {
        let target = match platform {
            Platform::MacosX64 => "x86_64-apple-darwin",
            Platform::MacosArm64 => "aarch64-apple-darwin",
            Platform::LinuxX64Gnu => "x86_64-unknown-linux-gnu",
            Platform::LinuxArm64Gnu => "aarch64-unknown-linux-gnu",
            Platform::LinuxX64Musl => "x86_64-unknown-linux-musl",
            Platform::LinuxArm64Musl => "aarch64-unknown-linux-musl",
            Platform::WindowsX64 => "x86_64-pc-windows-msvc",
            Platform::WindowsArm64 => "aarch64-pc-windows-msvc",
            Platform::Any => return None,
        };
        Some(Self {
            target,
            version: version.clone(),
        })
    }

    pub fn target(&self) -> &'static str {
        self.target
    }

    fn windows(&self) -> bool {
        self.target.ends_with("-windows-msvc")
    }

    pub fn filename(&self) -> String {
        let extension = if self.windows() { "zip" } else { "tar.gz" };
        format!("uv-{}.{extension}", self.target)
    }

    pub fn url(&self) -> String {
        format!(
            "{ORIGIN}{}/{}",
            release_path(&self.version),
            self.filename()
        )
    }

    /// Unix archives nest binaries under `uv-<target>/`; Windows zips are flat.
    pub fn lock_artifact(&self, sha256: String) -> Artifact {
        let (format, root_prefix, suffix) = if self.windows() {
            (Format::Zip, None, ".exe")
        } else {
            (Format::TarGz, Some(format!("uv-{}", self.target)), "")
        };
        Artifact {
            url: self.url(),
            sha256,
            format,
            root_prefix,
            destination: None,
            executables: BTreeMap::from(
                ["uv", "uvx"].map(|name| (name.to_owned(), format!("{name}{suffix}"))),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_names_layouts_and_identities() {
        let version = Version::new(0, 13, 0);
        let linux = UvArtifact::for_platform(&version, Platform::LinuxX64Gnu).unwrap();
        let artifact = linux.lock_artifact("a".repeat(64));
        assert_eq!(
            artifact.url,
            "https://releases.astral.sh/github/uv/releases/download/0.13.0/uv-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            artifact.root_prefix.as_deref(),
            Some("uv-x86_64-unknown-linux-gnu")
        );
        assert_eq!(artifact.executables["uvx"], "uvx");
        let windows = UvArtifact::for_platform(&version, Platform::WindowsArm64).unwrap();
        let artifact = windows.lock_artifact("a".repeat(64));
        assert_eq!(windows.filename(), "uv-aarch64-pc-windows-msvc.zip");
        assert_eq!((artifact.format, artifact.root_prefix), (Format::Zip, None));
        assert_eq!(artifact.executables["uv"], "uv.exe");
        assert!(UvArtifact::for_platform(&version, Platform::Any).is_none());
        for value in ["0.13.0", "1.0.0"] {
            assert!(exact_release(value).is_some());
        }
        for value in ["0.13", "0.13.0-rc.1", "0.13.0+1", "v0.13.0", "00.13.0"] {
            assert!(exact_release(value).is_none(), "{value}");
        }
    }
}
