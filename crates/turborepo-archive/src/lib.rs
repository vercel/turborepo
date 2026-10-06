//! Verified TAR/TAR.GZ and ZIP extraction; no transport, installation or CLI.
//!
//! The extractor creates and owns a fresh staging directory, never accepts an
//! existing extraction tree, and deletes staging on failure/drop. The OS temp
//! parent and same-user processes must be trusted (no hostile concurrent tree
//! mutation). Unix staging is created with mode 0700; Windows relies on the
//! user's temp-directory ACL. Paths use a conservative ASCII portable subset.
//!
//! Relative Node TAR symlinks are validated against the complete installation
//! tree and created only after all extraction writes. Targets must exist inside
//! the expected root; intermediate target components must be real directories.
//! Cycles (including directory cycles) are rejected. Symlink creation is Unix
//! only; hard links, GNU long links, special files and sparse/PAX TAR remain
//! unsupported. This is not a Node installer. GNU long names use bounded
//! raw-header processing. Byte limits include compressed input and the whole
//! decoded TAR (headers/padding/trailers), not just advertised entry sizes.
//! Unix file rwx bits are preserved with owner read/write enabled and special
//! bits stripped; directories stay private/writable for cleanup. Windows has
//! no Unix executable-mode equivalent. ZIP supports single-disk ZIP32 stored
//! and DEFLATE entries, with optional data descriptors. Encryption, ZIP64,
//! self-extracting prefixes, reordered/overlapping local records, and unknown
//! extra fields/host attributes are rejected rather than interpreted loosely.
//! ZIP input, total decoded payload, entry count and path work are bounded;
//! metadata is borrowed from the bounded input, never allocated from ZIP
//! counts. Native platform qualification is required before claiming support.

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::{BTreeMap, VecDeque},
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
    #[error("ZIP links, hard links, and GNU long links are unsupported")]
    UnsupportedLink,
    #[error("archive symlinks require a qualified Unix host; unsupported on this platform")]
    UnsupportedSymlinkPlatform,
    #[error("archive symlink target is unsafe, missing, aliased or traverses another link")]
    UnsafeLink,
    #[error("archive symlinks form a cycle")]
    LinkCycle,
    #[error("unsupported archive entry, encryption, or metadata")]
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
    Zip,
}

/// Bounds input bytes, decoded TAR bytes (ZIP payload bytes), raw entries and
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
        Format::Zip => zip::extract_zip(artifact, &mut tree)?,
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
            .is_some_and(|n| n.path == path && !n.directory && n.link.is_none())
        {
            return Err(Error::LayoutMismatch);
        }
    }
    tree.finish_links()?;
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
    link: Option<String>,
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
        link: Option<&[u8]>,
        reader: impl Read,
    ) -> Result<(), Error> {
        let path = portable_path(raw, directory, self.limits)?;
        if path != self.root && !path.starts_with(&format!("{}/", self.root)) {
            return Err(Error::LayoutMismatch);
        }
        if (path == self.root && !directory) || (directory && size != 0) {
            return Err(Error::LayoutMismatch);
        }
        let link = link
            .map(|raw| {
                if size != 0 || directory {
                    return Err(Error::InvalidArchive);
                }
                let target = std::str::from_utf8(raw).map_err(|_| Error::UnsafePath)?;
                link_target(&path, target, self.limits, None)?;
                Ok(target.to_owned())
            })
            .transpose()?;
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
                    link: if explicit { link.clone() } else { None },
                };
                self.nodes.insert(key, node);
            }
        }
        if !directory && link.is_none() {
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

    fn finish_links(&self) -> Result<(), Error> {
        if self.nodes.values().all(|node| node.link.is_none()) {
            return Ok(());
        }
        // Directory containment plus link-target edges form the filesystem graph.
        // Kahn's algorithm rejects file chains and directory traversal cycles
        // without recursion or creating even one symlink on a validation failure.
        let indices: BTreeMap<_, _> = self
            .nodes
            .values()
            .enumerate()
            .map(|(i, node)| (node.path.as_str(), i))
            .collect();
        let mut edges = vec![Vec::new(); indices.len()];
        let mut incoming = vec![0usize; indices.len()];
        for node in self.nodes.values() {
            let index = indices[node.path.as_str()];
            if let Some((parent, _)) = node.path.rsplit_once('/') {
                edges[indices[parent]].push(index);
                incoming[index] += 1;
            }
            if let Some(target) = &node.link {
                let target = link_target(&node.path, target, self.limits, Some(&self.nodes))?;
                let destination = indices[target.as_str()];
                edges[index].push(destination);
                incoming[destination] += 1;
            }
        }
        let mut ready: VecDeque<_> = incoming
            .iter()
            .enumerate()
            .filter_map(|(i, count)| (*count == 0).then_some(i))
            .collect();
        let mut visited = 0;
        while let Some(index) = ready.pop_front() {
            visited += 1;
            for &destination in &edges[index] {
                incoming[destination] -= 1;
                if incoming[destination] == 0 {
                    ready.push_back(destination);
                }
            }
        }
        if visited != indices.len() {
            return Err(Error::LinkCycle);
        }
        #[cfg(unix)]
        for node in self.nodes.values() {
            if let Some(target) = &node.link {
                let destination = self.staging.path().join(&node.path);
                std::os::unix::fs::symlink(target, &destination)?;
                // macOS applies umask to symlinks and checks their permissions
                // for read_link. Never chmod through a link into its target.
                #[cfg(target_os = "macos")]
                nix::sys::stat::fchmodat(
                    None,
                    &destination,
                    nix::sys::stat::Mode::from_bits_truncate(0o777),
                    nix::sys::stat::FchmodatFlags::NoFollowSymlink,
                )
                .map_err(io::Error::from)?;
            }
        }
        Ok(())
    }
}

fn link_target(
    path: &str,
    target: &str,
    limits: Limits,
    nodes: Option<&BTreeMap<String, Node>>,
) -> Result<String, Error> {
    if target.len() > limits.path_bytes || target.split('/').count() > limits.depth {
        return Err(Error::LimitExceeded);
    }
    let mut parts: Vec<_> = path.split('/').collect();
    parts.pop();
    for component in target.split('/') {
        // Check the original traversal, not just its normalized result: a/../b
        // cannot hide a symlink or regular-file intermediate component.
        if let Some(nodes) = nodes {
            let prefix = parts.join("/");
            if !nodes
                .get(&prefix.to_ascii_lowercase())
                .is_some_and(|n| n.path == prefix && n.directory)
            {
                return Err(Error::UnsafeLink);
            }
        }
        match component {
            "" => return Err(Error::UnsafeLink),
            "." => {}
            ".." => {
                if parts.len() <= 1 {
                    return Err(Error::UnsafeLink);
                }
                parts.pop();
            }
            _ => {
                portable_path(component.as_bytes(), false, limits)?;
                parts.push(component);
            }
        }
        portable_path(parts.join("/").as_bytes(), false, limits)?;
    }
    let resolved = parts.join("/");
    if let Some(nodes) = nodes
        && !nodes
            .get(&resolved.to_ascii_lowercase())
            .is_some_and(|n| n.path == resolved)
    {
        return Err(Error::UnsafeLink);
    }
    Ok(resolved)
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
        if kind.is_hard_link() || kind.is_gnu_longlink() {
            return Err(Error::UnsupportedLink);
        }
        #[cfg(not(unix))]
        if kind.is_symlink() {
            return Err(Error::UnsupportedSymlinkPlatform);
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
        if !kind.is_file() && !kind.is_dir() && !kind.is_symlink() {
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
        let link = if kind.is_symlink() {
            check_tar_field(&entry.header().as_old().linkname)?;
            Some(
                entry
                    .link_name_bytes()
                    .ok_or(Error::UnsafeLink)?
                    .into_owned(),
            )
        } else {
            None
        };
        tree.entry(
            &name,
            kind.is_dir(),
            mode,
            size,
            link.as_deref(),
            &mut entry,
        )?;
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

mod zip;

#[cfg(test)]
mod tests;
