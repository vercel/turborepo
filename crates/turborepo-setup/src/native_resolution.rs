//! Complete native candidate for the reconcile callback: the JavaScript cohort
//! when declared, plus the root uv selection when the snapshot captured the
//! Python ecosystem. Only `version_ids` resolve; every preserved selection is
//! revalidated without network. No installation, probes or task execution.

use std::collections::BTreeMap;

use crate::{
    js_resolution,
    lock::{self, Document, Lock, reconcile::Resolution},
    node_provision::NodeTransport,
    registry_resolution::RegistryTransport,
    uv_resolution::{self, UvTransport},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Js(#[from] js_resolution::Error),
    #[error(transparent)]
    Uv(#[from] uv_resolution::Error),
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error("unsupported native resolution: {0}")]
    Unsupported(&'static str),
}

pub async fn resolve(
    request: Resolution<'_>,
    node: &NodeTransport,
    registry: &RegistryTransport,
    uv: &UvTransport,
) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    let declared = snapshot.declarations();
    let mut tools = if ["node", "npm", "pnpm"]
        .iter()
        .any(|id| declared.contains_key(*id))
    {
        js_resolution::resolve_tools(&request, node, registry).await?
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
            uv.resolve(&requirement).await?
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
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    if !candidate.matches_native(declared)? {
        return Err(Error::Unsupported(
            "candidate differs from captured provenance",
        ));
    }
    snapshot.ensure_current()?;
    Ok(candidate)
}
