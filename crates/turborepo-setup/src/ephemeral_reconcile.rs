//! Missing-real-lock library transaction. No CLI admission or execution
//! authority.
use std::{io, path::Path};

use turborepo_tool_install::{GenerationExpectation, Store, Tool};

use crate::{
    lock::{
        Lock, Platform, Snapshot,
        reconcile::{self, Mode, Resolution},
    },
    native_baseline::{Baseline, NativeRecord},
    node_provision::NodePlan,
    pnpm_provision::PnpmPlan,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ephemeral reconciliation requires a missing real lock and native Node cohort")]
    Unsupported,
    #[error(transparent)]
    Native(#[from] crate::native_baseline::Error),
    #[error(transparent)]
    Reconcile(#[from] reconcile::Error),
    #[error(transparent)]
    Storage(#[from] crate::lock::StorageError),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error(transparent)]
    Node(#[from] crate::node_provision::Error),
    #[error(transparent)]
    Pnpm(#[from] crate::pnpm_provision::Error),
    #[error(transparent)]
    Check(#[from] io::Error),
}

enum Generation {
    Baseline(Box<Baseline>),
    Empty(GenerationExpectation),
}
impl Generation {
    fn expectation(&self) -> &GenerationExpectation {
        match self {
            Self::Baseline(baseline) => baseline.generation(),
            Self::Empty(expected) => expected,
        }
    }
}

/// Private fields retain actual disk CAS and live Store evidence, not a fake
/// lock.
pub struct Staged {
    snapshot: Snapshot,
    generation: Generation,
    native: NativeRecord,
    node: NodePlan,
    pnpm: Option<PnpmPlan>,
    desired: Vec<Tool>,
}

pub fn stage(
    snapshot: &Snapshot,
    store: &Store,
    platform: Platform,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, reconcile::Error>,
    mut check: impl FnMut() -> io::Result<()>,
) -> Result<Staged, Error> {
    check()?;
    NativeRecord::preflight(snapshot)?;
    if snapshot.previous_lock().is_some()
        || !snapshot.declarations().contains_key("node")
        || snapshot.repository_root() != store.repository_root()?
        || !matches!(
            platform,
            Platform::MacosX64
                | Platform::MacosArm64
                | Platform::LinuxX64Gnu
                | Platform::LinuxArm64Gnu
        )
    {
        return Err(Error::Unsupported);
    }
    let generation = match Baseline::capture(snapshot, store)? {
        Some(baseline) => Generation::Baseline(Box::new(baseline)),
        None => {
            let expected = store.generation()?;
            // A present generation requires sealed native provenance, even when healthy.
            if expected.current().is_some() {
                return Err(Error::Unsupported);
            }
            Generation::Empty(expected)
        }
    };
    let previous = match &generation {
        Generation::Baseline(baseline) => Some(baseline.native(snapshot, store)?),
        Generation::Empty(_) => None,
    };
    let candidate = reconcile::reconcile_previous(
        snapshot,
        previous.map(NativeRecord::selection),
        Mode::NoLock,
        offline,
        resolve,
        |lock| {
            NativeRecord::from_snapshot(snapshot, lock)
                .map(|_| ())
                .map_err(io::Error::other)
        },
        || {
            check()?;
            snapshot.ensure_current().map_err(io::Error::other)?;
            store
                .check_generation(generation.expectation())
                .map_err(io::Error::other)
        },
    )?;
    let native = NativeRecord::from_snapshot(snapshot, &candidate.lock)?;
    let node = NodePlan::from_lock(native.selection(), platform)?;
    let pnpm = native
        .package_manager()
        .map(|manager| PnpmPlan::from_declaration(native.selection(), platform, &node, manager))
        .transpose()?;
    let mut desired = vec![node.inventory_tool().clone()];
    if let Some(pnpm) = &pnpm {
        desired.push(pnpm.inventory_tool().clone());
    }
    let staged = Staged {
        snapshot: snapshot.clone(),
        generation,
        native,
        node,
        pnpm,
        desired,
    };
    staged.check(snapshot, store)?;
    Ok(staged)
}

impl Staged {
    pub fn selection(&self) -> &Lock {
        self.native.selection()
    }
    pub fn desired(&self) -> &[Tool] {
        &self.desired
    }
    pub fn node_plan(&self) -> &NodePlan {
        &self.node
    }
    pub fn pnpm_plan(&self) -> Option<&PnpmPlan> {
        self.pnpm.as_ref()
    }

    /// Recheck after caller-owned waits/preparation, never recapture source
    /// CAS.
    pub fn check(&self, snapshot: &Snapshot, store: &Store) -> Result<(), Error> {
        if self.snapshot.repository_root() != snapshot.repository_root()
            || snapshot.repository_root() != store.repository_root()?
        {
            return Err(Error::Unsupported);
        }
        self.snapshot.ensure_current()?;
        snapshot.ensure_current()?;
        store.check_generation(self.generation.expectation())?;
        Ok(())
    }

    pub fn publish(
        self,
        snapshot: &Snapshot,
        store: &mut Store,
        force: bool,
        stage_tool: impl FnMut(&Tool, &Path) -> Result<(), turborepo_tool_install::Error>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> Result<turborepo_tool_install::Outcome, Error> {
        check()?;
        self.check(snapshot, store)?;
        let guard = self.snapshot.guard()?;
        self.publish_guarded(snapshot, store, &guard, force, stage_tool, check)
    }

    /// Caller acquires WRITER before Store and retains both through
    /// publication. Never reacquire the writer while a caller-held guard is
    /// live.
    pub fn publish_guarded(
        self,
        snapshot: &Snapshot,
        store: &mut Store,
        guard: &crate::writer_storage::WriterStorage,
        force: bool,
        mut stage_tool: impl FnMut(&Tool, &Path) -> Result<(), turborepo_tool_install::Error>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> Result<turborepo_tool_install::Outcome, Error> {
        check()?;
        self.snapshot.check_guard(guard)?;
        self.check(snapshot, store)?;
        Ok(store.reconcile_recorded_checked(
            &self.desired,
            self.native.record(),
            self.generation.expectation(),
            force,
            |tool, tree| {
                stage_tool(tool, tree)?;
                if tool.id == "node" {
                    self.node
                        .verify_bundled_npm(tree)
                        .map_err(io::Error::other)?;
                } else {
                    verify_pnpm(tree, tool).map_err(io::Error::other)?;
                }
                Ok(())
            },
            || {
                check()?;
                self.snapshot.check_guard(guard).map_err(io::Error::other)?;
                snapshot.ensure_current().map_err(io::Error::other)?;
                Ok(())
            },
        )?)
    }
}

fn verify_pnpm(tree: &Path, tool: &Tool) -> io::Result<()> {
    use std::io::Read;
    let path = tree.join("package.json");
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("invalid pnpm package"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(crate::registry_metadata::MAX_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let crate::node_discovery::UniqueJson(package) = serde_json::from_slice(&bytes)?;
    if bytes.len() > crate::registry_metadata::MAX_METADATA_BYTES
        || package.get("name").and_then(|v| v.as_str()) != Some("pnpm")
        || package.get("version").and_then(|v| v.as_str()) != Some(tool.version.as_str())
        || package
            .get("bin")
            .and_then(|v| v.as_object())
            .is_none_or(|bin| {
                bin.get("pnpm").and_then(|v| v.as_str()) != Some("bin/pnpm.cjs")
                    || tool.executables.keys().any(|name| !bin.contains_key(name))
                    || bin.iter().any(|(name, path)| match name.as_str() {
                        "pnpm" => path.as_str() != Some("bin/pnpm.cjs"),
                        "pnpx" => path.as_str() != Some("bin/pnpx.cjs"),
                        _ => true,
                    })
            })
    {
        return Err(io::Error::other("invalid pnpm package"));
    }
    Ok(())
}
