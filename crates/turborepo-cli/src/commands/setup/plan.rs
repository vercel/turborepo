//! Read-only plans: locked reports, plus proposed first-lock, drift and refresh
//! resolutions. Never a writer, repair, download or execution permission.
use std::{
    io,
    path::{Path, PathBuf},
};

use turborepo_setup::{
    activation::ActivationPlan,
    lock::{Declaration, Installation, Platform, Snapshot, Tool, reconcile},
    source_policy::ToolDownloadPolicy,
};
use turborepo_tool_install::{Export, UserStore};

use super::{
    Error, LockMode, SetupRequest, check, dependencies::Dependencies, provision, root::Discovery,
};

/// A missing lock, local declaration drift or `--update-lock` proposes the
/// candidate setup would publish. `--force` only reinstalls locked selections.
pub(super) fn report(
    discovery: &Discovery,
    sources: Snapshot,
    request: &SetupRequest,
    dependencies: &Dependencies,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let proposed = request.update_lock
        || match sources.previous_lock() {
            None => true,
            Some(lock) => {
                request.lock == LockMode::Write && !lock.matches_native(sources.declarations())?
            }
        };
    if request.force || !proposed {
        let tool_store = transports
            .as_ref()
            .and_then(provision::Transports::tool_store);
        return inspect_with(
            discovery,
            sources,
            request.force,
            dependencies,
            tool_store.as_deref(),
            preflight,
        );
    }
    propose(
        discovery,
        sources,
        request,
        dependencies,
        transports,
        preflight,
    )
}

/// A tools-only locked plan, for direct tests.
#[cfg(all(test, unix))]
pub(super) fn inspect(
    discovery: &Discovery,
    sources: Snapshot,
    force: bool,
    tool_store: &std::path::Path,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let dependencies = Dependencies::tools_only();
    inspect_with(
        discovery,
        sources,
        force,
        &dependencies,
        Some(tool_store),
        preflight,
    )
}

fn inspect_with(
    discovery: &Discovery,
    sources: Snapshot,
    force: bool,
    dependencies: &Dependencies,
    tool_store: Option<&std::path::Path>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let context = check::context()?;
    let platform = context.artifact_platform();
    preflight()?;
    let lock = sources.previous_lock().ok_or(Error::Unsupported(
        "--plan --force reports reinstalls of locked selections and requires turbo.lock",
    ))?;
    // Share frozen provisioning's native request, artifact and integrity gates.
    let inventory = provision::plans(&sources, lock)?.tools();
    provision::storage(discovery.snapshot_root()?.as_std_path())?;
    let root = discovery.snapshot_root()?.as_std_path();
    let plan = ActivationPlan::captured(&sources, context)?;
    let store = super::trust::user_store(tool_store, root)?;
    let recorded = super::trust::TrustStore::new(&store, root)?.recorded_env()?;
    enum Env {
        Missing,
        Mismatched,
        Damaged,
        Healthy,
    }
    let env = match recorded {
        None => Env::Missing,
        Some(env) if &env != plan.selection().key() => Env::Mismatched,
        Some(env) => match store.activate(root, &env) {
            Ok(_) => Env::Healthy,
            Err(
                turborepo_tool_install::Error::NotInstalled(_)
                | turborepo_tool_install::Error::Damaged(_),
            ) => Env::Damaged,
            Err(error) => return Err(error.into()),
        },
    };
    let mut lines = vec!["Using versions in turbo.lock:".to_owned()];
    let tools = lock
        .tools()
        .iter()
        .map(|(id, tool)| {
            let path = match env {
                Env::Healthy | Env::Damaged if !force => installed(&store, &inventory, id, tool)?,
                _ => None,
            };
            Ok((id, tool, path))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let trees_damaged =
        matches!(env, Env::Damaged) && tools.iter().any(|(.., path)| path.is_none());
    let mut toolchains = Vec::new();
    for (id, tool, path) in tools {
        let (action, path) = match (&env, path) {
            _ if force => ("re-verify (--force)", None),
            (Env::Missing, _) => ("install (missing installation)", None),
            (Env::Mismatched, _) => ("repair (installation does not match turbo.lock)", None),
            (Env::Damaged, Some(path)) if trees_damaged => ("reuse", Some(path)),
            (Env::Damaged, _) => ("repair (damaged installation)", None),
            (Env::Healthy, path) => ("reuse", path),
        };
        toolchains.push(describe(id, tool, &platform, action, path.as_deref()));
    }
    heading(&mut lines, "Toolchains", toolchains);
    heading(&mut lines, "Dependencies", [dependencies.plan(root, lock)?]);
    recheck(discovery, &sources, &preflight)?;
    Ok(lines.join("\n"))
}

fn propose(
    discovery: &Discovery,
    sources: Snapshot,
    request: &SetupRequest,
    dependencies: &Dependencies,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let platform = provision::platform()?;
    let root = discovery.snapshot_root()?.as_std_path();
    let tool_store = transports
        .as_ref()
        .and_then(provision::Transports::tool_store);
    provision::resolvable(&sources)?;
    // The same gates as setup's resolution, minus the writer it would take.
    let check = || {
        preflight()?;
        revalidate(discovery)?;
        provision::storage(root)
    };
    check()?;
    let policy = preflight()?;
    let upstream = transports.map(Ok).unwrap_or_else(|| {
        provision::Transports::official(&policy, discovery.policy().mirrors())
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mode = match request.lock {
        LockMode::Frozen => reconcile::Mode::Frozen,
        _ if request.update_lock => reconcile::Mode::Refresh,
        _ => reconcile::Mode::Local,
    };
    let outcome = reconcile::propose(
        &sources,
        mode,
        request.offline,
        |resolution| {
            // Setup's own resolver, so the proposal is the candidate it would write.
            provision::resolve(&runtime, resolution, &upstream)
        },
        |candidate| {
            provision::plans(&sources, candidate)
                .map(drop)
                .map_err(io::Error::other)
        },
        || check().map_err(io::Error::other),
    )?;
    let desired = provision::plans(&sources, &outcome.lock)?.tools();
    // Same per-tool reuse as setup's preparation: a sealed tree in the store.
    let store = super::trust::user_store(tool_store.as_deref(), root)?;
    let tools = outcome
        .lock
        .tools()
        .iter()
        .map(|(id, tool)| Ok((id, tool, installed(&store, &desired, id, tool)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let none_installed = tools.iter().all(|(.., path)| path.is_none());
    let previous = sources.previous_lock();
    let change = match previous {
        None => Some("created"),
        Some(previous) if previous != &outcome.lock => Some("updated"),
        Some(_) => None,
    };
    let mut lines = vec![
        match change {
            Some("created") => "No turbo.lock yet. Setup would create one with these versions:",
            Some(_) => "Setup would update turbo.lock to these versions:",
            None => "turbo.lock is already up to date:",
        }
        .to_owned(),
    ];
    let toolchains = tools.iter().map(|(id, tool, path)| {
        let action = match path {
            Some(_) => "reuse",
            None if none_installed => "install (missing installation)",
            None => "install (not in current installation)",
        };
        describe(id, tool, &platform, action, path.as_deref())
    });
    heading(&mut lines, "Toolchains", toolchains);
    if change.is_some() {
        // Exactly the diff setup prints after writing, minus its commit header.
        let diff = provision::lock_report(previous, &outcome.lock);
        let diff = diff.lines().skip(1).map(str::to_owned).collect();
        heading(&mut lines, "turbo.lock changes", [diff]);
    }
    heading(
        &mut lines,
        "Dependencies",
        [dependencies.plan(root, &outcome.lock)?],
    );
    recheck(discovery, &sources, &preflight)?;
    Ok(lines.join("\n"))
}

fn installed(
    store: &UserStore,
    inventory: &[turborepo_tool_install::Tool],
    id: &str,
    tool: &Tool,
) -> Result<Option<PathBuf>, Error> {
    let owner = match &tool.installation {
        Installation::Bundled { owner } => owner,
        _ => id,
    };
    let Some(owner) = inventory.iter().find(|inventory| inventory.id == owner) else {
        return Ok(None);
    };
    let Some(root) = store.sealed_tree(owner)? else {
        return Ok(None);
    };
    let path = match owner.executables.get(id) {
        Some(Export::File(path)) => path,
        Some(Export::Launch(launch)) if launch.program.tool == owner.id => &launch.program.path,
        _ => return Ok(Some(root)),
    };
    Ok(Some(
        path.split('/').fold(root, |path, part| path.join(part)),
    ))
}

fn heading(lines: &mut Vec<String>, title: &str, blocks: impl IntoIterator<Item = Vec<String>>) {
    let mut blocks = blocks
        .into_iter()
        .filter(|block| !block.is_empty())
        .peekable();
    if blocks.peek().is_none() {
        return;
    }
    lines.push(String::new());
    lines.push(title.to_owned());
    for (i, block) in blocks.enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(block.into_iter().map(|line| format!("  {line}")));
    }
}

/// devEngines splits one request across `…/name` and `…/version`.
fn name_and_version<'a>(
    declaration: &'a Declaration,
    next: Option<&Declaration>,
) -> Option<(&'a str, String)> {
    let parent = declaration.field.as_deref()?.strip_suffix("/name")?;
    let next = next.filter(|next| next.file == declaration.file)?;
    (next.field.as_deref()?.strip_suffix("/version") == Some(parent)).then(|| {
        let name = declaration.request.as_deref().unwrap_or_default();
        let version = next.request.as_deref().unwrap_or_default();
        (parent, format!("{name}@{version}"))
    })
}

pub(super) fn describe(
    id: &str,
    tool: &Tool,
    platform: &Platform,
    action: &str,
    installed: Option<&Path>,
) -> Vec<String> {
    let mut lines = vec![format!("{id} {}: {action}", tool.version)];
    if let Some(path) = installed {
        lines.push(format!("  Installed: {}", path.display()));
    }
    let mut declarations = tool.declarations.iter().peekable();
    while let Some(declaration) = declarations.next() {
        let (field, request) = match name_and_version(declaration, declarations.peek().copied()) {
            Some((parent, request)) => {
                declarations.next();
                (Some(parent), request)
            }
            None => (
                declaration.field.as_deref(),
                declaration.request.clone().unwrap_or_default(),
            ),
        };
        lines.push(format!(
            "  Declaration: {}{} ({request})",
            declaration.file,
            field.map(|field| format!("#{field}")).unwrap_or_default(),
        ));
    }
    if let Installation::Managed { artifacts } = &tool.installation {
        let parts = artifacts
            .get(platform)
            .or_else(|| artifacts.get(&Platform::Any));
        let labeled = parts.is_some_and(|parts| parts.len() > 1);
        for (part, artifact) in parts.into_iter().flatten() {
            let label = if labeled {
                format!(" ({part})")
            } else {
                String::new()
            };
            lines.push(format!("  Source{label}: {}", artifact.url));
            lines.push(format!("  SHA-256{label}: {}", artifact.sha256));
        }
    }
    lines
}

fn revalidate(discovery: &Discovery) -> Result<(), Error> {
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during plan; retry turbo setup",
        ));
    }
    Ok(())
}

/// Recheck original discovery, policy and exact source/lock bytes before
/// printing anything. Never take a writer guard or install anything.
fn recheck(
    discovery: &Discovery,
    sources: &Snapshot,
    preflight: &impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<(), Error> {
    preflight()?;
    revalidate(discovery)?;
    sources.ensure_current()?;
    provision::storage(discovery.snapshot_root()?.as_std_path())
}
