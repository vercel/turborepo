//! Runtime classification of the Linux host's C library.
//!
//! Released Linux `turbo` binaries are statically linked against musl, so the
//! compiled target says nothing about the host. Detection reads the ELF
//! interpreter of `/bin/sh`, then for glibc the release banner in the
//! `libc.so.6` beside that loader, and for musl the release string the loader
//! (musl's whole `libc.so`) embeds. It never executes anything, and every read
//! is bounded.

use std::{
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{Architecture, Platform};

/// A Linux host's C library, independent of how this binary was linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Libc {
    /// glibc, with the release version it reports (for example `2.39`).
    Glibc { version: String },
    /// musl, with the release version its loader embeds (for example `1.2.5`).
    Musl { version: String },
}

/// The host's C library could not be classified. Callers must fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibcError {
    path: PathBuf,
    reason: &'static str,
}

impl LibcError {
    fn new(path: &Path, reason: &'static str) -> Self {
        Self {
            path: path.to_owned(),
            reason,
        }
    }

    /// The system file that could not be classified.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for LibcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for LibcError {}

const SHELL: &str = "bin/sh";
const MAX_SHELL_PREFIX: u64 = 64 * 1024;
const MAX_LIBC_BYTES: u64 = 32 * 1024 * 1024;
const PT_INTERP: u32 = 3;

/// The running host's C library, classified once per process. `Ok(None)`
/// outside Linux.
///
/// A glibc-linked `turbo` reports the glibc that loaded it. Any other Linux
/// build reads system files as described in the module documentation.
pub fn host_libc() -> Result<Option<Libc>, LibcError> {
    if !cfg!(target_os = "linux") {
        return Ok(None);
    }
    static HOST: OnceLock<Result<Libc, LibcError>> = OnceLock::new();
    HOST.get_or_init(|| match linked_glibc() {
        Some(libc) => Ok(libc),
        None => detect(Path::new("/"), Platform::current().arch()),
    })
    .clone()
    .map(Some)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn linked_glibc() -> Option<Libc> {
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const std::ffi::c_char;
    }
    // SAFETY: glibc returns a pointer to its static, NUL-terminated version.
    let version = unsafe { std::ffi::CStr::from_ptr(gnu_get_libc_version()) };
    Some(Libc::Glibc {
        version: version.to_str().ok()?.to_owned(),
    })
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn linked_glibc() -> Option<Libc> {
    None
}

/// Classify the C library under `root` for binaries of `arch`.
fn detect(root: &Path, arch: Architecture) -> Result<Libc, LibcError> {
    let (machine, glibc_loader, musl_arch) = match arch {
        Architecture::X64 => (62, "ld-linux-x86-64.so.2", "x86_64"),
        Architecture::Arm64 => (183, "ld-linux-aarch64.so.1", "aarch64"),
        Architecture::Unknown => return Err(LibcError::new(root, "unsupported architecture")),
    };
    let shell = root.join(SHELL);
    let bytes = read(&shell, MAX_SHELL_PREFIX, true)?;
    let interpreter = interpreter(&bytes, machine).map_err(|r| LibcError::new(&shell, r))?;
    let relative = Path::new(&interpreter)
        .strip_prefix("/")
        .map_err(|_| LibcError::new(&shell, "relative ELF interpreter"))?;
    let name = relative.file_name().and_then(|name| name.to_str());
    let musl = name == Some(format!("ld-musl-{musl_arch}.so.1").as_str());
    if !musl && name != Some(glibc_loader) {
        return Err(LibcError::new(&shell, "unrecognized ELF interpreter"));
    }
    let loader = root.join(relative);
    let loader = fs::canonicalize(&loader)
        .map_err(|_| LibcError::new(&loader, "cannot resolve the C library loader"))?;
    if musl {
        let version = musl_version(&read(&loader, MAX_LIBC_BYTES, false)?, musl_arch)
            .ok_or_else(|| LibcError::new(&loader, "no musl release version"))?;
        return Ok(Libc::Musl { version });
    }
    let libc = loader.with_file_name("libc.so.6");
    let version = glibc_version(&read(&libc, MAX_LIBC_BYTES, false)?)
        .ok_or_else(|| LibcError::new(&libc, "no glibc release version"))?;
    Ok(Libc::Glibc { version })
}

/// Read a regular file. `prefix` reads at most `limit` bytes; otherwise a
/// larger file is rejected.
fn read(path: &Path, limit: u64, prefix: bool) -> Result<Vec<u8>, LibcError> {
    let failed = |_: io::Error| LibcError::new(path, "cannot read");
    if !fs::metadata(path).map_err(failed)?.is_file() {
        return Err(LibcError::new(path, "not a regular file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(failed)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if bytes.len() as u64 > limit {
        if !prefix {
            return Err(LibcError::new(path, "file is too large"));
        }
        bytes.truncate(limit as usize);
    }
    Ok(bytes)
}

/// The `PT_INTERP` path of a 64-bit little-endian ELF executable for `machine`.
fn interpreter(bytes: &[u8], machine: u16) -> Result<String, &'static str> {
    const MALFORMED: &str = "truncated or malformed ELF header";
    let field = |offset: u64, len: usize| {
        let start = usize::try_from(offset).ok()?;
        bytes.get(start..start.checked_add(len)?)
    };
    let u16_at = |offset| field(offset, 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |offset| field(offset, 4).and_then(|b| b.try_into().ok().map(u32::from_le_bytes));
    let u64_at = |offset| field(offset, 8).and_then(|b| b.try_into().ok().map(u64::from_le_bytes));
    if bytes.get(..6) != Some(b"\x7fELF\x02\x01") {
        return Err("not a 64-bit little-endian ELF executable");
    }
    if u16_at(18) != Some(machine) {
        return Err("ELF machine does not match this turbo binary");
    }
    let (Some(table), Some(size), Some(count)) = (u64_at(32), u16_at(54), u16_at(56)) else {
        return Err(MALFORMED);
    };
    if size < 56 {
        return Err(MALFORMED);
    }
    for index in 0..u64::from(count) {
        let header = index
            .checked_mul(size.into())
            .and_then(|offset| offset.checked_add(table))
            .ok_or(MALFORMED)?;
        if u32_at(header).ok_or(MALFORMED)? != PT_INTERP {
            continue;
        }
        let (Some(offset), Some(len)) = (u64_at(header + 8), u64_at(header + 32)) else {
            return Err(MALFORMED);
        };
        let raw = usize::try_from(len)
            .ok()
            .and_then(|len| field(offset, len))
            .ok_or(MALFORMED)?;
        let raw = raw.strip_suffix(b"\0").unwrap_or(raw);
        return match std::str::from_utf8(raw) {
            Ok(path) if !path.is_empty() && !path.contains('\0') => Ok(path.to_owned()),
            _ => Err("invalid ELF interpreter"),
        };
    }
    Err("no ELF interpreter")
}

/// The version from glibc's `GNU C Library ... release version X.Y.` banner,
/// matched within a single C string.
fn glibc_version(bytes: &[u8]) -> Option<String> {
    const BANNER: &[u8] = b"GNU C Library ";
    const RELEASE: &[u8] = b" release version ";
    let mut rest = bytes;
    while let Some(start) = find(rest, BANNER) {
        let string = rest[start..].split(|b| *b == 0).next().unwrap_or_default();
        let string = &string[..string.len().min(512)];
        if let Some(at) = find(string, RELEASE) {
            let version = string[at + RELEASE.len()..]
                .split(|b| !(b.is_ascii_digit() || *b == b'.'))
                .next()
                .unwrap_or_default();
            let version = version.strip_suffix(b".").unwrap_or(version);
            let parts = || version.split(|b| *b == b'.');
            if version.len() <= 32 && parts().count() >= 2 && parts().all(|part| !part.is_empty()) {
                return String::from_utf8(version.to_vec()).ok();
            }
        }
        rest = &rest[start + BANNER.len()..];
    }
    None
}

/// The release musl compiles into its loader as the NUL-terminated
/// `__libc_version`, which the loader's own `musl libc (<arch>)` usage banner
/// prints. Requires that banner and exactly one distinct `X.Y.Z` string, with
/// an optional `-suffix` such as a git build's; anything else fails closed.
fn musl_version(bytes: &[u8], arch: &str) -> Option<String> {
    find(
        bytes,
        format!("musl libc ({arch})\nVersion %s\n").as_bytes(),
    )?;
    let release = |string: &[u8]| {
        let (core, suffix) = string.split_at(
            string
                .iter()
                .position(|b| *b == b'-')
                .unwrap_or(string.len()),
        );
        let parts = || core.split(|b| *b == b'.');
        string.len() <= 32
            && parts().count() == 3
            && parts().all(|part| !part.is_empty() && part.iter().all(u8::is_ascii_digit))
            && suffix.len() != 1
            && suffix
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(b))
    };
    let mut found = None;
    for string in bytes.split(|b| *b == 0).filter(|string| release(string)) {
        if found.is_some_and(|found| found != string) {
            return None;
        }
        found = Some(string);
    }
    String::from_utf8(found?.to_vec()).ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const X64: u16 = 62;
    const ARM64: u16 = 183;
    const UBUNTU: &[u8] =
        b"\0GNU C Library (Ubuntu GLIBC 2.39-0ubuntu8.4) stable release version 2.39.\n\0";

    /// A minimal ELF64 executable with one program header.
    fn elf(machine: u16, interpreter: Option<&str>) -> Vec<u8> {
        let mut bytes = vec![0; 64 + 56];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        // PT_LOAD for a static shell.
        bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
        if let Some(path) = interpreter {
            let offset = bytes.len() as u64;
            bytes[64..68].copy_from_slice(&PT_INTERP.to_le_bytes());
            bytes[72..80].copy_from_slice(&offset.to_le_bytes());
            bytes[96..104].copy_from_slice(&(path.len() as u64 + 1).to_le_bytes());
            bytes.extend_from_slice(path.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    fn host(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        root
    }

    fn rejected(root: &tempfile::TempDir, arch: Architecture) -> &'static str {
        detect(root.path(), arch).unwrap_err().reason
    }

    #[test]
    fn glibc_hosts_report_the_loader_sibling_version() {
        let shell = elf(X64, Some("/lib64/ld-linux-x86-64.so.2"));
        let root = host(&[
            ("bin/sh", &shell),
            ("lib64/ld-linux-x86-64.so.2", b"loader"),
            ("lib64/libc.so.6", &[b"padding".as_slice(), UBUNTU].concat()),
        ]);
        let glibc = |version: &str| Libc::Glibc {
            version: version.into(),
        };
        assert_eq!(detect(root.path(), Architecture::X64), Ok(glibc("2.39")));

        // Older banners continue after the version; aarch64 uses its own loader.
        let shell = elf(ARM64, Some("/lib/ld-linux-aarch64.so.1"));
        let banner = b"GNU C Library (GNU libc) stable release version 2.31, by Roland McGrath";
        let root = host(&[
            ("bin/sh", &shell),
            ("lib/ld-linux-aarch64.so.1", b"loader"),
            ("lib/libc.so.6", banner),
        ]);
        assert_eq!(detect(root.path(), Architecture::Arm64), Ok(glibc("2.31")));
    }

    /// A musl loader's strings: the usage banner, unrelated text, a GCC note
    /// that is not a standalone string, and the embedded release.
    fn musl_loader(arch: &str, releases: &[&str]) -> Vec<u8> {
        let mut bytes = format!(
            "\x7fELF\0musl libc ({arch})\nVersion %s\nDynamic Program \
             Loader\n\0/proc/self/fd/\0GCC: (Alpine 14.2.0) 14.2.0\0"
        )
        .into_bytes();
        for release in releases {
            bytes.extend_from_slice(release.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn musl_hosts_report_the_release_their_loader_embeds() {
        let musl = |version: &str| {
            Ok::<_, LibcError>(Libc::Musl {
                version: version.into(),
            })
        };
        for (arch, machine, name) in [
            (Architecture::X64, X64, "x86_64"),
            (Architecture::Arm64, ARM64, "aarch64"),
        ] {
            let loader = format!("lib/ld-musl-{name}.so.1");
            let shell = elf(machine, Some(&format!("/{loader}")));
            for (releases, expected) in [
                (&["1.2.5"][..], musl("1.2.5")),
                // Repeats of one release and git builds are unambiguous.
                (&["1.1.24", "1.1.24"][..], musl("1.1.24")),
                (
                    &["1.2.5-git-12-gabcdef0"][..],
                    musl("1.2.5-git-12-gabcdef0"),
                ),
            ] {
                let contents = musl_loader(name, releases);
                let root = host(&[("bin/sh", &shell), (loader.as_str(), &contents)]);
                assert_eq!(detect(root.path(), arch), expected);
            }
        }
    }

    #[test]
    fn unrecognized_musl_loaders_fail_closed() {
        let loader = "lib/ld-musl-x86_64.so.1";
        let shell = elf(X64, Some("/lib/ld-musl-x86_64.so.1"));
        let root = host(&[("bin/sh", &shell)]);
        assert_eq!(
            rejected(&root, Architecture::X64),
            "cannot resolve the C library loader"
        );
        for contents in [
            // Another architecture's loader, or no banner at all.
            musl_loader("aarch64", &["1.2.5"]),
            b"1.2.5\0".to_vec(),
            // No release, a malformed one, or two different ones.
            musl_loader("x86_64", &[]),
            musl_loader("x86_64", &["1.2", "1.2.5-", "v1.2.5", "1.2.x"]),
            musl_loader("x86_64", &["1.2.4", "1.2.5"]),
        ] {
            let root = host(&[("bin/sh", &shell), (loader, &contents)]);
            assert_eq!(
                rejected(&root, Architecture::X64),
                "no musl release version"
            );
        }
    }

    #[test]
    fn unclassifiable_hosts_fail_closed() {
        let glibc = Some("/lib64/ld-linux-x86-64.so.2");
        assert_eq!(rejected(&host(&[]), Architecture::X64), "cannot read");
        for (shell, reason) in [
            (elf(X64, None), "no ELF interpreter"),
            (
                elf(ARM64, glibc),
                "ELF machine does not match this turbo binary",
            ),
            (
                elf(X64, Some("/lib/ld-other.so.1")),
                "unrecognized ELF interpreter",
            ),
            (
                elf(X64, Some("lib64/ld-linux-x86-64.so.2")),
                "relative ELF interpreter",
            ),
            (
                elf(X64, glibc)[..80].to_vec(),
                "truncated or malformed ELF header",
            ),
            (
                b"#!/bin/busybox sh\n".to_vec(),
                "not a 64-bit little-endian ELF executable",
            ),
        ] {
            assert_eq!(
                rejected(&host(&[("bin/sh", &shell)]), Architecture::X64),
                reason
            );
        }
        let shell = elf(X64, glibc);
        let root = host(&[("bin/sh", &shell)]);
        assert_eq!(
            rejected(&root, Architecture::X64),
            "cannot resolve the C library loader"
        );
        assert_eq!(
            rejected(&root, Architecture::Unknown),
            "unsupported architecture"
        );
        for libc in [
            b"no banner".as_slice(),
            b"GNU C Library (GNU libc)\0 stable release version 2.39.",
            b"GNU C Library (GNU libc) stable release version 2.",
        ] {
            let root = host(&[
                ("bin/sh", &shell),
                ("lib64/ld-linux-x86-64.so.2", b"loader"),
                ("lib64/libc.so.6", libc),
            ]);
            assert_eq!(
                rejected(&root, Architecture::X64),
                "no glibc release version"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_files_agree_with_the_running_host() {
        // glibc-linked test binaries compare the file-based result with the
        // version reported by the glibc that loaded this process.
        let files = detect(Path::new("/"), Platform::current().arch());
        assert_eq!(host_libc(), files.map(Some));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn other_operating_systems_have_no_host_libc() {
        assert_eq!(host_libc(), Ok(None));
    }
}
