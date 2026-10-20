//! rustup's vendor spellings, URLs and published release metadata. Platform
//! identity is shared; these host triples and checksum formats are not.

use semver::Version;
use serde::Deserialize;

use crate::lock::Platform;

pub(crate) const ORIGIN: &str = "https://static.rust-lang.org";
pub const MAX_RELEASE_BYTES: usize = 4 * 1024;
pub const MAX_CHECKSUM_BYTES: usize = 1024;
/// rustup publishes an installer for every native lock platform, including
/// musl. Recording them all keeps a new host from mutating a frozen lock.
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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid rustup release metadata")]
    Release,
    #[error("invalid rustup-init checksum file")]
    Checksum,
}

/// rustup host triple and lock spelling. Vendor naming only, not runtime libc
/// detection.
pub(crate) fn target(platform: Platform) -> Option<(&'static str, &'static str)> {
    Some(match platform {
        Platform::MacosX64 => ("x86_64-apple-darwin", "macos-x64"),
        Platform::MacosArm64 => ("aarch64-apple-darwin", "macos-arm64"),
        Platform::LinuxX64Gnu => ("x86_64-unknown-linux-gnu", "linux-x64-gnu"),
        Platform::LinuxArm64Gnu => ("aarch64-unknown-linux-gnu", "linux-arm64-gnu"),
        Platform::LinuxX64Musl => ("x86_64-unknown-linux-musl", "linux-x64-musl"),
        Platform::LinuxArm64Musl => ("aarch64-unknown-linux-musl", "linux-arm64-musl"),
        Platform::WindowsX64 => ("x86_64-pc-windows-msvc", "windows-x64"),
        Platform::WindowsArm64 => ("aarch64-pc-windows-msvc", "windows-arm64"),
        Platform::Any => return None,
    })
}

/// The rustup host triple for a lock platform spelling, the inverse of
/// [`target`].
pub(crate) fn host(spelling: &str) -> Option<&'static str> {
    PLATFORMS
        .into_iter()
        .filter_map(target)
        .find_map(|(triple, lock)| (lock == spelling).then_some(triple))
}

pub(crate) fn windows(platform: Platform) -> bool {
    matches!(platform, Platform::WindowsX64 | Platform::WindowsArm64)
}

/// Hosts promote only their own family, by lock spelling: GNU Linux and macOS
/// on Unix, x64 on Windows. Windows arm64 and musl stay locked but
/// unpromoted; the caller still qualifies the host itself.
pub(crate) fn promotable(spelling: &str) -> bool {
    match spelling.strip_prefix("windows-") {
        Some(arch) => cfg!(windows) && arch == "x64",
        None => cfg!(unix) && !spelling.ends_with("-musl"),
    }
}

pub(crate) fn installer_file(platform: Platform) -> &'static str {
    if windows(platform) {
        "rustup-init.exe"
    } else {
        "rustup-init"
    }
}

/// The installed multicall binary. A `rustup` file stem selects rustup mode,
/// so resolving the shim can never start the interactive `rustup-init`.
pub(crate) fn binary_path(platform: Platform) -> &'static str {
    if windows(platform) {
        "bin/rustup.exe"
    } else {
        "bin/rustup"
    }
}

/// Immutable versioned archive location, never the floating `dist` alias.
pub(crate) fn installer_url(version: &Version, platform: Platform) -> Option<String> {
    let (triple, _) = target(platform)?;
    Some(format!(
        "{ORIGIN}/rustup/archive/{version}/{triple}/{}",
        installer_file(platform)
    ))
}

pub(crate) fn canonical_release(value: &str) -> Option<Version> {
    let version = Version::parse(value).ok()?;
    (version.pre.is_empty() && version.build.is_empty() && version.to_string() == value)
        .then_some(version)
}

/// `release-stable.toml`, rustup's own self-update pointer.
pub fn parse_release(bytes: &[u8]) -> Result<Version, Error> {
    #[derive(Deserialize)]
    struct Release {
        #[serde(rename = "schema-version")]
        schema: String,
        version: String,
    }
    if bytes.len() > MAX_RELEASE_BYTES {
        return Err(Error::Release);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Release)?;
    let release: Release = toml::from_str(text).map_err(|_| Error::Release)?;
    if release.schema != "1" {
        return Err(Error::Release);
    }
    canonical_release(&release.version).ok_or(Error::Release)
}

/// One published `<sha256> [*][./]<file>` line. Windows files omit the name.
pub fn parse_checksum(bytes: &[u8], file: &str) -> Result<String, Error> {
    if bytes.len() > MAX_CHECKSUM_BYTES {
        return Err(Error::Checksum);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Checksum)?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut fields = line.split([' ', '\t']).filter(|field| !field.is_empty());
    let digest = fields.next().ok_or(Error::Checksum)?;
    let name = fields.next();
    if line.contains(['\n', '\r'])
        || fields.next().is_some()
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Checksum);
    }
    if let Some(name) = name {
        let name = name.strip_prefix('*').unwrap_or(name);
        if name.strip_prefix("./").unwrap_or(name) != file {
            return Err(Error::Checksum);
        }
    }
    Ok(digest.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_platform_has_an_official_versioned_installer() {
        let version = Version::new(1, 29, 1);
        let expected = [
            ("x86_64-apple-darwin", ""),
            ("aarch64-apple-darwin", ""),
            ("x86_64-unknown-linux-gnu", ""),
            ("aarch64-unknown-linux-gnu", ""),
            ("x86_64-unknown-linux-musl", ""),
            ("aarch64-unknown-linux-musl", ""),
            ("x86_64-pc-windows-msvc", ".exe"),
            ("aarch64-pc-windows-msvc", ".exe"),
        ];
        for (platform, (triple, exe)) in PLATFORMS.into_iter().zip(expected) {
            let file = format!("rustup-init{exe}");
            assert_eq!(target(platform).unwrap().0, triple);
            assert_eq!(installer_file(platform), file);
            assert_eq!(binary_path(platform), format!("bin/rustup{exe}"));
            assert_eq!(
                installer_url(&version, platform).unwrap(),
                format!("https://static.rust-lang.org/rustup/archive/1.29.1/{triple}/{file}")
            );
        }
        assert_eq!(target(Platform::Any), None);
        assert_eq!(installer_url(&version, Platform::Any), None);
    }

    #[test]
    fn only_the_hosts_own_family_is_promotable() {
        for (spelling, promotable) in [
            ("linux-x64-gnu", cfg!(unix)),
            ("macos-arm64", cfg!(unix)),
            ("linux-arm64-musl", false),
            ("windows-x64", cfg!(windows)),
            ("windows-arm64", false),
        ] {
            assert_eq!(super::promotable(spelling), promotable, "{spelling}");
        }
    }

    #[test]
    fn release_metadata_requires_schema_one_and_a_canonical_stable_release() {
        assert_eq!(
            parse_release(b"schema-version = '1'\nversion = '1.29.1'\n"),
            Ok(Version::new(1, 29, 1))
        );
        for invalid in [
            &b"schema-version = '2'\nversion = '1.29.1'\n"[..],
            b"version = '1.29.1'\n",
            b"schema-version = '1'\nversion = '1.29'\n",
            b"schema-version = '1'\nversion = '01.29.1'\n",
            b"schema-version = '1'\nversion = '1.29.1-beta.1'\n",
            b"schema-version = '1'\nversion = '1.29.1+local'\n",
            b"schema-version = '1'\nschema-version = '1'\nversion = '1.29.1'\n",
            b"\xff",
        ] {
            assert_eq!(parse_release(invalid), Err(Error::Release));
        }
        assert_eq!(
            parse_release(&vec![b' '; MAX_RELEASE_BYTES + 1]),
            Err(Error::Release)
        );
    }

    #[test]
    fn published_unix_and_windows_checksum_spellings() {
        let digest = "dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71";
        for (body, file) in [
            (format!("{digest} *./rustup-init\n"), "rustup-init"),
            (format!("{digest}  rustup-init\n"), "rustup-init"),
            (format!("{digest}\n"), "rustup-init.exe"),
            (format!("{digest}\r\n"), "rustup-init.exe"),
        ] {
            assert_eq!(parse_checksum(body.as_bytes(), file).unwrap(), digest);
        }
        for body in [
            format!("{digest} *./rustup-init.exe\n"),
            format!("{digest} *./rustup-init extra\n"),
            format!("{digest}\n{digest}\n"),
            digest.to_ascii_uppercase(),
            digest[..63].to_owned(),
            String::new(),
        ] {
            assert_eq!(
                parse_checksum(body.as_bytes(), "rustup-init"),
                Err(Error::Checksum)
            );
        }
    }
}
