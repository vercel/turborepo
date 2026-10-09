//! Captured native Node + exact pnpm → portable cohort. Only reconcile's
//! permitted IDs resolve; unaffected selections (including Node exports) retain
//! their exact bytes. No publication, installation, probes or task execution.

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
    #[error("unsupported native JS resolution: {0}")]
    Unsupported(&'static str),
}

// Without an authoritative pin, only one unambiguous exact identity is in
// scope. Never choose the first dev alternative or query a floating index.
pub(crate) fn exact_pnpm(declaration: &Declaration) -> Result<String, Error> {
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
                .ok_or(Error::Unsupported("floating pnpm request"))
        })
        .collect::<Result<_, _>>()?;
    if versions.len() != 1 {
        return Err(Error::Unsupported("ambiguous exact pnpm alternatives"));
    }
    let version = versions
        .first()
        .ok_or(Error::Unsupported("missing pnpm pin"))?;
    declaration.preflight_integrity(version)?;
    let version = version.to_string();
    if version.len() > 128 {
        return Err(Error::Unsupported("pnpm identity exceeds lock limit"));
    }
    Ok(version)
}

// Pure portable payload checks, matching the locked pnpm adapter without its
// host/promotion restrictions. Inspect EVERY variant, not the current platform.
pub(crate) fn validate_pnpm(tool: &Tool, declaration: &Declaration) -> Result<(), Error> {
    let invalid = || Error::Registry(crate::registry_resolution::Error::InvalidLock);
    let version = semver::Version::parse(&tool.version).map_err(|_| invalid())?;
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid());
    };
    if tool.adapter != "pnpm" || !tool.options.is_empty() {
        return Err(invalid());
    }
    for parts in artifacts.values() {
        let artifact = parts.values().next().ok_or_else(invalid)?;
        let paths = &artifact.executables;
        if parts.len() != 1
            || artifact.url != format!("https://registry.npmjs.org/pnpm/-/pnpm-{version}.tgz")
            || artifact.format != Format::TarGz
            || artifact.root_prefix.as_deref() != Some("package")
            || artifact.destination.is_some()
            || paths.get("pnpm").map(String::as_str) != Some("bin/pnpm.cjs")
            || paths.iter().any(|(name, path)| match name.as_str() {
                "pnpm" => path != "bin/pnpm.cjs",
                "pnpx" => path != "bin/pnpx.cjs",
                _ => true,
            })
        {
            return Err(invalid());
        }
        declaration.locked_integrity(&version, &artifact.sha256)?;
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
    let pnpm_version = manager
        .as_ref()
        .filter(|_| request.version_ids().contains("pnpm"))
        .map(exact_pnpm)
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
        .filter(|(id, _)| snapshot.declarations().contains_key(*id))
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
    if !request.version_ids().contains("pnpm")
        && let Some(declaration) = &manager
    {
        validate_pnpm(&tools["pnpm"], declaration)?;
    }
    // Preserved shapes and native choices are checked before EITHER transport.
    if request.version_ids().contains("node") {
        let mut selected = node.resolve_native(&requirements, true).await?.into_tool();
        selected.declarations = snapshot.declarations()["node"].clone();
        tools.insert("node".into(), selected);
    }
    snapshot.ensure_current()?;
    if let (Some(version), Some(declaration)) = (pnpm_version, &manager) {
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
