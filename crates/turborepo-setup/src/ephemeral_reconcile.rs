//! Missing-real-lock library transaction. No CLI admission or execution
//! authority. The previous clone-local selection comes from the caller's
//! user record; publication is the caller's store session.
use std::{io, path::Path};

use turborepo_tool_install::Tool;

use crate::{
    lock::{
        Lock, Platform, Snapshot,
        reconcile::{self, Mode, Resolution},
    },
    native_baseline::NativeRecord,
    node_provision::NodePlan,
    npm_provision::NpmPlan,
    package_manager::Manager,
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
    Node(#[from] crate::node_provision::Error),
    #[error(transparent)]
    Pnpm(#[from] crate::pnpm_provision::Error),
    #[error(transparent)]
    Check(#[from] io::Error),
}

pub struct Staged {
    snapshot: Snapshot,
    native: NativeRecord,
    node: NodePlan,
    pnpm: Option<PnpmPlan>,
    /// Only an independently provisioned npm; Node-bundled npm has no tool.
    npm: Option<NpmPlan>,
    desired: Vec<Tool>,
}

/// `previous` is the clone-local selection the repository's user record
/// holds, if any. An invalid one is never silently replaced.
pub fn stage(
    snapshot: &Snapshot,
    previous: Option<&[u8]>,
    platform: Platform,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, reconcile::Error>,
    mut check: impl FnMut() -> io::Result<()>,
) -> Result<Staged, Error> {
    check()?;
    NativeRecord::preflight(snapshot)?;
    if snapshot.previous_lock().is_some()
        || !snapshot.declarations().contains_key("node")
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
    let previous = previous.map(NativeRecord::decode).transpose()?;
    let candidate = reconcile::reconcile_previous(
        snapshot,
        previous.as_ref().map(NativeRecord::selection),
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
            snapshot.ensure_current().map_err(io::Error::other)
        },
    )?;
    let native = NativeRecord::from_snapshot(snapshot, &candidate.lock)?;
    let node = NodePlan::from_lock(native.selection(), platform)?;
    let (mut pnpm, mut npm) = (None, None);
    match native.package_manager() {
        Some(manager) if manager.manager == Manager::Npm => {
            npm = NpmPlan::from_declaration(native.selection(), platform, &node, manager)?;
        }
        Some(manager) => {
            pnpm = Some(PnpmPlan::from_declaration(
                native.selection(),
                platform,
                &node,
                manager,
            )?);
        }
        None => {}
    }
    let mut desired = vec![node.inventory_tool().clone()];
    desired.extend(npm.iter().map(NpmPlan::inventory_tool).cloned());
    desired.extend(pnpm.iter().map(PnpmPlan::inventory_tool).cloned());
    let staged = Staged {
        snapshot: snapshot.clone(),
        native,
        node,
        pnpm,
        npm,
        desired,
    };
    staged.check(snapshot)?;
    Ok(staged)
}

impl Staged {
    pub fn selection(&self) -> &Lock {
        self.native.selection()
    }
    /// The canonical selection bytes the user record keeps.
    pub fn selection_bytes(&self) -> &[u8] {
        self.native.bytes()
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
    pub fn npm_plan(&self) -> Option<&NpmPlan> {
        self.npm.as_ref()
    }

    /// Recheck after caller-owned waits/preparation, never recapture source
    /// CAS.
    pub fn check(&self, snapshot: &Snapshot) -> Result<(), Error> {
        if self.snapshot.repository_root() != snapshot.repository_root() {
            return Err(Error::Unsupported);
        }
        self.snapshot.ensure_current()?;
        snapshot.ensure_current()?;
        Ok(())
    }

    /// Checks a freshly staged tree of this selection: the exact bundled npm
    /// for Node, the exact manager package otherwise.
    pub fn verify(&self, tool: &Tool, tree: &Path) -> io::Result<()> {
        if tool.id == "node" {
            self.node.verify_bundled_npm(tree).map_err(io::Error::other)
        } else {
            verify_manager(tree, tool)
        }
    }
}

/// The installed pnpm or npm package is exactly the selected manager with its
/// official executable layout, as setup staged it.
pub(crate) fn verify_manager(tree: &Path, tool: &Tool) -> io::Result<()> {
    use std::io::Read;
    let bins: &[(&str, &str)] = match tool.id.as_str() {
        "pnpm" => &[("pnpm", "bin/pnpm.cjs"), ("pnpx", "bin/pnpx.cjs")],
        "npm" => &[("npm", "bin/npm-cli.js"), ("npx", "bin/npx-cli.js")],
        _ => return Err(io::Error::other("unsupported package manager")),
    };
    let invalid = || io::Error::other(format!("invalid {} package", tool.id));
    let path = tree.join("package.json");
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(crate::registry_metadata::MAX_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let crate::node_discovery::UniqueJson(package) = serde_json::from_slice(&bytes)?;
    if bytes.len() > crate::registry_metadata::MAX_METADATA_BYTES
        || package.get("name").and_then(|v| v.as_str()) != Some(tool.id.as_str())
        || package.get("version").and_then(|v| v.as_str()) != Some(tool.version.as_str())
        || package
            .get("bin")
            .and_then(|v| v.as_object())
            .is_none_or(|bin| {
                bin.get(bins[0].0).and_then(|v| v.as_str()) != Some(bins[0].1)
                    || tool.executables.keys().any(|name| !bin.contains_key(name))
                    || bin.iter().any(|(name, path)| {
                        !bins.iter().any(|&(known, expected)| {
                            known == name.as_str() && path.as_str() == Some(expected)
                        })
                    })
            })
    {
        return Err(invalid());
    }
    Ok(())
}
