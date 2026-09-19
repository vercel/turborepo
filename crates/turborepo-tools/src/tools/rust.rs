//! Rust via rustup, with `RUSTUP_HOME` and `CARGO_HOME` scoped to the
//! repository so the pinned channel never touches `~/.rustup`.
//!
//! rustup is the only sane way to install a channel (`stable`,
//! `nightly-2026-07-03`, `1.80.0`) with its components and targets, so
//! `turbo setup` bootstraps a private rustup with `rustup-init` and lets it
//! do the work. The proxies rustup writes to `CARGO_HOME/bin` are linked into
//! `.turbo/tools/bin`; they honor `rust-toolchain.toml` the same way a global
//! rustup does, and `RUSTUP_HOME` is exported to every task so they find the
//! repository-scoped toolchain.

use std::collections::BTreeMap;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{Error, InstalledTool, install::InstallContext};

pub(crate) const TOOL: &str = "rust";
pub(crate) const RUSTUP_HOME_ENV: &str = "RUSTUP_HOME";

/// Directory layout below `.turbo/tools/rust`.
struct RustDirs {
    root: AbsoluteSystemPathBuf,
    rustup_home: AbsoluteSystemPathBuf,
    cargo_home: AbsoluteSystemPathBuf,
}

impl RustDirs {
    fn new(ctx: &InstallContext<'_>) -> Self {
        let root = ctx.tools.root().join_component(TOOL);
        Self {
            rustup_home: root.join_component("rustup"),
            cargo_home: root.join_component("cargo"),
            root,
        }
    }

    fn rustup(&self, ctx: &InstallContext<'_>) -> AbsoluteSystemPathBuf {
        self.cargo_home
            .join_components(&["bin", &ctx.platform.exe("rustup")])
    }

    fn env(&self) -> [(String, String); 2] {
        [
            (RUSTUP_HOME_ENV.to_string(), self.rustup_home.to_string()),
            ("CARGO_HOME".to_string(), self.cargo_home.to_string()),
        ]
    }
}

pub(crate) async fn install(
    ctx: &InstallContext<'_>,
    channel: &str,
    components: &[String],
    targets: &[String],
    profile: Option<&str>,
    source: &str,
) -> Result<InstalledTool, Error> {
    let dirs = RustDirs::new(ctx);
    let rustup = dirs.rustup(ctx);
    let env_owned = dirs.env();
    let env: Vec<(&str, &str)> = env_owned
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    if !rustup.exists() {
        bootstrap_rustup(ctx, &dirs, &env).await?;
    }

    let mut args = vec!["toolchain", "install", channel];
    if let Some(profile) = profile {
        args.extend(["--profile", profile]);
    }
    for component in components {
        args.extend(["--component", component.as_str()]);
    }
    for target in targets {
        args.extend(["--target", target.as_str()]);
    }
    ctx.run(rustup.as_std_path(), &args, &env, ctx.repo_root)
        .await?;
    // Make the channel the default inside this RUSTUP_HOME so the proxies
    // work from any directory, not only below a rust-toolchain file.
    ctx.run(
        rustup.as_std_path(),
        &["default", channel],
        &env,
        ctx.repo_root,
    )
    .await?;

    let bins = link_proxies(ctx, &dirs.cargo_home.join_component("bin"))?;
    let runtime_env: BTreeMap<String, String> = [(
        RUSTUP_HOME_ENV.to_string(),
        ctx.tools.relative_of(&dirs.rustup_home)?,
    )]
    .into();
    ctx.installed_tool(channel, source, &dirs.root, bins, runtime_env)
}

async fn bootstrap_rustup(
    ctx: &InstallContext<'_>,
    dirs: &RustDirs,
    env: &[(&str, &str)],
) -> Result<(), Error> {
    let name = ctx.platform.exe("rustup-init");
    let url = format!(
        "{}/dist/{}/{name}",
        ctx.sources.rustup_update_root,
        ctx.platform.rust_triple()
    );
    let checksum = ctx.sha256_sidecar(&format!("{url}.sha256"), &name).await?;
    let init = dirs.root.join_component(&name);
    ctx.download_file(&url, Some(&checksum), &init).await?;
    #[cfg(unix)]
    init.set_mode(0o755)
        .map_err(|source| Error::io(init.as_str(), source))?;

    ctx.run(
        init.as_std_path(),
        &[
            "-y",
            "--no-modify-path",
            "--default-toolchain",
            "none",
            "--profile",
            "minimal",
        ],
        env,
        ctx.repo_root,
    )
    .await?;
    let _ = init.remove_file();
    Ok(())
}

/// Links every proxy rustup created (`cargo`, `rustc`, `rustup`, `rustfmt`,
/// `cargo-clippy`, …) into the shared bin directory.
fn link_proxies(
    ctx: &InstallContext<'_>,
    proxy_dir: &AbsoluteSystemPath,
) -> Result<Vec<String>, Error> {
    let entries = std::fs::read_dir(proxy_dir.as_std_path())
        .map_err(|source| Error::io(proxy_dir.as_str(), source))?;
    let mut bins = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::io(proxy_dir.as_str(), source))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let name = file_name.strip_suffix(".exe").unwrap_or(file_name);
        let target = proxy_dir.join_component(file_name);
        bins.push((name.to_string(), target));
    }
    bins.sort();
    ctx.link_bins(&bins)
}
