//! Content-addressed trees and environments in the user store, the setup
//! session that converges one repository onto an environment, and the
//! lockless activation run, exec and the shell hook render from it.
//!
//! ```text
//! <store>/gc.lock                      shared by setups (collection takes it exclusively)
//! <store>/gc-state.json                when collection first saw each entry unused
//! <store>/tmp/                         private staging on the store's filesystem
//! <store>/trees/<tree>/{seal.json, root/}   immutable, verified once at install
//! <store>/envs/<env>/{seal.json, bin/}      immutable, shared by equal selections
//! <store>/repos/<repo>/bin -> ../../envs/<env>/bin
//! <repo>/.turbo/bin -> <store>/repos/<repo>/bin     editors only; never read
//! ```

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Export, Tool, TreeRef, UserStore, VarValue, create_directory, directory, is_sha256,
    make_read_only, read_bounded, real_directory, relative_path, selection, sha256_hex,
    shared_directory, shim_name, sync_directory, sync_tree, tree_hash, validate_tools,
    write_private,
};

/// Bump when setup stages different bytes for the same artifact.
const TREE_REVISION: u32 = 1;
/// Bump when environment rendering changes, so no turbo trusts shims another
/// rendered differently.
const ENV_REVISION: u32 = 1;
const SEAL_SCHEMA: u32 = 1;

/// Names a selection's environment: the SHA-256 of its canonical tool list.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EnvKey(String);

impl EnvKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EnvKey {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Error> {
        if is_sha256(&value) {
            Ok(Self(value))
        } else {
            Err(Error::InvalidInventory)
        }
    }
}

impl From<EnvKey> for String {
    fn from(key: EnvKey) -> Self {
        key.0
    }
}

impl fmt::Display for EnvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A complete, validated tool set for one host, sorted by id. Its key names
/// the environment every repository selecting exactly these tools shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    tools: Vec<Tool>,
    key: EnvKey,
}

impl Selection {
    pub fn new(mut tools: Vec<Tool>) -> Result<Self, Error> {
        validate_tools(&tools)?;
        tools.sort_by(|a, b| a.id.cmp(&b.id));
        let mut bytes = format!("turbo-env\0{ENV_REVISION}\0").into_bytes();
        bytes.extend(serde_json::to_vec(&tools)?);
        let key = EnvKey(sha256_hex(&bytes));
        Ok(Self { tools, key })
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn key(&self) -> &EnvKey {
        &self.key
    }

    fn tool(&self, id: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.id == id)
    }
}

/// What makes a tree's bytes, never its exports or the other tools it serves,
/// so selections that stage the same artifact share one tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeSpec {
    id: String,
    version: String,
    platform: String,
    artifact_sha256: String,
}

impl TreeSpec {
    fn of(tool: &Tool) -> Self {
        Self {
            id: tool.id.clone(),
            version: tool.version.clone(),
            platform: tool.platform.clone(),
            artifact_sha256: tool.artifact_sha256.clone(),
        }
    }

    fn key(&self) -> String {
        let mut bytes = format!("turbo-tree\0{TREE_REVISION}").into_bytes();
        for field in [
            &self.id,
            &self.version,
            &self.platform,
            &self.artifact_sha256,
        ] {
            bytes.extend((field.len() as u64).to_le_bytes());
            bytes.extend(field.as_bytes());
        }
        sha256_hex(&bytes)
    }
}

pub(crate) fn tree_key(tool: &Tool) -> String {
    TreeSpec::of(tool).key()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeSeal {
    schema: u32,
    spec: TreeSpec,
    tree_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvSeal {
    schema: u32,
    tools: Vec<Tool>,
}

/// A shared hold on `gc.lock`: while it lives, nothing collects trees or
/// environments. Publishing into the store requires one, so code that skips
/// the lease does not compile.
struct SharedLease {
    _lock: File,
}

impl SharedLease {
    fn acquire(store: &Path) -> Result<Self, Error> {
        let lock = lock_file(store, true)?;
        lock.lock_shared()?;
        Ok(Self { _lock: lock })
    }
}

/// `<store>/gc.lock`, unlocked.
pub(crate) fn lock_file(store: &Path, create: bool) -> Result<File, Error> {
    let path = store.join("gc.lock");
    match fs::symlink_metadata(&path) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(Error::UnsafePath),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Never unlink the lock file: every process must lock the same inode.
    Ok(options.open(path)?)
}

/// How setup decides a sealed tree or environment is reusable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reuse {
    /// Trust the seal setup wrote when it verified the artifact.
    Sealed,
    /// Re-hash every selected tree and re-check the environment first, and
    /// replace whatever no longer matches (`turbo setup --force`).
    Verified,
}

/// What [`SetupSession::ensure`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Ensured {
    pub env: EnvKey,
    /// Tools whose trees this setup staged; every other tree was reused.
    pub installed: BTreeSet<String>,
}

/// One setup of one repository, holding the shared collection lease.
pub struct SetupSession<'s> {
    store: &'s UserStore,
    lease: SharedLease,
    repo: PathBuf,
    reuse: Reuse,
    // Verified reuse hashes each tree once per session.
    verified: RefCell<BTreeMap<String, bool>>,
}

/// What renders a recorded environment for run, exec, the shell hook and
/// setup's dependency installers. Every path is absolute; only `stable_bin`
/// moves when the repository switches environments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    env: EnvKey,
    bin: PathBuf,
    stable_bin: PathBuf,
    vars: BTreeMap<String, OsString>,
    targets: Vec<String>,
}

impl Activation {
    pub fn env(&self) -> &EnvKey {
        &self.env
    }

    /// The environment's own immutable `bin`.
    pub fn bin(&self) -> &Path {
        &self.bin
    }

    /// The repository's link to its current environment's `bin`.
    pub fn stable_bin(&self) -> &Path {
        &self.stable_bin
    }

    /// Absolute runtime variables, including the repository's `CARGO_HOME`.
    /// They contain local paths: never hash them.
    pub fn vars(&self) -> &BTreeMap<String, OsString> {
        &self.vars
    }

    /// Each export's file name in `bin`; on Windows with its PATHEXT type.
    pub fn targets(&self) -> &[String] {
        &self.targets
    }

    /// `bin` in front of the inherited PATH, preserving its suffix.
    pub fn path(&self, inherited: Option<&OsStr>) -> Result<OsString, std::env::JoinPathsError> {
        let mut paths = vec![self.bin.clone()];
        if let Some(inherited) = inherited {
            paths.extend(std::env::split_paths(inherited));
        }
        std::env::join_paths(paths)
    }
}

impl UserStore {
    pub(crate) fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }

    fn tree_dir(&self, key: &str) -> PathBuf {
        self.root.join("trees").join(key)
    }

    fn tree_root(&self, tool: &Tool) -> PathBuf {
        self.tree_dir(&tree_key(tool)).join("root")
    }

    pub(crate) fn env_dir(&self, key: &EnvKey) -> PathBuf {
        self.root.join("envs").join(key.as_str())
    }

    /// Takes the shared collection lease and creates the store skeleton.
    /// Abandoned staging is collection's to remove: only it can prove that no
    /// setup is still using it.
    pub fn begin_setup(&self, repo: &Path, reuse: Reuse) -> Result<SetupSession<'_>, Error> {
        let repo = fs::canonicalize(repo)?;
        for dir in ["tmp", "trees", "envs", "repos"] {
            directory(&self.root.join(dir))?;
        }
        let lease = SharedLease::acquire(&self.root)?;
        Ok(SetupSession {
            store: self,
            lease,
            repo,
            reuse,
            verified: RefCell::default(),
        })
    }

    /// The recorded environment `env` for the canonical repository root
    /// `repo`. Reads only the environment's seal: never a tree, never a
    /// lock. The caller has already bound `repo` to `env` with its own
    /// evidence.
    pub fn activate(&self, repo: &Path, env: &EnvKey) -> Result<Activation, Error> {
        let dir = self.env_dir(env);
        let selection = self
            .env_seal(&dir, env)?
            .ok_or_else(|| Error::NotInstalled(format!("environment {env}")))?;
        // A missing shim must fail closed, never fall through to a host tool
        // later on PATH. Shims and seals only: O(exports), no tree contents.
        if !self.env_bin_matches(&selection, &dir.join("bin"))? {
            return Err(Error::Damaged(format!("environment {env}")));
        }
        for tool in selection.tools() {
            if self.tree_seal(tool)?.is_none() {
                return Err(Error::NotInstalled(format!("{} {}", tool.id, tool.version)));
            }
        }
        let mut vars = BTreeMap::new();
        let mut targets = Vec::new();
        for tool in selection.tools() {
            targets.extend(tool.bin_entries()?);
            for (var, value) in &tool.vars {
                let value = match value {
                    VarValue::Tree(reference) => self.tree_path(&selection, &dir, reference),
                    VarValue::Word(word) => word.into(),
                    VarValue::RepoCargoHome => {
                        let home = self.cargo_home(repo);
                        // Cargo creates a missing home on first use; a link or
                        // a file there would redirect a private one.
                        match fs::symlink_metadata(&home) {
                            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                                return Err(Error::UnsafePath);
                            }
                            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                                return Err(e.into());
                            }
                            _ => {}
                        }
                        home.into_os_string()
                    }
                };
                if vars
                    .insert(var.name().to_owned(), value.clone())
                    .is_some_and(|old| old != value)
                {
                    return Err(Error::InvalidInventory);
                }
            }
        }
        Ok(Activation {
            env: env.clone(),
            bin: dir.join("bin"),
            stable_bin: self.repo(repo).join("bin"),
            vars,
            targets,
        })
    }

    /// The root of the sealed tree setup would reuse for `tool`, from its
    /// seal alone, for reports that never install.
    pub fn sealed_tree(&self, tool: &Tool) -> Result<Option<PathBuf>, Error> {
        Ok(self.tree_seal(tool)?.map(|_| self.tree_root(tool)))
    }

    /// `turbo setup --check`: re-hash every tree `selection` uses against
    /// its seal, and compare its environment's `bin` with what setup renders.
    /// Takes no lock and writes nothing.
    pub fn check(&self, selection: &Selection) -> Result<(), Error> {
        for tool in selection.tools() {
            let label = || format!("{} {}", tool.id, tool.version);
            let seal = self
                .tree_seal(tool)?
                .ok_or_else(|| Error::NotInstalled(label()))?;
            if tree_hash(&self.tree_root(tool))? != seal.tree_sha256 {
                return Err(Error::Damaged(label()));
            }
        }
        let dir = self.env_dir(selection.key());
        let label = || format!("environment {}", selection.key());
        self.env_seal(&dir, selection.key())?
            .ok_or_else(|| Error::NotInstalled(label()))?;
        if !self.env_bin_matches(selection, &dir.join("bin"))? {
            return Err(Error::Damaged(label()));
        }
        Ok(())
    }

    /// A tree path as activation sets it. A variable may name a tool the
    /// selection lacks (uv without a managed Python): it resolves to a path
    /// in the environment that never exists, never to anything the user owns.
    fn tree_path(&self, selection: &Selection, env: &Path, reference: &TreeRef) -> OsString {
        let root = match selection.tool(&reference.tool) {
            Some(tool) => self.tree_root(tool),
            None => env.join("unselected").join(&reference.tool),
        };
        match reference.path.as_str() {
            "" => root,
            path => root.join(relative_path(path)),
        }
        .into_os_string()
    }

    /// The tree's seal when `trees/<key>` is a complete installed tree for
    /// exactly `tool`'s bytes. Reads no tree contents.
    fn tree_seal(&self, tool: &Tool) -> Result<Option<TreeSeal>, Error> {
        let dir = self.tree_dir(&tree_key(tool));
        let bytes = match read_bounded(&dir.join("seal.json")) {
            Ok(bytes) => bytes,
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(Error::UnsafePath) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Ok(seal) = serde_json::from_slice::<TreeSeal>(&bytes) else {
            return Ok(None);
        };
        if seal.schema != SEAL_SCHEMA || seal.spec != TreeSpec::of(tool) {
            return Ok(None);
        }
        match real_directory(&dir.join("root")) {
            Ok(()) => Ok(Some(seal)),
            Err(Error::UnsafePath) => Ok(None),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The selection an environment directory seals, when its seal names
    /// exactly `key` and `bin` is a real directory.
    pub(crate) fn env_seal(&self, dir: &Path, key: &EnvKey) -> Result<Option<Selection>, Error> {
        let bytes = match read_bounded(&dir.join("seal.json")) {
            Ok(bytes) => bytes,
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(Error::UnsafePath) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Ok(seal) = serde_json::from_slice::<EnvSeal>(&bytes) else {
            return Ok(None);
        };
        let Ok(selection) = Selection::new(seal.tools) else {
            return Ok(None);
        };
        if seal.schema != SEAL_SCHEMA || selection.key() != key {
            return Ok(None);
        }
        match real_directory(&dir.join("bin")) {
            Ok(()) => Ok(Some(selection)),
            Err(Error::UnsafePath) => Ok(None),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Exactly the rendered shims, and nothing else, in `bin`.
    fn env_bin_matches(&self, selection: &Selection, bin: &Path) -> Result<bool, Error> {
        let mut expected = BTreeSet::new();
        for tool in selection.tools() {
            for (name, export) in &tool.executables {
                let name = shim_name(name, export)?;
                match check_shim(&bin.join(&name), self.shim(selection, tool, export)?) {
                    Ok(true) => {}
                    Ok(false) => return Ok(false),
                    Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                    Err(e) => return Err(e),
                }
                expected.insert(name);
            }
        }
        let actual = fs::read_dir(bin)?
            .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<BTreeSet<_>, io::Error>>()?;
        Ok(actual == expected)
    }

    /// Exactly what an environment's `bin` entry for `export` must be.
    fn shim(&self, selection: &Selection, tool: &Tool, export: &Export) -> Result<Shim, Error> {
        #[cfg(windows)]
        {
            let tree = |id: &str| {
                let root = self.tree_root(selection.tool(id)?);
                let root = root.to_str()?;
                // Forwarders interpolate the root into a batch line.
                (!root.contains(['%', '!', '"'])).then(|| root.to_owned())
            };
            Ok(match export {
                Export::File(path) if crate::windows_kind(path)? == "exe" => {
                    Shim::HardLink(self.tree_root(tool).join(relative_path(path)))
                }
                Export::File(path) => Shim::Script(selection::windows_forwarder(
                    &crate::Launch {
                        program: TreeRef::new(tool.id.clone(), path.clone()),
                        args: Vec::new(),
                    },
                    tree,
                )?),
                Export::Launch(launch) => Shim::Script(selection::windows_forwarder(launch, tree)?),
            })
        }
        #[cfg(not(windows))]
        {
            // Relative to the store, so a store restored at another path
            // keeps working.
            let tree = |id: &str| match selection.tool(id) {
                Some(tool) => format!("trees/{}/root", tree_key(tool)),
                None => format!("envs/{}/unselected/{id}", selection.key()),
            };
            Ok(match export {
                Export::File(path) => Shim::Link(
                    Path::new("../../..")
                        .join(tree(&tool.id))
                        .join(relative_path(path)),
                ),
                Export::Launch(launch) => {
                    Shim::Script(selection::unix_launcher(tool, launch, tree)?)
                }
            })
        }
    }
}

impl SetupSession<'_> {
    pub fn store(&self) -> &UserStore {
        self.store
    }

    /// The installed tree setup reuses for `tool`, for adapters that read
    /// installed resources. `None` means setup must stage it.
    pub fn tree(&self, tool: &Tool) -> Result<Option<PathBuf>, Error> {
        let Some(seal) = self.store.tree_seal(tool)? else {
            return Ok(None);
        };
        let root = self.store.tree_root(tool);
        if self.reuse == Reuse::Verified {
            let key = tree_key(tool);
            let healthy = match self.verified.borrow().get(&key) {
                Some(healthy) => *healthy,
                None => match tree_hash(&root) {
                    Ok(hash) => hash == seal.tree_sha256,
                    Err(Error::UnsafePath) => false,
                    Err(e) => return Err(e),
                },
            };
            self.verified.borrow_mut().insert(key, healthy);
            if !healthy {
                return Ok(None);
            }
        }
        Ok(Some(root))
    }

    /// [`SetupSession::tree`] for one member of the complete selection
    /// `desired`, which is validated first, so a cohort setup would refuse
    /// never downloads anything.
    pub fn reusable_tree(&self, tool: &Tool, desired: &[Tool]) -> Result<Option<PathBuf>, Error> {
        validate_tools(desired)?;
        if !desired.contains(tool) {
            return Err(Error::InvalidInventory);
        }
        self.tree(tool)
    }

    /// A private directory on the store's filesystem, for adapters that stage
    /// by renaming.
    pub fn scratch(&self, prefix: &str) -> Result<tempfile::TempDir, Error> {
        staging(&self.store.tmp(), prefix)
    }

    /// Converges this repository onto `selection`: stages each tree the store
    /// lacks through `stage`, which writes one tool's verified bytes into an
    /// empty private directory, builds the environment if it is missing, calls
    /// `record` with the environment's key so the caller can bind the
    /// repository to it, then points the repository's stable links at it and
    /// removes the repository's pre-store `.turbo/tools`. Running it again
    /// stages and builds nothing. A crash at any point leaves complete
    /// entries or private staging that a later setup removes.
    pub fn ensure(
        &self,
        selection: &Selection,
        mut stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        record: impl FnOnce(&EnvKey) -> Result<(), Error>,
    ) -> Result<Ensured, Error> {
        let mut installed = BTreeSet::new();
        for tool in selection.tools() {
            if self.tree(tool)?.is_none() {
                self.install_tree(tool, &mut stage)?;
                installed.insert(tool.id.clone());
            }
        }
        let dir = self.store.env_dir(selection.key());
        // Shims are checked every time, as activation checks them: a damaged
        // env that run/exec reject must be one plain setup repairs.
        let usable = self.store.env_seal(&dir, selection.key())?.is_some()
            && self.store.env_bin_matches(selection, &dir.join("bin"))?;
        if !usable {
            self.install_env(selection, &dir)?;
        }
        record(selection.key())?;
        self.link(selection.key())?;
        // Pre-store generations: never read, only removed. Best effort, like
        // any other removal a running process may still hold open.
        let old = self.repo.join(".turbo").join("tools");
        if fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir()) {
            let _ = fs::remove_dir_all(&old);
        }
        Ok(Ensured {
            env: selection.key().clone(),
            installed,
        })
    }

    fn install_tree(
        &self,
        tool: &Tool,
        stage: &mut impl FnMut(&Tool, &Path) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let target = self.store.tree_dir(&tree_key(tool));
        // Setup stages only what it found unusable.
        let replace = fs::symlink_metadata(&target).is_ok();
        let staged = staging(&self.store.tmp(), "tree-")?;
        let root = staged.path().join("root");
        directory(&root)?;
        stage(tool, &root)?;
        make_read_only(&root)?;
        let seal = TreeSeal {
            schema: SEAL_SCHEMA,
            spec: TreeSpec::of(tool),
            tree_sha256: tree_hash(&root)?,
        };
        write_private(
            &staged.path().join("seal.json"),
            &serde_json::to_vec(&seal)?,
        )?;
        publish(
            &self.lease,
            staged,
            &target,
            &self.store.tmp(),
            replace,
            || Ok(self.store.tree_seal(tool)?.is_some()),
        )?;
        self.verified.borrow_mut().insert(tree_key(tool), true);
        Ok(())
    }

    fn install_env(&self, selection: &Selection, target: &Path) -> Result<(), Error> {
        let replace = fs::symlink_metadata(target).is_ok();
        let staged = staging(&self.store.tmp(), "env-")?;
        let bin = staged.path().join("bin");
        directory(&bin)?;
        for tool in selection.tools() {
            crate::check_executables(tool, |id| {
                selection.tool(id).map(|tool| self.store.tree_root(tool))
            })?;
            for (name, export) in &tool.executables {
                make_shim(
                    &bin.join(shim_name(name, export)?),
                    self.store.shim(selection, tool, export)?,
                )?;
            }
        }
        let seal = EnvSeal {
            schema: SEAL_SCHEMA,
            tools: selection.tools().to_vec(),
        };
        write_private(
            &staged.path().join("seal.json"),
            &serde_json::to_vec(&seal)?,
        )?;
        publish(
            &self.lease,
            staged,
            target,
            &self.store.tmp(),
            replace,
            || {
                Ok(self.store.env_seal(target, selection.key())?.is_some()
                    && self.store.env_bin_matches(selection, &target.join("bin"))?)
            },
        )
    }

    /// `<store>/repos/<repo>/bin` names the environment, and `.turbo/bin` in
    /// the repository names that link, for editors. Turbo never reads either.
    fn link(&self, env: &EnvKey) -> Result<(), Error> {
        let state = self.store.repo(&self.repo);
        directory(&state)?;
        let stable = state.join("bin");
        let linked = replace_link(
            &stable,
            &Path::new("..")
                .join("..")
                .join("envs")
                .join(env.as_str())
                .join("bin"),
            &self.store.env_dir(env).join("bin"),
        )
        .and_then(|()| {
            let turbo = self.repo.join(".turbo");
            create_directory(&turbo, shared_directory)?;
            let editor = turbo.join("bin");
            // Never replace a directory or file someone else put there.
            match fs::symlink_metadata(&editor) {
                Ok(m) if !m.file_type().is_symlink() => Ok(()),
                _ => replace_link(&editor, &stable, &stable),
            }
        });
        // Windows links need a privilege many users lack; activation and
        // `turbo exec --print-env` work without them.
        #[cfg(windows)]
        let linked = linked.or(Ok(()));
        linked
    }
}

/// Atomically points `link` at `target` (a relative Unix target, or the
/// absolute `directory` on Windows).
fn replace_link(link: &Path, target: &Path, directory: &Path) -> Result<(), Error> {
    #[cfg(not(windows))]
    let (target, _) = (target, directory);
    #[cfg(windows)]
    let (target, _) = (directory, target);
    if fs::read_link(link).is_ok_and(|current| current == target) {
        return Ok(());
    }
    let parent = link.parent().ok_or(Error::UnsafePath)?;
    let staged = tempfile::Builder::new()
        .prefix(".link-")
        .make_in(parent, |path| {
            #[cfg(unix)]
            return std::os::unix::fs::symlink(target, path);
            #[cfg(windows)]
            return std::os::windows::fs::symlink_dir(target, path);
            #[cfg(not(any(unix, windows)))]
            return Err(io::Error::from(io::ErrorKind::Unsupported));
        })?;
    // A Windows directory link cannot be renamed over; remove it first.
    #[cfg(windows)]
    if fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink()) {
        fs::remove_dir(link)?;
    }
    staged.persist(link).map_err(|e| Error::Io(e.error))?;
    sync_directory(parent)
}

/// Moves complete `staged` contents to `target` with a rename that never
/// replaces a published entry. `replace` moves aside an entry setup already
/// judged unusable. A target that appears meanwhile is another setup winning
/// a race; `published` accepts it when it is valid.
fn publish(
    _lease: &SharedLease,
    staged: tempfile::TempDir,
    target: &Path,
    tmp: &Path,
    replace: bool,
    published: impl Fn() -> Result<bool, Error>,
) -> Result<(), Error> {
    sync_tree(staged.path())?;
    if replace && fs::symlink_metadata(target).is_ok() {
        let trash = staging(tmp, "trash-")?.keep();
        fs::rename(target, trash.join("entry"))?;
        let _ = fs::remove_dir_all(&trash);
    }
    match fs::rename(staged.path(), target) {
        Ok(()) => {
            // The renamed directory is no longer the TempDir's to remove.
            let _ = staged.keep();
            sync_directory(target.parent().ok_or(Error::UnsafePath)?)
        }
        Err(_) if published()? => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn staging(tmp: &Path, prefix: &str) -> Result<tempfile::TempDir, Error> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir_in(tmp)?)
}

/// Exactly what an environment's `bin` entry must be.
enum Shim {
    /// A relative symlink with this target.
    #[cfg(not(windows))]
    Link(PathBuf),
    /// A hard link to this tree file, on the store's volume. The image path
    /// becomes `bin\name.exe`, so `.exe` exports must not load adjacent
    /// resources.
    #[cfg(windows)]
    HardLink(PathBuf),
    /// A generated script with these bytes.
    Script(Vec<u8>),
}

fn make_shim(path: &Path, shim: Shim) -> Result<(), Error> {
    match shim {
        #[cfg(not(windows))]
        Shim::Link(target) => {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, path)?;
            #[cfg(not(unix))]
            let _ = (target, path);
            Ok(())
        }
        #[cfg(windows)]
        Shim::HardLink(target) => Ok(fs::hard_link(target, path)?),
        Shim::Script(bytes) => {
            use std::io::Write;
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

fn check_shim(path: &Path, shim: Shim) -> Result<bool, Error> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let regular = metadata.is_file()
        && std::os::windows::fs::MetadataExt::file_attributes(&metadata) & 0x400 == 0;
    #[cfg(not(windows))]
    let regular = metadata.is_file();
    Ok(match shim {
        #[cfg(not(windows))]
        Shim::Link(target) => metadata.file_type().is_symlink() && fs::read_link(path)? == target,
        #[cfg(windows)]
        Shim::HardLink(target) => regular && same_file::is_same_file(path, target)?,
        Shim::Script(bytes) => {
            regular && fs::read(path)? == bytes && {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o100 != 0
                }
                #[cfg(not(unix))]
                true
            }
        }
    })
}
