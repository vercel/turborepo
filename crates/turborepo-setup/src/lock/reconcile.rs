//! Bounded native JS transaction precursor, not an adapter resolver or CLI.
//! The injected resolver MUST validate native version/integrity constraints and
//! return a complete collision-checked cohort. In particular, a declared npm
//! needs an npm Tool even when its version matches Node's bundled npm. This
//! coordinator validates provenance and preserves unaffected exact selections;
//! it does not fetch metadata, install, or infer bundled npm release
//! identities.

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
    #[error("native declarations changed for {0:?}; run setup locally and commit turbo.lock")]
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
    version_ids: BTreeSet<String>,
    ownership_ids: BTreeSet<String>,
    offline: bool,
}
impl Resolution<'_> {
    pub fn snapshot(&self) -> &Snapshot {
        self.snapshot
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
        Installation::VerifySystem { executables } => {
            executables.retain(|name| name != "npm" && name != "npx");
        }
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
    snapshot.check_native_coverage(None)?;
    snapshot.ensure_current()?;
    let previous = snapshot.previous_lock();
    let current = snapshot.declarations();
    let mut changed = BTreeSet::new();
    for id in current
        .keys()
        .chain(previous.into_iter().flat_map(|lock| lock.tools().keys()))
    {
        let old = previous.and_then(|lock| lock.tools().get(id));
        // Provenance is a set, not declaration precedence. A noncanonical
        // existing lock must not cause a floating version to refresh.
        let same = match (old, current.get(id)) {
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
    if !changed.is_empty() && (mode == Mode::Frozen || (mode == Mode::NoLock && previous.is_some()))
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
    let candidate = if version_ids.is_empty() && ownership_ids.is_empty() {
        let tools = previous
            .into_iter()
            .flat_map(|lock| lock.tools())
            .filter(|(id, _)| current.contains_key(*id))
            .map(|(id, tool)| (id.clone(), tool.clone()))
            .collect();
        Lock::new(Document {
            schema_version: SCHEMA_VERSION,
            tools,
        })?
    } else {
        resolve(Resolution {
            snapshot,
            version_ids: version_ids.clone(),
            ownership_ids: ownership_ids.clone(),
            offline,
        })?
    };
    snapshot.check_native_coverage(Some(&candidate))?;
    if !candidate.matches_native(current)? {
        return Err(
            super::Error::Invalid("candidate provenance does not match captured sources").into(),
        );
    }
    if let Some(previous) = previous {
        for (id, tool) in candidate.tools() {
            if version_ids.contains(id) {
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
    let publication = match mode {
        Mode::Local | Mode::Refresh => Some(snapshot.commit(&candidate)?),
        Mode::Frozen | Mode::NoLock => {
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
