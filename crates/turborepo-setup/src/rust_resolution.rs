//! Root rust-toolchain request + official channel manifest -> portable `rust`
//! lock tool. Each host's parts are the component tarballs rustup installs for
//! the profile, components and targets, pinned by the manifest's SHA-256. No
//! installation, rustup execution or task execution.

use std::collections::BTreeMap;

use crate::{
    lock::{self, Artifact, Document, Format, Installation, Lock, Tool},
    rust_discovery::{Channel, Toolchain},
    rust_metadata::{self, Manifest},
    rustup,
    rustup_provision::RustupTransport,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Metadata(#[from] rust_metadata::Error),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Transport(#[from] crate::rustup_provision::Error),
    #[error("the official Rust channel manifest does not match the requested channel")]
    Channel,
    #[error("Rust component or target `{0}` is not published for any supported host")]
    Unavailable(String),
}

/// Executables a component installs, relative to the toolchain root.
pub(crate) fn executables(package: &str) -> &'static [&'static str] {
    match package {
        "rustc" => &["rustc", "rustdoc"],
        "cargo" => &["cargo"],
        "clippy-preview" => &["cargo-clippy", "clippy-driver"],
        "rustfmt-preview" => &["rustfmt", "cargo-fmt"],
        "rust-analyzer-preview" => &["rust-analyzer"],
        _ => &[],
    }
}

/// The exact rustup toolchain this manifest publishes for the request:
/// `1.99.0` for stable releases, `nightly-2026-10-11` for dated channels.
pub fn toolchain_name(channel: &Channel, manifest: &Manifest) -> Result<String, Error> {
    let version = manifest.rust_version().ok_or(Error::Channel)?;
    let (name, tag, date) = match channel {
        Channel::Stable | Channel::Release(_) => {
            rustup::canonical_release(version).ok_or(Error::Channel)?;
            if let Channel::Release(request) = channel
                && version != request
                && (request.split('.').count() != 2 || !version.starts_with(&format!("{request}.")))
            {
                return Err(Error::Channel);
            }
            return Ok(version.into());
        }
        Channel::Beta(date) => ("beta", "-beta", date),
        Channel::Nightly(date) => ("nightly", "-nightly", date),
    };
    if !version.contains(tag) || date.as_ref().is_some_and(|date| date != manifest.date()) {
        return Err(Error::Channel);
    }
    Ok(format!("{name}-{}", manifest.date()))
}

/// Rust selections for one reconcile request: unaffected ones verbatim, and
/// changed or refreshed declarations resolved. A components, targets or
/// profile edit keeps the locked release of an unchanged floating channel;
/// only an explicit refresh moves it. Other ecosystems are the caller's.
pub async fn resolve_request(
    request: &crate::lock::reconcile::Resolution<'_>,
    transport: &RustupTransport,
) -> Result<BTreeMap<String, Tool>, Error> {
    let snapshot = request.snapshot();
    let previous = snapshot.previous_lock().map(Lock::tools);
    let mut tools = BTreeMap::new();
    for id in ["rustup", "rust"] {
        let Some(declarations) = snapshot.declarations().get(id) else {
            continue;
        };
        let locked = previous.and_then(|tools| tools.get(id));
        if !request.version_ids().contains(id) {
            // Reconcile requires these bytes unchanged; absence is its drift.
            tools.extend(locked.map(|tool| (id.to_owned(), tool.clone())));
            continue;
        }
        if request.offline() {
            return Err(Error::Unavailable("offline Rust metadata".into()));
        }
        let tool = if id == "rustup" {
            transport.resolve(declarations.clone()).await?
        } else {
            let mut toolchain = snapshot
                .rust_toolchain()
                .map_err(|_| rust_metadata::Error::Invalid)?
                .ok_or(Error::Channel)?;
            let channel = |values: &[lock::Declaration]| {
                values
                    .iter()
                    .filter(|d| d.field.as_deref().is_none_or(|f| f == "toolchain.channel"))
                    .map(|d| d.request.clone())
                    .collect::<Vec<_>>()
            };
            if let Some(locked) = locked
                && !request.refresh()
                && channel(&locked.declarations) == channel(declarations)
            {
                toolchain.channel = Channel::parse(&locked.version).ok_or(Error::Channel)?;
            }
            resolve_official(transport, &toolchain).await?
        };
        tools.insert(id.to_owned(), tool);
    }
    Ok(tools)
}

/// Fetch the official manifest for the request, then resolve it.
pub async fn resolve_official(
    transport: &RustupTransport,
    toolchain: &Toolchain,
) -> Result<Tool, Error> {
    let path = toolchain.channel.manifest_path();
    let manifest = transport
        .read(&path, rust_metadata::MAX_MANIFEST_BYTES)
        .await?;
    resolve(toolchain, &manifest)
}

pub fn resolve(toolchain: &Toolchain, manifest: &[u8]) -> Result<Tool, Error> {
    let manifest = Manifest::parse(manifest)?;
    let version = toolchain_name(&toolchain.channel, &manifest)?;
    let profile = manifest
        .profile(toolchain.profile.name())
        .ok_or(rust_metadata::Error::Invalid)?;
    let requested: Vec<_> = toolchain
        .components
        .iter()
        .map(|component| manifest.package(component))
        .collect();
    let mut artifacts = BTreeMap::new();
    'hosts: for platform in rustup::PLATFORMS {
        let Some((host, _)) = rustup::target(platform) else {
            continue;
        };
        // Profile members a host lacks (rust-mingw) are skipped, as rustup does;
        // explicit components and targets are required on every recorded host.
        let optional = profile
            .iter()
            .map(String::as_str)
            .filter(|package| manifest.tarball(package, host).is_some());
        let targets = toolchain.targets.iter().filter(|target| *target != host);
        let mut parts = BTreeMap::new();
        for (package, target) in optional
            .chain(requested.iter().copied())
            .map(|package| (package, host))
            .chain(targets.map(|target| ("rust-std", target.as_str())))
        {
            let Some(tarball) = manifest.tarball(package, target) else {
                continue 'hosts;
            };
            let (name, directory): (String, String) = match (package, target == host) {
                ("rust-std" | "rust-analysis", true) => {
                    (package.into(), format!("{package}-{host}"))
                }
                (_, true) => (package.into(), package.into()),
                (_, false) => {
                    let name = format!("{package}-{target}");
                    (name.clone(), name)
                }
            };
            let exe = if rustup::windows(platform) {
                ".exe"
            } else {
                ""
            };
            let artifact = Artifact {
                url: tarball.url.into(),
                sha256: tarball.sha256.into(),
                format: Format::TarGz,
                root_prefix: Some(format!("{}/{directory}", tarball.stem)),
                destination: None,
                executables: executables(package)
                    .iter()
                    .map(|name| ((*name).into(), format!("bin/{name}{exe}")))
                    .collect(),
            };
            parts.insert(name, artifact);
        }
        if parts.contains_key("rustc") {
            artifacts.insert(platform, parts);
        }
    }
    if artifacts.is_empty() {
        let hosts = || rustup::PLATFORMS.into_iter().filter_map(rustup::target);
        let missing = requested
            .iter()
            .map(|package| (*package, None))
            .chain(
                toolchain
                    .targets
                    .iter()
                    .map(|target| ("rust-std", Some(target))),
            )
            .find(|(package, target)| {
                !hosts().any(|(host, _)| {
                    let target = target.map_or(host, String::as_str);
                    manifest.tarball(package, target).is_some()
                })
            });
        return Err(Error::Unavailable(match missing {
            Some((_, Some(target))) => target.clone(),
            Some((package, None)) => package.into(),
            None => "rustc".into(),
        }));
    }
    let mut options = BTreeMap::from([(
        "profile".to_owned(),
        vec![toolchain.profile.name().to_owned()],
    )]);
    for (key, values) in [
        ("components", &toolchain.components),
        ("targets", &toolchain.targets),
    ] {
        if !values.is_empty() {
            options.insert(key.into(), values.iter().cloned().collect());
        }
    }
    let tool = Tool {
        adapter: "rust".into(),
        version,
        declarations: toolchain.declarations().to_vec(),
        options,
        installation: Installation::Managed { artifacts },
    };
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("rust".into(), tool.clone())]),
    })?;
    Ok(tool)
}

#[cfg(test)]
mod tests;
