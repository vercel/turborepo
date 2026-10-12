//! Complete native candidate for the reconcile callback: the JavaScript cohort
//! when declared, plus root uv and CPython selections when the snapshot
//! captured the Python ecosystem. Only `version_ids` resolve; every preserved
//! selection is revalidated without network. Resolving CPython runs the
//! verified locked uv offline to list downloads; nothing is installed.

use std::collections::BTreeMap;

use crate::{
    js_resolution,
    lock::{self, Document, Lock, Platform, reconcile::Resolution},
    node_provision::NodeTransport,
    python_resolution::{self, PythonTransport},
    registry_resolution::RegistryTransport,
    uv_provision::{self, UvPlan},
    uv_resolution::{self, UvTransport},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Js(#[from] js_resolution::Error),
    #[error(transparent)]
    Uv(#[from] uv_resolution::Error),
    #[error(transparent)]
    UvInstall(#[from] uv_provision::Error),
    #[error(transparent)]
    Python(#[from] python_resolution::Error),
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error("unsupported native resolution: {0}")]
    Unsupported(&'static str),
}

pub struct Transports<'a> {
    pub node: &'a NodeTransport,
    pub registry: &'a RegistryTransport,
    pub bun: &'a crate::bun_provision::BunTransport,
    pub uv: &'a UvTransport,
    pub python: &'a PythonTransport,
    /// Host target used only to run the locked uv's offline listing.
    pub platform: Platform,
}

pub async fn resolve(request: Resolution<'_>, sources: Transports<'_>) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    let declared = snapshot.declarations();
    let mut tools = if ["node", "npm", "pnpm", "yarn", "bun"]
        .iter()
        .any(|id| declared.contains_key(*id))
    {
        js_resolution::resolve_tools(&request, sources.node, sources.registry, sources.bun).await?
    } else {
        BTreeMap::new()
    };
    if let Some(requirement) = snapshot.uv_requirement()? {
        let tool = if request.version_ids().contains("uv") {
            if request.offline() {
                return Err(Error::Unsupported(
                    "offline resolution metadata is not cached",
                ));
            }
            sources.uv.resolve(&requirement).await?
        } else {
            let previous = snapshot
                .previous_lock()
                .and_then(|lock| lock.tools().get("uv"))
                .ok_or(Error::Unsupported("missing locked uv"))?;
            uv_resolution::validate(previous, &requirement)?;
            previous.clone()
        };
        tools.insert("uv".into(), tool);
    }
    if let Some(requirement) = snapshot.python_requirement()? {
        let tool = if request.version_ids().contains("python") {
            if request.offline() {
                return Err(Error::Unsupported(
                    "offline resolution metadata is not cached",
                ));
            }
            let uv = tools.get("uv").ok_or(Error::Unsupported(
                "managed Python requires a root uv required-version",
            ))?;
            let uv = Lock::new(Document {
                schema_version: lock::SCHEMA_VERSION,
                tools: BTreeMap::from([("uv".into(), uv.clone())]),
            })?;
            // Verified scratch copy, dropped after listing; never activated.
            // Its archive is retained so provisioning does not download it again.
            let prepared = UvPlan::from_lock(&uv, sources.platform)?
                .download_retained(sources.uv)
                .await?;
            let listing = python_resolution::list(&prepared.binary())?;
            sources.python.resolve(&requirement, &listing).await?
        } else {
            let previous = snapshot
                .previous_lock()
                .and_then(|lock| lock.tools().get("python"))
                .ok_or(Error::Unsupported("missing locked Python"))?;
            python_resolution::validate(previous, &requirement)?;
            previous.clone()
        };
        tools.insert("python".into(), tool);
    }
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    // Only this cohort: Rust and Go compose their own selections, and
    // reconcile checks the complete candidate.
    let mut cohort = declared.clone();
    cohort.retain(|id, _| {
        matches!(
            id.as_str(),
            "node" | "npm" | "pnpm" | "yarn" | "bun" | "uv" | "python"
        )
    });
    if !candidate.matches_native(&cohort)? {
        return Err(Error::Unsupported(
            "candidate differs from captured provenance",
        ));
    }
    snapshot.ensure_current()?;
    Ok(candidate)
}
