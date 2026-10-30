//! Clone-local installation transactions, not resolution or activation.
//! Callers supply verified trees; one manifest replacement selects a complete
//! generation with owned shims. Replaced generations stay inactive through a
//! cleanup grace period. This ignored inventory is NOT authorization: future
//! activation must authorize setup-recorded tools using user state outside the
//! repository, never repo data. Repository parents are trusted; same-user
//! hostile mutation is out of scope, as with archive staging. Managed paths
//! reject symlinks; staging is private. Windows exports are `.exe` hard links
//! or generated `.cmd` forwarders found through PATHEXT, and directory handles
//! are pinned without delete sharing instead of comparing inode identity.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

mod repair;
mod selection;
mod user_store;

pub use selection::{Export, Launch, LaunchArg, ManagedVar, TreeRef, VarValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use user_store::{TOOL_STORE_VAR, UserStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsafe managed installation path or tree")]
    UnsafePath,
    #[error("invalid inventory, tool identity or executable collision")]
    InvalidInventory,
    /// Hash drift only, after all generation paths, exports and records
    /// validate.
    #[error("managed tool contents changed within a structurally valid generation")]
    DamagedContents,
    #[error("another setup transaction is running")]
    Busy,
    #[error("managed shims require a qualified Unix or Windows host and executable type")]
    UnsupportedPlatform,
    #[error("generation published but durability sync failed; recheck local inventory")]
    Published(#[source] Box<Error>),
    #[error("managed installation I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid local installation manifest")]
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
/// not mirrors. Executables map portable names to what `bin` exports for
/// them; `vars` are the variables those programs need.
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
}

impl Tool {
    /// Whether `other` stages the same tree bytes. Variables are rendered
    /// only into `bin`, so a tree whose variables changed is still reusable.
    fn same_tree(&self, other: &Tool) -> bool {
        (&self.id, &self.version, &self.platform) == (&other.id, &other.version, &other.platform)
            && self.artifact_sha256 == other.artifact_sha256
            && self.executables == other.executables
    }

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    tool: Tool,
    tree_sha256: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema: u32,
    generation: String,
    tools: Vec<Installed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record_sha256: Option<String>,
    /// The `CARGO_HOME` this generation's launchers and activation use,
    /// recorded only when a tool needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cargo_home: Option<PathBuf>,
}

const RECORD_LIMIT: u64 = 1024 * 1024;

/// How much installed content a readiness check reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    /// Re-hash every installed file's bytes against the recorded tree hash.
    Full,
    /// Never reads installed trees. Checks only the generation layout, each
    /// tool's exported executables and their shims, the exact `bin` listing
    /// and the generation record. Valid only for callers that bind the
    /// manifest, whose generation and per-tree hashes name the verified
    /// bytes, to external evidence such as a user-owned setup record written
    /// after setup verified those trees. `Full` finds damage in place.
    Shallow,
}

/// Opaque bookkeeping, never execution authorization. The caller must exclude
/// secrets, raw source manifests and machine paths, and validate canonical
/// encoding, provenance and native semantics. This backend cannot validate
/// those properties; it only bounds and binds bytes to a complete generation.
#[derive(Clone, PartialEq, Eq)]
pub struct Record(Vec<u8>);

impl Record {
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() as u64 > RECORD_LIMIT {
            return Err(Error::InvalidInventory);
        }
        Ok(Self(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    fn hash(&self) -> String {
        hex::encode(Sha256::digest(&self.0))
    }
}

impl Inventory {
    /// Holds a tree `tool` can reuse.
    fn contains(&self, tool: &Tool) -> bool {
        self.tools
            .iter()
            .any(|installed| installed.tool.same_tree(tool))
    }
}

pub struct Store {
    root: PathBuf,
    // Never unlink a lock file: all processes must lock the same inode.
    _lock: File,
    owner_pid: u32,
    root_handle: File,
    repo_handle: File,
    identity: Arc<()>,
    cargo_home: Option<PathBuf>,
}

/// Sealed evidence from one live Store guard, not a caller-authored path or
/// Current. Recheck after waits; hold caller source/writer guards through
/// reconciliation. This is integrity evidence, never execution authorization.
pub struct GenerationExpectation {
    identity: Arc<()>,
    manifest: Option<Vec<u8>>,
    current: Option<Current>,
    repair: Option<(PathBuf, Option<String>)>,
}

impl GenerationExpectation {
    /// False for repair evidence: semantic selection is not installed
    /// readiness.
    pub fn healthy(&self) -> bool {
        self.repair.is_none()
    }

    /// Hash-bound selection metadata. Resources are inspected for CAS, but
    /// usable only when healthy(); repair evidence never authorizes reuse.
    /// None means truly absent manifest, not a broken selected generation.
    pub fn current(&self) -> Option<&Current> {
        self.current.as_ref()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // Closing alone can leave flock held by a forked child's inherited
        // open-file description. A fork copy must only close its File, never
        // unlock the surviving acquiring process's shared description.
        if self.owner_pid == std::process::id() {
            let _ = self._lock.unlock();
        }
    }
}

pub struct Current {
    pub tools: Vec<Tool>,
    /// For bookkeeping only; not a trusted activation PATH.
    pub bin: PathBuf,
    /// Hash-checked bookkeeping from this generation. Caller validation
    /// required.
    pub record: Option<Record>,
    /// The `CARGO_HOME` the generation was published with, if a tool needs
    /// one.
    pub cargo_home: Option<PathBuf>,
}

impl Current {
    /// Resources for an inspected member, not execution authorization.
    pub fn tool_tree(&self, tool: &Tool) -> Option<PathBuf> {
        self.tools.contains(tool).then_some(())?;
        Some(self.bin.parent()?.join("tools").join(&tool.id))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Replaced,
}

/// Entry count and apparent bytes, for reporting disk use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskUse {
    pub entries: usize,
    pub bytes: u64,
}

/// Disk use of one clone's or worktree's `.turbo/tools`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageUse {
    /// The selected generation.
    pub selected: DiskUse,
    /// Inactive entries past their grace period: removed by
    /// [`Store::remove_inactive`], or removable when only inspected.
    pub removable: DiskUse,
    /// Inactive entries still within their grace period.
    pub retained: DiskUse,
}

impl DiskUse {
    fn add(&mut self, path: &Path) -> io::Result<()> {
        self.entries += 1;
        self.bytes += apparent_bytes(path)?;
        Ok(())
    }
}

// Sizes without following links, so a planted link cannot inflate reports.
fn apparent_bytes(path: &Path) -> io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    fs::read_dir(path)?.try_fold(
        0,
        |total, entry| Ok(total + apparent_bytes(&entry?.path())?),
    )
}

impl Store {
    /// Locks this clone's ignored .turbo/tools, without touching global state.
    pub fn open(repo: &Path) -> Result<Self, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedPlatform);
        }
        let repo = fs::canonicalize(repo)?;
        shared_directory(&repo)?;
        let repo_handle = open_directory(&repo)?;
        let turbo = repo.join(".turbo");
        create_directory(&turbo, shared_directory)?;
        let root = turbo.join("tools");
        directory(&root)?;
        let root_handle = open_directory(&root)?;
        let path = root.join("transaction.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(Error::UnsafePath),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path)?;
        let owner_pid = std::process::id();
        lock.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => Error::Busy,
            std::fs::TryLockError::Error(e) => Error::Io(e),
        })?;
        let store = Self {
            root,
            _lock: lock,
            owner_pid,
            root_handle,
            repo_handle,
            identity: Arc::new(()),
            cargo_home: None,
        };
        store.check_root()?;
        Ok(store)
    }

    /// Publishes generations whose tools need a `CARGO_HOME` with this
    /// repository's one in `user_store`. Without it, publishing such a tool
    /// fails.
    pub fn with_user_store(mut self, user_store: &UserStore) -> Result<Self, Error> {
        self.cargo_home = Some(user_store.cargo_home(self.repository_root()?));
        Ok(self)
    }

    /// Canonical actual caller-selected repository, never derived from
    /// metadata.
    pub fn repository_root(&self) -> Result<&Path, Error> {
        self.root
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::UnsafePath)
    }

    fn check_root(&self) -> Result<(), Error> {
        shared_directory(self.repository_root()?)?;
        shared_directory(self.root.parent().ok_or(Error::UnsafePath)?)?;
        real_directory(&self.root)?;
        if self.owner_pid != std::process::id() || fs::canonicalize(&self.root)? != self.root {
            return Err(Error::UnsafePath);
        }
        for (handle, path) in [
            (&self.root_handle, self.root.as_path()),
            (&self.repo_handle, self.repository_root()?),
        ] {
            same_directory(handle, path)?;
        }
        Ok(())
    }

    fn manifest_bytes(root: &Path) -> Result<Option<Vec<u8>>, Error> {
        let path = root.join("manifest.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(Error::UnsafePath),
            _ => {}
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(RECORD_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > RECORD_LIMIT {
            return Err(Error::InvalidInventory);
        }
        Ok(Some(bytes))
    }

    /// Capture the actual manifest and full healthy selection under this guard.
    /// Present invalid metadata or a missing generation is not an absent
    /// baseline.
    pub fn generation(&self) -> Result<GenerationExpectation, Error> {
        self.check_root()?;
        let manifest = Self::manifest_bytes(&self.root)?;
        let current = self.current()?;
        if manifest.is_some() && current.is_none() {
            return Err(Error::InvalidInventory);
        }
        let expected = GenerationExpectation {
            identity: self.identity.clone(),
            manifest,
            current,
            repair: None,
        };
        self.check_generation(&expected)?;
        Ok(expected)
    }

    /// Reverify exact bytes, held root identity, generation layout/shims and
    /// record. Tokens are valid only on the acquiring live Store, including
    /// after waits.
    pub fn check_generation(&self, expected: &GenerationExpectation) -> Result<(), Error> {
        self.check_root()?;
        if !Arc::ptr_eq(&self.identity, &expected.identity)
            || Self::manifest_bytes(&self.root)? != expected.manifest
            || if let Some((_, state)) = &expected.repair {
                self.repair_state(expected)? != *state
            } else {
                self.current()?.is_none() != expected.current.is_none()
            }
            || Self::manifest_bytes(&self.root)? != expected.manifest
        {
            return Err(Error::InvalidInventory);
        }
        self.check_root()
    }

    /// Read-only readiness at this exact repository root, never an ancestor.
    /// Nothing is created or written: no directories, transaction lock, or
    /// user state. This is integrity/bookkeeping evidence, NOT authorization
    /// to execute repo data. Reject publication observed during inspection;
    /// callers must still handle later changes (this does not lease a
    /// generation against hostile edits). Shallow.
    pub fn inspect(repo: &Path) -> Result<Option<Current>, Error> {
        Self::inspect_with(repo, Verification::Shallow)
    }

    /// `inspect` with an explicit content verification level.
    pub fn inspect_with(repo: &Path, verification: Verification) -> Result<Option<Current>, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedPlatform);
        }
        let repo = fs::canonicalize(repo)?;
        shared_directory(&repo)?;
        let turbo = repo.join(".turbo");
        let root = turbo.join("tools");
        let checks: [(&Path, DirectoryCheck); 2] =
            [(&turbo, shared_directory), (&root, real_directory)];
        for (path, check) in checks {
            match check(path) {
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                result => result?,
            }
        }
        let root_handle = open_directory(&root)?;
        let before = Self::manifest_bytes(&root)?;
        let Some(inventory) = Self::inventory(&root)? else {
            return Ok(None);
        };
        let record = Self::check(&root, &inventory, verification);
        if Self::manifest_bytes(&root)? != before {
            return Err(Error::InvalidInventory);
        }
        shared_directory(&turbo)?;
        real_directory(&root)?;
        same_directory(&root_handle, &root)?;
        Ok(Some(Current {
            bin: root.join(inventory.generation).join("bin"),
            tools: inventory.tools.into_iter().map(|t| t.tool).collect(),
            record: record?,
            cargo_home: inventory.cargo_home,
        }))
    }

    /// Whether this exact root's manifest selects a generation carrying a
    /// caller record, from bounded manifest metadata alone; absent inventory
    /// is `false`. No tree, shim or record checks: a routing hint, never
    /// readiness or authorization.
    pub fn selects_record(repo: &Path) -> Result<bool, Error> {
        let root = fs::canonicalize(repo)?.join(".turbo").join("tools");
        Ok(Self::inventory_metadata(&root)?
            .is_some_and(|inventory| inventory.record_sha256.is_some()))
    }

    fn inventory_metadata(root: &Path) -> Result<Option<Inventory>, Error> {
        let Some(bytes) = Self::manifest_bytes(root)? else {
            return Ok(None);
        };
        let inventory: Inventory = serde_json::from_slice(&bytes)?;
        if inventory.schema != 1
            || inventory
                .record_sha256
                .as_ref()
                .is_some_and(|h| !is_sha256(h))
            || !inventory.generation.starts_with("generation-")
            || inventory
                .cargo_home
                .as_ref()
                .is_some_and(|home| !home.is_absolute())
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
        Ok(Some(inventory))
    }

    fn inventory(root: &Path) -> Result<Option<Inventory>, Error> {
        let Some(inventory) = Self::inventory_metadata(root)? else {
            return Ok(None);
        };
        match real_directory(&root.join(&inventory.generation)) {
            Ok(()) => Ok(Some(inventory)),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn current(&self) -> Result<Option<Current>, Error> {
        self.check_root()?;
        let Some(inventory) = Self::inventory(&self.root)? else {
            return Ok(None);
        };
        let record = Self::check(&self.root, &inventory, Verification::Shallow)?;
        Ok(Some(Current {
            bin: self.root.join(inventory.generation).join("bin"),
            tools: inventory.tools.into_iter().map(|t| t.tool).collect(),
            record,
            cargo_home: inventory.cargo_home,
        }))
    }

    /// Validate a complete desired inventory without reading existing state or
    /// performing I/O. Adapters must apply this before preparation, including
    /// fresh preparation that deliberately skips reuse of damaged generations.
    pub fn validate_desired(desired: &[Tool]) -> Result<(), Error> {
        validate_tools(desired)
    }

    /// Readiness for a complete desired tool set, without downloading or
    /// staging or reading trees. A broken layout, export or record returns
    /// false so provisioning can repair it atomically.
    pub fn is_current(&self, desired: &[Tool]) -> Result<bool, Error> {
        validate_tools(desired)?;
        let desired = sorted(desired);
        Ok(self
            .healthy_inventory()?
            .is_some_and(|old| old.tools.iter().map(|t| &t.tool).eq(desired.iter())))
    }

    /// Whether reconciliation can copy this exact tool without preparation.
    /// A broken sibling layout or shim invalidates reuse of the entire
    /// generation. Trees are not read; see `reusable_tree`.
    pub fn can_reuse(&self, tool: &Tool) -> Result<bool, Error> {
        validate_tool(tool)?;
        Ok(self
            .healthy_inventory()?
            .is_some_and(|old| old.contains(tool)))
    }

    /// Healthy tree for adapter-specific resource verification while holding
    /// this Store guard. Not activation authorization or resolution semantics.
    /// An unchanged cohort publishes nothing, so its trees are not read. A
    /// changed cohort copies this tree into a new generation, so it must
    /// match its recorded hash; otherwise the caller prepares it afresh.
    /// Unchanged means the same tools under the same record; callers check
    /// `republishes_damage` first.
    pub fn reusable_tree(&self, tool: &Tool, desired: &[Tool]) -> Result<Option<PathBuf>, Error> {
        validate_tools(desired)?;
        if !desired.contains(tool) {
            return Err(Error::InvalidInventory);
        }
        let Some(old) = self.healthy_inventory()? else {
            return Ok(None);
        };
        let Some(entry) = old.tools.iter().find(|entry| entry.tool.same_tree(tool)) else {
            return Ok(None);
        };
        let tree = self.root.join(&old.generation).join("tools").join(&tool.id);
        let unchanged = old.tools.iter().map(|t| &t.tool).eq(sorted(desired).iter());
        if !unchanged {
            match tree_hash(&tree) {
                Ok(hash) if hash == entry.tree_sha256 => {}
                Ok(_) | Err(Error::UnsafePath) => return Ok(None),
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        Ok(Some(tree))
    }

    /// Whether reconciling `desired` with `record` republishes the current
    /// tools under a different record while a tree no longer matches its
    /// recorded hash. That new generation copies and re-hashes every tree,
    /// which `reusable_tree` does not check for the tools alone, so the
    /// caller must prepare every tool afresh. Hashes trees only when the
    /// record alone changes.
    pub fn republishes_damage(
        &self,
        desired: &[Tool],
        record: Option<&Record>,
    ) -> Result<bool, Error> {
        validate_tools(desired)?;
        let Some(old) = self.healthy_inventory()? else {
            return Ok(false);
        };
        let mut desired = desired.to_vec();
        desired.sort_by(|a, b| a.id.cmp(&b.id));
        if !old.tools.iter().map(|t| &t.tool).eq(desired.iter())
            || old.record_sha256 == record.map(Record::hash)
        {
            return Ok(false);
        }
        match Self::check(&self.root, &old, Verification::Full) {
            Ok(_) => Ok(false),
            Err(Error::DamagedContents | Error::UnsafePath) => Ok(true),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e),
        }
    }

    fn healthy_inventory(&self) -> Result<Option<Inventory>, Error> {
        self.check_root()?;
        let Some(old) = Self::inventory(&self.root)? else {
            return Ok(None);
        };
        match Self::check(&self.root, &old, Verification::Shallow) {
            Ok(_) => Ok(Some(old)),
            Err(Error::InvalidInventory | Error::UnsafePath | Error::DamagedContents) => Ok(None),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn record(root: &Path, inventory: &Inventory) -> Result<Option<Record>, Error> {
        let path = root.join(&inventory.generation).join("record.json");
        let metadata = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound && inventory.record_sha256.is_none() => {
                return Ok(None);
            }
            result => result?,
        };
        let expected = inventory
            .record_sha256
            .as_ref()
            .ok_or(Error::InvalidInventory)?;
        Self::read_record(&path, &metadata, expected).map(Some)
    }

    fn read_record(path: &Path, metadata: &fs::Metadata, expected: &str) -> Result<Record, Error> {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::UnsafePath);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 || metadata.mode() & 0o7022 != 0 {
                return Err(Error::UnsafePath);
            }
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(RECORD_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        let record = Record::new(bytes)?;
        if record.hash() != expected {
            return Err(Error::InvalidInventory);
        }
        Ok(record)
    }

    fn check(
        root: &Path,
        inventory: &Inventory,
        verification: Verification,
    ) -> Result<Option<Record>, Error> {
        let generation = root.join(&inventory.generation);
        real_directory(&generation)?;
        let tools = generation.join("tools");
        real_directory(&tools)?;
        let bin = generation.join("bin");
        real_directory(&bin)?;
        let mut names = BTreeSet::new();
        let mut damaged = false;
        for installed in &inventory.tools {
            let tree = tools.join(&installed.tool.id);
            if verification == Verification::Full && tree_hash(&tree)? != installed.tree_sha256 {
                damaged = true;
            }
            check_executables(&tools, &installed.tool)?;
            for (name, export) in &installed.tool.executables {
                let shim = shim_name(name, export)?;
                check_shim(
                    &bin.join(&shim),
                    &tools,
                    &installed.tool,
                    export,
                    inventory.cargo_home.as_deref(),
                )?;
                names.insert(shim);
            }
        }
        let actual = fs::read_dir(bin)?
            .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<BTreeSet<_>, io::Error>>()?;
        if actual != names {
            return Err(Error::InvalidInventory);
        }
        let record = Self::record(root, inventory)?;
        // Never let content drift mask an unsafe shim, path or corrupt record.
        if damaged {
            return Err(Error::DamagedContents);
        }
        Ok(record)
    }

    /// Mutable access serializes transactions sharing this lock handle.
    /// Stages a whole desired generation before publishing it. The callback
    /// writes only into the provided empty private tool directory. It must
    /// not mutate any prior generation or run untrusted installation hooks.
    /// Tools in the selection are copied and re-hashed; a copy that no
    /// longer matches its recorded hash is staged through the callback
    /// instead. An unchanged cohort is not re-read: `Verification::Full`
    /// finds damage in place, and forced reconciliation repairs it.
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
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.reconcile_prepared(desired, None, None, stage_tool, before_publish)
    }

    /// Replace the whole generation using fresh adapter preparations, including
    /// healthy identical tools. Never copy old trees or change readiness truth.
    /// The same validation and final precondition apply as in ordinary
    /// reconcile.
    pub fn force_reconcile_checked(
        &mut self,
        desired: &[Tool],
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.reconcile_prepared(desired, Some(&[]), None, stage_tool, before_publish)
    }

    /// Forced replacement that copies `reused` tools from the healthy
    /// selection instead of staging them, for reinstalls (such as offline)
    /// with fresh bytes for only some tools. A reused tool missing from the
    /// healthy selection, or whose copy does not match its recorded hash, is
    /// staged, so the caller must be able to fail it.
    pub fn force_reconcile_reusing_checked(
        &mut self,
        desired: &[Tool],
        reused: &[Tool],
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.reconcile_prepared(desired, Some(reused), None, stage_tool, before_publish)
    }

    /// Select caller-validated metadata atomically with the complete cohort.
    /// Capture generation(), validate its record and full tool resources under
    /// caller guards, and check_generation() after waits. This call rechecks at
    /// entry and after the final caller check on BOTH unchanged and changed
    /// paths. Metadata differences replace the generation; force freshly
    /// stages every tool without changing can_reuse health truth. Ordinary
    /// wrappers select record=None, rather than preserving potentially
    /// stale caller metadata.
    pub fn reconcile_recorded_checked(
        &mut self,
        desired: &[Tool],
        record: &Record,
        expected: &GenerationExpectation,
        force: bool,
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.reconcile_prepared(
            desired,
            force.then_some(&[][..]),
            Some((record, expected)),
            stage_tool,
            before_publish,
        )
    }

    // `force` lists the tools a forced replacement still copies.
    fn reconcile_prepared(
        &mut self,
        desired: &[Tool],
        force: Option<&[Tool]>,
        recorded: Option<(&Record, &GenerationExpectation)>,
        mut stage_tool: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        validate_tools(desired)?;
        if let Some((record, expected)) = recorded {
            self.check_generation(expected)?;
            self.preserve_record(record)?;
        }
        let record_sha256 = recorded.map(|(record, _)| record.hash());
        let cargo_home = desired
            .iter()
            .any(|tool| tool.vars.values().any(|v| *v == VarValue::RepoCargoHome))
            .then(|| self.cargo_home.clone().ok_or(Error::InvalidInventory))
            .transpose()?;
        let old = self.healthy_inventory()?;
        let valid_old = old.as_ref().filter(|_| force.is_none());
        let desired = sorted(desired);
        if valid_old.is_some_and(|old| {
            old.tools.iter().map(|t| &t.tool).eq(desired.iter())
                && old.record_sha256 == record_sha256
                && old.cargo_home == cargo_home
        }) {
            before_publish()?;
            self.check_root()?;
            if let Some((record, expected)) = recorded {
                self.check_generation(expected)?;
                self.preserve_record(record)?;
            }
            return Ok(Outcome::Unchanged);
        }
        let mut builder = tempfile::Builder::new();
        builder.prefix("generation-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let stage = builder.tempdir_in(&self.root)?;
        let tools = stage.path().join("tools");
        directory(&tools)?;
        let bin = stage.path().join("bin");
        directory(&bin)?;
        let mut installed = Vec::new();
        for tool in desired {
            let tree = tools.join(&tool.id);
            directory(&tree)?;
            let reuse = match force {
                Some(reused) => old.as_ref().filter(|_| reused.contains(&tool)),
                None => valid_old,
            };
            let reused = reuse.and_then(|old| {
                let entry = old.tools.iter().find(|entry| entry.tool.same_tree(&tool))?;
                Some((&old.generation, &entry.tree_sha256))
            });
            let copied = match reused {
                Some((generation, expected)) => {
                    let source = self.root.join(generation).join("tools").join(&tool.id);
                    copy_tree(&source, &tree)?;
                    match tree_hash(&tree) {
                        Ok(hash) if hash == *expected => Some(hash),
                        Ok(_) | Err(Error::UnsafePath) => {
                            fs::remove_dir_all(&tree)?;
                            directory(&tree)?;
                            None
                        }
                        Err(e) => return Err(e),
                    }
                }
                None => None,
            };
            if copied.is_none() {
                stage_tool(&tool, &tree)?;
            }
            let modes_changed = make_read_only(&tree)?;
            let tree_sha256 = match copied {
                Some(hash) if !modes_changed => hash,
                _ => tree_hash(&tree)?,
            };
            installed.push(Installed { tool, tree_sha256 });
        }
        // Launchers may name other trees, so export once every tree is staged.
        for installed in &installed {
            export(&tools, &bin, &installed.tool, cargo_home.as_deref())?;
        }
        if let Some((record, _)) = recorded {
            write_private(&stage.path().join("record.json"), record.bytes())?;
        }
        let inventory = Inventory {
            schema: 1,
            record_sha256,
            generation: stage
                .path()
                .file_name()
                .ok_or(Error::UnsafePath)?
                .to_string_lossy()
                .into_owned(),
            tools: installed,
            cargo_home,
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
        self.check_root()?;
        Self::check(&self.root, &inventory, Verification::Shallow)?;
        if let Some((record, expected)) = recorded {
            self.check_generation(expected)?;
            self.preserve_record(record)?;
        }
        self.retire_selected()?;
        manifest
            .persist(self.root.join("manifest.json"))
            .map_err(|e| Error::Io(e.error))?;
        sync_directory(&self.root).map_err(|e| Error::Published(Box::new(e)))?;
        Ok(Outcome::Replaced)
    }

    /// Read-only disk use at this exact repository root, for plans and checks.
    /// Takes no guard and creates nothing, so a concurrent transaction can
    /// make it stale.
    pub fn storage_use(repo: &Path, grace: Duration) -> Result<Option<StorageUse>, Error> {
        // The same hosts and directory rules as `inspect_with`.
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedPlatform);
        }
        let repo = fs::canonicalize(repo)?;
        shared_directory(&repo)?;
        let turbo = repo.join(".turbo");
        let root = turbo.join("tools");
        for (path, check) in [
            (&turbo, shared_directory as DirectoryCheck),
            (&root, real_directory as DirectoryCheck),
        ] {
            match check(path) {
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                result => result?,
            }
        }
        Self::usage(&root, grace, false).map(Some)
    }

    /// Remove inactive turbo-owned entries once they have been inactive for
    /// `grace`, never the selection, manifest, lock, links or unrecognized
    /// entries. `turbo run` uses the selection without this guard, so a
    /// replaced generation's grace starts when it is replaced. An
    /// interruption leaves only inactive remnants for a later call.
    pub fn remove_inactive(&mut self, grace: Duration) -> Result<StorageUse, Error> {
        self.check_root()?;
        let usage = Self::usage(&self.root, grace, true)?;
        if usage.removable.entries > 0 {
            sync_directory(&self.root)?;
        }
        Ok(usage)
    }

    // A present but invalid manifest fails closed instead of making every
    // generation look inactive.
    fn selected_generation(root: &Path) -> Result<Option<String>, Error> {
        let Some(bytes) = Self::manifest_bytes(root)? else {
            return Ok(None);
        };
        let inventory: Inventory = serde_json::from_slice(&bytes)?;
        if inventory.schema != 1 || !inventory.generation.starts_with("generation-") {
            return Err(Error::InvalidInventory);
        }
        component(&inventory.generation.to_ascii_lowercase())?;
        Ok(Some(inventory.generation))
    }

    fn retire_selected(&self) -> Result<(), Error> {
        let Some(generation) = Self::selected_generation(&self.root)? else {
            return Ok(());
        };
        let path = self.root.join(generation);
        match real_directory(&path) {
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        }
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES is sufficient to set the retirement time.
            // Open the directory itself, without following reparse points or
            // permitting replacement while its timestamp is updated.
            let directory = OpenOptions::new()
                .access_mode(0x100)
                .custom_flags(0x0200_0000 | 0x0020_0000)
                .share_mode(0x1 | 0x2)
                .open(&path)?;
            real_directory(&path)?;
            directory
        };
        #[cfg(not(windows))]
        let directory = File::open(path)?;
        directory.set_modified(SystemTime::now())?;
        Ok(())
    }

    // Replaced generations carry their retirement time. Leftovers from
    // interrupted transactions were never selected; their own modification
    // time is a conservative stand-in.
    fn usage(root: &Path, grace: Duration, remove: bool) -> Result<StorageUse, Error> {
        let selected = Self::selected_generation(root)?;
        let now = SystemTime::now();
        let mut usage = StorageUse::default();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let (path, kind) = (entry.path(), entry.file_type()?);
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() && selected.as_deref() == Some(name.as_ref()) {
                usage.selected.add(&path)?;
                continue;
            }
            let generation = name.starts_with("generation-") && kind.is_dir();
            // NamedTempFile's default prefix, only ever a manifest candidate.
            let temporary = name.starts_with(".tmp") && kind.is_file();
            if !generation && !temporary {
                continue;
            }
            // A future timestamp is never settled.
            let inactive_for = now.duration_since(entry.metadata()?.modified()?);
            if !inactive_for.is_ok_and(|age| age >= grace) {
                usage.retained.add(&path)?;
                continue;
            }
            usage.removable.add(&path)?;
            if remove && generation {
                fs::remove_dir_all(&path)?;
            } else if remove {
                fs::remove_file(&path)?;
            }
        }
        Ok(usage)
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

fn sorted(tools: &[Tool]) -> Vec<Tool> {
    let mut tools = tools.to_vec();
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    tools
}

fn validate_tool(tool: &Tool) -> Result<(), Error> {
    component(&tool.id)?;
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

type DirectoryCheck = fn(&Path) -> Result<(), Error>;

/// A directory setup creates: real, owned by the effective user, and not
/// writable by anyone else. Ownership keeps a store below a group-writable
/// `.turbo` from being substituted by another user.
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
/// keep the modes the user's umask gave them (0775 under Ubuntu's default).
/// Whoever can write the root can already change what setup provisions, and
/// the owned store below cannot be substituted, so only world-writable
/// directories are rejected.
fn shared_directory(path: &Path) -> Result<(), Error> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o002 != 0 {
        return Err(Error::UnsafePath);
    }
    // Junctions are symlinks above; also reject every other reparse point.
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&m) & 0x400 != 0 {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

/// A held directory handle. Windows handles also deny delete sharing, so the
/// pinned directory cannot be renamed or replaced while this Store is alive.
fn open_directory(path: &Path) -> Result<File, Error> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT; share
        // read and write only, matching writer storage.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .share_mode(0x1 | 0x2)
            .open(path)?;
        real_directory(path)?;
        Ok(file)
    }
    #[cfg(not(windows))]
    Ok(File::open(path)?)
}

fn same_directory(held: &File, path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (held, now) = (held.metadata()?, fs::metadata(path)?);
        if (held.dev(), held.ino()) != (now.dev(), now.ino()) {
            return Err(Error::UnsafePath);
        }
    }
    #[cfg(windows)]
    if same_file::Handle::from_file(held.try_clone()?)? != same_file::Handle::from_path(path)? {
        return Err(Error::UnsafePath);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (held, path);
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

/// Exactly what a bin entry must be.
enum Shim {
    /// A relative symlink with this target.
    #[cfg(not(windows))]
    Link(PathBuf),
    /// A hard link to this tree file. The image path becomes `bin\name.exe`,
    /// so `.exe` exports must not load adjacent resources.
    #[cfg(windows)]
    HardLink(PathBuf),
    /// A generated script with these bytes.
    Script(Vec<u8>),
}

fn shim(
    tools: &Path,
    tool: &Tool,
    export: &Export,
    cargo_home: Option<&Path>,
) -> Result<Shim, Error> {
    #[cfg(windows)]
    {
        let _ = cargo_home;
        Ok(match export {
            Export::File(path) if windows_kind(path)? == "exe" => {
                Shim::HardLink(tools.join(&tool.id).join(relative_path(path)))
            }
            Export::File(path) => Shim::Script(selection::windows_forwarder(&Launch {
                program: TreeRef::new(tool.id.clone(), path.clone()),
                args: Vec::new(),
            })),
            Export::Launch(launch) => Shim::Script(selection::windows_forwarder(launch)),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = tools;
        Ok(match export {
            Export::File(path) => Shim::Link(PathBuf::from("../tools").join(&tool.id).join(path)),
            Export::Launch(launch) => {
                Shim::Script(selection::unix_launcher(tool, launch, cargo_home)?)
            }
        })
    }
}

fn make_shim(path: &Path, shim: Shim) -> Result<(), Error> {
    match shim {
        #[cfg(not(windows))]
        Shim::Link(target) => make_link(&target, path),
        #[cfg(windows)]
        Shim::HardLink(target) => Ok(fs::hard_link(target, path)?),
        Shim::Script(bytes) => {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?
                .write_all(&bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
            }
            Ok(())
        }
    }
}

/// Check a staged generation's exports of `tool`, then export them into `bin`.
fn export(tools: &Path, bin: &Path, tool: &Tool, cargo_home: Option<&Path>) -> Result<(), Error> {
    check_executables(tools, tool)?;
    for (name, export) in &tool.executables {
        make_shim(
            &bin.join(shim_name(name, export)?),
            shim(tools, tool, export, cargo_home)?,
        )?;
    }
    Ok(())
}

fn check_shim(
    path: &Path,
    tools: &Path,
    tool: &Tool,
    export: &Export,
    cargo_home: Option<&Path>,
) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let regular = metadata.is_file()
        && std::os::windows::fs::MetadataExt::file_attributes(&metadata) & 0x400 == 0;
    #[cfg(not(windows))]
    let regular = metadata.is_file();
    let valid = match shim(tools, tool, export, cargo_home)? {
        #[cfg(not(windows))]
        Shim::Link(target) => metadata.file_type().is_symlink() && fs::read_link(path)? == target,
        #[cfg(windows)]
        Shim::HardLink(target) => regular && same_file::is_same_file(path, target)?,
        Shim::Script(bytes) => {
            regular && metadata.len() <= RECORD_LIMIT && fs::read(path)? == bytes && {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o100 != 0
                }
                #[cfg(not(unix))]
                true
            }
        }
    };
    if !valid {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

fn directory(path: &Path) -> Result<(), Error> {
    create_directory(path, real_directory)
}

/// Create an owner-only directory, or check an existing one with `existing`.
/// A umask only clears bits, so any umask that leaves the owner its own
/// permissions yields exactly 0700.
fn create_directory(path: &Path, existing: DirectoryCheck) -> Result<(), Error> {
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
            builder.create(path)?;
            sync_directory(path.parent().ok_or(Error::UnsafePath)?)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
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

/// Every tree file an export runs or passes exists inside its own tree, and
/// every file it runs is executable.
fn check_executables(tools: &Path, tool: &Tool) -> Result<(), Error> {
    for export in tool.executables.values() {
        for (id, path, executed) in export.tree_files(&tool.id) {
            let root = tools.join(id);
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
    state_hash(root, true)
}

fn state_hash(root: &Path, healthy_links: bool) -> Result<String, Error> {
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
            if healthy_links && (target.is_absolute() || !fs::canonicalize(path)?.starts_with(root))
            {
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

/// Directories stay writable so `rm -rf` and `git clean` still work. Unix
/// only: on Windows the read-only attribute also blocks the deletion and
/// replacement the store relies on.
fn make_read_only(root: &Path) -> Result<bool, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut changed = false;
        walk(root, &mut |path| {
            let m = fs::symlink_metadata(path)?;
            let mode = m.permissions().mode() & 0o7777;
            if m.is_file() && mode & 0o222 != 0 {
                fs::set_permissions(path, fs::Permissions::from_mode(mode & !0o222))?;
                changed = true;
            }
            Ok(())
        })?;
        Ok(changed)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Ok(false)
    }
}

/// A new owner-only file in a private staged generation.
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
#[cfg(all(test, windows))]
mod windows_tests;
