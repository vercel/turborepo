//! Per-toolchain resolution and installation.

pub(crate) mod go;
pub(crate) mod node;
pub(crate) mod package_manager;
pub(crate) mod python;
pub(crate) mod rust;

use crate::{Error, InstalledTool, Manifest, declared::Declaration, install::InstallContext, shim};

/// Turns a declaration into the exact version string that will be installed
/// (and recorded in the manifest).
pub(crate) async fn resolve(
    ctx: &InstallContext<'_>,
    declaration: &Declaration,
) -> Result<String, Error> {
    match declaration {
        Declaration::Node { version, source } => node::resolve(ctx, version, source).await,
        Declaration::PackageManager {
            name,
            version,
            source,
            ..
        } => package_manager::resolve(ctx, name, version, source).await,
        Declaration::Rust { channel, .. } => Ok(channel.clone()),
        Declaration::Uv { version, source } => python::resolve_uv(ctx, version, source).await,
        Declaration::Python { request, .. } => Ok(request.clone()),
        Declaration::Go { version, source } => go::resolve(ctx, version, source).await,
    }
}

/// Installs `declaration` at the resolved `version`.
pub(crate) async fn install(
    ctx: &InstallContext<'_>,
    declaration: &Declaration,
    version: &str,
    manifest: &Manifest,
) -> Result<InstalledTool, Error> {
    match declaration {
        Declaration::Node { source, .. } => node::install(ctx, version, source).await,
        Declaration::PackageManager {
            name,
            checksum,
            source,
            ..
        } => package_manager::install(ctx, name, version, checksum.as_ref(), source).await,
        Declaration::Rust {
            channel,
            components,
            targets,
            profile,
            source,
        } => {
            rust::install(
                ctx,
                channel,
                components,
                targets,
                profile.as_deref(),
                source,
            )
            .await
        }
        Declaration::Uv { source, .. } => python::install_uv(ctx, version, source).await,
        Declaration::Python { request, source } => {
            python::install_python(ctx, request, source, manifest).await
        }
        Declaration::Go { source, .. } => go::install(ctx, version, source).await,
    }
}

/// Whether a manifest entry's files are still on disk.
pub(crate) fn is_present(
    ctx: &InstallContext<'_>,
    _declaration: &Declaration,
    installed: &InstalledTool,
) -> bool {
    let bin_dir = ctx.tools.bin_dir();
    ctx.tools.resolve_relative(&installed.path).exists()
        && installed
            .bins
            .iter()
            .all(|name| shim::exists(&bin_dir, name))
}
