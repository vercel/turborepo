//! Captured native Node + exact npm/pnpm/Yarn/Bun → portable cohort. Only
//! reconcile's permitted IDs resolve; Node ownership changes only within its
//! npm/npx grant. Unaffected release/artifact bytes stay exact. No installation
//! or execution.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    bun_provision::BunTransport,
    lock::{self, Document, Installation, Lock, Tool, reconcile::Resolution},
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
    #[error(transparent)]
    Bun(#[from] crate::bun_provision::Error),
    #[error("unsupported native JS resolution: {0}")]
    Unsupported(&'static str),
}

// Without an authoritative pin, only one unambiguous exact identity is in
// scope. Never choose the first dev alternative or query a floating index.
pub(crate) fn exact_manager(declaration: &Declaration) -> Result<String, Error> {
    let (floating, ambiguous, missing, limit) = match declaration.manager {
        Manager::Npm => (
            "floating npm request",
            "ambiguous exact npm alternatives",
            "missing npm pin",
            "npm identity exceeds lock limit",
        ),
        Manager::Yarn => (
            "floating Yarn request",
            "ambiguous exact Yarn alternatives",
            "missing Yarn pin",
            "Yarn identity exceeds lock limit",
        ),
        Manager::Bun => (
            "floating Bun request",
            "ambiguous exact Bun alternatives",
            "missing Bun pin",
            "Bun identity exceeds lock limit",
        ),
        _ => (
            "floating pnpm request",
            "ambiguous exact pnpm alternatives",
            "missing pnpm pin",
            "pnpm identity exceeds lock limit",
        ),
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
    if declaration.manager != Manager::Bun {
        crate::registry_metadata::validate_selection(declaration.manager, &version)
            .map_err(crate::registry_resolution::Error::from)?;
    }
    Ok(version)
}

// Pure portable payload checks, matching the locked registry adapters without
// host/promotion restrictions. Inspect EVERY variant, not the current platform.
pub(crate) fn validate_registry(tool: &Tool, declaration: &Declaration) -> Result<(), Error> {
    let invalid = || Error::Registry(crate::registry_resolution::Error::InvalidLock);
    let (name, version) =
        crate::registry_metadata::validate_selection(declaration.manager, &tool.version)
            .map_err(crate::registry_resolution::Error::from)?;
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid());
    };
    let id = match declaration.manager {
        Manager::Npm => "npm",
        Manager::Pnpm => "pnpm",
        Manager::Yarn => "yarn",
        _ => return Err(invalid()),
    };
    if tool.adapter != id || !tool.options.is_empty() {
        return Err(invalid());
    }
    for (platform, parts) in artifacts {
        let (package, _) = crate::registry_provision::locked_parts(
            declaration.manager,
            &tool.version,
            *platform,
            parts,
        )
        .map_err(|_| invalid())?;
        if crate::registry_metadata::is_modern_yarn(declaration.manager, &version) {
            declaration.preflight_integrity(&version)?;
        } else {
            declaration.locked_integrity(&version, &package.sha256)?;
        }
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
const JS: [&str; 5] = ["node", "npm", "pnpm", "yarn", "bun"];

/// Concrete async resolver for the existing synchronous reconcile callback.
/// Callers can block_on OUTSIDE async; reconcile owns checked publication.
/// Offline resolution has no metadata cache and fails before any traffic.
/// Returns only the JS cohort; callers compose other ecosystems' tools.
pub async fn resolve(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
) -> Result<Lock, Error> {
    resolve_inner(request, node, registry, None).await
}

/// Resolve the JS cohort with an explicitly authorized Bun transport.
pub async fn resolve_with_bun(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
    bun: &BunTransport,
) -> Result<Lock, Error> {
    resolve_inner(request, node, registry, Some(bun)).await
}

async fn resolve_inner(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
    bun: Option<&BunTransport>,
) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    let tools = resolve_tools_with_bun(&request, node, registry, bun).await?;
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    // Preserved payloads passed preflight; new payloads come from validated
    // adapter resolutions, with native byte-specific integrity checked above.
    let mut js = snapshot.declarations().clone();
    js.retain(|id, _| JS.contains(&id.as_str()));
    if !candidate.matches_native(&js)? {
        return Err(Error::Unsupported(
            "candidate differs from captured provenance",
        ));
    }
    snapshot.ensure_current()?;
    Ok(candidate)
}

/// The JavaScript cohort only (Node and its package manager), for composition
/// with other ecosystems' selections before final lock validation.
pub(crate) async fn resolve_tools_with_bun(
    request: &Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
    bun: Option<&BunTransport>,
) -> Result<BTreeMap<String, Tool>, Error> {
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
        Some(Manager::Npm) => Some("npm"),
        Some(Manager::Pnpm) => Some("pnpm"),
        Some(Manager::Yarn) => Some("yarn"),
        Some(Manager::Bun) => Some("bun"),
        Some(_) => return Err(Error::Unsupported("package manager declaration resolution")),
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
    let mut tools: BTreeMap<_, _> = request
        .previous_selection()
        .into_iter()
        .flat_map(Lock::tools)
        .filter(|(id, _)| snapshot.declarations().contains_key(*id) && JS.contains(&id.as_str()))
        .map(|(id, tool)| (id.clone(), tool.clone()))
        .collect();
    if !request.version_ids().contains("node") {
        let previous = request
            .previous_selection()
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
        } else if declaration.manager == Manager::Bun {
            crate::bun_provision::validate(tool, declaration)?;
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
            let installation = if declaration.manager == Manager::Bun {
                bun.ok_or(Error::Unsupported(
                    "Bun resolution requires its authorized transport",
                ))?
                .resolve_exact(&version)
                .await?
            } else {
                let selected = registry
                    .resolve_exact(declaration.manager, &version, None)
                    .await?;
                let pin =
                    declaration.locked_integrity(selected.version(), selected.pinned_sha256())?;
                selected.verify_authored(pin.as_ref())?;
                Installation::Managed {
                    artifacts: selected.into_artifacts(),
                }
            };
            Tool {
                adapter: id.into(),
                version,
                declarations: snapshot.declarations()[id].clone(),
                options: BTreeMap::new(),
                installation,
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
    snapshot.ensure_current()?;
    Ok(tools)
}

#[cfg(all(test, unix))]
mod tests;
