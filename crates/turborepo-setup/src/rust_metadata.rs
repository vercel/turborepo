//! Bounded rustup channel manifests (`channel-rust-*.toml`, v2). Only the
//! release identity, profiles, renames and per-target component tarballs are
//! read; everything else is ignored, never interpreted.

use std::collections::BTreeMap;

use serde::Deserialize;

pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const DIST: &str = "https://static.rust-lang.org/dist/";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid Rust channel manifest")]
    Invalid,
}

#[derive(Deserialize)]
pub struct Manifest {
    #[serde(rename = "manifest-version")]
    manifest_version: String,
    date: String,
    pkg: BTreeMap<String, Package>,
    #[serde(default)]
    renames: BTreeMap<String, Rename>,
    #[serde(default)]
    profiles: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct Package {
    version: String,
    #[serde(default)]
    target: BTreeMap<String, Target>,
}

#[derive(Deserialize)]
struct Rename {
    to: String,
}

#[derive(Deserialize)]
struct Target {
    available: bool,
    url: Option<String>,
    hash: Option<String>,
}

/// One official gzip component tarball, as rustup's manifest pins it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tarball<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
    /// The archive's single top-level directory.
    pub stem: &'a str,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Invalid);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        let manifest: Self = toml::from_str(text).map_err(|_| Error::Invalid)?;
        let date = manifest.date.as_bytes();
        if manifest.manifest_version != "2"
            || date.len() != 10
            || !date.iter().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    *b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
        {
            return Err(Error::Invalid);
        }
        Ok(manifest)
    }

    pub fn date(&self) -> &str {
        &self.date
    }

    /// `1.99.0` or `1.101.0-nightly` from `pkg.rust.version`.
    pub fn rust_version(&self) -> Option<&str> {
        self.pkg.get("rust")?.version.split(' ').next()
    }

    pub fn profile(&self, name: &str) -> Option<&[String]> {
        self.profiles.get(name).map(Vec::as_slice)
    }

    /// Short names (`clippy`) map to their published package
    /// (`clippy-preview`).
    pub fn package<'a>(&'a self, component: &'a str) -> &'a str {
        self.renames
            .get(component)
            .map_or(component, |rename| rename.to.as_str())
    }

    /// The tarball for this host target, or the target-independent `*` one.
    /// Unavailable, non-gzip and off-origin entries are absent, not errors.
    pub fn tarball(&self, package: &str, target: &str) -> Option<Tarball<'_>> {
        let targets = &self.pkg.get(package)?.target;
        let entry = targets.get(target).or_else(|| targets.get("*"))?;
        let url = entry.url.as_deref().filter(|_| entry.available)?;
        let sha256 = entry.hash.as_deref()?;
        let file = url.strip_prefix(DIST)?.rsplit('/').next()?;
        let stem = file.strip_suffix(".tar.gz")?;
        let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        (sha256.len() == 64 && sha256.bytes().all(hex) && !stem.is_empty()).then_some(Tarball {
            url,
            sha256,
            stem,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    pub(crate) const HOSTS: [&str; 2] = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"];

    /// A trimmed official-shaped manifest for two hosts plus wasm and `*`.
    pub(crate) fn fixture(version: &str, date: &str) -> String {
        let name = if version.contains("nightly") {
            "nightly"
        } else {
            version
        };
        let mut text = format!(
            "manifest-version = \"2\"\ndate = \"{date}\"\n[renames.clippy]\nto = \
             \"clippy-preview\"\n[profiles]\nminimal = [\"rustc\", \"cargo\", \"rust-std\", \
             \"rust-mingw\"]\ndefault = [\"rustc\", \"cargo\", \"rust-std\", \
             \"clippy-preview\"]\n[pkg.rust]\nversion = \"{version} (b940084d7 {date})\"\n"
        );
        let std = [HOSTS[0], HOSTS[1], "wasm32-unknown-unknown"];
        for (package, file, targets) in [
            ("rustc", "rustc", &HOSTS[..]),
            ("cargo", "cargo", &HOSTS[..]),
            ("rust-std", "rust-std", &std[..]),
            ("clippy-preview", "clippy", &HOSTS[..]),
            ("rust-src", "rust-src", &["*"][..]),
        ] {
            text.push_str(&format!("[pkg.{package}]\nversion = \"{version}\"\n"));
            for target in targets {
                let suffix = if *target == "*" {
                    String::new()
                } else {
                    format!("-{target}")
                };
                let url = format!("{DIST}{date}/{file}-{name}{suffix}.tar.gz");
                let hash = format!("{:x}", Sha256::digest(&url));
                text.push_str(&format!(
                    "[pkg.{package}.target.\"{target}\"]\navailable = true\nurl = \"{url}\"\nhash \
                     = \"{hash}\"\nxz_url = \"{DIST}{date}/x.tar.xz\"\n"
                ));
            }
        }
        text
    }

    #[test]
    fn identity_profiles_renames_and_tarballs() {
        let manifest = Manifest::parse(fixture("1.99.0", "2026-10-01").as_bytes()).unwrap();
        assert_eq!(manifest.date(), "2026-10-01");
        assert_eq!(manifest.rust_version(), Some("1.99.0"));
        assert_eq!(manifest.profile("minimal").unwrap().len(), 4);
        assert_eq!(manifest.package("clippy"), "clippy-preview");
        assert_eq!(manifest.package("rustc"), "rustc");
        let clippy = manifest
            .tarball("clippy-preview", "aarch64-apple-darwin")
            .unwrap();
        assert_eq!(clippy.stem, "clippy-1.99.0-aarch64-apple-darwin");
        assert!(
            clippy
                .url
                .ends_with("/2026-10-01/clippy-1.99.0-aarch64-apple-darwin.tar.gz")
        );
        assert_eq!(
            manifest
                .tarball("rust-src", "aarch64-apple-darwin")
                .unwrap()
                .stem,
            "rust-src-1.99.0"
        );
        assert_eq!(
            manifest.tarball("rust-mingw", "x86_64-unknown-linux-gnu"),
            None
        );
        assert_eq!(manifest.tarball("rustc", "x86_64-pc-windows-msvc"), None);
    }

    #[test]
    fn malformed_unavailable_and_off_origin_entries_are_rejected() {
        let base = fixture("1.99.0", "2026-10-01");
        for invalid in [
            base.replace("manifest-version = \"2\"", "manifest-version = \"1\""),
            base.replace("date = \"2026-10-01\"", "date = \"yesterday\""),
            base.replace("[pkg.rust]\n", "[pkg.rust]\n[pkg.rust]\n"),
            "\u{0}".into(),
        ] {
            assert_eq!(
                Manifest::parse(invalid.as_bytes()).err(),
                Some(Error::Invalid)
            );
        }
        assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
        let host = "x86_64-unknown-linux-gnu";
        for (from, to) in [
            ("available = true", "available = false"),
            (
                "https://static.rust-lang.org/dist/",
                "https://mirror.example/dist/",
            ),
            (".tar.gz\"", ".tar.zst\""),
        ] {
            let manifest = Manifest::parse(base.replacen(from, to, 1).as_bytes()).unwrap();
            assert_eq!(manifest.tarball("rustc", host), None, "{from}");
        }
    }
}
