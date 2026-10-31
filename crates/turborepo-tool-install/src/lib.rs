//! The user's shared tool store. Each verified tool tree is installed once
//! per user under a key derived from what makes its bytes, and each complete
//! selection of tools gets one immutable environment of shims that every
//! repository selecting it shares. Repositories hold none of this state:
//! [`UserStore`] locates the store, [`SetupSession`] converges it, and
//! [`UserStore::activate`] renders a recorded environment without locks and
//! without reading trees. The store is NOT authorization: callers bind a
//! repository to an environment with their own user-owned evidence. Managed
//! paths reject symlinks; staging is private. Windows exports are `.exe`
//! hard links or generated `.cmd` forwarders found through PATHEXT.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

mod gc;
mod selection;
mod store;
mod user_store;

pub use gc::{Collected, GRACE, RepoState, Usage};
pub use selection::{Export, Launch, LaunchArg, ManagedVar, TreeRef, VarValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use store::{Activation, Ensured, EnvKey, Reuse, Selection, SetupSession};
pub use user_store::{TOOL_STORE_VAR, UserStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsafe managed installation path or tree")]
    UnsafePath,
    #[error("invalid tool selection, tool identity or executable collision")]
    InvalidInventory,
    #[error("managed {0} is not installed in the tool store; run turbo setup")]
    NotInstalled(String),
    #[error("managed {0} changed since turbo setup installed it; run turbo setup --force")]
    Damaged(String),
    #[error("managed shims require a qualified Unix or Windows host and executable type")]
    UnsupportedPlatform,
    #[error("managed installation I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid tool store metadata")]
    Json(#[from] serde_json::Error),
    #[error("cannot locate the user tool store; set TURBO_TOOL_STORE to an absolute directory")]
    NoStore,
    #[error(
        "unsafe tool store {0}; TURBO_TOOL_STORE must name an absolute directory outside the \
         repository that only you can write"
    )]
    UnsafeStore(PathBuf),
}

/// Exact identity independent of turbo.lock; artifact/platform identify bytes,
/// not mirrors. Executables map portable names to what an environment's
/// `bin` exports for them; `vars` are the variables those programs need.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: String,
    pub version: String,
    pub platform: String,
    pub artifact_sha256: String,
    pub executables: BTreeMap<String, Export>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<ManagedVar, VarValue>,
    pub staging: Staging,
}

/// Everything beyond the verified artifact that decides the bytes setup
/// stages into a tool's tree: the adapter whose staging code runs, that
/// code's revision, and every input it reads. Trees are keyed by it, so two
/// tools that stage the same artifact differently never share a tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Staging {
    adapter: String,
    revision: u32,
    inputs: serde_json::Value,
}

impl Staging {
    /// `inputs` is one typed value holding every input the adapter's staging
    /// reads. Bump `revision` whenever that staging code would write
    /// different bytes for the same inputs.
    pub fn new(adapter: &str, revision: u32, inputs: &impl Serialize) -> Result<Self, Error> {
        component(adapter)?;
        Ok(Self {
            adapter: adapter.to_owned(),
            revision,
            inputs: serde_json::to_value(inputs)?,
        })
    }
}

impl Tool {
    /// The paths in this tool's own tree that its exports run or pass.
    pub fn tree_files(&self) -> Vec<&str> {
        (self.executables.values())
            .flat_map(|export| export.tree_files(&self.id))
            .filter(|(id, ..)| *id == self.id)
            .map(|(_, path, _)| path)
            .collect()
    }

    /// The file names this tool's exports take in `bin`. On Windows the
    /// extension is the export's PATHEXT type.
    pub fn bin_entries(&self) -> Result<Vec<String>, Error> {
        self.executables
            .iter()
            .map(|(name, export)| shim_name(name, export))
            .collect()
    }
}

/// Entry count and apparent bytes, for reporting disk use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskUse {
    pub entries: usize,
    pub bytes: u64,
}

const RECORD_LIMIT: u64 = 1024 * 1024;

fn component(value: &str) -> Result<(), Error> {
    // Conservative cross-filesystem spelling: no traversal, ADS, DOS devices or
    // case aliases. Version labels are opaque; only IDs/names become components.
    let stem = value.split('.').next().unwrap_or("");
    if value.is_empty()
        || value.len() > 128
        || value.starts_with('.')
        || value.ends_with('.')
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        || matches!(stem, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        })
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

fn artifact_component(value: &str) -> Result<(), Error> {
    // Preserve artifact spelling (scopes, case and dotfiles), not bookkeeping
    // identity rules. Keep the archive boundary's conservative portable subset.
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value.ends_with(['.', ' '])
        || value
            .bytes()
            .any(|b| !(32..127).contains(&b) || b"\\/:<>\"|?*~".contains(&b))
    {
        return Err(Error::UnsafePath);
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_lowercase();
    if matches!(
        stem.as_str(),
        "con" | "prn" | "aux" | "nul" | "clock$" | "conin$" | "conout$"
    ) || ["com", "lpt"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
    }) {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

fn relative(value: &str) -> Result<(), Error> {
    if value.len() > 4096 {
        return Err(Error::UnsafePath);
    }
    for part in value.split('/') {
        artifact_component(part)?;
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_tool(tool: &Tool) -> Result<(), Error> {
    component(&tool.id)?;
    component(&tool.staging.adapter)?;
    if tool.version.is_empty()
        || tool.platform.is_empty()
        || !is_sha256(&tool.artifact_sha256)
        || tool.executables.is_empty()
    {
        return Err(Error::InvalidInventory);
    }
    for name in tool.executables.keys() {
        component(name)?;
    }
    selection::validate(tool)
}

/// A complete selection: every tool valid, ids and export names unique, and
/// every launcher naming only selected trees.
fn validate_tools(tools: &[Tool]) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for tool in tools {
        validate_tool(tool)?;
        if !ids.insert(tool.id.as_str()) || !tool.executables.keys().all(|n| names.insert(n)) {
            return Err(Error::InvalidInventory);
        }
    }
    for tool in tools {
        for export in tool.executables.values() {
            if !export
                .tree_files(&tool.id)
                .iter()
                .all(|(id, ..)| ids.contains(id))
            {
                return Err(Error::InvalidInventory);
            }
        }
    }
    Ok(())
}

/// A directory setup creates: real, owned by the effective user, and not
/// writable by anyone else.
fn real_directory(path: &Path) -> Result<(), Error> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !private_to_user(m.mode(), m.uid()) {
            return Err(Error::UnsafePath);
        }
    }
    // Junctions are symlinks above; also reject every other reparse point.
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&m) & 0x400 != 0 {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn private_to_user(mode: u32, uid: u32) -> bool {
    mode & 0o022 == 0 && uid == rustix::process::geteuid().as_raw()
}

/// The repository root and the `.turbo` directory other turbo commands share
/// keep the modes the user's umask gave them (0775 under Ubuntu's default),
/// so only world-writable directories are rejected.
fn shared_directory(path: &Path) -> Result<(), Error> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o002 != 0 {
        return Err(Error::UnsafePath);
    }
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&m) & 0x400 != 0 {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

/// Lock paths use `/`; never join them unsplit onto a Windows verbatim path.
fn relative_path(value: &str) -> PathBuf {
    value.split('/').collect()
}

/// PATHEXT type of a Windows export. Forwarders interpolate the target into a
/// batch line, so `.cmd` targets use a conservative spelling without `%`/`!`.
#[cfg(windows)]
fn windows_kind(path: &str) -> Result<&'static str, Error> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        Ok("exe")
    } else if lower.ends_with(".cmd")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
    {
        Ok("cmd")
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

/// The PATH entry for a portable export name. Windows keeps the target's
/// PATHEXT extension so `name` resolves to `name.exe` or `name.cmd`.
fn shim_name(name: &str, export: &Export) -> Result<String, Error> {
    #[cfg(windows)]
    return Ok(match export {
        Export::File(path) => format!("{name}.{}", windows_kind(path)?),
        Export::Launch(_) => format!("{name}.cmd"),
    });
    #[cfg(not(windows))]
    {
        let _ = export;
        Ok(name.to_owned())
    }
}

/// Create an owner-only directory, or check an existing one with `existing`.
/// A umask only clears bits, so any umask that leaves the owner its own
/// permissions yields exactly 0700.
fn create_directory(path: &Path, existing: fn(&Path) -> Result<(), Error>) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => existing(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => {}
                // A concurrent setup created it first.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return existing(path),
                Err(e) => return Err(e.into()),
            }
            sync_directory(path.parent().ok_or(Error::UnsafePath)?)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn directory(path: &Path) -> Result<(), Error> {
    create_directory(path, real_directory)
}

/// Every tree file an export runs or passes exists inside its own tree, and
/// every file it runs is executable. `tree` maps a tool id to its tree root.
fn check_executables(tool: &Tool, tree: impl Fn(&str) -> Option<PathBuf>) -> Result<(), Error> {
    for export in tool.executables.values() {
        for (id, path, executed) in export.tree_files(&tool.id) {
            let root = fs::canonicalize(tree(id).ok_or(Error::InvalidInventory)?)?;
            let resolved = fs::canonicalize(root.join(relative_path(path)))?;
            if !resolved.starts_with(&root) || !resolved.is_file() {
                return Err(Error::UnsafePath);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Installer-owned files need owner execute; group/other execute
                // alone does not make the exported binary runnable by its owner.
                if executed && fs::metadata(resolved)?.permissions().mode() & 0o100 == 0 {
                    return Err(Error::InvalidInventory);
                }
            }
            #[cfg(not(unix))]
            let _ = executed;
        }
    }
    Ok(())
}

// Walk without following directory links. Accept only relative, existing
// symlinks confined to this tool's full tree, preserving adjacent
// libraries/resources.
fn tree_hash(root: &Path) -> Result<String, Error> {
    real_directory(root)?;
    let mut hash = Sha256::new();
    walk(root, &mut |path| {
        let m = fs::symlink_metadata(path)?;
        let name = path
            .strip_prefix(root)
            .map_err(|_| Error::UnsafePath)?
            .to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if m.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            let resolved = match fs::canonicalize(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::UnsafePath),
                resolved => resolved?,
            };
            if target.is_absolute() || !resolved.starts_with(root) {
                return Err(Error::UnsafePath);
            }
            hash.update(b"link");
            let target = target.to_str().ok_or(Error::UnsafePath)?;
            hash.update((target.len() as u64).to_le_bytes());
            hash.update(target.as_bytes());
        } else if m.is_file() {
            // Unix never hard-links into trees. Windows `.exe` exports are hard
            // links to tree files, so its link count is not checked.
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if m.nlink() != 1 || m.permissions().mode() & 0o7000 != 0 {
                    return Err(Error::UnsafePath);
                }
                hash.update((m.permissions().mode() & 0o777).to_le_bytes());
            }
            hash.update(b"file");
            hash.update(m.len().to_le_bytes());
            let mut file = File::open(path)?;
            let mut bytes = [0; 8192];
            loop {
                let n = file.read(&mut bytes)?;
                if n == 0 {
                    break;
                }
                hash.update(&bytes[..n]);
            }
        } else if m.is_dir() {
            hash.update(b"directory");
        } else {
            return Err(Error::UnsafePath);
        }
        Ok(())
    })?;
    Ok(hex::encode(hash.finalize()))
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path) -> Result<(), Error>) -> Result<(), Error> {
    let mut entries = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::UnsafePath)?;
        artifact_component(name)?;
        visit(&path)?;
        if fs::symlink_metadata(&path)?.is_dir() {
            walk(&path, visit)?;
        }
    }
    Ok(())
}

/// Files only: directories stay writable so the store can still be deleted
/// with `rm -rf`. Unix only: on Windows the read-only attribute also blocks
/// the deletion and replacement the store relies on.
fn make_read_only(root: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        walk(root, &mut |path| {
            let m = fs::symlink_metadata(path)?;
            let mode = m.permissions().mode() & 0o7777;
            if m.is_file() && mode & 0o222 != 0 {
                fs::set_permissions(path, fs::Permissions::from_mode(mode & !0o222))?;
            }
            Ok(())
        })?;
    }
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}

/// A new owner-only file in a private staging directory.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

/// Bounded read of a regular, non-symlink file.
fn read_bounded(path: &Path) -> Result<Vec<u8>, Error> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > RECORD_LIMIT {
        return Err(Error::UnsafePath);
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(Error::UnsafePath);
    }
    Ok(bytes)
}

fn sync_directory(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn sync_tree(root: &Path) -> Result<(), Error> {
    let mut dirs = vec![root.to_path_buf()];
    walk(root, &mut |path| {
        let m = fs::symlink_metadata(path)?;
        if m.is_dir() {
            dirs.push(path.to_path_buf());
        } else if m.is_file() {
            // FlushFileBuffers requires write access on Windows.
            OpenOptions::new()
                .read(true)
                .write(cfg!(windows))
                .open(path)?
                .sync_all()?;
        }
        Ok(())
    })?;
    for dir in dirs.into_iter().rev() {
        sync_directory(&dir)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
