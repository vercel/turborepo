use camino::Utf8Path;
use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{cargo::CargoToolchain, package_graph::PackageGraph};
use turborepo_types::{EnvMode, UIMode};

use crate::{
    cli, config::resolve_configuration_from_args, run::builder::load_root_package_json,
    turbo_json::RawTurboJson, Args,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOutput<'a> {
    api_url: &'a str,
    login_url: &'a str,
    team_slug: Option<&'a str>,
    team_id: Option<&'a str>,
    signature: bool,
    preflight: bool,
    timeout: u64,
    upload_timeout: u64,
    enabled: bool,
    ui: UIMode,
    // Absent for a pure Cargo workspace, which has no JavaScript package
    // manager.
    #[serde(skip_serializing_if = "Option::is_none")]
    package_manager: Option<&'static str>,
    daemon: Option<bool>,
    env_mode: EnvMode,
    scm_base: Option<&'a str>,
    scm_head: Option<&'a str>,
    cache_dir: &'a Utf8Path,
    concurrency: Option<&'a str>,
}

pub async fn run(repo_root: AbsoluteSystemPathBuf, args: Args) -> Result<(), cli::Error> {
    let config = resolve_configuration_from_args(&repo_root, &args)?;
    let root_turbo_json_path = config.root_turbo_json_path(&repo_root)?;
    let future_flags = RawTurboJson::read(&repo_root, &root_turbo_json_path, true)?
        .and_then(|raw| raw.future_flags.map(|flags| flags.into_inner()))
        .unwrap_or_default();
    let cargo_enabled = future_flags.experimental_cargo_workspaces;
    let root_package_json = load_root_package_json(&repo_root, cargo_enabled)?;

    let mut builder = PackageGraph::builder_optional(&repo_root, root_package_json)
        .with_allow_no_package_manager(config.allow_no_package_manager());
    if cargo_enabled {
        builder = builder.with_toolchain(CargoToolchain::new(repo_root.clone()));
    }
    let package_graph = builder.build().await?;

    let package_manager = package_graph.package_manager().map(|pm| pm.name());

    println!(
        "{}",
        serde_json::to_string_pretty(&ConfigOutput {
            api_url: config.api_url(),
            login_url: config.login_url(),
            team_slug: config.team_slug(),
            team_id: config.team_id(),
            signature: config.signature(),
            preflight: config.preflight(),
            timeout: config.timeout(),
            upload_timeout: config.upload_timeout(),
            enabled: config.enabled(),
            ui: config.ui(),
            package_manager,
            daemon: config.daemon,
            env_mode: config.env_mode(),
            scm_base: config.scm_base(),
            scm_head: config.scm_head(),
            cache_dir: config.cache_dir(),
            concurrency: config.concurrency.as_deref()
        })?
    );
    Ok(())
}
