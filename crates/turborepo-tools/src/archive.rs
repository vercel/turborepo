//! Extraction of the archive formats upstream distributions ship:
//! gzip-compressed tarballs and zip files.

use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

impl ArchiveKind {
    /// Guesses the archive kind from a URL or file name.
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.split(['?', '#']).next().unwrap_or(name);
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".zip") {
            Some(Self::Zip)
        } else {
            None
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Zip => "zip",
        }
    }
}

/// Extracts `archive` into `dest`, dropping the first `strip_components`
/// path components of every entry (upstream archives usually wrap their
/// contents in a single top-level directory).
///
/// This is blocking; callers on an async runtime should use
/// `tokio::task::spawn_blocking`.
pub fn extract(
    kind: ArchiveKind,
    archive: &Path,
    dest: &Path,
    strip_components: usize,
) -> Result<(), Error> {
    fs::create_dir_all(dest).map_err(|source| Error::io(dest.display().to_string(), source))?;
    let result = match kind {
        ArchiveKind::TarGz => extract_tar_gz(archive, dest, strip_components),
        ArchiveKind::Zip => extract_zip(archive, dest, strip_components),
    };
    result.map_err(|reason| Error::Archive {
        archive: archive.display().to_string(),
        reason,
    })
}

/// Sanitizes an entry path: rejects absolute paths and `..`, strips the
/// requested number of leading components. Returns `None` for entries that
/// vanish entirely after stripping (e.g. the wrapper directory itself).
fn sanitized_relative_path(raw: &Path, strip_components: usize) -> Result<Option<PathBuf>, String> {
    let mut components = Vec::new();
    for component in raw.components() {
        match component {
            Component::Normal(part) => components.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "archive entry {} escapes the destination",
                    raw.display()
                ));
            }
        }
    }
    if components.len() <= strip_components {
        return Ok(None);
    }
    Ok(Some(components[strip_components..].iter().collect()))
}

fn extract_tar_gz(archive: &Path, dest: &Path, strip_components: usize) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|err| err.to_string())?;
    let decoder = flate2::read::GzDecoder::new(io::BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    tar.set_preserve_permissions(true);
    tar.set_unpack_xattrs(false);
    // Symlinks inside toolchain archives point at siblings (e.g. node's
    // `bin/npm -> ../lib/node_modules/npm/bin/npm-cli.js`); the tar crate
    // validates that they stay inside `dest` when we unpack entry by entry.
    for entry in tar.entries().map_err(|err| err.to_string())? {
        let mut entry = entry.map_err(|err| err.to_string())?;
        let raw = entry.path().map_err(|err| err.to_string())?.into_owned();
        let Some(relative) = sanitized_relative_path(&raw, strip_components)? else {
            continue;
        };
        let target = dest.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        // `unpack_in` refuses paths that leave `dest`, including via symlink
        // targets; it also handles hard links, directories, and modes.
        entry.set_preserve_permissions(true);
        if entry.header().entry_type().is_symlink() {
            // The tar crate's `unpack_in` rejects symlinks whose targets are
            // not yet present in strictly ordered archives; unpack directly
            // after validating the link target stays relative.
            let link = entry
                .link_name()
                .map_err(|err| err.to_string())?
                .ok_or_else(|| format!("symlink {} has no target", raw.display()))?;
            if link.is_absolute() {
                return Err(format!(
                    "symlink {} points at absolute path {}",
                    raw.display(),
                    link.display()
                ));
            }
            let _ = fs::remove_file(&target);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target).map_err(|err| err.to_string())?;
            #[cfg(windows)]
            {
                // Windows archives never carry symlinks; fall back to a copy of
                // the target when they do.
                let resolved = target
                    .parent()
                    .map(|parent| parent.join(&link))
                    .unwrap_or_else(|| link.to_path_buf());
                if resolved.is_dir() {
                    std::os::windows::fs::symlink_dir(&link, &target)
                        .map_err(|err| err.to_string())?;
                } else {
                    fs::copy(&resolved, &target).map_err(|err| err.to_string())?;
                }
            }
            continue;
        }
        entry.unpack(&target).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, strip_components: usize) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|err| err.to_string())?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|err| err.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|err| err.to_string())?;
        let Some(raw) = entry.enclosed_name() else {
            return Err(format!(
                "archive entry {} escapes the destination",
                entry.name()
            ));
        };
        let Some(relative) = sanitized_relative_path(&raw, strip_components)? else {
            continue;
        };
        let target = dest.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|err| err.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mode = entry.unix_mode();
        #[cfg(unix)]
        if mode.is_some_and(|mode| mode & 0o170000 == 0o120000) {
            let mut link = String::new();
            entry
                .read_to_string(&mut link)
                .map_err(|err| err.to_string())?;
            let _ = fs::remove_file(&target);
            std::os::unix::fs::symlink(&link, &target).map_err(|err| err.to_string())?;
            continue;
        }
        let mut out = fs::File::create(&target).map_err(|err| err.to_string())?;
        io::copy(&mut entry, &mut out).map_err(|err| err.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o7777))
                .map_err(|err| err.to_string())?;
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn build_tar_gz(path: &Path) {
        let file = fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        let contents = b"#!/bin/sh\necho hi\n";
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "pkg-1.0/bin/tool", &contents[..])
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_cksum();
        builder
            .append_link(&mut header, "pkg-1.0/bin/alias", "tool")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn build_zip(path: &Path) {
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);
        writer.start_file("wrapper/tool.exe", options).unwrap();
        writer.write_all(b"binary").unwrap();
        writer.add_directory("wrapper/empty", options).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_tar_gz_with_stripped_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("pkg.tar.gz");
        build_tar_gz(&archive);
        let dest = tmp.path().join("out");
        extract(ArchiveKind::TarGz, &archive, &dest, 1).unwrap();
        assert!(dest.join("bin/tool").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dest.join("bin/tool"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit preserved");
            assert_eq!(
                fs::read_link(dest.join("bin/alias")).unwrap(),
                PathBuf::from("tool")
            );
        }
    }

    #[test]
    fn extracts_zip_with_stripped_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("pkg.zip");
        build_zip(&archive);
        let dest = tmp.path().join("out");
        extract(ArchiveKind::Zip, &archive, &dest, 1).unwrap();
        assert_eq!(fs::read(dest.join("tool.exe")).unwrap(), b"binary");
        assert!(dest.join("empty").is_dir());
    }

    #[test]
    fn rejects_escaping_entries() {
        assert!(sanitized_relative_path(Path::new("../etc/passwd"), 0).is_err());
        assert!(sanitized_relative_path(Path::new("/etc/passwd"), 0).is_err());
        assert_eq!(
            sanitized_relative_path(Path::new("wrapper"), 1).unwrap(),
            None
        );
        assert_eq!(
            sanitized_relative_path(Path::new("./wrapper/bin/x"), 1).unwrap(),
            Some(PathBuf::from("bin/x"))
        );
    }

    #[test]
    fn detects_kind_from_name() {
        assert_eq!(
            ArchiveKind::from_name("https://x/node-v1.tar.gz?x=1"),
            Some(ArchiveKind::TarGz)
        );
        assert_eq!(
            ArchiveKind::from_name("pnpm-9.0.0.tgz"),
            Some(ArchiveKind::TarGz)
        );
        assert_eq!(ArchiveKind::from_name("bun.zip"), Some(ArchiveKind::Zip));
        assert_eq!(ArchiveKind::from_name("rustup-init"), None);
    }
}
