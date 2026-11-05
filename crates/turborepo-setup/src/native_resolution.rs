//! Complete native candidate for the reconcile callback: the JavaScript cohort
//! when declared, plus root uv, CPython and Go selections captured by enabled
//! ecosystems. Only `version_ids` resolve; every preserved selection is
//! revalidated without network. Resolving CPython runs the verified locked uv
//! offline to list downloads; nothing is installed.

use std::collections::BTreeMap;

use crate::{
    bun_provision::BunTransport,
    go_resolution::{self, GoTransport},
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
    Rust(#[from] crate::rust_resolution::Error),
    #[error(transparent)]
    Go(#[from] go_resolution::Error),
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
    pub uv: &'a UvTransport,
    pub python: &'a PythonTransport,
    pub go: &'a GoTransport,
    /// Host target used only to run the locked uv's offline listing.
    pub platform: Platform,
    /// The tool store, so listing Python downloads runs an installed uv
    /// instead of downloading one.
    pub store: Option<&'a turborepo_tool_install::UserStore>,
}

pub async fn resolve(request: Resolution<'_>, sources: Transports<'_>) -> Result<Lock, Error> {
    resolve_all(&request, &sources, None, None).await
}

/// Compose the Rust cohort with JavaScript, uv and CPython resolution while
/// preserving each unaffected selection and the existing offline uv listing.
pub async fn resolve_with_rust(
    request: Resolution<'_>,
    sources: Transports<'_>,
    rustup: &crate::rustup_provision::RustupTransport,
) -> Result<Lock, Error> {
    resolve_all(&request, &sources, Some(rustup), None).await
}

/// Compose authorized Bun downloads with all native cohorts without changing
/// the existing transport bundle or the Rust-only public entry point.
pub async fn resolve_with_rust_and_bun(
    request: Resolution<'_>,
    sources: Transports<'_>,
    rustup: &crate::rustup_provision::RustupTransport,
    bun: &BunTransport,
) -> Result<Lock, Error> {
    resolve_all(&request, &sources, Some(rustup), Some(bun)).await
}

/// Each ecosystem's metadata comes from its own origins and no ecosystem
/// reads another's result, except Python, which lists downloads with the
/// selected uv. So they resolve concurrently, and resolution takes as long as
/// the slowest one instead of all of them in turn.
async fn resolve_all(
    request: &Resolution<'_>,
    sources: &Transports<'_>,
    rustup: Option<&crate::rustup_provision::RustupTransport>,
    bun: Option<&BunTransport>,
) -> Result<Lock, Error> {
    if request.offline() && request.version_ids().contains("go") {
        return Err(Error::Unsupported(
            "offline resolution metadata is not cached",
        ));
    }
    let snapshot = request.snapshot();
    let declared = snapshot.declarations();
    let javascript = async {
        if ["node", "npm", "pnpm", "yarn", "bun"]
            .iter()
            .any(|id| declared.contains_key(*id))
        {
            Ok::<_, Error>(
                js_resolution::resolve_tools_with_bun(request, sources.node, sources.registry, bun)
                    .await?,
            )
        } else {
            Ok(BTreeMap::new())
        }
    };
    let rust = async {
        match rustup {
            Some(rustup) => {
                Ok::<_, Error>(crate::rust_resolution::resolve_request(request, rustup).await?)
            }
            None => Ok(BTreeMap::new()),
        }
    };
    let go = async {
        Ok::<_, Error>(
            go_resolution::select(snapshot, request.version_ids().contains("go"), sources.go)
                .await?,
        )
    };
    let (mut tools, python, rust, go) =
        futures::try_join!(javascript, uv_and_python(request, sources), rust, go)?;
    tools.extend(python);
    tools.extend(rust);
    tools.extend(go.map(|go| ("go".to_owned(), go)));
    candidate(request, tools)
}

/// Root uv, then CPython, which lists its downloads with that uv.
async fn uv_and_python(
    request: &Resolution<'_>,
    sources: &Transports<'_>,
) -> Result<BTreeMap<String, lock::Tool>, Error> {
    let snapshot = request.snapshot();
    let mut tools = BTreeMap::new();
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
            let plan = UvPlan::from_lock(&uv, sources.platform)?;
            let installed = sources
                .store
                .and_then(|store| plan.installed_binary(store))
                .and_then(|uv| python_resolution::list(&uv).ok());
            let listing = match installed {
                Some(listing) => listing,
                None => {
                    // Verified scratch copy, dropped after listing; never
                    // activated. Its archive is retained so provisioning does
                    // not download it again.
                    let prepared = plan.download_retained(sources.uv).await?;
                    python_resolution::list(&prepared.binary())?
                }
            };
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
    Ok(tools)
}

fn candidate(request: &Resolution<'_>, tools: BTreeMap<String, lock::Tool>) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    if !candidate.matches_native(snapshot.declarations())? {
        return Err(Error::Unsupported(
            "candidate differs from captured provenance",
        ));
    }
    snapshot.ensure_current()?;
    Ok(candidate)
}
