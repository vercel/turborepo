//! Captured native Node + exact npm/pnpm → portable cohort. Only reconcile's
//! permitted IDs resolve; Node ownership changes only within its npm/npx grant.
//! Unaffected release/artifact bytes stay exact. No installation or execution.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
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
    Bundled(#[from] crate::bundled_npm::Error),
    #[error("unsupported native JS resolution: {0}")]
    Unsupported(&'static str),
}

// Without an authoritative pin, only one unambiguous exact identity is in
// scope. Never choose the first dev alternative or query a floating index.
pub(crate) fn exact_manager(declaration: &Declaration) -> Result<String, Error> {
    let (floating, ambiguous, missing, limit) = if declaration.manager == Manager::Npm {
        (
            "floating npm request",
            "ambiguous exact npm alternatives",
            "missing npm pin",
            "npm identity exceeds lock limit",
        )
    } else {
        (
            "floating pnpm request",
            "ambiguous exact pnpm alternatives",
            "missing pnpm pin",
            "pnpm identity exceeds lock limit",
        )
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
    let version = versions.first().ok_or(Error::Unsupported(missing))?;
    declaration.preflight_integrity(version)?;
    let version = version.to_string();
    if version.len() > 128 {
        return Err(Error::Unsupported(limit));
    }
    crate::registry_metadata::validate_selection(declaration.manager, &version)
        .map_err(crate::registry_resolution::Error::from)?;
    Ok(version)
}

// Pure portable payload checks, matching the locked registry adapters without
// host/promotion restrictions. Inspect EVERY variant, not the current platform.
pub(crate) fn validate_registry(tool: &Tool, declaration: &Declaration) -> Result<(), Error> {
    let invalid = || Error::Registry(crate::registry_resolution::Error::InvalidLock);
    let (id, version) =
        crate::registry_metadata::validate_selection(declaration.manager, &tool.version)
            .map_err(crate::registry_resolution::Error::from)?;
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid());
    };
    if tool.adapter != id || !tool.options.is_empty() {
        return Err(invalid());
    }
    let mappings = if declaration.manager == Manager::Npm {
        [("npm", "bin/npm-cli.js"), ("npx", "bin/npx-cli.js")]
    } else {
        [("pnpm", "bin/pnpm.cjs"), ("pnpx", "bin/pnpx.cjs")]
    };
    for parts in artifacts.values() {
        let artifact = parts.values().next().ok_or_else(invalid)?;
        let paths = &artifact.executables;
        if parts.len() != 1
            || artifact.url != format!("https://registry.npmjs.org/{id}/-/{id}-{version}.tgz")
            || artifact.format != Format::TarGz
            || artifact.root_prefix.as_deref() != Some("package")
            || artifact.destination.is_some()
            || paths.get(id).map(String::as_str) != Some(mappings[0].1)
            || (declaration.manager == Manager::Npm && paths.len() != 2)
            || paths
                .iter()
                .any(|(name, path)| !mappings.iter().any(|(n, p)| name == n && path == p))
        {
            return Err(invalid());
        }
        declaration.locked_integrity(&version, &artifact.sha256)?;
    }
    Ok(())
}

fn remove_node_npm_exports(node: &mut Tool) {
    node.options.remove("bundled-npm");
    if let Installation::Managed { artifacts } = &mut node.installation {
        for artifact in artifacts.values_mut().flat_map(BTreeMap::values_mut) {
            artifact.executables.remove("npm");
            artifact.executables.remove("npx");
        }
    }
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
    let manager_id = manager.as_ref().map(|m| match m.manager {
        Manager::Npm => "npm",
        _ => "pnpm", // Native discovery admits only npm and pnpm.
    });
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
        let tool = &tools[id];
        if matches!(tool.installation, Installation::Bundled { .. }) {
            let version = semver::Version::parse(&tool.version)
                .map_err(|_| Error::Unsupported("invalid bundled npm"))?;
            if declaration.preflight_integrity(&version)?.is_some() {
                return Err(Error::Unsupported(
                    "bundled npm cannot prove archive integrity",
                ));
            }
        } else {
            validate_registry(tool, declaration)?;
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
        let bundled = if declaration.manager == Manager::Npm {
            match crate::bundled_npm::resolve(snapshot, &tools["node"]) {
                Ok(tool) => Some(tool),
                Err(crate::bundled_npm::Error::NotMatching) => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let tool = if let Some(tool) = bundled {
            tool
        } else {
            let selected = registry
                .resolve_exact(declaration.manager, &version, None)
                .await?;
            let pin =
                declaration.locked_integrity(selected.version(), &selected.artifact().sha256)?;
            selected.verify_authored(pin.as_ref())?;
            Tool {
                adapter: id.into(),
                version,
                declarations: snapshot.declarations()[id].clone(),
                options: BTreeMap::new(),
                installation: Installation::Managed {
                    artifacts: BTreeMap::from([(
                        Platform::Any,
                        BTreeMap::from([("package".into(), selected.into_artifact())]),
                    )]),
                },
            }
        };
        tools.insert(id.into(), tool);
    }
    if tools
        .get("npm")
        .is_some_and(|tool| matches!(tool.installation, Installation::Managed { .. }))
    {
        // Independent npm owns npm/npx, never Node's release/artifact identity.
        if request.version_ids().contains("node") || request.ownership_ids().contains("node") {
            remove_node_npm_exports(
                tools
                    .get_mut("node")
                    .ok_or(Error::Unsupported("missing Node selection"))?,
            );
        }
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
