//! Clone-local installation transactions, not resolution or activation.
//! Callers supply verified trees; one manifest replacement selects a complete
//! generation with owned shims. Old generations stay inactive for later
//! cleanup. This ignored inventory is NOT authorization: future activation must
//! authorize setup-recorded tools using user state outside the repository,
//! never repo data. Repository parents are trusted; same-user hostile mutation
//! is out of scope, as with archive staging. Managed paths reject symlinks;
//! staging is private. Native Windows qualification is deferred.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsafe managed installation path or tree")]
    UnsafePath,
    #[error("invalid inventory, tool identity or executable collision")]
    InvalidInventory,
    #[error("another setup transaction is running")]
    Busy,
    #[error("managed shims require a qualified Unix host")]
    UnsupportedPlatform,
    #[error("generation published but durability sync failed; recheck local inventory")]
    Published(#[source] Box<Error>),
    #[error("managed installation I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid local installation manifest")]
    Json(#[from] serde_json::Error),
}

/// Exact identity independent of turbo.lock; artifact/platform identify bytes,
/// not mirrors. Executables map portable names to paths within the full tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: String,
    pub version: String,
    pub platform: String,
    pub artifact_sha256: String,
    pub executables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    tool: Tool,
    tree_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema: u32,
    generation: String,
    tools: Vec<Installed>,
}

impl Inventory {
    fn contains(&self, tool: &Tool) -> bool {
        self.tools.iter().any(|installed| &installed.tool == tool)
    }
}

pub struct Store {
    root: PathBuf,
    // Never unlink a lock file: all processes must lock the same inode.
    _lock: File,
}

pub struct Current {
    pub tools: Vec<Tool>,
    /// For bookkeeping only; not a trusted activation PATH.
    pub bin: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Replaced,
}

impl Store {
    /// Locks this clone's ignored .turbo/tools, without touching global state.
    pub fn open(repo: &Path) -> Result<Self, Error> {
        if !cfg!(unix) {
            return Err(Error::UnsupportedPlatform);
        }
        let repo = fs::canonicalize(repo)?;
        real_directory(&repo)?;
        let turbo = repo.join(".turbo");
        directory(&turbo)?;
        let root = turbo.join("tools");
        directory(&root)?;
        let path = root.join("transaction.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(Error::UnsafePath),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        lock.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => Error::Busy,
            std::fs::TryLockError::Error(e) => Error::Io(e),
        })?;
        Ok(Self { root, _lock: lock })
    }

    fn inventory(&self) -> Result<Option<Inventory>, Error> {
        let path = self.root.join("manifest.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(Error::UnsafePath),
            Ok(_) => {}
        }
        let inventory: Inventory = serde_json::from_reader(File::open(path)?.take(1024 * 1024))?;
        if inventory.schema != 1
            || !inventory.generation.starts_with("generation-")
            || inventory
                .tools
                .iter()
                .any(|installed| !is_sha256(&installed.tree_sha256))
        {
            return Err(Error::InvalidInventory);
        }
        component(&inventory.generation.to_ascii_lowercase())?;
        validate_tools(
            &inventory
                .tools
                .iter()
                .map(|t| t.tool.clone())
                .collect::<Vec<_>>(),
        )?;
        match real_directory(&self.root.join(&inventory.generation)) {
            Ok(()) => Ok(Some(inventory)),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn current(&self) -> Result<Option<Current>, Error> {
        let Some(inventory) = self.inventory()? else {
            return Ok(None);
        };
        self.check(&inventory)?;
        Ok(Some(Current {
            bin: self.root.join(inventory.generation).join("bin"),
            tools: inventory.tools.into_iter().map(|t| t.tool).collect(),
        }))
    }

    /// Readiness for a complete desired tool set, without downloading or
    /// staging. Damaged trees return false so provisioning can repair them
    /// atomically.
    pub fn is_current(&self, desired: &[Tool]) -> Result<bool, Error> {
        validate_tools(desired)?;
        let mut desired = desired.to_vec();
        desired.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(self
            .healthy_inventory()?
            .is_some_and(|old| old.tools.iter().map(|t| &t.tool).eq(desired.iter())))
    }

    /// Whether reconciliation can copy this exact tool without preparation.
    /// A damaged sibling tree or shim invalidates reuse of the entire
    /// generation.
    pub fn can_reuse(&self, tool: &Tool) -> Result<bool, Error> {
        validate_tools(std::slice::from_ref(tool))?;
        Ok(self
            .healthy_inventory()?
            .is_some_and(|old| old.contains(tool)))
    }

    /// Healthy tree for adapter-specific resource verification while holding
    /// this Store guard. Not activation authorization or resolution semantics.
    pub fn reusable_tree(&self, tool: &Tool, desired: &[Tool]) -> Result<Option<PathBuf>, Error> {
        validate_tools(desired)?;
        if !desired.contains(tool) {
            return Err(Error::InvalidInventory);
        }
        Ok(self
            .healthy_inventory()?
            .filter(|old| old.contains(tool))
            .map(|old| self.root.join(old.generation).join("tools").join(&tool.id)))
    }

    fn healthy_inventory(&self) -> Result<Option<Inventory>, Error> {
        let Some(old) = self.inventory()? else {
            return Ok(None);
        };
        match self.check(&old) {
            Ok(()) => Ok(Some(old)),
            // Metadata/path validation above remains fail-closed. Damaged install
            // contents can be rebuilt, but must never be copied into staging.
            Err(Error::InvalidInventory | Error::UnsafePath) => Ok(None),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn check(&self, inventory: &Inventory) -> Result<(), Error> {
        let generation = self.root.join(&inventory.generation);
        let tools = generation.join("tools");
        real_directory(&tools)?;
        let bin = generation.join("bin");
        real_directory(&bin)?;
        let mut names = BTreeSet::new();
        for installed in &inventory.tools {
            let tree = tools.join(&installed.tool.id);
            if tree_hash(&tree)? != installed.tree_sha256 {
                return Err(Error::InvalidInventory);
            }
            check_executables(&tree, &installed.tool)?;
            for (name, executable) in &installed.tool.executables {
                names.insert(name.clone());
                let shim = bin.join(name);
                if !fs::symlink_metadata(&shim)?.file_type().is_symlink()
                    || fs::read_link(shim)? != shim_target(&installed.tool.id, executable)
                {
                    return Err(Error::UnsafePath);
                }
            }
        }
        let actual = fs::read_dir(bin)?
            .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<BTreeSet<_>, io::Error>>()?;
        if actual != names {
            return Err(Error::InvalidInventory);
        }
        Ok(())
    }

    /// Mutable access serializes transactions sharing this lock handle.
    /// Stages a whole desired generation before publishing it. The callback
    /// writes only into the provided empty private tool directory. It must
    /// not mutate any prior generation or run untrusted installation hooks.
    /// Failures before publication leave the prior manifest and trees
    /// untouched. After publication a durability error may mean the new
    /// state is selected.
    pub fn reconcile(
        &mut self,
        desired: &[Tool],
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.reconcile_checked(desired, stage_tool, || Ok(()))
    }

    /// Check the caller's transaction precondition after staging/flush and
    /// before selecting the complete generation (also checked for an
    /// unchanged result). Hold any caller-owned writer guard across this
    /// call, not just the callback.
    pub fn reconcile_checked(
        &mut self,
        desired: &[Tool],
        mut stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        validate_tools(desired)?;
        let old = self.healthy_inventory()?;
        let valid_old = old.as_ref();
        let mut desired = desired.to_vec();
        desired.sort_by(|a, b| a.id.cmp(&b.id));
        if valid_old.is_some_and(|old| old.tools.iter().map(|t| &t.tool).eq(desired.iter())) {
            before_publish()?;
            return Ok(Outcome::Unchanged);
        }
        let stage = tempfile::Builder::new()
            .prefix("generation-")
            .tempdir_in(&self.root)?;
        let tools = stage.path().join("tools");
        directory(&tools)?;
        let bin = stage.path().join("bin");
        directory(&bin)?;
        let mut installed = Vec::new();
        for tool in desired {
            let tree = tools.join(&tool.id);
            directory(&tree)?;
            if let Some(old) = valid_old.filter(|old| old.contains(&tool)) {
                copy_tree(
                    &self.root.join(&old.generation).join("tools").join(&tool.id),
                    &tree,
                )?;
            } else {
                stage_tool(&tool, &tree)?;
            }
            let tree_sha256 = tree_hash(&tree)?;
            check_executables(&tree, &tool)?;
            for (name, path) in &tool.executables {
                make_link(&shim_target(&tool.id, path), &bin.join(name))?;
            }
            installed.push(Installed { tool, tree_sha256 });
        }
        let inventory = Inventory {
            schema: 1,
            generation: stage
                .path()
                .file_name()
                .ok_or(Error::UnsafePath)?
                .to_string_lossy()
                .into_owned(),
            tools: installed,
        };
        // Flush all files and containing directories BEFORE selecting this tree.
        sync_tree(stage.path())?;
        let mut manifest = tempfile::NamedTempFile::new_in(&self.root)?;
        let bytes = serde_json::to_vec(&inventory)?;
        if bytes.len() >= 1024 * 1024 {
            return Err(Error::InvalidInventory);
        }
        manifest.write_all(&bytes)?;
        manifest.write_all(b"\n")?;
        manifest.as_file().sync_all()?;
        // Keep before publication: a crash can leave an inactive orphan, never a
        // manifest pointing to a dropped TempDir. Cleanup is a separate operation.
        let _ = stage.keep();
        sync_directory(&self.root)?;
        before_publish()?;
        manifest
            .persist(self.root.join("manifest.json"))
            .map_err(|e| Error::Io(e.error))?;
        sync_directory(&self.root).map_err(|e| Error::Published(Box::new(e)))?;
        Ok(Outcome::Replaced)
    }
}

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

fn validate_tools(tools: &[Tool]) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for tool in tools {
        component(&tool.id)?;
        if !ids.insert(&tool.id)
            || tool.version.is_empty()
            || tool.platform.is_empty()
            || !is_sha256(&tool.artifact_sha256)
            || tool.executables.is_empty()
        {
            return Err(Error::InvalidInventory);
        }
        for (name, path) in &tool.executables {
            component(name)?;
            relative(path)?;
            if !names.insert(name) {
                return Err(Error::InvalidInventory);
            }
        }
    }
    Ok(())
}

fn real_directory(path: &Path) -> Result<(), Error> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o022 != 0 {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

fn directory(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => real_directory(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
            sync_directory(path.parent().ok_or(Error::UnsafePath)?)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn shim_target(id: &str, path: &str) -> PathBuf {
    PathBuf::from("../tools").join(id).join(path)
}

fn make_link(target: &Path, path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (target, path);
        Err(Error::UnsupportedPlatform)
    }
}

fn check_executables(root: &Path, tool: &Tool) -> Result<(), Error> {
    for path in tool.executables.values() {
        let resolved = fs::canonicalize(root.join(path))?;
        if !resolved.starts_with(root) || !resolved.is_file() {
            return Err(Error::UnsafePath);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Installer-owned files need owner execute; group/other execute
            // alone does not make the exported binary runnable by its owner.
            if fs::metadata(resolved)?.permissions().mode() & 0o100 == 0 {
                return Err(Error::InvalidInventory);
            }
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
            if target.is_absolute() || !fs::canonicalize(path)?.starts_with(root) {
                return Err(Error::UnsafePath);
            }
            hash.update(b"link");
            let target = target.to_str().ok_or(Error::UnsafePath)?;
            hash.update((target.len() as u64).to_le_bytes());
            hash.update(target.as_bytes());
        } else if m.is_file() {
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

fn copy_tree(source: &Path, dest: &Path) -> Result<(), Error> {
    walk(source, &mut |path| {
        let out = dest.join(path.strip_prefix(source).map_err(|_| Error::UnsafePath)?);
        let m = fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() {
            make_link(&fs::read_link(path)?, &out)?;
        } else if m.is_dir() {
            directory(&out)?;
        } else {
            fs::copy(path, out)?;
        }
        Ok(())
    })
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
            File::open(path)?.sync_all()?;
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
