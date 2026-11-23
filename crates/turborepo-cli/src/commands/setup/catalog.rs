use std::collections::{BTreeMap, BTreeSet};

use turborepo_setup::{lock::Snapshot, source_policy::ToolDownloadPolicy};
use turborepo_setup_catalog::{Catalog, CatalogTransport, Registry};
use turborepo_turbo_json::SetupRegistry;

use super::{
    Error, LockMode, Mode, SetupRequest,
    output::{LeftOut, ResolvedTool},
    provision,
};

#[cfg(any(test, feature = "test-support"))]
pub(super) struct Fixture {
    /// Stands in for the built-in registry.
    pub aqua: Registry<'static>,
    pub transport: CatalogTransport,
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) enum Fixture {}

fn resolves(request: &SetupRequest) -> bool {
    request.lock == LockMode::Write && request.mode != Mode::Check && !request.force
}

pub(super) fn refuse_offline(snapshot: &Snapshot, request: &SetupRequest) -> Result<(), Error> {
    let ids: BTreeSet<_> = snapshot
        .catalog_requests()
        .map(|request| request.id.clone())
        .collect();
    if request.offline && resolves(request) && !ids.is_empty() {
        return Err(Error::CatalogOffline(ids));
    }
    Ok(())
}

/// The registries that the captured `setup.registries` declares, in order.
fn catalog<'a>(snapshot: &'a Snapshot, mut aqua: Option<Registry<'a>>) -> Catalog<'a> {
    let registries = snapshot.registries().iter().map(|registry| match registry {
        SetupRegistry::Aqua => aqua.take().unwrap_or_else(Registry::aqua),
    });
    Catalog::new(registries.collect())
}

pub(super) fn resolve(
    snapshot: Snapshot,
    request: &SetupRequest,
    fixture: Option<Fixture>,
    policy: &ToolDownloadPolicy,
    status: &dyn Fn(Option<&str>),
) -> Result<(Snapshot, Vec<ResolvedTool>), Error> {
    if !resolves(request) || request.offline {
        return Ok((snapshot, Vec::new()));
    }
    let incomplete = request
        .update_lock
        .then(|| snapshot.incomplete_catalog_requests());
    let mut requests: Vec<_> = snapshot
        .catalog_requests()
        .chain(incomplete.into_iter().flatten())
        .cloned()
        .collect();
    if requests.is_empty() {
        return Ok((snapshot, Vec::new()));
    }
    requests.sort_by(|a, b| a.id.cmp(&b.id));
    let host = provision::platform()?;
    let (aqua, fixture) = match fixture {
        #[cfg(any(test, feature = "test-support"))]
        Some(fixture) => (Some(fixture.aqua), Some(fixture.transport)),
        None => (None, None),
    };
    let catalog = catalog(&snapshot, aqua);
    let plans = requests
        .iter()
        .map(|request| catalog.plan(request))
        .collect::<Result<Vec<_>, _>>()?;
    let transport = match fixture {
        Some(transport) => transport,
        None => CatalogTransport::new(policy, &plans)?,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut tools = BTreeMap::new();
    let mut resolved = Vec::new();
    for (request, plan) in requests.iter().zip(&plans) {
        status(Some(&format!(
            "Resolving {} {}",
            request.id, request.version
        )));
        let tool = runtime.block_on(turborepo_setup_catalog::resolve(
            request, plan, host, &transport,
        ))?;
        resolved.push(ResolvedTool {
            id: request.id.clone(),
            version: request.version.clone(),
            source: tool.source,
            release: tool.release,
            left_out: tool
                .skipped
                .iter()
                .map(|(platform, reason)| LeftOut {
                    platform: turborepo_setup_catalog::spelling(*platform),
                    reason: reason.to_string(),
                })
                .collect(),
        });
        tools.insert(request.id.clone(), tool.tool);
    }
    status(None);
    Ok((snapshot.with_catalog_tools(tools)?, resolved))
}
