//! Captured native Node + exact pnpm/Yarn/Bun → portable cohort. Only
//! reconcile's permitted IDs resolve; unaffected selections (including Node
//! exports) retain their exact bytes. No publication, installation, probes or
//! task execution.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    bun_provision::BunTransport,
    lock::{self, Document, Format, Installation, Lock, Platform, Tool, reconcile::Resolution},
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
    #[error(transparent)]
    Bun(#[from] crate::bun_provision::Error),
    #[error("unsupported native JS resolution: {0}")]
    Unsupported(&'static str),
}

// Without an authoritative pin, only one unambiguous exact identity is in
// scope. Never choose the first dev alternative or query a floating index.
fn exact_manager(declaration: &Declaration) -> Result<String, Error> {
    let (floating, ambiguous) = match declaration.manager {
        Manager::Yarn => ("floating Yarn request", "ambiguous exact Yarn alternatives"),
        Manager::Bun => ("floating Bun request", "ambiguous exact Bun alternatives"),
        _ => ("floating pnpm request", "ambiguous exact pnpm alternatives"),
    };
    let requests = declaration
        .package_manager
        .as_ref()
        .map_or(declaration.dev_engines.as_slice(), std::slice::from_ref);
    let versions: BTreeSet<_> = requests
        .iter()
        .map(|request| {
            request
                .request
                .as_ref()
                .and_then(crate::VersionRequest::exact_version)
                .ok_or(Error::Unsupported(floating))
        })
        .collect::<Result<_, _>>()?;
    if versions.len() != 1 {
        return Err(Error::Unsupported(ambiguous));
    }
    let version = versions
        .first()
        .ok_or(Error::Unsupported("missing manager pin"))?;
    declaration.preflight_integrity(version)?;
    let version = version.to_string();
    if version.len() > 128 {
        return Err(Error::Unsupported("manager identity exceeds lock limit"));
    }
    Ok(version)
}

// Pure portable payload checks, matching the locked registry adapters without
// their host/promotion restrictions. Inspect EVERY variant, not the current
// platform. Modern Yarn bundle pins are checked against bytes when prepared.
fn validate_registry(tool: &Tool, declaration: &Declaration) -> Result<(), Error> {
    let invalid = || Error::Registry(crate::registry_resolution::Error::InvalidLock);
    let (name, version) =
        crate::registry_metadata::validate_selection(declaration.manager, &tool.version)
            .map_err(|_| invalid())?;
    let (id, entry, alias) = match declaration.manager {
        Manager::Yarn => ("yarn", "bin/yarn.js", ("yarnpkg", "bin/yarn.js")),
        _ => ("pnpm", "bin/pnpm.cjs", ("pnpx", "bin/pnpx.cjs")),
    };
    let tarball = crate::registry_metadata::tarball_path(name, &tool.version);
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid());
    };
    if tool.adapter != id || !tool.options.is_empty() {
        return Err(invalid());
    }
    for parts in artifacts.values() {
        let artifact = parts.values().next().ok_or_else(invalid)?;
        let paths = &artifact.executables;
        if parts.len() != 1
            || artifact.url != format!("https://registry.npmjs.org/{tarball}")
            || artifact.format != Format::TarGz
            || artifact.root_prefix.as_deref() != Some("package")
            || artifact.destination.is_some()
            || paths.get(id).map(String::as_str) != Some(entry)
            || paths
                .iter()
                .any(|(name, path)| ![(id, entry), alias].contains(&(name.as_str(), path.as_str())))
        {
            return Err(invalid());
        }
        if crate::registry_metadata::is_modern_yarn(declaration.manager, &version) {
            declaration.preflight_integrity(&version)?;
        } else {
            declaration.locked_integrity(&version, &artifact.sha256)?;
        }
    }
    Ok(())
}

/// Concrete async resolver for the existing synchronous reconcile callback.
/// Callers can block_on OUTSIDE async; reconcile owns checked publication.
/// Offline resolution has no metadata cache and fails before any traffic.
pub async fn resolve(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
    bun: &BunTransport,
) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    snapshot.ensure_current()?;
    if !snapshot.declarations().contains_key("node") {
        return Err(Error::Unsupported(
            "a captured Node declaration is required",
        ));
    }
    let manager = snapshot.package_manager()?;
    let manager_id = match manager.as_ref().map(|m| m.manager) {
        None => None,
        Some(Manager::Pnpm) => Some("pnpm"),
        Some(Manager::Yarn) => Some("yarn"),
        Some(Manager::Bun) => Some("bun"),
        Some(_) => return Err(Error::Unsupported("npm declaration resolution")),
    };
    let manager_version = manager
        .as_ref()
        .filter(|_| manager_id.is_some_and(|id| request.version_ids().contains(id)))
        .map(exact_manager)
        .transpose()?;
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
    if !request.version_ids().contains("node") {
        let previous = snapshot
            .previous_lock()
            .ok_or(Error::Unsupported("missing locked Node"))?;
        let Installation::Managed { artifacts } = &previous.tools()["node"].installation else {
            return Err(Error::Unsupported(
                "only managed Node artifacts are supported",
            ));
        };
        for platform in artifacts.keys() {
            NodePlan::from_lock(previous, *platform)?;
        }
        let version = semver::Version::parse(&previous.tools()["node"].version)
            .map_err(|_| Error::Unsupported("invalid locked Node"))?;
        if !requirements.matches_locked_version(&version) {
            return Err(Error::Unsupported(
                "locked Node contradicts native constraints",
            ));
        }
    }
    if let (Some(id), Some(declaration)) = (manager_id, &manager)
        && !request.version_ids().contains(id)
    {
        if declaration.manager == Manager::Bun {
            crate::bun_provision::validate(&tools[id], declaration)?;
        } else {
            validate_registry(&tools[id], declaration)?;
        }
    }
    // Preserved shapes and native choices are checked before EITHER transport.
    if request.version_ids().contains("node") {
        let mut selected = node.resolve_native(&requirements, true).await?.into_tool();
        selected.declarations = snapshot.declarations()["node"].clone();
        tools.insert("node".into(), selected);
    }
    snapshot.ensure_current()?;
    if let (Some(version), Some(declaration), Some(id)) = (manager_version, &manager, manager_id) {
        let installation = if declaration.manager == Manager::Bun {
            bun.resolve_exact(&version).await?
        } else {
            let selected = registry
                .resolve_exact(declaration.manager, &version, None)
                .await?;
            let pin = declaration.locked_integrity(selected.version(), selected.pinned_sha256())?;
            selected.verify_authored(pin.as_ref())?;
            Installation::Managed {
                artifacts: BTreeMap::from([(
                    Platform::Any,
                    BTreeMap::from([("package".into(), selected.into_artifact())]),
                )]),
            }
        };
        tools.insert(
            id.into(),
            Tool {
                adapter: id.into(),
                version,
                declarations: snapshot.declarations()[id].clone(),
                options: BTreeMap::new(),
                installation,
            },
        );
    }
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    // Preserved payloads passed preflight; new payloads come from validated
    // adapter resolutions, with native byte-specific integrity checked above.
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
