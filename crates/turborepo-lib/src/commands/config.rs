use camino::Utf8Path;
use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{package_graph::PackageGraph, package_json::PackageJson};

use crate::{cli, cli::EnvMode, commands::CommandBase, turbo_json::UIMode, Args};

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
    package_manager: &'static str,
    daemon: Option<bool>,
    env_mode: EnvMode,
    scm_base: Option<&'a str>,
    scm_head: Option<&'a str>,
    cache_dir: &'a Utf8Path,
    concurrency: Option<&'a str>,
}

pub async fn run(repo_root: AbsoluteSystemPathBuf, args: Args) -> Result<(), cli::Error> {
    let config = CommandBase::load_config(&repo_root, &args)?;
    let root_package_json = PackageJson::load(&repo_root.join_component("package.json"))?;

    let package_graph = PackageGraph::builder(&repo_root, root_package_json)
        .build()
        .await?;

    let package_manager = package_graph.package_manager().name();

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
