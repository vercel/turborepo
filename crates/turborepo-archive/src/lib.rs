//! TURBO-6300: verified TAR/TAR.GZ extraction; no transport, installation or
//! CLI.
//!
//! The extractor creates and owns a fresh staging directory, never accepts an
//! existing extraction tree, and deletes staging on failure/drop. The OS temp
//! parent and same-user processes must be trusted (no hostile concurrent tree
//! mutation). Unix staging is created with mode 0700; Windows relies on the
//! user's temp-directory ACL. Paths use a conservative ASCII portable subset.
//!
//! Links (including valid Node TAR symlinks), special files, and sparse/PAX TAR
//! are explicitly unsupported. Safe Node link support is separate work; this
//! is not a full Node installer. GNU long names are supported with bounded
//! raw-header processing. Byte limits include compressed input and the whole
//! decoded TAR (headers/padding/trailers), not just advertised entry sizes.
//! Unix file rwx bits are preserved with owner read/write enabled and special
//! bits stripped; directories stay private/writable for cleanup. Windows has
//! no Unix executable-mode equivalent. ZIP belongs to parent TURBO-6204.
//! Native platform qualification is required before claiming support.

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use turborepo_download::VerifiedArtifact;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nonzero, representable extraction limits are required")]
    InvalidLimits,
    #[error("archive byte, entry, path-length or path-depth limit exceeded")]
    LimitExceeded,
    #[error("archive contains an unsafe or nonportable path")]
    UnsafePath,
    #[error("archive paths collide, repeat, or conflict with a parent file")]
    PathConflict,
    #[error("archive links are unsupported; safe Node symlink support is separate work")]
    UnsupportedLink,
    #[error("unsupported archive entry or metadata (special files, sparse or PAX TAR)")]
    UnsupportedEntry,
    #[error("archive does not match the required root and regular-file layout")]
    LayoutMismatch,
    #[error("invalid or corrupt archive")]
    InvalidArchive,
    #[error("archive or staging I/O operation failed")]
    Io(#[from] io::Error),
}

#[derive(Clone, Copy)]
pub enum Format {
    Tar,
    TarGz,
}

/// Bounds input bytes, decoded TAR bytes, raw entries and
/// created nodes (including implicit directories), and each path's bytes/depth.
/// Allocator/parser overhead is not part of the byte limit. No CPU time budget.
#[derive(Clone, Copy)]
pub struct Limits {
    bytes: usize,
    entries: usize,
    path_bytes: usize,
    depth: usize,
}
impl Limits {
    /// All limits must be nonzero; path bytes cannot exceed the byte budget.
    /// The byte budget must leave room for a one-byte overflow probe.
    pub fn new(
        bytes: usize,
        entries: usize,
        path_bytes: usize,
        depth: usize,
    ) -> Result<Self, Error> {
        if [bytes, entries, path_bytes, depth].contains(&0)
            || bytes == usize::MAX
            || path_bytes > bytes
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            bytes,
            entries,
            path_bytes,
            depth,
        })
    }
}

/// All entries must belong to this single root directory. Required file paths
/// are relative to the root, exact/case-sensitive, and must be regular files.
/// A nonempty required-files list is mandatory; other files within root are OK.
pub struct Layout<'a> {
    pub root: &'a str,
    pub required_files: &'a [&'a str],
}

/// Owns cleanup; callers may inspect staging but promotion/installation is not
/// provided. Keep this object alive while using the returned paths.
pub struct ExtractedArtifact {
    staging: TempDir,
    root: String,
}
impl ExtractedArtifact {
    pub fn staging_path(&self) -> &Path {
        self.staging.path()
    }
    pub fn root_path(&self) -> PathBuf {
        self.staging.path().join(&self.root)
    }
}

pub fn extract(
    artifact: &VerifiedArtifact,
    format: Format,
    limits: Limits,
    layout: Layout<'_>,
) -> Result<ExtractedArtifact, Error> {
    if artifact.as_bytes().len() > limits.bytes {
        return Err(Error::LimitExceeded);
    }
    let root = portable_path(layout.root.as_bytes(), false, limits)?;
    if root.contains('/') || layout.required_files.is_empty() {
        return Err(Error::LayoutMismatch);
    }
    if layout.required_files.len() > limits.entries {
        return Err(Error::LimitExceeded);
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix("turborepo-archive-");
    #[cfg(unix)]
    builder.permissions(fs::Permissions::from_mode(0o700));
    let staging = builder.tempdir()?;
    #[cfg(unix)]
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700))?;
    let artifact = artifact.as_bytes();
    let mut tree = Tree {
        staging,
        root,
        nodes: BTreeMap::new(),
        limits,
        bytes: 0,
    };
    match format {
        Format::Tar => extract_tar(artifact, &mut tree)?,
        Format::TarGz => {
            let mut decoded = Vec::new();
            flate2::read::MultiGzDecoder::new(artifact)
                .take(limits.bytes as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(|_| Error::InvalidArchive)?;
            if decoded.len() > limits.bytes {
                return Err(Error::LimitExceeded);
            }
            extract_tar(&decoded, &mut tree)?;
        }
    }
    for file in layout.required_files {
        let path = portable_path(format!("{}/{file}", tree.root).as_bytes(), false, limits)?;
        if !tree
            .nodes
            .get(&path.to_ascii_lowercase())
            .is_some_and(|n| n.path == path && !n.directory)
        {
            return Err(Error::LayoutMismatch);
        }
    }
    Ok(ExtractedArtifact {
        staging: tree.staging,
        root: tree.root,
    })
}

fn portable_path(raw: &[u8], directory: bool, limits: Limits) -> Result<String, Error> {
    if raw.len() > limits.path_bytes {
        return Err(Error::LimitExceeded);
    }
    // ASCII avoids Unicode case/normalization aliases; '~' avoids DOS short names.
    if raw
        .iter()
        .any(|b| !(32..127).contains(b) || b"\\:<>\"|?*~".contains(b))
    {
        return Err(Error::UnsafePath);
    }
    let path = std::str::from_utf8(raw).map_err(|_| Error::UnsafePath)?;
    let path = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    if path.split('/').count() > limits.depth {
        return Err(Error::LimitExceeded);
    }
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.len() > 255
        {
            return Err(Error::UnsafePath);
        }
        let upper = component
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
        ) || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(Error::UnsafePath);
        }
    }
    Ok(path.to_owned())
}

struct Node {
    path: String,
    directory: bool,
    explicit: bool,
}
struct Tree {
    staging: TempDir,
    root: String,
    nodes: BTreeMap<String, Node>,
    limits: Limits,
    bytes: usize,
}
impl Tree {
    fn entry(
        &mut self,
        raw: &[u8],
        directory: bool,
        mode: u32,
        size: u64,
        reader: impl Read,
    ) -> Result<(), Error> {
        let path = portable_path(raw, directory, self.limits)?;
        if path != self.root && !path.starts_with(&format!("{}/", self.root)) {
            return Err(Error::LayoutMismatch);
        }
        if (path == self.root && !directory) || (directory && size != 0) {
            return Err(Error::LayoutMismatch);
        }
        if size > (self.limits.bytes - self.bytes) as u64 {
            return Err(Error::LimitExceeded);
        }
        let mut prefix = String::new();
        let parts: Vec<_> = path.split('/').collect();
        for (i, component) in parts.iter().enumerate() {
            if i != 0 {
                prefix.push('/');
            }
            prefix.push_str(component);
            let explicit = i + 1 == parts.len();
            let is_dir = !explicit || directory;
            let key = prefix.to_ascii_lowercase();
            if let Some(node) = self.nodes.get_mut(&key) {
                if node.path != prefix || !node.directory || !is_dir || (explicit && node.explicit)
                {
                    return Err(Error::PathConflict);
                }
                node.explicit |= explicit;
            } else {
                if self.nodes.len() == self.limits.entries {
                    return Err(Error::LimitExceeded);
                }
                if is_dir {
                    let mut dir = fs::DirBuilder::new();
                    #[cfg(unix)]
                    dir.mode(0o700);
                    let destination = self.staging.path().join(&prefix);
                    dir.create(&destination)?;
                    #[cfg(unix)]
                    fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
                }
                let node = Node {
                    path: prefix.clone(),
                    directory: is_dir,
                    explicit,
                };
                self.nodes.insert(key, node);
            }
        }
        if !directory {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut output = options.open(self.staging.path().join(&path))?;
            let remaining = self.limits.bytes - self.bytes;
            let actual = copy_payload(reader.take(remaining as u64 + 1), &mut output)?;
            if actual > remaining as u64 {
                return Err(Error::LimitExceeded);
            }
            if actual != size {
                return Err(Error::InvalidArchive);
            }
            self.bytes += actual as usize;
            #[cfg(unix)]
            output.set_permissions(fs::Permissions::from_mode(0o600 | (mode & 0o777)))?;
            #[cfg(not(unix))]
            let _ = mode;
        }
        Ok(())
    }
}

fn extract_tar(bytes: &[u8], tree: &mut Tree) -> Result<(), Error> {
    if !bytes.len().is_multiple_of(512) {
        return Err(Error::InvalidArchive);
    }
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut pending = None;
    for (i, entry) in archive
        .entries()
        .map_err(|_| Error::InvalidArchive)?
        .raw(true)
        .enumerate()
    {
        if i >= tree.limits.entries {
            return Err(Error::LimitExceeded);
        }
        let mut entry = entry.map_err(|_| Error::InvalidArchive)?;
        let size = checked_tar_size(&entry.header().as_old().size)?;
        if size != entry.size() {
            return Err(Error::InvalidArchive);
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() || kind.is_gnu_longlink() {
            return Err(Error::UnsupportedLink);
        }
        if kind.is_gnu_longname() {
            if pending.is_some() {
                return Err(Error::InvalidArchive);
            }
            if size > tree.limits.path_bytes as u64 + 1 {
                return Err(Error::LimitExceeded);
            }
            let mut name = Vec::new();
            entry
                .read_to_end(&mut name)
                .map_err(|_| Error::InvalidArchive)?;
            if name.pop() != Some(0) {
                return Err(Error::InvalidArchive);
            }
            pending = Some(name);
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(Error::UnsupportedEntry);
        }
        // Check raw fields before tar erases NUL suffixes or USTAR backslashes.
        check_tar_field(&entry.header().as_old().name)?;
        if let Some(ustar) = entry.header().as_ustar() {
            check_tar_field(&ustar.prefix)?;
        }
        let name = pending
            .take()
            .unwrap_or_else(|| entry.path_bytes().into_owned());
        let mode = entry.header().mode().map_err(|_| Error::InvalidArchive)?;
        tree.entry(&name, kind.is_dir(), mode, size, &mut entry)?;
    }
    let position = archive.into_inner().position() as usize;
    if pending.is_some()
        || bytes.len().saturating_sub(position) < 512
        || bytes[position..].iter().any(|b| *b != 0)
    {
        return Err(Error::InvalidArchive);
    }
    Ok(())
}

fn copy_payload(mut reader: impl Read, mut writer: impl io::Write) -> Result<u64, Error> {
    Ok(io::copy(&mut reader, &mut writer)?)
}

fn checked_tar_size(raw: &[u8; 12]) -> Result<u64, Error> {
    if raw[0] & 0x80 != 0 {
        // tar 0.4.45 ignores the high bytes; check all 95 payload bits instead.
        let mut bytes = *raw;
        bytes[0] &= 0x7f;
        bytes
            .iter()
            .try_fold(0u64, |n, b| n.checked_mul(256)?.checked_add(u64::from(*b)))
            .ok_or(Error::InvalidArchive)
    } else {
        let text = std::str::from_utf8(raw).map_err(|_| Error::InvalidArchive)?;
        let text = text.trim_start_matches(' ').trim_end_matches([' ', '\0']);
        if !text.bytes().all(|b| matches!(b, b'0'..=b'7')) {
            return Err(Error::InvalidArchive);
        }
        u64::from_str_radix(text, 8).map_err(|_| Error::InvalidArchive)
    }
}

fn check_tar_field(field: &[u8]) -> Result<(), Error> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    if field.contains(&b'\\') || field[end..].iter().any(|b| *b != 0) {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
