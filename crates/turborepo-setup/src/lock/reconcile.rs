//! Bounded native JS transaction precursor, not an adapter resolver or CLI.
//! The injected resolver MUST validate native version/integrity constraints and
//! return a complete collision-checked cohort. In particular, a declared npm
//! needs an npm Tool even when its version matches Node's bundled npm. This
//! coordinator validates provenance and preserves unaffected exact selections;
//! it does not fetch metadata, install, or infer bundled npm release
//! identities. Declared additional tools are exact, so they are always the
//! snapshot's projection and never reach the resolver.

use std::collections::BTreeSet;

use super::{
    Document, Installation, Lock, SCHEMA_VERSION, Snapshot, StorageError, Tool, WriteOutcome,
};

/// Mutually exclusive resolution/publication modes. Reinstallation/force is
/// intentionally absent: it never authorizes version resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Local,
    Refresh,
    Frozen,
    NoLock,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Lock(#[from] super::Error),
    #[error("frozen setup requires turbo.lock; run setup locally and commit it")]
    Missing,
    #[error("tool declarations changed for {0:?}; run `turbo setup` locally and commit turbo.lock")]
    Drift(BTreeSet<String>),
    #[error("resolver changed unaffected locked selection {0}")]
    Unaffected(String),
    #[error("native resolution failed: {0}")]
    Resolution(String),
}

/// Only the coordinator constructs this bounded request. No caller-authored
/// declaration maps; native requirements and previous exact pins come from the
/// immutable Snapshot. `version_ids` permits resolution for changed/refreshed
/// declarations only. `ownership_ids` permits ONLY npm/npx exports and the
/// bundled-npm option to change on an otherwise byte-identical Node selection.
/// Offline resolvers must use approved cached metadata or return an error.
pub struct Resolution<'a> {
    snapshot: &'a Snapshot,
    previous: Option<&'a Lock>,
    version_ids: BTreeSet<String>,
    ownership_ids: BTreeSet<String>,
    offline: bool,
    refresh: bool,
}
impl Resolution<'_> {
    pub fn snapshot(&self) -> &Snapshot {
        self.snapshot
    }
    /// Semantic previous selection; never changes Snapshot's real disk CAS.
    pub fn previous_selection(&self) -> Option<&Lock> {
        self.previous
    }
    pub fn version_ids(&self) -> &BTreeSet<String> {
        &self.version_ids
    }
    pub fn ownership_ids(&self) -> &BTreeSet<String> {
        &self.ownership_ids
    }
    pub fn offline(&self) -> bool {
        self.offline
    }
    /// Explicit `--update-lock`: unchanged floating requests may move too.
    pub fn refresh(&self) -> bool {
        self.refresh
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub lock: Lock,
    /// None is read-only/ephemeral, not a committed lock write.
    pub publication: Option<WriteOutcome>,
}

fn without_npm_exports(tool: &Tool) -> Tool {
    let mut tool = tool.clone();
    tool.options.remove("bundled-npm");
    match &mut tool.installation {
        Installation::Managed { artifacts } => {
            for parts in artifacts.values_mut() {
                for artifact in parts.values_mut() {
                    artifact.executables.remove("npm");
                    artifact.executables.remove("npx");
                }
            }
        }
        Installation::Bundled { .. } => {}
    }
    tool
}

/// Zero or one bounded callback, then source/lock CAS; never implicit retries.
/// Removal-only changes need no version resolution. Manager cohort changes may
/// still require Node ownership adjustment WITHOUT refreshing its release or
/// artifact bytes. No-lock rejects drift in an existing lock and never writes.
pub fn reconcile(
    snapshot: &Snapshot,
    mode: Mode,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, Error>,
) -> Result<Outcome, Error> {
    reconcile_checked(snapshot, mode, offline, resolve, |_| Ok(()), || Ok(()))
}

/// Consumer preconditions supplement, never replace, immutable Snapshot CAS.
/// Candidate validation is read-only and runs even for unchanged selections,
/// before any writer. Checks run before resolution, after waits and staging.
pub fn reconcile_checked(
    snapshot: &Snapshot,
    mode: Mode,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, Error>,
    validate: impl FnOnce(&Lock) -> std::io::Result<()>,
    check: impl FnMut() -> std::io::Result<()>,
) -> Result<Outcome, Error> {
    coordinate(
        snapshot,
        snapshot.previous_lock(),
        mode,
        offline,
        resolve,
        validate,
        check,
        true,
    )
}

/// Read-only `--plan`: the same validated candidate `reconcile_checked` would
/// select, but Local/Refresh never take a writer, stage or publish. Callers
/// report `previous_lock` → `lock` as the proposed change; `publication` is
/// always None.
pub fn propose(
    snapshot: &Snapshot,
    mode: Mode,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, Error>,
    validate: impl FnOnce(&Lock) -> std::io::Result<()>,
    check: impl FnMut() -> std::io::Result<()>,
) -> Result<Outcome, Error> {
    coordinate(
        snapshot,
        snapshot.previous_lock(),
        mode,
        offline,
        resolve,
        validate,
        check,
        false,
    )
}

// Only the sealed baseline consumer can supply a nondisk previous selection.
pub(crate) fn reconcile_previous(
    snapshot: &Snapshot,
    previous: Option<&Lock>,
    mode: Mode,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, Error>,
    validate: impl FnOnce(&Lock) -> std::io::Result<()>,
    check: impl FnMut() -> std::io::Result<()>,
) -> Result<Outcome, Error> {
    coordinate(
        snapshot, previous, mode, offline, resolve, validate, check, true,
    )
}

#[allow(clippy::too_many_arguments)]
fn coordinate(
    snapshot: &Snapshot,
    previous: Option<&Lock>,
    mode: Mode,
    offline: bool,
    resolve: impl FnOnce(Resolution<'_>) -> Result<Lock, Error>,
    validate: impl FnOnce(&Lock) -> std::io::Result<()>,
    mut check: impl FnMut() -> std::io::Result<()>,
    publish: bool,
) -> Result<Outcome, Error> {
    check().map_err(StorageError::from)?;
    snapshot.check_native_coverage(None)?;
    snapshot.ensure_current()?;
    let current = snapshot.declarations();
    let additional = snapshot.additional_tools();
    let mut changed = BTreeSet::new();
    for id in current
        .keys()
        .chain(additional.keys())
        .chain(previous.into_iter().flat_map(|lock| lock.tools().keys()))
    {
        let old = previous.and_then(|lock| lock.tools().get(id));
        // Provenance is a set, not declaration precedence. A noncanonical
        // existing lock must not cause a floating version to refresh.
        // Additional tools record their whole declaration, so any edit to a
        // source, digest, layout, or platform list is drift.
        let same = match (old, current.get(id)) {
            _ if additional.contains_key(id) => old == additional.get(id),
            (Some(tool), Some(values)) => {
                tool.declarations.len() == values.len()
                    && tool.declarations.iter().all(|value| values.contains(value))
            }
            _ => false,
        };
        if !same {
            changed.insert(id.clone());
        }
    }
    if mode == Mode::Frozen && previous.is_none() {
        return Err(Error::Missing);
    }
    if !changed.is_empty()
        && (mode == Mode::Frozen || (mode == Mode::NoLock && snapshot.previous_lock().is_some()))
    {
        return Err(Error::Drift(changed));
    }
    let version_ids: BTreeSet<_> = current
        .keys()
        .filter(|id| mode == Mode::Refresh || changed.contains(*id))
        .cloned()
        .collect();
    let ownership_ids = if current.contains_key("node")
        && !version_ids.contains("node")
        && ["npm", "pnpm"]
            .iter()
            .any(|id| changed.contains(*id) || version_ids.contains(*id))
    {
        BTreeSet::from(["node".into()])
    } else {
        BTreeSet::new()
    };
    let mut tools = if version_ids.is_empty() && ownership_ids.is_empty() {
        previous
            .into_iter()
            .flat_map(|lock| lock.tools())
            .filter(|(id, _)| current.contains_key(*id))
            .map(|(id, tool)| (id.clone(), tool.clone()))
            .collect()
    } else {
        resolve(Resolution {
            snapshot,
            previous,
            version_ids: version_ids.clone(),
            ownership_ids: ownership_ids.clone(),
            offline,
            refresh: mode == Mode::Refresh,
        })?
        .document()
        .tools
        .clone()
    };
    tools.extend(additional.clone());
    // Revalidates the merged cohort, including executable collisions.
    let candidate = Lock::new(Document {
        schema_version: SCHEMA_VERSION,
        tools,
    })?;
    snapshot.check_native_coverage(Some(&candidate))?;
    if !candidate.matches_native(current)? || !snapshot.matches_additional(&candidate) {
        return Err(
            super::Error::Invalid("candidate provenance does not match captured sources").into(),
        );
    }
    if let Some(previous) = previous {
        for (id, tool) in candidate.tools() {
            if version_ids.contains(id) || additional.contains_key(id) {
                continue;
            }
            let old = &previous.tools()[id];
            let preserved = if ownership_ids.contains(id) {
                without_npm_exports(tool) == without_npm_exports(old)
            } else {
                tool == old
            };
            if !preserved {
                return Err(Error::Unaffected(id.clone()));
            }
        }
    }
    // Every complete candidate, including unchanged/removal-only selections,
    // must pass the consumer's host/plan validation BEFORE any writer exists.
    validate(&candidate).map_err(StorageError::from)?;
    let publication = match mode {
        Mode::Local | Mode::Refresh if publish => Some(snapshot.commit_checked(&candidate, check)?),
        Mode::Local | Mode::Refresh | Mode::Frozen | Mode::NoLock => {
            check().map_err(StorageError::from)?;
            snapshot.ensure_current()?;
            None
        }
    };
    Ok(Outcome {
        lock: candidate,
        publication,
    })
}

#[cfg(test)]
mod tests;
