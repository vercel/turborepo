//! Captured native Node + exact pnpm → portable cohort. Only reconcile's
//! permitted IDs resolve; unaffected selections (including Node exports) retain
//! their exact bytes. No publication, installation, probes or task execution.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    lock::{self, Document, Installation, Lock, Platform, Tool, reconcile::Resolution},
    node_provision::{NodePlan, NodeTransport},
    package_manager::{self, Declaration, Manager},
    registry_resolution::RegistryTransport,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Node(#[from] crate::node_provision::Error),
    #[error(transparent)]
    Registry(#[from] crate::registry_resolution::Error),
    #[error(transparent)]
    Native(#[from] package_manager::Error),
    #[error("unsupported native JS resolution: {0}")]
    Unsupported(&'static str),
}

// Without an authoritative pin, only one unambiguous exact identity is in
// scope. Never choose the first dev alternative or query a floating index.
fn exact_pnpm(declaration: &Declaration) -> Result<String, Error> {
    let requests: Vec<_> = declaration
        .package_manager
        .iter()
        .chain(&declaration.dev_engines)
        .collect();
    if requests.iter().any(|request| {
        request
            .integrity
            .as_ref()
            .is_some_and(|pin| !matches!(pin.algorithm, "sha256" | "sha512"))
    }) {
        return Err(Error::Unsupported("authored integrity algorithm"));
    }
    fn exact(request: &package_manager::Request) -> Option<&semver::Version> {
        request
            .request
            .as_ref()
            .and_then(crate::VersionRequest::exact_version)
    }
    let version = if let Some(pin) = &declaration.package_manager {
        exact(pin)
            .ok_or(Error::Unsupported("floating pnpm request"))?
            .clone()
    } else {
        let versions: BTreeSet<_> = requests
            .iter()
            .map(|request| {
                exact(request)
                    .cloned()
                    .ok_or(Error::Unsupported("floating pnpm alternative"))
            })
            .collect::<Result<_, _>>()?;
        if versions.len() != 1 {
            return Err(Error::Unsupported("ambiguous exact pnpm alternatives"));
        }
        versions
            .into_iter()
            .next()
            .ok_or(Error::Unsupported("missing pnpm pin"))?
    };
    if !declaration.matches(&version) {
        return Err(Error::Unsupported(
            "pnpm pin contradicts native constraints",
        ));
    }
    let version = version.to_string();
    if version.len() > 128 {
        return Err(Error::Unsupported("pnpm identity exceeds lock limit"));
    }
    Ok(version)
}

/// Concrete async resolver for the existing synchronous reconcile callback.
/// Callers can block_on OUTSIDE async; reconcile owns checked publication.
/// Offline resolution has no metadata cache and fails before any traffic.
pub async fn resolve(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    snapshot.ensure_current()?;
    if !snapshot.declarations().contains_key("node") {
        return Err(Error::Unsupported(
            "a captured Node declaration is required",
        ));
    }
    let manager = snapshot.package_manager()?;
    if manager.as_ref().is_some_and(|m| m.manager != Manager::Pnpm) {
        return Err(Error::Unsupported("npm declaration resolution"));
    }
    let pnpm_version = if request.version_ids().contains("pnpm") {
        Some(exact_pnpm(
            manager
                .as_ref()
                .ok_or(Error::Unsupported("missing pnpm declaration"))?,
        )?)
    } else {
        None
    };
    if request.offline() && !request.version_ids().is_empty() {
        return Err(Error::Unsupported(
            "offline resolution metadata/artifacts are not cached",
        ));
    }
    let requirements = snapshot.node_requirements()?;
    let mut tools: BTreeMap<_, _> = snapshot
        .previous_lock()
        .into_iter()
        .flat_map(Lock::tools)
        .filter(|(id, _)| snapshot.declarations().contains_key(*id))
        .map(|(id, tool)| (id.clone(), tool.clone()))
        .collect();
    if request.version_ids().contains("node") {
        let mut selected = node.resolve_native(&requirements, true).await?.into_tool();
        // Use Snapshot's canonical full provenance, not selection precedence.
        selected.declarations = snapshot.declarations()["node"].clone();
        tools.insert("node".into(), selected);
    } else {
        let locked = tools
            .get("node")
            .ok_or(Error::Unsupported("missing locked Node"))?;
        let version = semver::Version::parse(&locked.version)
            .map_err(|_| Error::Unsupported("invalid locked Node"))?;
        if !requirements.matches_locked_version(&version) {
            return Err(Error::Unsupported(
                "locked Node contradicts native constraints",
            ));
        }
        // Ownership-only requests never refresh Node or infer missing npm
        // identity. Preserve even a previously node-only export cohort.
    }
    snapshot.ensure_current()?;
    if let Some(version) = pnpm_version {
        let declaration = manager
            .as_ref()
            .ok_or(Error::Unsupported("missing pnpm declaration"))?;
        let selected = registry
            .resolve_exact(Manager::Pnpm, &version, None)
            .await?;
        let pin = declaration.locked_integrity(selected.version(), &selected.artifact().sha256)?;
        selected.verify_authored(pin.as_ref())?;
        tools.insert(
            "pnpm".into(),
            Tool {
                adapter: "pnpm".into(),
                version,
                declarations: snapshot.declarations()["pnpm"].clone(),
                options: BTreeMap::new(),
                installation: Installation::Managed {
                    artifacts: BTreeMap::from([(
                        Platform::Any,
                        BTreeMap::from([("package".into(), selected.into_artifact())]),
                    )]),
                },
            },
        );
    }
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    let Installation::Managed { artifacts } = &candidate.tools()["node"].installation else {
        return Err(Error::Unsupported(
            "only managed Node artifacts are supported",
        ));
    };
    for platform in artifacts.keys() {
        NodePlan::from_lock(&candidate, *platform)?;
    }
    if let Some(declaration) = &manager {
        let pnpm = candidate
            .tools()
            .get("pnpm")
            .ok_or(Error::Unsupported("missing locked pnpm"))?;
        let version = semver::Version::parse(&pnpm.version)
            .map_err(|_| Error::Unsupported("invalid locked pnpm"))?;
        let Installation::Managed { artifacts } = &pnpm.installation else {
            return Err(Error::Unsupported("only managed pnpm is supported"));
        };
        for artifact in artifacts.values().flat_map(BTreeMap::values) {
            declaration.locked_integrity(&version, &artifact.sha256)?;
        }
    }
    if !candidate.matches_native(snapshot.declarations())? {
        return Err(Error::Unsupported(
            "candidate differs from captured provenance",
        ));
    }
    snapshot.ensure_current()?;
    Ok(candidate)
}

#[cfg(all(test, unix))]
mod tests;
